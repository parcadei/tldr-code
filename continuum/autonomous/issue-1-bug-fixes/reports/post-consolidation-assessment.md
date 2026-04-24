# Post-consolidation assessment

## Summary
- **M1 (walker consolidation)**: commit `dc896a7` — `ProjectWalker` built on `ignore::WalkBuilder` with default excludes; 30 call sites migrated; `--no-default-ignore` flag added; `vuln` now honors `--lang`.
- **M2 (detector consolidation)**: commit `c492f49` — single `Language::from_directory` with manifest-priority detection; three duplicate `detect_language` helpers removed.
- **Fixtures**: `/tmp/tldr-real/dub` (dubinc/dub monorepo; 3928 `.ts/.tsx` files; 2877 nested `node_modules`; 2.4 GB; pnpm workspace; clean `main`, HEAD `2324250`).
- **Binary**: `tldr 0.1.6` at `/Users/cosimo/.cargo/bin/tldr` (symlinked to `/Users/cosimo/.local/bin/tldr`).
- **Date**: 2026-04-24.

## Status of each reported bug

### Bug 1 — `resources` path-traversal on Next.js paths

- **Status**: RESOLVED (pre-M1, already fixed in 0.1.6).
- **Command**: `tldr resources apps/web/lib/get-highest-severity.ts getHighestSeverity --lang typescript --format json --quiet`.
- **Observed**: valid JSON, 0 resources, exit 0, 9 ms.
- **Diagnosis**: the earlier path-traversal guard correctly rejected v0.1.2 canonicalized paths on macOS; current binary accepts them.

### Bug 2 — `change-impact .` returns empty JSON success

- **Status**: STILL BROKEN (unchanged by M1/M2).
- **Commands + observed output**:
  ```
  $ tldr change-impact . --lang typescript --format json --quiet
  { "changed_files": [], ..., "detection_method": "git:HEAD",
    "metadata": { "language": "typescript", "call_graph_nodes": 0, ... } }
  ```
  (Tree is clean; HEAD had nothing vs. its own predecessor.)
  Positive control with `--files apps/web/lib/get-highest-severity.ts`: finds 1 changed file and 1 affected function; `detection_method: "explicit"`; 3.4 s.
- **Root cause (✓ VERIFIED)**:
  - `crates/tldr-cli/src/commands/change_impact.rs:94-107` — `determine_detection_method` defaults to `DetectionMethod::GitHead` when neither `--files`, `--base`, `--staged`, nor `--uncommitted` is supplied.
  - `crates/tldr-core/src/analysis/change_impact.rs:188-193` — `GitHead` with no changes falls through: `Ok(_) => (vec![], method.clone())`.
  - Same file lines 231-244 — empty `changed_files` returns a cheerful empty report with `detection_method: "git:HEAD"`, zero call-graph nodes/edges, exit 0.
  - Sessions `Err(_) => (vec![], DetectionMethod::Session)` at lines 192, 204, 210, 214 collapse "git not available" and "git staged/uncommitted error" into the same empty shape.
- **What users see**: running against any repo without uncommitted changes looks identical to "this tool is broken".
- **Minimal fix**: change the output so `detection_method: "git:HEAD"` with zero changed files emits a stderr diagnostic ("no changes relative to HEAD — try `--base=origin/main` or `--files <path>`") and/or exits non-zero when *no* files matched *and* session mode wasn't explicitly requested. A zero-work run against a clean tree should be explicit, not silent.

### Bug 3 — `vuln .` on TS repo

- **Status**: PARTIALLY RESOLVED by M1; new issues exposed.
- **Before/after on dub root**:
  | Command | v0.1.2 | 0.1.6 |
  |---------|--------|-------|
  | `tldr vuln .` (no `--lang`) | `files_scanned: 2` | `files_scanned: 0` |
  | `tldr vuln . --lang typescript` | (not supported) | `files_scanned: 1000` |
- **Diagnosis (✓ VERIFIED)**:
  - `crates/tldr-cli/src/commands/remaining/vuln.rs:494` — directory walk now uses `ProjectWalker` (M1 fix). No more infinite descent into `node_modules`.
  - Line 499-500: hard cap `MAX_DIRECTORY_FILES = 1000`. dub has 3928 TS files, so scans are silently truncated.
  - Line 523-551 (`is_supported_source_file`): with `--lang typescript`, extensions match correctly. Without `--lang`, line 549 falls back to `"py" | "rs"` only — which is why `tldr vuln .` on a TS repo now shows 0 files.
- **Verdict**: M1 stopped the infinite walk and the M1 `--lang` gating for vuln is working. What's left is (a) the file cap (silently truncates), (b) the no-lang fallback (default behavior scans nothing on TS/JS/Java/... repos), (c) zero findings on 1000 real TS files — likely because `scan_file_vulns` patterns in `crates/tldr-core/src/security/vuln.rs` are Python/Rust-biased (the untested half), but this was not traced in depth this session.

### Bug 4 — `smells .` hangs on TS repo

- **Status**: PARTIALLY RESOLVED by M1; the analyzer itself has an exponential bug on `.tsx`.
- **Per-scope behavior**:
  | Scope | Result |
  |-------|--------|
  | `tldr smells .` | hits 90 s timeout (killed) |
  | `tldr smells apps/web` | hits 60 s timeout |
  | `tldr smells packages` | completes in 1.2 s; 142 smells |
  | `tldr smells apps/web/lib/get-highest-severity.ts` (pure TS) | 13 ms |
  | `tldr smells "apps/web/app/(ee)/partners.dub.co/(apply)/[programSlug]/(default)/apply/success/screenshot.tsx"` (1584-line `.tsx`) | >30 s timeout |
- **Single-file repro bypasses the walker**, so the hang is in the analyzer, not file enumeration.
- **Bisection by `--smell-type`** on `screenshot.tsx`:
  | Smell type | Time |
  |------------|------|
  | `long-method` | 23 ms |
  | `god-class` | 23 ms |
  | `long-parameter-list` | 24 ms |
  | `deep-nesting` | 28 ms |
  | `data-class` | 28 ms |
  | `primitive-obsession` | 30 ms |
  | `lazy-element` | 26 ms |
  | **`message-chain`** | **>15 s timeout** |
- **Exponential growth on `message-chain`** (truncated file prefix):
  | lines | time (ms) |
  |-------|-----------|
  | 300 | 21 |
  | 390 | 228 |
  | 395 | 721 |
  | 400 | 2710 |
  | 405 | 10652 |
  | ≥420 | timeout |
  Each 5 extra lines triples the runtime — classic exponential.
- **Root cause (✓ VERIFIED, two issues compound)**:
  1. `crates/tldr-core/src/ast/parser.rs:40-42` — `ParserPool::get_ts_language` maps both `TypeScript` and `JavaScript` to `tree_sitter_typescript::LANGUAGE_TYPESCRIPT`. That grammar does *not* understand JSX. For `.tsx` files, the parser enters error-recovery and produces a pathological tree with many ERROR/synthesis nodes.
  2. `crates/tldr-core/src/quality/smells.rs:1436-1486` — `find_message_chains` walks every tree-sitter node; at each chain-like node (`member_expression`, `call_expression`, `attribute`, etc.) it calls `measure_chain_length` (lines 1489-1526), which also recurses through `object`/`value`/`function` fields. Individually each pass is linear, but when combined with the broken error-recovery tree produced by (1), the recursion blows up: lines 380-410 of `screenshot.tsx` contain nested JSX with template-literal expressions like `` fill={`url(#${id}-r)`} ``, which the TS (non-TSX) grammar can't match, producing deeply nested ERROR nodes that both traversals re-descend into.
  - Note: `crates/tldr-core/src/callgraph/languages/mod.rs:543` and `crates/tldr-core/src/callgraph/types.rs:644` correctly use `LANGUAGE_TSX` for call graph building, so the mismatch is isolated to the `ParserPool` used by quality analyzers.
- **Corroborating evidence**: `tldr cognitive screenshot.tsx` (18 ms), `tldr structure screenshot.tsx` (18 ms), and `tldr clones screenshot.tsx` (25 ms) all finish instantly, so the AST *is* being produced; only the message-chain detector's traversal pattern is pathological against the malformed tree.
- **Minimum-viable fix**: in `crates/tldr-core/src/ast/parser.rs:40-42`, select between `LANGUAGE_TYPESCRIPT` and `LANGUAGE_TSX` by file extension (or pass a TSX hint through the ParserPool API). Alternative defensive fix: add an iteration guard to `find_message_chains` (max nodes visited or max depth).

### Bug 5 — `secure .` findings inside `node_modules/`

- **Status**: RESOLVED by M1.
- **Observed**: `tldr secure . --lang typescript` (40 s, exit 0) produces real taint findings under `apps/web/app/(ee)/api/cron/streams/update-workspace-clicks/route.ts:110`, etc. Zero findings path is under `node_modules/` (grep returned no hits).
- **Diagnosis**: the `secure` dashboard aggregates `vuln`/`smells`/other tier-2 commands — M1 swapped `WalkDir` for `ProjectWalker`, which honors the default excludes.

### Bug 6 — `impact`/`whatbreaks` report 0 callers on real functions

- **Status**: STILL BROKEN.
- **Observed at dub root** (`tldr impact getHighestSeverity . --lang typescript`):
  ```json
  { "targets": { "./apps/web/lib/get-highest-severity.ts:getHighestSeverity":
      { "caller_count": 0, "callers": [],
        "note": "Function found via AST but has no call edges in analyzed scope" } },
    "total_targets": 1 }
  ```
- **Grep proves real callers exist**:
  ```
  apps/web/lib/api/fraud/get-partner-application-risks.ts:1:
      import { getHighestSeverity } from "@/lib/get-highest-severity";
  apps/web/lib/api/fraud/get-partner-application-risks.ts:50:
      const riskSeverity = getHighestSeverity(triggeredRules);
  ```
- **Running from `apps/web/` (where `tsconfig.json` with `"@/lib/*": ["lib/*"]` lives)**: still reports 0 callers.
- **Deeper probe with `tldr calls apps/web/ --lang typescript --max-items 10000`**:
  - 3802 edges total, 1891 files in the graph.
  - `apps/web/lib/get-highest-severity.ts` is absent from every edge (neither source nor destination).
  - Caller `apps/web/lib/api/fraud/get-partner-application-risks.ts` IS in the graph with 4 outgoing edges — but all four resolve through relative `./rules/*` imports. The four `@/` imports it uses (including `@/lib/get-highest-severity`) produce zero edges.
  - Other `@/lib/api/errors` style imports DO resolve (88 callers of `lib/api/errors.ts`), so `@/` aliasing is not globally broken — just not for this file's import patterns.
- **Root cause — multi-part (✓ VERIFIED)**:
  1. `crates/tldr-core/src/callgraph/builder.rs:29-47` — `build_project_call_graph` has a `workspace_config: Option<&WorkspaceConfig>` parameter at line 32. **All 18 production callers pass `None`** (verified: `tldr-cli/src/commands/impact.rs:83`, `hubs.rs:111`, `dead.rs:123`, `daemon.rs:366/730/750/780/834`, `bugbot/first_run.rs:347`, `archived/arch.rs:132/340`, `tldr-mcp/src/tools/callgraph.rs:34`, `tldr-core/src/analysis/whatbreaks.rs` and `context/builder.rs` — none construct a `WorkspaceConfig`). The feature exists but is dead code.
  2. `crates/tldr-core/src/callgraph/module_index.rs:1025-1059` — `detect_ts_base_url` and `detect_ts_paths` read only `<root>/tsconfig.json`. They do not scan for sibling tsconfigs in subdirectories. For dub, which has no root `tsconfig.json`, this yields empty `ts_paths`. Even running from `apps/web/`, the module index is keyed to that single root; the index for `lib/get-highest-severity.ts` is built successfully, but some in-edge (the `@/lib/get-highest-severity` import from `get-partner-application-risks.ts`) fails to resolve for reasons internal to the import resolver — could not be traced further in this session.
  3. `crates/tldr-core/src/analysis/impact.rs:126-184` — `impact_analysis_with_ast_fallback`. When call-graph lookup fails (as here), it falls back to AST search (`find_function_in_ast`, line 190), finds the function definition, and returns a `CallerTree` with `caller_count: 0, callers: vec![], note: "Function found via AST but has no call edges in analyzed scope"` (lines 145-164). This note reads reassuring ("it's isolated, don't worry") but here it's a *false report*: the function has callers; we just didn't resolve them.
- **Minimum-viable fix**: when `caller_count: 0` is returned via the AST-fallback path on a function that is `export`-visible AND the scanned module index contains other files, emit a stderr warning like: `"Note: caller_count=0 may be incomplete — @/* path aliases may not be resolving; try running from the tsconfig.json root or pass --workspace-config"`. At minimum, stop calling this case "has no call edges in analyzed scope" unconditionally.
- **Root-cause fix**: workspace-aware module indexing. Scan the root for sibling `tsconfig.json` / `package.json` / `pnpm-workspace.yaml` / Cargo workspace manifests and build a multi-root module index. The plumbing (`WorkspaceConfig`, `resolve_scan_roots` in `scanner.rs:175-228`) is already there and working — but nothing populates it. Either (a) add auto-discovery when `workspace_config` is `None`, or (b) expose `--workspace-root` flags on `impact`, `whatbreaks`, `change-impact`, `dead`, and let users opt in. Either way, the single-root assumption in `detect_ts_paths` (`module_index.rs:1041`) needs to become multi-root.

### Bug 7 — Language auto-detect fails / defaults to Python

- **Status**: RESOLVED by M2.
- **Observed**: `tldr structure .` (no `--lang`) on dub root reports `"language": "typescript"`. `tldr hotspots .` runs without error. `tldr impact getHighestSeverity .` reports `"language": "typescript"` in its metadata.
- **Diagnosis**: M2's manifest-priority detection correctly identifies `pnpm-workspace.yaml` + `package.json` + sub-directory `tsconfig.json` as TypeScript.

## What M1+M2 actually fixed

- Bug 1: was already fixed in 0.1.6, unchanged.
- Bug 3: walker no longer descends into `node_modules`, scan completes in under a second where it previously reported 2 files. Language gating now honored.
- Bug 4: `smells` on `packages/` completes in 1.2 s; the node_modules hang is gone on scopes that don't include heavy JSX files.
- Bug 5: zero findings under `node_modules/`. Real taint findings in app source surface reliably.
- Bug 7: autodetect on a pnpm TS monorepo returns TypeScript.

## What's left

### Bug 2 — `change-impact` silent empty-success
Still broken. Fix is small: distinguish (a) clean tree, (b) git unavailable, (c) explicit session mode in output shape; optionally emit a stderr warning when default `GitHead` returned zero changed files.

### Bug 4 (partial) — `message-chain` exponential blow-up on `.tsx`
Still broken. The walker aspect is fixed, but the analyzer itself pathologizes on any JSX-heavy `.tsx` file. Two fixes stack:
- Use `LANGUAGE_TSX` for `.tsx`/`.jsx` files in `ast/parser.rs:40-42` (structural fix; closes the whole class of problems with ERROR-node pathology).
- Add iteration bounds to `find_message_chains` (defensive).

### Bug 6 — `impact`/`whatbreaks` false 0-callers
Still broken. Two levels:
- Surface-level honesty: stop reporting `"Function found via AST but has no call edges in analyzed scope"` as if it were authoritative. At a minimum add a stderr note about possible alias-resolution miss when the function has `export` in its AST but no graph edges.
- Structural: workspace-aware module index. `WorkspaceConfig` is plumbed but unused. Either auto-discover sibling tsconfigs or add an `--workspace-root` flag that threads through `build_project_call_graph`.

## Recommendation

Ranked by leverage × tractability:

1. **Bug 4 (TSX grammar fix)** — one-line change to `ast/parser.rs:40-42` that will also benefit `patterns/`, `smells/`, any other analyzer going through `ParserPool`. Highest leverage, small code change.
2. **Bug 6 (honest note, cheap diagnostic)** — a conditional stderr warning when the AST-fallback path returns zero callers on an `export`-visible function. No architecture change, big correctness-perception win.
3. **Bug 2 (empty-success disambiguation)** — trivially cheap, medium user-facing win.
4. **Bug 6 (workspace-aware module index)** — largest architectural effort. Do this *after* the quick wins above.
5. **Bug 3 (file cap / no-lang TS default)** — relatively low impact; only matters for repos > 1000 files. Defer.
