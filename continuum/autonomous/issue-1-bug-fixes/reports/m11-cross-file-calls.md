# Cross-file Call Resolution (VAL-011)

**Date:** 2026-04-25
**Commit base:** post-VAL-010 (matrix audit found 8 WEAK languages)
**Binary tested:** `/Users/cosimo/.cargo/bin/tldr` (release build, default features = `["semantic"]`)

## Goal

Close the cross-file call resolution gap identified in M10 for 8 languages
(C, C++, Ruby, Kotlin, Swift, PHP, Luau, OCaml). The canonical 2-file
fixture has `main()` calling both `helper()` (same file) and `b_util()`
(file B). M10's `tldr calls` reported 1 edge for these 8 languages instead
of the expected 2.

After VAL-011, ALL 18 supported languages return >= 2 edges on the
canonical fixture. The matrix-test threshold is tightened to `>= 2` for
the calls row (was `>= 1`), which now FAILS rather than silently passes
any language that misses the cross-file edge.

## Result

234 of 234 matrix tests pass with the tightened assertion. 4737 of 4737
core-lib tests pass with no regression. Clippy clean across the workspace.

## Per-language Breakdown

### C (`#include "util.h"`)

**Idiom now understood:** Bareword `b_util()` called from `main.c` after
`#include "util.h"`, where `b_util` is defined in `util.c`. C has external
linkage by default — the linker matches names across translation units
without any source-level import.

**Root cause of prior failure:** The Direct call `b_util()` reaches
`resolve_call::CallType::Direct` in
`crates/tldr-core/src/callgraph/resolution.rs`. Local-module lookup fails
(b_util is in util.c, not main.c). The import_map only has
`include "util.h"` which doesn't link a function name to a path. Falls
through to `None`.

**Fix applied:** Added `resolve_global_free_function` fallback in
`crates/tldr-core/src/callgraph/resolution.rs:1497-1547` that searches
`func_index.find_by_name(target)` and accepts a unique cross-file
free-function match. Gated on `language` matching `c | cpp | c++ | kotlin
| swift | ruby | php` (the languages where bareword resolution is the
norm). Deduplicates by `(file_path, line)` because `func_index` aliases
the same function under multiple module-name keys.

### C++ (`#include "util.hpp"`)

**Idiom:** Identical pattern to C — `b_util()` declared in `util.hpp`,
defined in `util.cpp`. Same external-linkage semantics.

**Root cause:** Same as C — Direct call falls through.

**Fix:** Same `resolve_global_free_function` fallback, gated on the same
language list (`cpp` and `c++` both included).

### Ruby (`require_relative 'util'`)

**Idiom:** `require_relative 'util'` loads `util.rb` and any top-level
`def` in that file becomes globally callable bareword from the requiring
file. The fixture uses `b_util()` (with parens — the M10-documented Ruby
bareword gap is unrelated to this milestone).

**Root cause:** `b_util()` is a Direct call. Ruby has no per-call
namespace prefix, so the resolver has no receiver to search through.

**Fix:** `resolve_global_free_function` fallback at the same site. Once
util.rb's `b_util` is in the global func_index (which it is, via the
normal indexing pipeline), the unique-cross-file match returns it.

### Kotlin (top-level functions, no explicit import)

**Idiom:** `bUtil()` defined as a top-level function in `Util.kt` is
callable bareword from `Main.kt` because both files share the same
package (the implicit default package when no `package` declaration is
present, which matches the canonical fixture).

**Root cause:** Direct call with no entry in import_map (no `import`
statement was emitted by the handler — none is needed in source).

**Fix:** Same `resolve_global_free_function` fallback, gated on language
== `kotlin`.

### Swift (top-level functions, same module)

**Idiom:** `bUtil()` defined as a top-level `func` in `Util.swift` is
callable bareword from `Main.swift` because same Swift package = same
module = same namespace.

**Root cause:** Identical to Kotlin — Direct call, no import_map entry.

**Fix:** Same `resolve_global_free_function` fallback, gated on
`language == "swift"`.

### PHP (`require_once 'util.php'`)

**Idiom:** `require_once 'util.php'` includes the file; functions defined
at file scope become globally callable. The fixture uses bareword
`b_util()`.

**Root cause:** Same general pattern as the other bareword languages —
Direct call falls through. Note: the PHP handler still emits a quirky
ImportDef where `alias: "once"` (the `require_once` keyword is partially
mis-parsed), but this doesn't matter for resolution since the fallback
doesn't go through the import_map.

**Fix:** Same `resolve_global_free_function` fallback, gated on `language
== "php"`. Documenting the import-parsing quirk as out-of-scope for this
milestone — fixing it would not change the calls-row outcome.

### Luau (`local util = require('./util')`)

**Idiom:** Rojo-style relative requires bind a name to the imported
module: `local util = require('./util')`, then `util.b_util()`.

**Root cause:** Two-fold:
1. The Luau handler did not extract the LHS variable as the import's
   `alias`. The lua handler does this via a two-pass extraction
   (`extract_aliased_require`), but luau.rs only had the second pass.
   Result: `module_imports` was keyed under `./util` (the literal import
   path) instead of `util` (the bound name), so receiver-lookup of
   `util.b_util` missed.
2. As a defense-in-depth, the lua/luau module-index aliases didn't
   include `./<module>` form, so even direct receiver lookup of `./util`
   would have missed.

**Fix:**
1. Added `extract_aliased_require` in
   `crates/tldr-core/src/callgraph/languages/luau.rs:236-282` modeled on
   the lua handler. Updated `parse_imports` to a two-pass extraction so
   `module_imports[alias_name] = resolved_module_path` is built
   correctly.
2. Extended `compute_module_aliases` for `lua | luau` in
   `crates/tldr-core/src/callgraph/module_index.rs:496-518` to also index
   `./<module>` forms.

### OCaml (`Util.b_util ()`)

**Idiom:** OCaml derives the module name from a file's basename with the
first letter capitalized — `util.ml` becomes module `Util`. Sibling
modules in the same directory are visible without an `open` statement,
and the canonical call form is `Util.b_util ()`.

**Root cause:** No explicit import, so `module_imports` is empty. The
call `Util.b_util ()` is an Attr call with `receiver = "Util"`, but the
func_index keys functions under the lowercase module name (`util`, from
`path_to_module_ocaml`). Standard receiver-resolution chain
(`resolve_module_import_receiver`, `resolve_method_in_class_or_bases`,
etc.) all miss.

**Fix:** Two parts:
1. Added OCaml-aware `compute_module_aliases` branch in
   `crates/tldr-core/src/callgraph/module_index.rs:545-557` so the index
   carries both `util` and `Util` aliases for `util.ml`.
2. Added `resolve_ocaml_module_receiver` fallback in
   `crates/tldr-core/src/callgraph/resolution.rs:1471-1499` and wired it
   into `resolve_call_with_receiver` after the existing capitalized
   receiver pass. It tries the lowercase receiver against the func_index
   first (matching the Python-style key the index actually uses), then
   the bare receiver.

## Test Tightening

`crates/tldr-cli/tests/language_command_matrix.rs:check_calls` was the
gate. Old assertion accepted `total_edges == 0` as failure but anything
>= 1 as pass. Tightened to `total_edges < 2 → fail` with a citing
message:

```rust
// VAL-011: tightened from `>= 1` to `>= 2`. The canonical fixture has
// `main -> helper` (intra-file) and `main -> b_util` (cross-file), so
// any handler that skips the cross-file edge now FAILS instead of
// silently passing as WEAK.
if total < 2 {
    fail_cell("calls", lang, ...);
}
```

This converts the 8 WEAK cells from M10 into hard FAIL signals. After
the per-language fixes above, all 18 cells in the calls row are OK.

## Verification

- `cargo test -p tldr-cli --test language_command_matrix` → 234 passed.
- `cargo test -p tldr-core --lib` → 4737 passed, 0 failed.
- `cargo clippy --workspace --all-features -- -D warnings` → clean.

No new ignored tests introduced. No assertions weakened. No `#[allow]`
sprinkled. The fixture file `crates/tldr-cli/tests/fixtures/mod.rs` was
NOT modified — the same canonical 2-file/3-function/2-edge contract from
M10 is what now passes the tightened assertion for all 18 languages.

## Out-of-scope (deferred)

- Ruby bareword call detection (`helper` without parens parses as
  `identifier`, not `call`). Documented in M10 as a separate handler
  bug; fixture continues to use `helper()` with parens as M10's
  workaround. Per VAL-011 contract, not addressed here.
- PHP `require_once` parsing emitting `alias: "once"`. Cosmetic in
  `tldr imports` output but does not affect calls resolution after
  VAL-011's bareword fallback.
