# v0.2.1 hotfix — autonomous plan

**Goal**: Ship v0.2.1 closing 4 GitHub issues against v0.2.0 (#5 path traversal, #11 SARIF mislabel, #12 MCP handshake, #19 MCP camelCase). All confirmed present in HEAD f7cccff.

**Starting state**: HEAD f7cccff (v0.2.0). Working tree has only Cargo.lock + 2 prior-orchestrator-artifact files modified (per session handoff, do NOT touch). Untracked `continuum/autonomous/{abstract-interp-float-pi-fix,bump-v0.1.3,c-grammar-struct-emission,definitions-bugs,tier2-reference-classifiers,ts-abstract-methods}/` are personal scratch from prior sessions, do NOT include in commits.

**Constraints (CLAUDE.md)**: No #[allow], no #[ignore] without file:line citation, no _-prefix on used vars, no weakened assertions, no #[cfg(test)] hide-issues. Workers stage ONLY files they intentionally modified — never `git add .` and never `cargo fmt --all`.

## Milestone graph

```
M1 (VAL-001 daemon path traversal)        ─┐
M2 (VAL-002 vuln SARIF mislabel)          ─┼─→ M5 (release prep)
M3 (VAL-003 MCP lifecycle)  ─→ M4 (VAL-004 MCP camelCase) ─┘
```

- M1, M2, M3 are independent and may run in parallel (M1 touches `crates/tldr-daemon/src/handlers/security.rs`; M2 touches `crates/tldr-cli/src/commands/remaining/vuln.rs` + a new fixture; M3 touches `crates/tldr-mcp/src/{protocol.rs,server.rs}`)
- M4 depends on M3 (both touch `crates/tldr-mcp/src/protocol.rs` — M4 must rebase on M3's commit)
- M5 depends on all four

**Dispatch decision**: Run M1 + M2 in parallel (disjoint files). Run M3 sequentially (since M4 will need to rebase on it). Run M4 after M3 commits. Then M5.

## TDD recipe (every milestone)

```
1. Verify file:line cited in assertion text (worker reads the file, confirms or updates line numbers in their report)
2. Write reproduction test asserting CORRECT behavior with specific value checks
3. Run test on current HEAD — MUST fail with the documented symptom
   - If passes RED on HEAD: STOP, report to orchestrator (status_resolution = "false_positive")
4. Apply fix (described in assertion text — front-loaded, no need to discover approach)
5. Re-run test — MUST pass
6. Run full matrix:
     cargo test -p tldr-cli --test exhaustive_matrix --features semantic --release
     cargo test -p tldr-cli --test language_command_matrix --features semantic --release
   Both MUST be 964/964
7. Run cargo clippy --workspace --all-features --tests -- -D warnings — MUST be clean
8. Commit (test + fix + report + contract.json status update) — message: "fix(M{N} VAL-{NNN}): close #{issue} — {description}"
9. Update artifacts:
   - contract.json: assertion status → "passed", status_resolution → paragraph w/ test name + file:line of fix + RED→GREEN commit SHAs
   - reports/m{N}-issue-{X}-{name}.md: full RED + fix + GREEN narrative
   - validation/m{N}-{name}.json: machine-readable gate results
```

## Assertion strength rules (USER'S SPECIFIC ASK — repeat to every worker)

A test passes the gate ONLY if it satisfies ALL of these:

1. **Parses the output** (JSON, text frames, etc.) — not "exit 0" or "is non-empty"
2. **Asserts a SPECIFIC VALUE** that differs before vs after the fix
   - GOOD: `assert_eq!(parsed["ruleId"], "CWE-502"); assert_ne!(parsed["ruleId"], "CWE-89")`
   - BANNED: `assert!(!output.is_empty())`, `assert!(result.is_ok())` (unless paired with value check on Ok)
3. **The same value check on unfixed HEAD must produce a failure showing the wrong value** — not a generic "test failed" or panic-due-to-other-cause

If a test fails RED + passes GREEN but doesn't satisfy (1)+(2)+(3), the milestone DOES NOT close. Re-roll the test.

## Hold/STOP conditions (global)

- Any reproduce test passes RED on HEAD before fix
- Matrix regresses (any cell goes from passing to failing or ignored)
- Clippy warning introduced
- More than 5 files touched per milestone (excl. contract.json + reports/)
- Any uncommitted file outside milestone scope is modified (especially the 109-file pre-existing fmt drift / WIP test files mentioned in the v0.2.0 handoff)
- Worker can't reproduce the bug — means triage cited wrong file:line, re-investigate before fixing

## Release steps (M5 / VAL-005)

| Step | Action |
|---|---|
| R1 | Bump workspace + 4 crate Cargo.toml versions 0.2.0 → 0.2.1 |
| R2 | CHANGELOG.md: add `## [0.2.1]` section dated today, list 4 fixes |
| R3 | Single commit: `chore: prep v0.2.1 release` |
| R4 | Push to main |
| R5 | `git tag v0.2.1 && git push --tags` (cargo-dist auto-builds 4 platform binaries via release.yml) |
| R6 | STOP. Output 4 `cargo publish -p <crate>` commands for user to run manually |
| R7 | Output 4 issue-closing comment drafts (link commit SHA + milestone report + release URL) for user to paste |

## Artifacts produced

- `continuum/autonomous/v0.2.1-hotfix/contract.json` (this run)
- `continuum/autonomous/v0.2.1-hotfix/plan.md` (this file)
- `continuum/autonomous/v0.2.1-hotfix/reports/m{1..5}-*.{json,md}`
- `continuum/autonomous/v0.2.1-hotfix/validation/m{1..5}-*.json`
- `thoughts/shared/handoffs/v0.2.1-shipped/{timestamp}_v0.2.1-released.yaml`

## Out of scope (deferred)

- Issue #5b (Windows TCP unauthenticated listener — design decision required: is multi-user daemon sharing intended?)
- Issue #17 (IPC OOM unbounded read — implementation decision: tokio-util LinesCodec vs hand-roll?)
- All other 9 open issues triaged for v0.2.2 / v0.3.0 (#6, #7, #8, #9, #10, #13, #14, #15, #16, #18)
