# Plan: consolidate walker + dir lang detector, then reassess #2/#6

## Background
GitHub issue parcadei/tldr-code#1 lists 6 bugs. Root-cause analysis showed they collapse to 2 meta-bugs:
- ~30 raw `walkdir::WalkDir` call sites, none honoring `.gitignore` or skipping vendor dirs. `ignore::WalkBuilder` is already a dep (`crates/tldr-core/Cargo.toml`) but only used in 4 places. Migration was started and never finished.
- 2 competing directory-level language detectors. `Language::from_directory` (types.rs, broken — walks node_modules) is used by 15 commands. `detect_lang_from_directory` (surface/mod.rs, has ignore list, MAX_DEPTH 3) is used by 1 command.

Full inventory captured in the conversation. Damage radius: 5-6× the reported bugs (~16 additional commands silently broken the same way).

## Strategy
Consolidate first. Stop. Reassess the two structural bugs (#2, #6) against the cleaned codebase.

## Milestones

### M1 — walker-consolidation (VAL-001)
Single worker. Creates `crates/tldr-core/src/walker.rs` wrapping `ignore::WalkBuilder` with project defaults. Migrates ~30 call sites across tldr-cli and tldr-core. Adds `--no-default-ignore` opt-out to the top 4 user-facing commands.

Expected side-effects: fixes #3, #4, #5 and ~16 additional unreported commands.

Validation gate:
- `cargo build --release` green
- `cargo test --workspace` green
- `cargo clippy --workspace -- -D warnings` green
- Manual dub fixture: smells completes <60s, secure/vuln have 0 node_modules findings, vuln with `--lang typescript` skips .py files
- Grep check: no `WalkDir::new` remains in the migrated files outside the new walker module

### M2 — detector-consolidation (VAL-002)
Depends on M1 (uses M1's walker). Single worker. Rewrites `Language::from_directory` to check manifest files first, then extension-majority via the new walker. Deletes `surface::detect_lang_from_directory` + `fn detect_lang_from_directory_recursive`. Deletes the 3 copy-pasted `fn detect_language` in `quality/{dead_code,health,martin}.rs`. Updates the majority-vote test.

Expected side-effects: fixes #7.

Validation gate:
- `cargo test --workspace` green
- `cargo clippy --workspace -- -D warnings` green
- Manual dub fixture: `tldr structure /tmp/tldr-real/dub` reports `language: typescript`
- Grep check: exactly one directory-level detector

### M3 — reassess-structural-bugs (VAL-003)
Depends on M1+M2. Single research worker. Re-runs all 6 repros on `/tmp/tldr-real/dub`, writes `reports/post-consolidation-assessment.md`. Do NOT fix #2 or #6 — just diagnose cleanly.

Expected outcome: #3/#4/#5/#7 pass; #2 and #6 remain with cleaner root-cause diagnoses.

## Risks

1. **Scope creep in M1.** Migrating 30 call sites mechanical but easy to miss subtle differences (depth caps, filter_entry closures, follow_links). Worker must preserve per-call-site semantics (e.g., vuln's `max_depth(10)` cap should translate to the new walker's `max_depth(10)`).

2. **M2 test update.** `test_language_from_directory_detects_majority` at `types.rs:1923` currently asserts pure extension majority. New behavior prefers manifests. Worker updates this test + adds new ones; that's not gaming, it's expressing the corrected invariant.

3. **Opt-out flag compatibility.** Adding `--no-default-ignore` to smells/secure/vuln/dead is new CLI surface — worker must update help text but should not affect any existing invocation.

4. **M3 discovering surprise.** Possible that after M1+M2, #6 actually resolves because the language was being detected wrong, which poisoned the call-graph build. Worth checking — cheap win if true.

## Out of scope for this session
- Fixing #2 (change-impact empty-success) — assessed in M3 only
- Fixing #6 (impact 0-callers-with-AST-hit) — assessed in M3 only
- Daemon walker migration (`daemon/warm.rs`) — IS in scope for M1, flagged here because it's the one "probably broken but not reported" command being fixed incidentally
- Any refactor of `api_check::ApiLanguage` or `fix/error_parser::detect_language` (different purposes, not the consolidation target)

## Acceptance
M1 + M2 complete + passing + M3 report written. User reviews M3's diagnosis before deciding whether to tackle #2 and #6.
