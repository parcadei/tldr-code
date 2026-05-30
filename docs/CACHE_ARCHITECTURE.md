# Cache Architecture

This document describes how caching currently works in `tldr-code`, with a
focus on daemon warm-up, query memoization, invalidation, and the search
callgraph timeout observed during daemon debugging.

## Cache layers

There are two important cache layers today:

1. **Daemon in-memory query cache**
   - Implemented by `QueryCache` in `crates/tldr-cli/src/commands/daemon/salsa.rs`.
   - Owned by the running daemon in `TLDRDaemon`.
   - Used by daemon-routed commands such as `calls`, `structure`, `tree`,
     `extract`, `imports`, `cfg`, `dfg`, and others.

2. **On-disk warm cache**
   - Written under `.tldr/cache/`.
   - The main callgraph warm artifact is `.tldr/cache/call_graph.json`.
   - Produced by the foreground `warm` path when no daemon is running.
   - Readable by helper APIs in `tldr-core`, but not currently used by the
     default CLI `search` path.

These layers are related, but they are not the same cache.

## Daemon query cache

The daemon query cache is a Salsa-style memoization map:

```text
QueryKey(query_name, args_hash, language) -> CacheEntry
```

`QueryKey` includes:

- `query_name`: command/query name, for example `calls`, `structure`, `tree`,
  `extract`, or `search`.
- `args_hash`: a hash of the arguments relevant to that query.
- `language`: language discriminator to prevent cross-language cache pollution.

`CacheEntry` stores:

- serialized JSON result bytes
- cache revision
- input file hashes used for invalidation
- creation/access timestamps
- estimated byte size

The default limits are:

- `10_000` entries
- `512 MB` estimated cached bytes

When either limit is exceeded, the cache evicts least-recently-used entries.

## Daemon cache insertion flow

Most daemon handlers follow this pattern:

```text
1. Build QueryKey from command name, arguments, and language.
2. Try QueryCache::get(key).
3. On hit, return cached JSON result.
4. On miss, compute the result.
5. Serialize result to JSON.
6. Insert into QueryCache with input dependency hashes.
```

Some file-specific handlers insert file dependencies:

- `extract(file)`
- `imports(file)`
- `cfg(file, function)`
- `dfg(file, function)`
- `slice(file, function, line)`

Those entries are inserted with `input_hashes = [hash_path(file)]`.

Many project-wide handlers currently insert with no input dependencies:

- `calls`
- `structure`
- `tree`
- `search`
- `context`
- `dead`
- `impact`
- `change_impact`

That means file-level invalidation can precisely invalidate file-specific
entries, but not all project-wide entries.

## Warm behavior

`tldr warm` has two different behaviors depending on whether the daemon is
running.

### When daemon is running

`tldr warm .` sends this daemon command:

```text
DaemonCommand::Warm { language: None }
```

The daemon warms its **in-memory query cache** by computing and inserting:

- call graph
- code structure
- file tree
- semantic index when the semantic feature is enabled

This path does not currently write `.tldr/cache/call_graph.json`.

### When daemon is not running

`tldr warm .` runs foreground warming. This path:

1. creates `.tldr/` if needed
2. creates `.tldrignore` if missing
3. detects languages
4. builds a call graph
5. writes `.tldr/cache/call_graph.json`

The disk cache shape is:

```json
{
  "edges": [
    {
      "from_file": "...",
      "from_func": "...",
      "to_file": "...",
      "to_func": "..."
    }
  ],
  "languages": ["rust"],
  "timestamp": 1234567890
}
```

This split means daemon warm and foreground warm do not currently produce the
same artifacts.

## Persistence

On graceful daemon shutdown, `persist_stats()` writes:

- `.tldr/cache/salsa_stats.json`
- `.tldr/cache/query_cache.bin`

`query_cache.bin` uses an atomic write with magic bytes, version, checksum, and
schema validation. Corrupt or stale-schema cache files are discarded safely.

At the time of writing, the daemon creates its active cache with
`QueryCache::with_defaults()`. Startup reload of `query_cache.bin` into the
active daemon cache does not appear to be wired into the main daemon lifecycle.

## Invalidation

File change notifications go through:

```text
tldr daemon notify <file>
```

The daemon then:

1. adds the file to a dirty set
2. computes `hash_path(file)`
3. calls `QueryCache::invalidate_by_input(file_hash)`
4. invalidates the semantic index
5. if the dirty-file threshold is reached, clears the dirty set

The default dirty threshold is `20` files.

Important limitation: only entries inserted with the matching file hash are
invalidated. Project-wide entries inserted with `input_hashes = []` are not
removed by file-level notify.

The threshold-based reindex path is not fully implemented yet. The code clears
the dirty set, but does not spawn a background rebuild.

## Search and callgraph enrichment

The default CLI `search` command currently runs client-side:

```text
SmartSearchArgs::run
  -> tldr_core::enriched_search
  -> try_enrich_with_callgraph
  -> build_project_call_graph(root, language, None, true)
```

That means full `tldr search` with callgraph enrichment does not use the daemon
in-memory warm cache.

There are helper APIs that can read a disk callgraph cache:

- `read_callgraph_cache(cache_path)`
- `enriched_search_with_callgraph_cache(...)`

However, the normal CLI `search` path does not currently pass
`.tldr/cache/call_graph.json` into those helpers.

## Observed search behavior

During daemon debugging, these experiments were observed:

| Scenario | Command | Result |
| --- | --- | --- |
| Warm daemon | `tldr search daemon . -k20 --no-callgraph` | ~0.8s |
| Warm daemon | `tldr search daemon . -k20` | timed out at 90s |
| Restarted daemon, no warm | `tldr search daemon . -k20 --no-callgraph` | ~0.8s |
| Restarted daemon, no warm | `tldr search daemon . -k20` | timed out at 90s |
| Warm daemon, subdir | `tldr search daemon crates/tldr-cli/src -k20 -l rust` | ~49.5s |

The daemon stayed healthy during these timeouts. Daemon cache statistics did not
change during `search`, confirming that this path was not using daemon cache.

Conclusion: the slow path is not cold daemon cache. It is live client-side
callgraph enrichment during `search`.

## Current workaround

Use `--no-callgraph` for fast search:

```bash
tldr search daemon . -k 20 --no-callgraph -f json
```

## Recommended design fixes

1. Make `search` use `.tldr/cache/call_graph.json` when it exists and matches
   the requested root/language.
2. Make daemon-backed `warm` write/update the same disk callgraph cache, not
   only the in-memory daemon cache.
3. Avoid live full-repo callgraph rebuilds in default `search`; make that path
   explicit via an opt-in flag if needed.
4. Add freshness metadata and invalidation rules for disk callgraph caches.
5. Broaden daemon invalidation for project-wide entries, or track per-file
   dependencies for project-wide cache values.

The desired end state is:

```text
tldr warm
  -> builds canonical callgraph once
  -> stores it in daemon memory and/or .tldr/cache/call_graph.json

tldr search ... with callgraph
  -> uses cached callgraph when fresh
  -> avoids rebuilding callgraph live by default
```
