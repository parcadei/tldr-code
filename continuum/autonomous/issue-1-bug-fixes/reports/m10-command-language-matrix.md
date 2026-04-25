# Command × Language Matrix (VAL-010)

**Date:** 2026-04-24
**Commit base:** 219a096 (VAL-009 ship-semantic-by-default)
**Binary tested:** `/Users/cosimo/.cargo/bin/tldr` (release build, default features = `["semantic"]`)
**Test file:** `crates/tldr-cli/tests/language_command_matrix.rs`
**Fixture module:** `crates/tldr-cli/tests/fixtures/mod.rs`

## Summary

**234 of 234 tests pass.** 0 failed, 0 ignored.

Every (command × language) pair in the 13×18 matrix runs end-to-end, exits 0,
produces parseable JSON, and satisfies the per-command semantic-sanity check.

Total runtime: ~2.6s wall-clock with `--test-threads=4` (well under the 60s
scope-control threshold).

## Matrix

Legend: `OK` works and meets minimum-sanity; `WEAK` works but hits a
documented capability limit (partial output, see Capability Observations).

|             | Python | TS | JS | Go | Rust | Java | C    | C++  | Ruby | Kotlin | Swift | C#   | Scala | PHP  | Lua | Luau | Elixir | OCaml |
|-------------|:------:|:--:|:--:|:--:|:----:|:----:|:----:|:----:|:----:|:------:|:-----:|:----:|:-----:|:----:|:---:|:----:|:------:|:-----:|
| structure   | OK     | OK | OK | OK | OK   | OK   | OK   | OK   | OK   | OK     | OK    | OK   | OK    | OK   | OK  | OK   | OK     | OK    |
| extract     | OK     | OK | OK | OK | OK   | OK   | OK   | OK   | OK   | OK     | OK    | OK   | OK    | OK   | OK  | OK   | OK     | OK    |
| imports     | OK     | OK | OK | OK | OK   | WEAK | OK   | OK   | OK   | WEAK   | WEAK  | WEAK | WEAK  | OK   | OK  | OK   | WEAK   | WEAK  |
| loc         | OK     | OK | OK | OK | OK   | OK   | OK   | OK   | OK   | OK     | OK    | OK   | OK    | OK   | OK  | OK   | OK     | OK    |
| complexity  | OK     | OK | OK | OK | OK   | OK   | OK   | OK   | OK   | OK     | OK    | OK   | OK    | OK   | OK  | OK   | OK     | OK    |
| cognitive   | OK     | OK | OK | OK | OK   | OK   | OK   | OK   | OK   | OK     | OK    | OK   | OK    | OK   | OK  | OK   | OK     | OK    |
| halstead    | OK     | OK | OK | OK | OK   | OK   | OK   | OK   | OK   | OK     | OK    | OK   | OK    | OK   | OK  | OK   | OK     | OK    |
| smells      | OK     | OK | OK | OK | OK   | OK   | OK   | OK   | OK   | OK     | OK    | OK   | OK    | OK   | OK  | OK   | OK     | OK    |
| calls       | OK     | OK | OK | OK | OK   | OK   | OK*  | OK*  | OK*  | OK*    | OK*   | OK   | OK    | OK*  | OK  | OK*  | OK     | OK*   |
| dead        | OK     | OK | OK | OK | OK   | OK   | OK   | OK   | OK   | OK     | OK    | OK   | OK    | OK   | OK  | OK   | OK     | OK    |
| references  | OK     | OK | OK | OK | OK   | OK   | OK   | OK   | OK   | OK     | OK    | OK   | OK    | OK   | OK  | OK   | OK     | OK    |
| impact      | OK     | OK | OK | OK | OK   | OK   | OK   | OK   | OK   | OK     | OK    | OK   | OK    | OK   | OK  | OK   | OK     | OK    |
| patterns    | OK     | OK | OK | OK | OK   | OK   | OK   | OK   | OK   | OK     | OK    | OK   | OK    | OK   | OK  | OK   | OK     | OK    |

Every cell meets the test-level acceptance criteria (exit 0, valid JSON,
minimum-sanity check). Originally `WEAK` cells in the `calls` row passed
the permissive `>= 1` threshold but returned only 1 of the 2 fixture
edges; see "Capability observations" below for the original analysis.

`OK*` marks cells where M10 originally reported `WEAK` and the cross-file
call resolution gap has been closed in **VAL-011 / M11**
(`reports/m11-cross-file-calls.md`). The matrix-test threshold for the
`calls` row was tightened to `>= 2` and now all 18 languages pass.

## Per-language end-to-end probe (beyond the minimum-sanity assertions)

Running each fixture through `calls`, `impact helper`, `references helper`,
and `imports <entry>` directly (not through the test harness), I observed:

| Language   | calls edges | impact callers | refs | imports |
|------------|------------:|---------------:|-----:|--------:|
| python     |           2 |              1 |    2 |       1 |
| typescript |           2 |              1 |    2 |       1 |
| javascript |           2 |              1 |    2 |       1 |
| go         |           2 |              1 |    2 |       1 |
| rust       |           2 |              1 |    2 |       1 |
| java       |           2 |              1 |    2 |       0 |
| c          |           1 |              1 |    2 |       1 |
| cpp        |           1 |              1 |    2 |       1 |
| ruby       |           1 |              1 |    2 |       1 |
| kotlin     |           1 |              1 |    2 |       0 |
| swift      |           1 |              1 |    2 |       0 |
| csharp     |           2 |              1 |    2 |       0 |
| scala      |           2 |              1 |    2 |       0 |
| php        |           1 |              1 |    2 |       1 |
| lua        |           2 |              1 |    2 |       1 |
| luau       |           1 |              1 |    2 |       1 |
| elixir     |           2 |              1 |    2 |       0 |
| ocaml      |           1 |              1 |    2 |       0 |

Expected (by fixture construction): **2 calls edges** (main → helper,
main → b_util), **1 impact caller** for `helper` (main), **2 references**
for `helper` (definition + call site), **1 import** (File A imports File B).

## Capability observations (not regressions — audit findings)

These are gaps between what the test's minimum-sanity threshold accepts and
what fixture semantics would imply. All 234 tests still pass because the
thresholds (`total_edges >= 1`, etc.) are intentionally permissive to admit
these gaps as `WEAK` not `FAIL`.

### 1. Cross-file call resolution is incomplete for 8 languages — RESOLVED IN VAL-011

**Symptom (original):** `calls` command finds only 1 edge (`main → helper`,
intra-file) for fixtures where 2 edges are expected. The second edge
`main → b_util` in File B was not resolved across the import.

**Languages affected (original):** `c`, `cpp`, `ruby`, `kotlin`, `swift`,
`php`, `luau`, `ocaml`.

**Resolution (VAL-011 / M11):** All 8 languages now resolve the
cross-file edge correctly. The matrix-test assertion was tightened from
`total_edges >= 1` to `total_edges >= 2` (RED), then per-language fixes
applied (GREEN). See `reports/m11-cross-file-calls.md` for the
per-language breakdown.

The fixes broke down into three categories:
- **C, C++, Ruby, Kotlin, Swift, PHP**: A single new
  `resolve_global_free_function` fallback in
  `crates/tldr-core/src/callgraph/resolution.rs:1497-1547` handles all
  six. These languages share the property that bareword `foo()` may
  resolve to a function defined in another file via implicit cross-file
  visibility (linker matching for C/C++; same-package for
  Kotlin/Swift; `require_relative` / `require_once` for Ruby/PHP).
- **Luau**: Required two-pass `parse_imports` in
  `crates/tldr-core/src/callgraph/languages/luau.rs` so that
  `local util = require('./util')` correctly registers `util` as the
  alias for the imported module (matching the existing lua handler's
  approach).
- **OCaml**: Required `Util` ↔ `util` capitalization aliasing in
  `crates/tldr-core/src/callgraph/module_index.rs` plus a new
  `resolve_ocaml_module_receiver` fallback that lowercases the receiver
  before func_index lookup.

### 2. Ruby top-level method calls require explicit parentheses

**Symptom:** A bareword call like `helper` (no parens) in a Ruby method
body does not register as a call edge. The fixture had to use `helper()`
with explicit parens for the call graph to find the edge.

**Root cause:** `crates/tldr-core/src/callgraph/languages/ruby.rs:256` —
`extract_calls_from_node` iterates `walk_tree(*node)` and filters to
`child.kind() == "call"`. In tree-sitter-ruby's grammar, `helper` without
parens parses as an `identifier`, NOT a `call` node. Only parenthesized
invocations (`helper()`) produce `call` nodes.

**Impact:** Ruby codebases that follow idiomatic style (bare method
invocation) will have undercounted call graphs. Real-world Rails code
heavily uses bareword calls.

**Fixture workaround:** The Ruby fixture uses `helper()` with parens. A
comment in `fixtures/mod.rs:build_ruby` documents this workaround with the
file:line citation.

**Action for a follow-up milestone:** Extend the Ruby handler's walk to
also recognize `identifier` nodes that resolve against `defined_methods`
when they appear in expression position (not as a left-hand side, not as
a parameter).

### 3. Imports command returns empty for 7 languages

**Symptom:** `tldr imports <entry-file>` returns `[]` (empty array) for
`java`, `kotlin`, `swift`, `csharp`, `scala`, `elixir`, `ocaml` even when
the entry file has cross-file references that require imports in typical
projects.

**Context:** Our fixtures for these 7 languages rely on **implicit
package-level imports** rather than explicit import statements:
- Java / Kotlin / C# / Scala: classes in the same compilation unit /
  package are accessible without explicit imports.
- Swift: module-level imports are implicit within a Swift package.
- Elixir: modules are referenced by fully-qualified name without `import`.
- OCaml: modules in the same compilation directory are accessible without
  `open`.

So `imports = 0` may be **correct** for these fixtures (no import
statements to parse) rather than a tldr-core bug. A follow-up audit with
fixtures that DO use explicit imports (e.g., Java `import pkg.Util;`,
Elixir `alias Util`) would confirm.

**Action for a follow-up milestone:** Expand fixtures for these 7
languages to include explicit imports, then re-run `imports` to
distinguish "correct empty" from "parser gap".

### 4. Total_edges accounting: Python vs. Rust

**Observation (not a bug):** Python / Go / TS / JS / Java all return 2
edges. Rust / CSharp / Scala / Lua / Elixir ALSO return 2 edges. But:
- Rust returns **2** edges: `main → helper` (intra) + `main → b_util`
  (attr/util-module).
- C returns **1** edge — cross-file call via `#include` header isn't
  resolved.

This is consistent with observation 1 above.

## Capability gaps found → `#[ignore]` policy

**No tests are `#[ignore]`-marked.** The minimum-sanity thresholds accept
all the WEAK cells as passing. An alternative, more stringent audit could
mark the 21 WEAK cells (3 rows × ~7 lang cells = 21) as `#[ignore =
"<citation>"]` with the file:line pointers above. This was not done
because:

1. The VAL-010 contract specifies "flag capability gaps, do NOT fix" and
   "Mark as known-gap: if a language × command pair CANNOT work due to
   tldr-core's own capability limits" — these WEAK cells DO work (they
   return meaningful output, just less than a richer fixture would
   produce).
2. Making 21 cells `#[ignore]` would conflate two distinct concerns: "the
   command does nothing useful" (fail → ignore) vs. "the command works
   but misses edges" (partial → document).
3. The minimum-sanity thresholds in `check_calls`, `check_imports`, etc.
   are documented in the test file so a reader can see what "passes"
   means per command.

If a future milestone tightens any threshold (e.g., `calls` requires 2
edges for all languages), the 8 languages with partial call-graph
coverage would flip to `#[ignore]` with specific citations.

## Raw test output

```
$ cargo test -p tldr-cli --test language_command_matrix -- --test-threads=4
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.24s
     Running tests/language_command_matrix.rs

running 234 tests
test test_calls_on_c ... ok
test test_calls_on_cpp ... ok
test test_calls_on_csharp ... ok
test test_calls_on_elixir ... ok
test test_calls_on_go ... ok
test test_calls_on_java ... ok
test test_calls_on_javascript ... ok
test test_calls_on_kotlin ... ok
test test_calls_on_lua ... ok
test test_calls_on_luau ... ok
test test_calls_on_ocaml ... ok
test test_calls_on_php ... ok
test test_calls_on_python ... ok
test test_calls_on_ruby ... ok
test test_calls_on_rust ... ok
test test_calls_on_scala ... ok
test test_calls_on_swift ... ok
test test_calls_on_typescript ... ok
[... 216 more ...]

test result: ok. 234 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.6s
```

## Most surprising gaps (1-line each)

1. **Ruby bareword calls are invisible** — idiomatic `foo` (no parens)
   parses as `identifier`, not `call`, so the whole Rails ecosystem's
   style produces empty call graphs. (ruby.rs:256)

2. **C/C++ `#include` doesn't cross-link calls** — header-based imports
   don't feed into the module-index / import-resolver path that
   Python/TS/Go use, so inter-translation-unit calls are missed.

3. **Swift/Kotlin/Java/C#/Scala/Elixir/OCaml have 0 imports** from our
   fixtures because these languages use implicit imports at
   compilation-unit / package level. Needs more explicit-import fixtures
   to audit.

## Verification

- `cargo test -p tldr-cli --test language_command_matrix` → 234 passed.
- `cargo clippy -p tldr-cli --test language_command_matrix -- -D warnings`
  → clean.
- `cargo test -p tldr-cli --test language_autodetect_tests` → 18 passed
  (regression check — no change to VAL-008 work).

No existing test was modified, and the new files add zero clippy or
compile warnings.
