# Plan: round 2 — structural fixes for remaining 4 bugs

## Context
After M1 (walker) + M2 (detector) + M3 (reassessment), four bugs remain with root-cause diagnoses. This round ships the root-cause fixes, not patches.

Shared theme: each remaining bug has a correct lower-level primitive that almost nobody opts into. The fix is to make the correct primitive the default.

## Milestones

### M4 — parser-dialect (VAL-004)
**Target**: `crates/tldr-core/src/ast/parser.rs:37-64`, `:120-163`.
Make `ParserPool` dialect-aware (TSX vs TS, JSX vs JS) when a path is available. `parse_file` already has the path — thread it through to grammar selection. Fixes: smells/patterns on `.tsx`, plus any future analyzer going through ParserPool. Scope: ~40 LOC + tests. Most leverage per line of code.

### M5 — change-impact-status (VAL-005)
**Target**: `crates/tldr-core/src/analysis/change_impact.rs:30-47, 188-244`, `crates/tldr-cli/src/commands/change_impact.rs:109-158`.
Add `ChangeImpactStatus` enum to the report. Delete silent `Err → Session` downgrades. CLI maps failure variants to exit code 3. ~60 LOC + tests.

### M6 — vuln-autodetect-and-cap (VAL-006)
**Target**: `crates/tldr-cli/src/commands/remaining/vuln.rs:499, 523-549, command entry`.
Delete `MAX_DIRECTORY_FILES`. Wire `Language::from_directory` into the `lang == None` case. Error-path when detected language isn't in taint engine's supported set. Small, ~20 LOC + tests.

### M7 — workspace-discovery (VAL-007)
**Target**: `crates/tldr-core/src/types.rs:918-923`, `crates/tldr-core/src/callgraph/builder.rs:29-51`, `crates/tldr-core/src/callgraph/module_index.rs:1025-1059`, `crates/tldr-core/src/analysis/impact.rs:126-184`.
Add `WorkspaceConfig::discover(root) -> Option<WorkspaceConfig>` reading pnpm-workspace.yaml / package.json workspaces / Cargo [workspace] / go.work. Auto-populate in `build_project_call_graph` when caller passes None. Multi-root tsconfig path resolution. Refine impact's AST-fallback note.
Largest milestone: ~200 LOC + tests. Enables real monorepo support across impact/whatbreaks/change-impact/dead.

## Sequencing

M4 and M5 are fully independent (different files, different subsystems). M6 depends on M2 (uses `Language::from_directory`) — already passed. M7 is independent of M4-M6.

**Execution order**:
1. **M4 first** — smallest, highest leverage, no deps. Also unblocks better manual testing because `smells` / `patterns` stop timing out.
2. **M5 and M6 in parallel** — different files, low risk of conflict. Can also serialize if parallel is risky.
3. **M7 last** — largest scope, structural change. Its own worker.

Sequential is safer given the previous rate-limit surprise with a big worker. Four workers serially, ~30-60 min each = 2-4 hour total.

## Risks

- **M4's parse cache change**: if keyed by (lang, dialect), callers that reuse parsers across files may see cache misses for a while. Acceptable.
- **M5 breaking change**: adding a required status field COULD break JSON consumers. Using `#[serde(default)]` plus additive semantics mitigates. Exit code change IS breaking for shell callers; documented.
- **M7 scope creep**: workspace discovery is a real feature. Keep the v1 simple: discover, merge path aliases flatly, don't do per-root scoping yet. The v1 must not regress single-root projects.
- **Existing tests**: M4 might break any test that parses `.tsx` expecting TS-grammar behavior. M7 might break tests that run in dirs with workspace markers they didn't know about. Worker must read test output, not blindly update.

## Acceptance
- All 4 milestones pass validation gate (cargo test + clippy + manual dub repro).
- On `/tmp/tldr-real/dub`: smells completes on apps/web, impact reports real callers for getHighestSeverity, change-impact exits 3 on clean tree, vuln autodetects TS and complains helpfully.
