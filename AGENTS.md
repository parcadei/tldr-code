# Agent Instructions


<!-- BEGIN TLDR-AGENT-SKILLS hash:a0a54c04 -->
## tldr-code — Code Exploration Rules

This project has **tldr-code** installed. Agents MUST use `tldr` commands and `tldr-*` skills for all codebase exploration — not shell tools.

### Rule: start the daemon once at session start

Run this **once** when the session begins — not before every command:

```bash
tldr daemon status | grep -q '"not_running"' && tldr daemon start
```

Check first; start only if the daemon is not already running. The daemon is **per-project** and persists across commands. You get ~35× faster queries once it is up.

### Rule: do NOT use shell tools for code exploration

**Do not use** `rg`, `grep`, `find`, `cat`, `sed`, `awk`, or `ls -R` to explore the repository when a `tldr` command or `tldr-*` skill can answer the question.

| Goal | Use |
|---|---|
| "Explore the repo structure" | `tldr-orient-codebase` skill |
| "Find where X is handled" | `tldr-locate-code` skill |
| "Understand this function/file" | `tldr-understand-function` skill |
| "Search for code by concept" | `tldr search "concept"` |
| "Map dependencies or coupling" | `tldr-architecture` skill |
| "Find callers or usages" | `tldr-trace-relationships` skill |
| `grep -r "pattern" src/` | `tldr search "pattern"` |
| `find . -name "*.rs" \| xargs grep "X"` | `tldr find X` |
| `rg --files` | `tldr structure .` |

**Why**: tldr is AST-based (not regex), token-efficient (replaces 3–10 file reads with one query), and ~35× faster when the daemon is warm.

### Required routing — intent to skill

When the user's request matches any of these intents, load the corresponding skill first:

| User intent | Load this skill |
|---|---|
| "Explore the repo" / "orient me" / "give me a tour" | `tldr-orient-codebase` |
| "Find where X is" / "locate this feature/symbol/concept" | `tldr-locate-code` |
| "Explain this function/file" / "understand X" | `tldr-understand-function` |
| "Map the architecture" / "show dependencies/coupling" | `tldr-architecture` |
| "Who calls X?" / "show callers/usages/relationships" | `tldr-trace-relationships` |

### Rule: run `tldr embed` before `tldr semantic` — always

`tldr embed` and `tldr semantic` are **two separate commands** with distinct jobs:

- `tldr embed <path>` — builds and caches the vector index (one-time cost, persists to disk)
- `tldr semantic "<query>" <path>` — searches the pre-built index (sub-second when cache exists)

**Before calling `tldr semantic` or `tldr similar` for the first time in a session, check that the embedding cache exists:**

```bash
ls ~/.tldr/embeddings/ 2>/dev/null | head -3 || echo "cold — run tldr embed first"
```

If the cache is cold, build it first:

```bash
tldr embed <path>
```

Wait for it to finish before running `tldr semantic`. On large codebases this takes minutes — running `tldr semantic` without a warm cache forces it to build the index inline, burning excessive CPU and RAM. **Never spawn multiple `tldr semantic` or `tldr embed` processes for the same path concurrently.**

`tldr semantic` and `tldr similar` also go **stale** after refactors, merges, or batch edits. If results seem wrong after large changes, re-run `tldr embed <path>` to refresh the index.

This is separate from `tldr warm` (which refreshes the Salsa structural cache). Run `tldr embed` when the embedding cache is cold or stale; run `tldr warm` when structural queries (`tree`, `calls`, `impact`, etc.) are slow.

### Allowed exceptions

Shell tools are permitted **only** when:

1. Reading `AGENTS.md` itself or other non-code files (markdown docs, configs) explicitly named by the user.
2. Running validation commands — tests, lint, typecheck, build.
3. `tldr` is unavailable or a `tldr` command fails — say so explicitly before falling back.
4. Applying or verifying an edit in a single already-identified file.

If using an exception, keep it narrow. Do not use shell tools for broad exploration.

### Available skills — pick by intent

- `tldr-locate-code` — find any symbol, function, or concept by name or description
- `tldr-understand-function` — deep-dive a specific function or method
- `tldr-orient-codebase` — structural overview of the repo
- `tldr-trace-relationships` — follow call chains and cross-file dependencies
- `tldr-trace-data-flow` — trace how data moves through the system
- `tldr-change-impact` — blast radius of a proposed change
- `tldr-architecture` — high-level architecture and module boundaries
- `tldr-runtime` — start/stop daemon, warm caches, view live stats
- `tldr-fix-and-detect` — find bugs, anti-patterns, and duplicates
- `tldr-audit-security` — security vulnerability review
- `tldr-audit-complexity` — complexity hotspots
- `tldr-audit-smells` — code smell detection
- `tldr-audit-coverage` — coverage gaps
- `tldr-audit-api` — API surface analysis
- `tldr-setup-check` — diagnose tldr installation and verify setup
<!-- END TLDR-AGENT-SKILLS -->
