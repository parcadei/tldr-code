# Behavioral Specification: Reference-Counting Dead Code Detection

**Version:** 1.0  
**Date:** 2026-02-12  
**Status:** Implementation Pending

## Overview

This specification defines the behavioral contracts for a **reference-counting based dead code detector** that will replace the current call-graph set-difference algorithm in `analysis/dead.rs::dead_code_analysis()`. The new algorithm counts identifier occurrences across the entire codebase using tree-sitter to determine if a function is dead (defined but never referenced elsewhere).

### Algorithm Summary

```
1. Parse all files with tree-sitter → Extract function definitions + all identifier tokens
2. Build reference count map: ref_count[func_name] = count of identifier occurrences
3. For each definition: if ref_count == 1 (only definition) AND not excluded → DEAD
4. Apply per-language exclusion patterns (entry points, dunders, traits, tests, decorators)
5. Classify: public uncalled → possibly_dead; private uncalled → dead_functions
6. Return DeadCodeReport (backward compatible with existing API)
```

**Key Insight:** Reference counting eliminates 71% of false positives compared to call-graph analysis because it catches callbacks, dict-registered functions, and dynamic dispatch patterns that graphs miss (validated by prototype analysis and industry tools like Vulture, Meta SCARF).

---

## Core Behavioral Contracts

### C1: Reference Count = 1 → DEAD (Unless Excluded)

**Rule:** A function with `ref_count == 1` (only its definition site) AND not matching any exclusion pattern → flagged as DEAD.

**Rationale:** If a function name appears exactly once in the entire codebase, that occurrence is the definition itself. No other code references it.

**Implementation Reference:** Will replace the current `called_functions.contains(func_ref)` check at `dead.rs:64-66`.

**Test Cases:**
```rust
// ref_count("unused_helper") = 1 (only definition)
fn unused_helper() { }  // ← DEAD

// ref_count("used_func") = 3 (definition + 2 calls)
fn used_func() { }
let f = used_func;
callbacks.push(used_func);  // ← ALIVE (rescued by references)
```

---

### C2: Reference Count > 1 → ALIVE (Rescued by Refcount)

**Rule:** A function with `ref_count > 1` → NOT DEAD (regardless of call graph edges).

**Rationale:** Multiple occurrences mean the function is referenced somewhere beyond its definition — in a callback, dict registration, string-based dispatch, or actual call.

**Implementation Reference:** This is the inverse of C1. The current algorithm only checks `called_functions` (edges), missing non-call references.

**Test Cases:**
```python
# Callback registration (no call graph edge, but ref_count = 2)
def on_click():  # ← definition (ref 1)
    pass
handlers['click'] = on_click  # ← reference (ref 2) → ALIVE
```

**Edge Cases Handled:**
- Function passed as argument: `sort(items, key=custom_comparator)` → `custom_comparator` ref_count += 1
- Dict/list registration: `routes['/api'] = handle_api` → `handle_api` ref_count += 1
- String references (caught as identifier token if unquoted): `getattr(obj, method_name)` doesn't help, but `obj.method_name` does

---

### C3: Short Names (<3 chars) → Refcount Check SKIPPED

**Rule:** Names shorter than 3 characters → reference count check SKIPPED, treated as LOW CONFIDENCE.

**Rationale:** Short names (`f`, `fn`, `cb`, `op`, `x`, `y`) are collision-prone across scopes and files. High risk of false negatives (missing truly dead code) if we trust refcount alone.

**Implementation Reference:** New logic to add. Check name length before relying on refcount.

**Action on Short Names:**
- Still apply exclusion patterns (entry points, dunders, etc.)
- If not excluded and `ref_count == 1`, flag as DEAD but mark **confidence: LOW**
- Output format should indicate low-confidence findings separately or with a marker

**Per-Language High-Risk Common Names (also skip or mark low-confidence):**
| Language | Common Names (High Collision Risk) |
|----------|-----------------------------------|
| Python   | `get`, `set`, `run`, `update`, `process`, `handle` |
| Go       | `New`, `Get`, `Set`, `String`, `Error`, `Close` |
| Java     | `get`, `set`, `toString`, `equals`, `hashCode`, `run` |
| Ruby     | `call`, `to_s`, `to_a`, `new`, `initialize` |
| Rust     | `new`, `from`, `into`, `default`, `fmt` |
| C        | `init`, `open`, `close`, `read`, `write` |

**Configuration (future):** Provide a `--skip-common-names` flag and per-language config files.

---

### C4: Entry Points → EXCLUDED (Regardless of Refcount)

**Rule:** Functions matching entry point patterns → EXCLUDED from dead code analysis.

**Implementation Reference:** Reuse existing `is_entry_point_name()` at `dead.rs:129-213`.

**Standard Entry Point Patterns:**

```rust
// Application entry (literal match)
"main", "__main__", "cli", "app", "run", "start"

// Test setup/teardown (literal match)
"setup", "teardown", "setUp", "tearDown"

// Python ASGI/WSGI (literal match)
"create_app", "make_app"

// Go HTTP (literal match)
"ServeHTTP", "Handler", "handler"

// C/system callbacks (literal match)
"OnLoad", "OnInit", "OnExit"

// Android/Kotlin lifecycle (literal match)
"onCreate", "onStart", "onStop", "onResume", "onPause", "onDestroy",
"onBind", "onClick", "onCreateView"

// Java Servlet/Spring (literal match)
"doGet", "doPost", "doPut", "doDelete", "init", "destroy", "service"

// Plugin/middleware hooks (literal match)
"load", "configure", "request", "response", "error", "invoke", "call", "execute"

// Test prefixes (prefix match)
"test_*", "pytest_*", "Test*", "Benchmark*", "Example*"

// Handler/hook prefixes (prefix match on bare name)
"handle*", "Handle*", "on_*", "before_*", "after_*"

// Custom patterns (user-provided via entry_points parameter)
Supports: exact match, prefix glob (pattern*), suffix glob (*pattern)
```

**Class.method Format Handling:**
- Extract bare method name: `MyServlet.doGet` → check `doGet`
- Reference: `dead.rs:156-160`

**Test Coverage:** Existing tests at `dead.rs:561-633` validate all patterns.

---

### C5: Dunder Methods → EXCLUDED (Regardless of Refcount)

**Rule:** Python dunder methods (names matching `__.*__`) → EXCLUDED.

**Rationale:** Called implicitly by Python runtime (operator overloads, lifecycle hooks, protocols).

**Implementation Reference:** `dead.rs:73-78`

```rust
let bare_name = func_ref.name.rsplit('.').next().unwrap_or(&func_ref.name);
if bare_name.starts_with("__") && bare_name.ends_with("__") {
    continue;  // Skip dunder methods
}
```

**Handles Class.method Format:**
- `MyClass.__init__` → extracts `__init__` → excluded
- `Serializer.__getstate__` → extracts `__getstate__` → excluded

**Comprehensive Dunder List (from oracle report Q4):**
```python
# Lifecycle
__init__, __new__, __del__

# String representation
__repr__, __str__, __format__, __bytes__

# Comparison
__eq__, __ne__, __lt__, __le__, __gt__, __ge__, __hash__, __bool__

# Container protocol
__len__, __getitem__, __setitem__, __delitem__, __contains__, __iter__, __next__, __reversed__

# Callable protocol
__call__

# Context manager
__enter__, __exit__, __aenter__, __aexit__

# Descriptor protocol
__get__, __set__, __delete__, __set_name__

# Attribute access
__getattr__, __getattribute__, __setattr__, __delattr__

# Arithmetic operators (all __add__, __sub__, __mul__, __r*__, __i*__ variants)
__add__, __sub__, __mul__, __truediv__, __floordiv__, __mod__, __pow__,
__radd__, __rsub__, __rmul__, __rtruediv__, __rfloordiv__, __rmod__, __rpow__,
__iadd__, __isub__, __imul__, __itruediv__, __ifloordiv__, __imod__, __ipow__

# Unary operators
__neg__, __pos__, __abs__, __invert__, __index__, __int__, __float__

# Metaclass/subclass hooks
__init_subclass__, __class_getitem__, __instancecheck__, __subclasscheck__

# Serialization
__getstate__, __setstate__, __reduce__, __reduce_ex__

# Copy protocol
__copy__, __deepcopy__
```

**Test Coverage:** Tested at `dead.rs:511` (`__init__` excluded).

---

### C6: Trait/Interface Methods → EXCLUDED (Regardless of Refcount)

**Rule:** Methods inside trait/interface/protocol/abstract classes → EXCLUDED.

**Rationale:** These methods are implementations of type system contracts, called via polymorphic dispatch which the reference counter cannot track.

**Implementation Reference:** `dead.rs:80-83` + `is_trait_or_interface()` at `dead.rs:409-455`

```rust
if func_ref.is_trait_method {
    continue;  // Skip trait/interface methods
}
```

**Detection Logic (from `is_trait_or_interface()`):**

1. **Base class patterns:**
   - Python: `ABC`, `ABCMeta`, `Protocol`, `Interface` in class bases
   - Reference: `dead.rs:415-421`

2. **Decorator patterns:**
   - Class decorated with `@abstract`, `@interface`, `@protocol`
   - Reference: `dead.rs:424-430`

3. **Language-specific naming conventions:**
   - **Rust:** Cannot reliably detect from name alone (AST extractor should tag explicitly)
   - **Java/Kotlin:** Interface naming convention `IFoo` (starts with `I` + uppercase second char)
   - **C#:** Same as Java/Kotlin (`IDisposable`, `IEnumerable`)
   - Reference: `dead.rs:432-454`

**How It's Set:**
- Populated in `collect_all_functions()` at `dead.rs:249` via `is_trait_or_interface(class, language)`
- Metadata attached to `FunctionRef.is_trait_method` field

**Test Coverage:** Tested at `dead.rs:688-702` (trait methods not flagged as dead).

---

### C7: Test Functions → EXCLUDED (Regardless of Refcount)

**Rule:** Functions identified as tests → EXCLUDED.

**Rationale:** Test functions are called by test runners, not by application code.

**Implementation Reference:** `dead.rs:85-88` + detection helpers at `dead.rs:274-308`

```rust
if func_ref.is_test {
    continue;  // Skip test functions
}
```

**Detection Logic (from `collect_all_functions()`):**

```rust
// dead.rs:233
let is_test = is_test_file || is_test_function_name(&func.name) || has_test_decorator(&func.decorators);
```

**1. Test File Detection (`is_test_file_path()` at `dead.rs:274-291`):**
```rust
// File name patterns
file_name.starts_with("test_")     // test_auth.py
|| file_name.ends_with("_test")    // auth_test.go
|| file_name.ends_with("_tests")   // auth_tests.py
|| file_name.ends_with("_spec")    // auth_spec.rb
|| file_name.starts_with("Test")   // TestAuth.java
|| file_name.ends_with("Test")     // AuthTest.kt
|| file_name.ends_with("Tests")    // AuthTests.swift
|| file_name.ends_with("Spec")     // AuthSpec.scala

// Directory patterns
path.contains("/test/")
|| path.contains("/tests/")
|| path.contains("/spec/")
|| path.contains("/__tests__/")    // JavaScript convention
```

**2. Test Function Name Detection (`is_test_function_name()` at `dead.rs:294-300`):**
```rust
bare_name.starts_with("test_")      // Python: test_login()
|| bare_name.starts_with("Test")    // Go: TestLogin(), Java: TestAuth
|| bare_name.starts_with("Benchmark") // Go: BenchmarkSort()
|| bare_name.starts_with("Example") // Go: ExampleParse()
```

**3. Test Decorator Detection (`has_test_decorator()` at `dead.rs:303-308`):**
```rust
decorators.iter().any(|d| {
    let lower = d.to_lowercase();
    lower == "test"                              // @test (JUnit, Rust)
    || lower == "pytest.mark.parametrize"        // @pytest.mark.parametrize
    || lower.starts_with("test")                 // @testmethod, @testcase
})
```

**Test Coverage:** Tested at `dead.rs:722-741` (test functions not flagged as dead).

---

### C8: Decorated Functions → EXCLUDED (Regardless of Refcount)

**Rule:** Functions with decorators/annotations → EXCLUDED.

**Rationale:** Decorators indicate framework-invoked functions (routes, commands, event handlers, dependency injection).

**Implementation Reference:** `dead.rs:90-93`

```rust
if func_ref.has_decorator {
    continue;  // Skip decorated/annotated functions
}
```

**How It's Set:**
- Populated in `collect_all_functions()` at `dead.rs:232`
- `has_decorator = !func.decorators.is_empty()`
- Metadata attached to `FunctionRef.has_decorator` field
- Also stores `FunctionRef.decorator_names` for debugging/reporting

**Common Framework Decorators (examples):**
```python
# Python
@route('/api')          # Flask/FastAPI
@app.command()          # Click/Typer
@pytest.fixture         # Pytest (also caught by C7)
@celery.task            # Celery
@login_required         # Django

# Java/Kotlin
@Bean                   # Spring
@PostConstruct, @PreDestroy  # Spring lifecycle
@GET, @POST, @PUT, @DELETE   # JAX-RS
@Override               # (also caught by C6 if trait method)

# TypeScript/JavaScript
@Component              # Angular
@Injectable             # NestJS
```

**Test Coverage:** Tested at `dead.rs:705-719` (decorated functions not flagged as dead).

---

### C9: Public vs Private Classification

**Rule:** Uncalled functions are classified based on visibility:
- **Public + uncalled** → `possibly_dead` (may be API surface, library export)
- **Private + uncalled** → `dead_functions` (definitely dead)

**Implementation Reference:** `dead.rs:95-105`

```rust
if func_ref.is_public {
    possibly_dead.push(func_ref.clone());
} else {
    dead_functions.push(func_ref.clone());
    by_file.entry(func_ref.file.clone()).or_default().push(func_ref.name.clone());
}
```

**Visibility Inference (`infer_visibility_from_name()` at `dead.rs:314-406`):**

| Language | Public Pattern | Notes |
|----------|---------------|-------|
| **Python** | No leading `_` | `def foo()` = public, `def _foo()` = private |
| **Go** | Uppercase first char | `func Export()` = public, `func internal()` = private |
| **Rust** | No leading `_` | Heuristic (AST should provide `pub` keyword info) |
| **TypeScript/JS** | No leading `_` | Convention-based |
| **Java/Kotlin/C#/Scala** | No leading `_` | Default to public (needs AST keyword info for accuracy) |
| **C/C++** | Always `true` | Cannot infer from name (`static` keyword needed) |
| **Ruby** | No leading `_` | Convention (actual visibility via `private` keyword) |
| **PHP** | No leading `_` | Convention (actual visibility via keywords) |
| **Elixir** | No leading `_` | `def` = public, `defp` = private (AST provides this) |
| **Lua/Luau** | Special handling | `_M:method` or `_M.method` = public, `_prefix` = private |
| **OCaml** | No leading `_` | .mli files define public API (needs cross-file analysis) |
| **Swift** | No leading `_` | Default is `internal`, not public (conservative) |

**Reference:** `dead.rs:324-406` has full per-language logic.

**Test Coverage:** Tested at `dead.rs:653-685` (public → possibly_dead, private → dead_functions).

---

### C10: Output Format (Backward Compatibility)

**Rule:** Return type MUST match existing `types::DeadCodeReport` structure for backward compatibility with CLI and quality wrappers.

**Implementation Reference:** `types.rs:797` (from architecture map)

```rust
pub struct DeadCodeReport {
    pub dead_functions: Vec<FunctionRef>,           // Private + uncalled
    pub possibly_dead: Vec<FunctionRef>,            // Public + uncalled
    pub by_file: HashMap<PathBuf, Vec<String>>,     // Grouped by file
    pub total_dead: usize,
    pub total_possibly_dead: usize,
    pub total_functions: usize,
    pub dead_percentage: f64,  // Based on dead_functions only (excludes possibly_dead)
}
```

**Field Population:**
```rust
// dead.rs:108-125
let total_dead = dead_functions.len();
let total_possibly_dead = possibly_dead.len();
let total_functions = all_functions.len();
let dead_percentage = if total_functions > 0 {
    (total_dead as f64 / total_functions as f64) * 100.0
} else {
    0.0
};
```

**Downstream Consumers (must continue working unchanged):**
- `tldr-cli/src/commands/dead.rs::DeadArgs::run()` → applies truncation, formats output
- `quality/dead_code.rs::analyze_dead_code_with_graph()` → transforms to richer `quality::DeadCodeReport`
- `wrappers/todo.rs` → calls `analyze_dead_code()` for dead code suggestions
- `quality/health.rs` → calls via quality wrapper for health metrics

**Test Coverage:** Tested at `dead.rs:544-558` (stats calculation), `dead.rs:793-806` (report structure).

---

### C11: Combined Parse Pass (Performance Optimization)

**Rule:** Parse each file ONCE with tree-sitter to extract both function definitions AND all identifier tokens.

**Rationale:** Current algorithm parses files twice (once for ModuleInfo, once for call graph). Reference counting requires identifier enumeration, which can be done in the same pass as AST extraction.

**Implementation Strategy:**

```rust
// Pseudocode for combined pass
for each file:
    parse with tree-sitter
    
    // Extract function definitions (existing logic)
    let functions = extract_function_definitions(tree)
    
    // NEW: Extract all identifier tokens for reference counting
    let identifiers = extract_all_identifiers(tree)
    
    // Build ref_count map
    for identifier in identifiers:
        ref_count[identifier.text] += 1
```

**Tree-sitter Query for Identifiers:**
```scheme
;; Generic query for all identifier tokens (language-agnostic fallback)
(identifier) @ref

;; Language-specific additions (from oracle report Q2)
(field_identifier) @ref       ;; Go methods
(property_identifier) @ref    ;; JS/TS properties
(simple_identifier) @ref      ;; Kotlin/Swift
(value_name) @ref             ;; OCaml
```

**Performance Target:**
- Current: 2 passes (AST extraction + call graph building)
- New: 1 pass (AST + identifier counting combined)
- Expected: 40-50% faster for large codebases

**Complexity:** O(N) where N = total tokens across all files (linear scan).

---

### C12: Per-Language Identifier Node Types (Tree-sitter Queries)

**Rule:** Use language-specific tree-sitter node types for accurate identifier extraction.

**Source:** Oracle report Q2 (lines 57-164)

#### Complete Node Type Reference

| Language | Function Definition Node | Name Field | Name Node Type | Method Definition Node | Notes |
|----------|-------------------------|------------|----------------|----------------------|-------|
| **Python** | `function_definition` | `name` | `identifier` | `function_definition` (inside class) | Same for functions/methods |
| **TypeScript** | `function_declaration` | `name` | `identifier` | `method_definition` → `name` → `property_identifier` | Also `arrow_function` (anonymous) |
| **JavaScript** | `function_declaration` | `name` | `identifier` | `method_definition` → `name` → `property_identifier` | Same as TS minus types |
| **Go** | `function_declaration` | `name` | `identifier` | `method_declaration` → `name` → `field_identifier` | Receiver in `parameters` |
| **Rust** | `function_item` | `name` | `identifier` | `function_item` (inside `impl_item`) | Note: `function_item`, not `function_definition` |
| **Java** | `method_declaration` | `name` | `identifier` | `method_declaration` | No separate "function" concept |
| **C** | `function_definition` | `declarator` → `function_declarator` → `declarator` | `identifier` | N/A | Nested declarator chain |
| **C++** | `function_definition` | `declarator` → `function_declarator` → `declarator` | `identifier` / `field_identifier` | Also `qualified_identifier` | Pointer returns add `pointer_declarator` |
| **Ruby** | `method` | `name` | `identifier` | `singleton_method` | Also `method` for instance |
| **PHP** | `function_definition` | `name` | `name` (not `identifier`) | `method_declaration` → `name` | PHP uses `name` node type |
| **Kotlin** | `function_declaration` | `name` → `simple_identifier` | `simple_identifier` | Same node inside class | Kotlin grammar uses `simple_identifier` |
| **Swift** | `function_declaration` | `name` | `simple_identifier` | Same node | Grammar: `alex-pinkus/tree-sitter-swift` |
| **C#** | `method_declaration` | `name` | `identifier` | Same | Also `local_function_statement` |
| **Scala** | `function_definition` | `name` | `identifier` | `function_definition` | Also `val_definition` for lambdas |
| **Elixir** | `call` (with `def`/`defp` target) | First argument | `identifier` or `call` | Same | Functions are macro calls |
| **Lua** | `function_declaration` | `name` | `identifier` or `dot_index_expression` | Same | `local_function_declaration` for local |
| **OCaml** | `value_definition` / `let_binding` | `pattern` → `value_name` | `value_name` | Same | Everything is a value binding |
| **Luau** | `function_declaration` | `name` | `identifier` or `dot_index_expression` | Same | Extends Lua grammar |

#### Tree-sitter Query Patterns (Definitions)

```scheme
;; Python
(function_definition name: (identifier) @func.def)

;; Go
(function_declaration name: (identifier) @func.def)
(method_declaration name: (field_identifier) @func.def)

;; Rust
(function_item name: (identifier) @func.def)

;; TypeScript/JavaScript
(function_declaration name: (identifier) @func.def)
(method_definition name: (property_identifier) @func.def)

;; Java
(method_declaration name: (identifier) @func.def)

;; C (nested declarator)
(function_definition
  declarator: (function_declarator
    declarator: (identifier) @func.def))

;; Ruby
(method name: (identifier) @func.def)
(singleton_method name: (identifier) @func.def)

;; PHP
(function_definition name: (name) @func.def)
(method_declaration name: (name) @func.def)

;; Kotlin
(function_declaration (simple_identifier) @func.def)

;; Elixir (def/defp are macro calls)
(call
  target: (identifier) @_defkind
  (arguments (call target: (identifier) @func.def))
  (#match? @_defkind "^defp?$"))

;; Lua
(function_declaration name: (identifier) @func.def)

;; OCaml
(let_binding pattern: (value_name) @func.def)
```

#### Identifier Reference Queries (All Occurrences)

```scheme
;; Most languages
(identifier) @ref

;; Go methods (also need field_identifier)
(field_identifier) @ref

;; JS/TS object properties
(property_identifier) @ref

;; Kotlin/Swift
(simple_identifier) @ref

;; OCaml
(value_name) @ref

;; PHP (uses 'name' node type)
(name) @ref
```

#### Language-Specific Gotchas (from oracle report Q2, lines 150-165)

| Language | Gotcha | Impact | Mitigation |
|----------|--------|--------|------------|
| **C/C++** | Nested declarators (pointer_declarator, function_declarator) | Name extraction requires walking chain | Walk declarator chain programmatically |
| **Go** | Method receivers use `field_identifier` not `identifier` | Misses method names if only looking for `identifier` | Query both `identifier` AND `field_identifier` |
| **Elixir** | `def`/`defp` are macro calls, not syntax nodes | No `function_definition` node exists | Match `call` nodes with `def`/`defp` target |
| **Lua** | `function tbl.x.y.z()` stores name as `dot_index_expression` | Name is compound, not simple identifier | Extract last segment of dot-chain |
| **OCaml** | Functions are `let` bindings, not separate nodes | Must distinguish function bindings from value bindings | Check if body is `fun_expression` |
| **Ruby** | `method_missing`, `define_method`, `send` | Dynamic method invocation invisible to static analysis | Exclude common metaprogramming patterns |
| **Python** | `__getattr__`, `getattr()`, `setattr()` | Dynamic attribute access | Exclude dunder methods (C5) |
| **Kotlin** | Extension functions: `fun String.myExt()` | Name includes receiver type in grammar | Extract simple_identifier only |
| **Swift** | `@objc` runtime dispatch, protocol extensions | Methods called via ObjC runtime invisible | Exclude `@objc` decorated methods |
| **PHP** | `__call`, `__callStatic` magic methods | Dynamic method dispatch | Exclude magic methods (like dunders) |
| **Luau** | Metatables with `__index` | Method calls via metatable | Exclude metamethods |

---

## Additional Language-Specific Exclusion Patterns

**Source:** Oracle report Q4 (lines 220-416)

### Python (Already Covered by C5)
All `__.*__` patterns excluded by dunder check.

### Go
```rust
// Already covered by C4 (entry points)
"main", "init", "ServeHTTP", "Handler"

// Test functions (C7)
"Test*", "Benchmark*", "Example*", "Fuzz*"

// Common interface methods (exclude if implementing known interfaces)
"String", "Error", "MarshalJSON", "UnmarshalJSON", "Format"
```

### Rust
```rust
// Entry points (C4)
"main"

// Test/bench (C7)
// Functions with #[test], #[bench] attributes (detected via decorator check C8)

// FFI exports (C8 - has attribute decorator)
// #[no_mangle], #[export_name], pub extern "C" functions

// Common trait methods (if is_trait_method, already excluded by C6)
"new", "default", "from", "into", "try_from", "try_into", "fmt", "drop",
"deref", "deref_mut", "next", "size_hint"
```

### Java
```rust
// Entry point (C4)
"main"

// Overrides (C6 - if @Override, exclude via decorator check C8)

// Object methods (common enough to consider excluding if not called)
"toString", "equals", "hashCode", "compareTo", "clone", "finalize"

// Serialization (rarely called explicitly)
"writeObject", "readObject", "readObjectNoData", "readResolve", "writeReplace"

// Test/lifecycle (C7/C8)
// @Test, @Before, @After, @BeforeEach, @AfterEach via decorator check

// Spring/JAX-RS (C8)
// @Bean, @PostConstruct, @PreDestroy, @GET, @POST, etc. via decorator check
```

### C/C++
```rust
// Entry point (C4)
"main"

// Constructors/destructors (detect via AST — names match class name or ~ClassName)

// Operator overloads (detect via AST — operator+ format)

// Callbacks (cannot detect — registered via function pointers)
// USER MUST provide custom patterns for callback registration
```

### Ruby
```rust
// Lifecycle/hooks (C4 entry points or add here)
"initialize", "method_missing", "respond_to_missing?"

// Conversion methods (commonly called implicitly)
"to_s", "to_a", "to_h", "to_i", "to_f", "to_str", "to_ary", "to_hash"

// Comparison operators
"==", "eql?", "hash", "<=>", "<", ">", "<=", ">="

// Module hooks
"included", "extended", "inherited", "prepended"
"method_added", "method_removed", "method_undefined"
"const_missing", "append_features"

// Rails (C8 - via decorators or add custom patterns)
// before_action, after_action, concerns, serializers
```

### PHP
```rust
// Magic methods (like Python dunders)
"__construct", "__destruct", "__call", "__callStatic", "__get", "__set",
"__isset", "__unset", "__sleep", "__wakeup", "__serialize", "__unserialize",
"__toString", "__invoke", "__set_state", "__clone", "__debugInfo"

// Laravel (C8 via decorators or custom patterns)
// boot, register, handle
```

### Kotlin
```rust
// Entry point (C4)
"main"

// Override methods (C6/C8)
// @JvmStatic, companion object functions

// Any methods (like Java Object)
"toString", "equals", "hashCode"

// Test (C7/C8)
// @Test via decorator check
```

### Swift
```rust
// @objc methods (C8 - decorator check)

// Protocol conformance (C6 - trait method check)

// Lifecycle
"init", "deinit"

// Codable (protocol methods, C6)
"encode", "init" (from decoder)

// Protocol methods (C6)
"description" (CustomStringConvertible), "hash" (Hashable), "==" (Equatable)

// UIKit lifecycle (C4 entry points)
"viewDidLoad", "viewWillAppear", "viewDidAppear", etc.
```

### C#
```rust
// Entry point (C4)
"Main"

// Override methods (C6/C8)

// Lifecycle
"Dispose", "ToString", "Equals", "GetHashCode"

// Event handlers (pattern: *_EventName)
// Detect via naming convention or custom patterns

// Test (C7/C8)
// [TestMethod], [Fact], [Theory] via decorator check

// Serialization callbacks
"OnSerializing", "OnSerialized", "OnDeserializing", "OnDeserialized"
```

### Scala
```rust
// Entry point (C4)
"main"

// Override methods (C6/C8)

// Companion object patterns
"apply", "unapply"

// Any methods
"toString", "equals", "hashCode"

// Implicit conversions (C8 - decorator/keyword check)

// Test (C7/C8)
// @Test via decorator check
```

### Elixir
```rust
// GenServer callbacks (C4 entry points)
"init", "handle_call", "handle_cast", "handle_info"

// Supervisor callbacks
"start_link", "child_spec"

// LiveView callbacks
"mount", "render", "handle_event"

// Macro hook
"__using__"

// Behaviours (C6 - trait method equivalent)
// Functions implementing @behaviour callbacks
```

### Lua/Luau
```rust
// Metamethods (like Python dunders)
"__index", "__newindex", "__call", "__tostring", "__add", "__sub", "__mul",
"__div", "__eq", "__lt", "__le", "__gc", "__len", "__concat", "__unm",
"__mod", "__pow"

// Module init (C4 entry point)
"_init"

// Luau-specific (Roblox)
"Init", "Start"  // Knit framework lifecycle
":Destroy", ":GetPropertyChangedSignal"  // Common Roblox patterns
```

### OCaml
```rust
// Entry point (C4)
"main"

// Module signature implementations (C6 - trait equivalent)
// Values matching .mli signatures should be excluded
// Requires cross-file analysis (future enhancement)
```

---

## Algorithm Implementation Phases

### Phase 1: Parse + Extract (Combined Pass)
```rust
for each file in project:
    tree = parse_file_with_tree_sitter(file, language)
    
    // Extract function definitions (existing logic)
    definitions = extract_function_definitions(tree, language)
    
    // NEW: Extract all identifier tokens
    identifiers = extract_all_identifiers(tree, language)
    
    // Store results
    all_definitions.extend(definitions)
    all_identifiers.extend(identifiers)
```

**Output:**
- `all_definitions: Vec<(file, name, line, metadata)>`
- `all_identifiers: Vec<(file, name, line)>`

### Phase 2: Build Reference Count Map
```rust
let mut ref_count: HashMap<String, usize> = HashMap::new();

for identifier in all_identifiers:
    ref_count.entry(identifier.name).and_modify(|c| *c += 1).or_insert(1);
```

**Optimization:** Use `FxHashMap` from `rustc-hash` for faster hashing (strings).

### Phase 3: Apply Exclusions + Classify
```rust
for func_def in all_definitions:
    // Skip if ref_count > 1 (alive — C2)
    if ref_count.get(&func_def.name).unwrap_or(&0) > &1 {
        continue;
    }
    
    // Skip short names (low confidence — C3)
    if func_def.name.len() < 3 {
        // Flag as low-confidence dead, but continue (or skip entirely)
        low_confidence_dead.push(func_def);
        continue;
    }
    
    // Skip entry points (C4)
    if is_entry_point_name(&func_def.name, custom_patterns) {
        continue;
    }
    
    // Skip dunders (C5)
    let bare = extract_bare_name(&func_def.name);
    if is_dunder(bare) {
        continue;
    }
    
    // Skip trait methods (C6)
    if func_def.is_trait_method {
        continue;
    }
    
    // Skip test functions (C7)
    if func_def.is_test {
        continue;
    }
    
    // Skip decorated functions (C8)
    if func_def.has_decorator {
        continue;
    }
    
    // Classify by visibility (C9)
    if func_def.is_public {
        possibly_dead.push(func_def);
    } else {
        dead_functions.push(func_def);
        by_file.entry(func_def.file).or_default().push(func_def.name);
    }
```

### Phase 4: Compute Stats + Build Report (C10)
```rust
let total_dead = dead_functions.len();
let total_possibly_dead = possibly_dead.len();
let total_functions = all_definitions.len();
let dead_percentage = if total_functions > 0 {
    (total_dead as f64 / total_functions as f64) * 100.0
} else {
    0.0
};

DeadCodeReport {
    dead_functions,
    possibly_dead,
    by_file,
    total_dead,
    total_possibly_dead,
    total_functions,
    dead_percentage,
}
```

---

## Testing Strategy

### Test Coverage Requirements

All contracts (C1-C12) must have failing tests BEFORE implementation:

1. **C1 (ref_count == 1 → DEAD):**
   - Test: Function defined once, never referenced → flagged as dead
   - Test: Function with refcount == 1 AND excluded pattern → NOT flagged

2. **C2 (ref_count > 1 → ALIVE):**
   - Test: Function passed as callback (ref_count = 2) → not dead
   - Test: Function in dict registration (ref_count = 2) → not dead
   - Test: Function called normally (ref_count = 3+) → not dead

3. **C3 (short names skipped):**
   - Test: Function named `f` with ref_count == 1 → low confidence or skipped
   - Test: Function named `get` with ref_count == 1 → low confidence or skipped
   - Test: Function named `long_descriptive_name` with ref_count == 1 → high confidence dead

4. **C4 (entry points excluded):**
   - Reuse existing tests (`dead.rs:561-633`) — all must pass
   - Add test for custom patterns: `--entry-points "handle_*"`

5. **C5 (dunders excluded):**
   - Test: `__init__` with ref_count == 1 → not dead
   - Test: `MyClass.__str__` with ref_count == 1 → not dead

6. **C6 (trait methods excluded):**
   - Reuse existing test (`dead.rs:688-702`)

7. **C7 (test functions excluded):**
   - Reuse existing test (`dead.rs:722-741`)

8. **C8 (decorated excluded):**
   - Reuse existing test (`dead.rs:705-719`)

9. **C9 (public vs private):**
   - Reuse existing tests (`dead.rs:653-685`, `dead.rs:744-778`)

10. **C10 (output format):**
    - Reuse existing tests (`dead.rs:544-558`, `dead.rs:793-806`)
    - Test CLI integration: output JSON matches schema
    - Test quality wrapper: transformation to `quality::DeadCodeReport` works

11. **C11 (combined parse pass):**
    - Performance benchmark: new algorithm ≥ 40% faster than old
    - Correctness: same files parsed, same functions extracted

12. **C12 (per-language queries):**
    - For each of 18 languages: test function definition extraction
    - For each of 18 languages: test identifier reference extraction
    - Use real-world code samples (Python: requests.py, Go: net/http, etc.)

### Integration Tests

```rust
#[test]
fn test_refcount_vs_callgraph_comparison() {
    // Compare old algorithm (call graph set-difference) vs new (refcount)
    // on a known corpus with labeled ground truth
    
    let corpus = load_test_corpus();  // Functions labeled DEAD/ALIVE by human
    
    let old_result = dead_code_analysis_callgraph(&corpus.graph, &corpus.functions, None);
    let new_result = dead_code_analysis_refcount(&corpus.graph, &corpus.functions, None);
    
    // New algorithm should have:
    // - Lower false positive rate (fewer alive functions flagged as dead)
    // - Same or better recall (finds same or more truly dead functions)
    assert!(new_result.false_positive_rate < old_result.false_positive_rate);
}
```

---

## Migration Plan

### Step 1: TDD — Write Failing Tests
1. Create test file: `crates/tldr-core/src/analysis/dead_refcount_tests.rs`
2. Implement tests for contracts C1-C12
3. All tests must fail initially (algorithm not yet implemented)

### Step 2: Implement Reference Counting Algorithm
1. Add `dead_code_analysis_refcount()` function in `dead.rs`
2. Implement phases 1-4 (parse + extract, ref count, exclusions, classify)
3. Tests should start passing one by one

### Step 3: Performance Validation
1. Benchmark on large corpus (10k+ files)
2. Validate: new algorithm ≥ 40% faster
3. Validate: memory usage ≤ 2x old algorithm (acceptable for speed gain)

### Step 4: Cutover
1. Replace `dead_code_analysis()` body with new algorithm
2. Keep old algorithm as `dead_code_analysis_legacy()` for A/B comparison
3. Run full test suite — all existing tests must pass

### Step 5: Deprecate Old Algorithm
1. After 2 weeks in production with no issues
2. Remove `dead_code_analysis_legacy()`
3. Update documentation

---

## Open Questions / Future Enhancements

1. **Cyclic dead code detection:** Should we detect mutually-referencing dead clusters (functions that call each other but are unreachable from entry points)? This is NOT caught by reference counting alone.

2. **Cross-file qualified names:** Should we count `module.function` and `function` separately or as the same reference? Meta SCARF uses fully-qualified names. Our current `FunctionRef` uses file+name, which is essentially qualified.

3. **String literals:** Should string literals like `"function_name"` count as references? Useful for catching config-driven dispatch, but high noise.

4. **Confidence scoring:** Should we expose confidence levels in the report? (HIGH: long unique name, MEDIUM: 4-7 chars, LOW: <4 chars or common name)

5. **OCaml .mli files:** Should we parse .mli signature files and exclude all values that appear in the public interface?

6. **C/C++ function pointers:** How do we handle callback registration that's invisible to static analysis? Require user to provide patterns?

7. **Parallel parsing:** Should we use `rayon` to parallelize tree-sitter parsing for large repos (100k+ files)?

---

## References

### Existing Code Files
- `crates/tldr-core/src/analysis/dead.rs` — Current algorithm (lines 33-126 is the core)
- `crates/tldr-core/src/types.rs` — `FunctionRef` (line 441), `DeadCodeReport` (line 797)
- `crates/tldr-cli/src/commands/dead.rs` — CLI entry point
- `crates/tldr-core/src/quality/dead_code.rs` — Quality wrapper

### Research Documents
- `/Users/cosimo/.opc-dev/.claude/cache/agents/oracle/output-2026-02-12-refcount-dead-code.md` — Oracle research on reference counting across 18 languages
- `/tmp/dead-command-map.md` — Architecture map of dead command pipeline

### Industry Tools (for validation)
- **Vulture (Python):** Uses AST name tracking (≈ ref counting) — closest algorithm to ours
- **Meta SCARF:** Uses textual search (BigGrep) as safety net — validates ref counting approach
- **unused tool:** Language-agnostic ctags + grep — proof of concept for our approach
- **Knip (TypeScript):** Mark-and-sweep from entry points — different algorithm, useful for comparison

---

## Success Criteria

Implementation is considered complete when:

1. All tests for C1-C12 pass
2. Backward compatibility: All existing tests in `dead.rs` tests module pass
3. Performance: ≥ 40% faster than old algorithm on 10k+ file corpus
4. False positive reduction: ≤ 30% FP rate (down from current ~71% per prototype)
5. CLI integration: `tldr dead` command produces identical output format
6. Quality integration: `tldr health` dead code metrics work unchanged
7. Documentation: CHANGELOG.md entry, README.md updated with new algorithm description

---

**END OF SPECIFICATION**
