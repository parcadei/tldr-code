# Changelog

## v0.2.1 — 2026-04-25

Hotfix release closing 4 GitHub issues filed against v0.2.0. All four were confirmed reproducible on `f7cccff` (v0.2.0 release commit) and fixed at root cause with new in-process integration tests pinning the bug shape. No regressions: full 964/964 matrix (730 exhaustive + 234 language-command) green across all four fix commits; `cargo clippy --workspace --all-features --tests -- -D warnings` clean.

### Fixed

- **#5 (security, Unix-side path traversal)**: `tldr-daemon` IPC handlers (`secrets`, `vuln`) now route every caller-supplied absolute path through `tldr_core::validate_file_path` before any filesystem read, refusing requests for paths outside the active project root with `BAD_REQUEST`. Pre-fix, the handlers accepted any `is_absolute()` path verbatim, which on a daemon already running could be exploited to extract `/Users/<other>/.aws/credentials`-shaped secrets. The Windows TCP unauthenticated listener portion of #5 remains an open design question (multi-user daemon sharing semantics) and is deferred to v0.3.0. ([commit 00ee2dc](https://github.com/parcadei/tldr-code/commit/00ee2dc))
- **#11**: `tldr vuln --format sarif` and `--format json` now correctly label `Deserialization` findings as deserialization (CWE-502) — pre-fix, the wildcard match arm `_ => VulnType::SqlInjection` at `crates/tldr-cli/src/commands/remaining/vuln.rs:645-651` silently mislabeled them as SQL injection (CWE-89). `Ssrf` was affected by the same wildcard and is now correctly mapped to CWE-918. The match is exhaustive — future `tldr_core::security::vuln::VulnType` variants will fail to compile until they are mapped, preventing the same bug pattern from recurring. ([commit 181f929](https://github.com/parcadei/tldr-code/commit/181f929))
- **#12**: `tldr-mcp` now speaks JSON-RPC 2.0 + MCP 2024-11-05 lifecycle correctly. Three sub-bugs fixed in one commit: (a) `JsonRpcRequest.id` is now `Option<Value>` with `#[serde(default)]` so notification frames (no `id`) deserialize cleanly; (b) the dispatcher now suppresses all response emission when `id` is `None`, per JSON-RPC 2.0 §4.1 ("a server MUST NOT reply to a notification"); (c) the canonical method `notifications/initialized` is routed (the legacy bare `initialized` typo was a v0.1.x scaffold mistake — never spec-correct in any MCP draft — and was removed rather than kept as an alias to avoid masking client bugs in the wider ecosystem). ([commit 1620b6d](https://github.com/parcadei/tldr-code/commit/1620b6d))
- **#19** (filed by @etal37): `tldr-mcp`'s `initialize` response now emits `protocolVersion` and `serverInfo` in camelCase per the MCP 2024-11-05 wire spec. Pre-fix, `InitializeResult` serialized snake_case (`protocol_version`, `server_info`) which Claude Code and other spec-compliant clients reject during the lifecycle handshake — the user-facing failure was "Claude Code cannot connect to tldr-mcp". A recursive scan of the day-one handshake responses (`initialize` + `tools/list`) now returns zero snake_case keys outside JSON Schema property declarations under `inputSchema.properties` (which are user-defined argument names extracted by tool handlers, not MCP-defined wire fields). ([commit 2726358](https://github.com/parcadei/tldr-code/commit/2726358))

### Notes

- `cargo install tldr-cli` and `cargo install tldr-cli --features semantic` continue to work as in v0.2.0 — no new install-time requirements.
- The 4 binary targets (aarch64-apple-darwin, x86_64-apple-darwin, x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu) are built automatically by cargo-dist via `.github/workflows/release.yml` on the `v0.2.1` tag.

## v0.2.0 — 2026-04-25

Major hardening release. Closes parcadei/tldr-code#1 + extends per-language coverage to all 18 supported languages across the full command surface.

### Fixes (from issue #1)

- **Walker hardening** (dc896a7): single `ProjectWalker` on `ignore::WalkBuilder` with default excludes for `node_modules`, `target`, `dist`, `build`, `.next`, `vendor`, `.git`, `__pycache__`. Replaces ~30 raw `walkdir` call sites. `tldr smells`/`secure`/`vuln` no longer descend into vendored code.
- **Language detector consolidation** (c492f49): single `Language::from_directory` with manifest-priority detection. TS projects no longer report as Python.
- **TSX parser dispatch** (9697d21): `ParserPool` selects `LANGUAGE_TSX` for `.tsx`/`.jsx` files. Resolves exponential blowup in `tldr smells` on JSX files.
- **`change-impact` honesty** (8a89f60): new `ChangeImpactStatus` enum {Completed, NoChanges, NoBaseline, DetectionFailed}. Empty results no longer return cheerful exit-0 success.
- **`vuln` autodetect + cap removed** (b1ceffa): `tldr vuln` autodetects language; emits clear error when taint backend (Python+Rust only) doesn't support detected language. Removed silent 1000-file cap.
- **Workspace discovery** (94cc6f0): call graph auto-discovers pnpm/npm/Cargo/go.work workspace roots. Multi-root tsconfig path resolution. `impact` and `whatbreaks` no longer return spurious 0-callers in monorepos.

### Coverage

- **18-language manifest detection** (d3a7e9f): added 7 missing languages (C, C++, C#, Scala, Lua, Luau, OCaml) with proper tie-breaking.
- **Cross-file call resolution** (2577737): closed gap for C, C++, Ruby, Kotlin, Swift, PHP, Luau, OCaml. All 18 languages now resolve cross-file calls.
- **Ruby bareword calls** (e3d9916): `helper` (no parens) now recognized as method call per Ruby semantics.
- **Elixir contracts** (4afae82): `def name do ... end` form now parses correctly.
- **`surface` for Luau + OCaml** (c6fe8a1): API surface extraction for the last 2 languages, including OCaml's `.mli` interface boundary.
- **`definition` for all 18 languages** (a868cbe): go-to-definition no longer Python-only.
- **`temporal` for all 18 languages** (cd81e05): method-call sequence mining no longer Python-only.

### Test infrastructure

- **234-cell command×language matrix** (2d8500c, 2577737): 13 representative commands × 18 languages, strong assertions including cross-file edge counts.
- **730-cell exhaustive matrix** (91ea0fb, c6fe8a1, a868cbe, cd81e05, e0c5e97): 38 language-applicable commands × 18 languages + orchestrator sanity.
- **Tightened weak assertions** (2cacc37, 51eb4e7, 0d35f1b, e0d2dfc): every PASS now verifies command output, not just clean exit. Surfaced and fixed 5 latent bugs (OCaml diff double-counting, OCaml `_`-pattern in structure/callgraph, bm25 hidden-root, context.rs intra-file-only, C# dead-code over-rescue).

### Known limitations

- The `semantic` feature is opt-in (`cargo install tldr-cli --features semantic`). Builds reliably on Mac; unverified on other platforms. PRs to make it portable are welcome.
- `tldr specs` is pytest-specific by design; generalizing requires per-framework parsers (Jest, RSpec, JUnit, etc.) — separate scope.
- `tldr coverage`, `tldr fix`, `tldr bugbot` operate on non-fixture inputs (XML/JSON/error-output/multi-stage) so they aren't in the per-language matrix.

### Notes

- `semantic` shipped as default in M9 was reverted to opt-in for v0.2.0 because ONNX Runtime linking is fragile on Linux aarch64 and we don't want broken `cargo install` on any platform.
