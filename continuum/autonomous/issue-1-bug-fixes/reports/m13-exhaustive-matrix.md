# Exhaustive Command×Language Matrix Audit (VAL-013)

**Date:** 2026-04-25
**Commit base:** post-VAL-012 (Ruby bareword fix, e3d9916)
**Binary tested:** `target/release/tldr` built with `--features semantic`

## Goal

Run every applicable `tldr` subcommand against the canonical 2-file
3-function fixture for each of the 18 supported languages. Each
(command, language) cell either PASSes or has a justified `#[ignore]`
that cites the source location of the support gap.

Three forbidden failure classes (per VAL-013):
1. **HANG** — process not killed by 30s wall-clock timeout.
2. **PANIC** — exit code with `panicked at` / `Stack backtrace:` in stderr.
3. **SILENT_FAIL** — exit 0 with empty/missing output where the canonical
   fixture should produce a result.

## Result

**658 cells PASS, 36 cells IGNORED with citation, 0 FAILED.**

All assertions hold. VAL-010/VAL-011 (234 cells) continue to pass.
Pre-existing `surface::typescript::tests::test_extract_exported_interfaces`
failure in tldr-core is unrelated to this milestone (no surface files
modified).

## Harness

`crates/tldr-cli/tests/exhaustive_matrix.rs` (~2570 lines) — built on top
of `fixtures/mod.rs` shared with VAL-010. Key infrastructure:

- `run_tldr_timed(args, 30s)` — spawns the child in a worker thread and
  uses `mpsc::recv_timeout` to detect HANG. PANIC detection scans stderr
  for `panicked at` / `Stack backtrace:` / negative exit codes.
- `check_baseline(cmd, lang, args)` — the no-panic / no-hang / exit ≤ 99
  guard.
- `check_success(cmd, lang, args)` — additionally requires that exit 0
  produced non-empty stdout (catches SILENT_FAIL).
- Per-command `check_*` functions that do shape verification on JSON
  output (e.g. `check_temporal` requires `metadata.files_analyzed > 0`).
- `embedding_mutex()` — a global `Mutex<()>` serializing `embed`/
  `semantic`/`similar` calls so the on-disk fastembed model cache is
  not raced when tests run with `--test-threads=4`.

## Groups & Commands Covered

### GROUP-DIR (project-level, arg = path)

20 commands × 18 languages: `hubs`, `whatbreaks`, `importers`, `secure`,
`api-check`, `vuln`, `deps`, `change-impact`, `debt`, `health`, `clones`,
`todo`, `invariants`, `verify`, `interface`, `search`, `context`,
`temporal`, `diagnostics`, `inheritance`.

### GROUP-FILE (file-level, arg = single file path)

2 commands × 18 languages: `definition`, `cohesion`.

### GROUP-FILE-SYMBOL (file + function name)

9 commands × 18 languages: `slice`, `chop`, `reaching-defs`, `available`,
`dead-stores`, `resources`, `explain`, `contracts`, `taint`.

### GROUP-PAIR-FILE (two files)

3 commands × 18 languages: `diff`, `dice`, `coupling`.

### Semantic family (model-backed)

3 commands × 18 languages: `embed`, `semantic`, `similar`.

### Surface (package introspection)

1 command × 18 languages: `surface`.

### Excluded — orchestrator only

10 commands × 1 sanity test each (verifies `--help` does not panic and
emits non-empty output): `coverage`, `fix`, `bugbot`, `daemon`, `cache`,
`stats`, `warm`, `doctor`, plus a single fixture-walk verification for
`tree`. (`help` itself is not tested separately — clap-generated.)

### Total

38 multi-language commands × 18 languages = 684 cells. With 9 sanity-only
tests + 1 `_languages_constant_is_eighteen` placeholder = 694 tests.

## Documented Capability Gaps (#[ignore]-d cells)

Each `#[ignore]` cites the source file:line where the support boundary
is documented. Future milestones may close these gaps.

### `definition` (17 cells)

**Citation:** `crates/tldr-cli/src/commands/remaining/definition.rs:317, :377`

```rust
if language != Language::Python {
    return Err(RemainingError::unsupported_language(...));
}
```

`tldr definition` only supports Python. The 17 non-Python cells use:
`#[ignore = "definition supports only Python (definition.rs:317, :377)"]`.

### `temporal` (17 cells)

**Citation:** `crates/tldr-cli/src/commands/patterns/temporal.rs:705`

```rust
if entry_path.extension().is_none_or(|ext| ext != "py") {
    continue;
}
```

`tldr temporal` walks `.py` files only. The 17 non-Python cells use:
`#[ignore = "temporal walks .py files only (temporal.rs:705)"]`.

### `surface` (2 cells)

**Citation:** `crates/tldr-core/src/surface/mod.rs:88-115`

The dispatch `match effective_lang` lists 16 languages; Luau and OCaml
fall through to `UnsupportedLanguage`:

```rust
other => return Err(crate::error::TldrError::UnsupportedLanguage(...))
```

The 2 ignored cells use:
`#[ignore = "luau not supported by surface backend (.../surface/mod.rs:109-114 ...)"]`
`#[ignore = "ocaml not supported by surface backend (.../surface/mod.rs:109-114 ...)"]`.

## Per-fix Section

### FIX-1: Elixir `def name do ... end` not located by `contracts`

**Symptom:** `tldr contracts <file> main` exited 1 with
`function 'main' not found` even though `def main do ... end` was
present in the file. The shorter `def main, do: expr` form worked.

**Root cause:** tree-sitter-elixir does not expose `arguments` as a
named field on the outer `def` call node. The pre-existing extractor at
`contracts.rs:1024-1043` only consulted
`child_by_field_name("arguments")`, which returns `None` for the
do-block syntax. The function-name node (which IS a direct child of the
arguments node, locatable by KIND) was therefore never read.

**Fix:** Added `extract_elixir_def_name(def_call, source)` at
`contracts.rs:1198-1248`. It scans the outer `def` call's direct
children for one of `kind == "arguments"`, then drills into its first
child to handle three surface forms:
1. Bare `identifier` (`def name do ... end`).
2. `call` wrapping it (`def name(p1, p2), do: ...`).
3. `binary_operator` for guards (`def name(x) when ..., do: ...`).

**Commit:** `4afae82` — `fix(contracts): locate Elixir def with do-block body (VAL-013)`.

**Verification:**
- `contracts` matrix cell `test_contracts_on_elixir` now passes.
- `def helper, do: 1` (canonical fixture short form) still passes.
- All other languages unaffected.

## Harness-Side Resolutions (no tldr fixes needed)

Several initial failures were harness over-strictness, not real tldr
bugs. They were resolved by tightening / relaxing the assertions:

### Documented exit codes 0..=99

Initial harness allowed only `{0, 1, 2, 3}`. tldr's documented exit-code
scheme (per `crates/tldr-core/src/error.rs:286-340`) reserves
0-49 for general categories plus 60-69 for diagnostics-specific codes
(60 = "no diagnostic tools available"; 61 = "all tools failed").
Updated `ok_exit` to accept any exit ∈ [0, 99].

### `temporal` exit 2 on no-constraints-found

Per `temporal.rs:951-959`, `tldr temporal` deliberately exits 2 when no
recurring patterns are detected. The harness now uses `check_baseline`
(not `check_success`) for temporal and verifies
`metadata.files_analyzed > 0`.

### `diagnostics` exit 1 on findings, exit 60 on no-tools

`tldr diagnostics` exits 1 when lint/typechecker findings exist (legit)
and exit 60 when no tools are available for the language (also legit —
e.g. no `dotnet build` installed). The harness handles both.

### `vuln` exit 2 on unsupported autodetect

Per `vuln.rs:586-588`, `is_natively_analyzed(lang)` returns true only
for Python and Rust. For other languages, `tldr vuln <dir>` (without
`--lang`) emits a clear stderr error and exits 2. The harness now
considers this a legitimate diagnostic, not a SILENT_FAIL.

### Embedding-cache concurrency

Concurrent runs of `embed`/`semantic`/`similar` race on the on-disk
fastembed model cache, occasionally producing "No such file or directory"
errors. The harness wraps each call in a process-wide
`Mutex<()>` (`embedding_mutex()`) so model fetch is serialized.

### C# uses `Main`, not `main`

Canonical csharp fixture defines `static void Main(string[] args)` per
.NET convention. The harness picks the entry function via
`entry_function(lang)` which returns `"Main"` for csharp and `"main"`
for everything else.

## Test Execution

```text
cargo test -p tldr-cli --test exhaustive_matrix --features semantic --release \
  -- --test-threads=4
```

Aggregate runtime ≈ **20.1 s** for 658 active tests (36 ignored, 0 failed).

## Final Verification

```text
1. cargo test -p tldr-cli --test exhaustive_matrix --features semantic --release
   -> 658 passed; 0 failed; 36 ignored
2. cargo test -p tldr-cli --test language_command_matrix --features semantic --release
   -> 234 passed; 0 failed; 0 ignored      (VAL-010/VAL-011 not regressed)
3. cargo test -p tldr-core --lib --features semantic --release
   -> 4740 passed; 1 failed; 356 ignored   (failure pre-existing, unrelated)
4. cargo clippy --workspace --all-features
   -> Finished, 0 warnings on changed files
```

## Residuals

- **`temporal` Python-only walker** (line 705) — fixing requires
  generalizing the file-extension check to dispatch by detected
  language. Out of scope for VAL-013; documented for a future
  milestone.
- **`definition` Python-only** (lines 317, 377) — requires extending
  the symbol-locator to other languages. Out of scope for VAL-013;
  see `definition.rs` for what's missing per language.
- **`surface` Luau/OCaml** — needs new backend modules in
  `crates/tldr-core/src/surface/` plus dispatch arms.
  Out of scope for VAL-013.
- **Pre-existing `surface::typescript::tests::test_extract_exported_interfaces`
  test failure** in tldr-core — present before VAL-013 (no surface
  files modified by this milestone).

## Files Touched

- `crates/tldr-cli/src/commands/contracts/contracts.rs` (+72/-15) —
  Elixir def name extraction fix.
- `crates/tldr-cli/tests/exhaustive_matrix.rs` (NEW, 2571 lines) —
  exhaustive command×language matrix harness.
- `continuum/autonomous/issue-1-bug-fixes/reports/m13-exhaustive-matrix.md`
  (NEW, this report).
- `continuum/autonomous/issue-1-bug-fixes/reports/m13-exhaustive-matrix.json`
  (NEW, machine-readable summary).

## VAL-017 Addendum (2026-04-25): churn + hotspots

### Summary

VAL-017 closes the last two coverage gaps in the exhaustive matrix:
**`tldr churn`** and **`tldr hotspots`**. Both commands were already
language-universal in source (verified):

- `crates/tldr-core/src/quality/churn.rs` — pure `git log` parsing, no
  language filter at all.
- `crates/tldr-core/src/quality/hotspots.rs:926, :991` — single
  `Language::from_path(...).is_none()` skip, which by VAL-008's unified
  detector covers all 18 supported languages.

The gap was infrastructural: the canonical `build_fixture` writes a
bare directory with no git history, so churn/hotspots invocations on
matrix fixtures saw an empty `git log` and emitted empty reports. The
matrix therefore previously routed both commands into the
orchestrator-sanity-only group (no per-language coverage).

### Fix

Added `build_git_fixture(lang, root)` helper at
`crates/tldr-cli/tests/fixtures/mod.rs`. It calls `build_fixture`,
then `git init`s the directory with a deterministic local-only
identity (no global git config touched), and makes **3 commits**:

  1. `initial` — full canonical fixture.
  2. `touch1` — appends a language-appropriate single-line comment to
     the entry file (`// touch1` / `# touch1` / `-- touch1` / `(* touch1 *)`).
  3. `touch2` — same idea, second comment line.

Three commits is the minimum that satisfies `tldr hotspots`'s default
`min_commits = 3` (`hotspots.rs:387`). Fewer would cause hotspots to
filter the file out and emit an empty `hotspots` array — a
SILENT_FAIL surface this matrix is supposed to catch.

### Tests added (36)

- `test_churn_on_<lang>` × 18 — asserts `ChurnReport.files` non-empty.
- `test_hotspots_on_<lang>` × 18 — asserts `HotspotsReport.hotspots`
  non-empty.

### Source changes to churn / hotspots

**None.** Both commands were already language-universal; VAL-017 is
fixture-infrastructure + tests only.

### New totals

| Stage | Cells (passed/failed/ignored) |
|-------|------------------------------|
| Pre-VAL-017 (M13/M14/M15/M16) | 694 / 0 / 0 |
| **Post-VAL-017** | **730 / 0 / 0** |

`language_command_matrix.rs` (VAL-010/VAL-011) remains 234/0/0.
`tldr-core --lib` remains 4755/0/356 (no regression).

### Files touched (VAL-017)

- `crates/tldr-cli/tests/fixtures/mod.rs` — added `build_git_fixture`,
  `comment_line`, `fixture_entry_relpath`, `run_git`,
  `append_trailing_line` helpers.
- `crates/tldr-cli/tests/exhaustive_matrix.rs` — added
  `make_git_fixture`, `check_churn`, `check_hotspots`, plus 36
  `#[test]` cells (`test_churn_on_<lang>` and `test_hotspots_on_<lang>`).
- `continuum/autonomous/issue-1-bug-fixes/reports/m13-exhaustive-matrix.md`
  — this addendum.
- `continuum/autonomous/issue-1-bug-fixes/reports/m17-churn-hotspots.json`
  (new machine-readable summary).
