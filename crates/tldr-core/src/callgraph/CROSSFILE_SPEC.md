# Cross-File Call Detection Behavioral Specification

**Status**: Reference Implementation Analysis  
**Date**: 2026-02-05  
**Goal**: Define correct behavior for cross-file call graph construction

## Problem Statement

The Rust implementation finds only **19 cross-file edges** while the Python implementation finds **~650 cross-file edges** on the same codebase (`/tmp/llm-tldr-test/tldr`).

This spec documents the Python implementation's correct behavior to guide the Rust fix.

---

## 1. Import Resolution Contract

### 1.1 Input: ImportDef

```rust
pub struct ImportDef {
    module: String,        // e.g., "helper", "pkg.submodule", "../utils"
    names: Vec<String>,    // e.g., ["process", "validate"]
    is_from: bool,         // true for "from X import Y"
    level: u8,             // 0 = absolute, 1 = ".", 2 = "..", etc.
    aliases: HashMap<String, String>,  // {"alias": "original_name"}
}
```

### 1.2 Output: Resolved Import Map

**Type**: `HashMap<String, (String, String)>`  
**Meaning**: `local_name -> (module_name, original_name)`

#### Python Example:
```python
# In main.py
from helper import process              # → {"process": ("helper", "process")}
from utils import validate as check     # → {"check": ("utils", "validate")}
import helper                           # → module_imports["helper"] = "helper"
import utils.core as core               # → module_imports["core"] = "utils.core"
```

#### Expected Rust Behavior:
```rust
// from helper import process
import_map.insert("process", ("helper", "process"));

// from utils import validate as check
import_map.insert("check", ("utils", "validate"));
import_map.insert("validate", ("utils", "validate"));  // BOTH names

// import helper
module_imports.insert("helper", "helper");

// import utils.core as core
module_imports.insert("core", "utils.core");
```

### 1.3 Relative Import Resolution (Python-Specific)

**PEP 328 Semantics**:

| Import Statement | From File | Resolved Module |
|-----------------|-----------|-----------------|
| `from . import X` | `pkg/sub/module.py` | `pkg.sub.X` |
| `from .. import X` | `pkg/sub/module.py` | `pkg.X` |
| `from ...utils import X` | `pkg/a/b/c.py` | `pkg.utils.X` |

**Algorithm** (Python Reference):
```python
# For file: pkg/sub/module.py (module = "pkg.sub.module")
# Import: from .. import X (level=2)

parts = current_module.split('.')  # ["pkg", "sub", "module"]
is_init = filename == "__init__.py"

if is_init:
    # __init__.py: module IS the package
    base_parts = parts
else:
    # Regular file: package is parent
    base_parts = parts[:-1]  # ["pkg", "sub"]

# Go up 'level-1' directories
num_up = level - 1  # 2 - 1 = 1
base_parts = base_parts[:len(base_parts) - num_up]  # ["pkg"]

# Append import module if non-empty
if import.module:
    resolved = ".".join(base_parts + [import.module])
else:
    resolved = ".".join(base_parts)
```

**Rust Implementation Must**: Match this exactly or ~200 edges will be lost.

---

## 2. Function Index Contract

### 2.1 Purpose

Map `(module, function_name) -> file_path` for ALL functions/classes in the project.

### 2.2 Indexing Strategy (Python Reference)

```python
# For file: pkg/core.py

# Derive module name from file path
module_name = "pkg.core"           # Full module path
simple_module = "core"             # Simple name (file stem)

# Index BOTH forms for each function
for func in parse_functions(file):
    index[(module_name, func.name)] = "pkg/core.py"
    index[(simple_module, func.name)] = "pkg/core.py"
    index[f"{module_name}.{func.name}"] = "pkg/core.py"  # String key
    index[f"{simple_module}.{func.name}"] = "pkg/core.py"
```

**Why Both Forms?**

- **Full module name**: Handles absolute imports (`from pkg.core import foo`)
- **Simple module name**: Handles short imports (`from core import foo` when in same package)

### 2.3 Class Method Indexing

```python
# For class User in models.py:
index[("models", "User")] = "models.py"
index[("models", "User.save")] = "models.py"  # Method
index[("models", "User.__init__")] = "models.py"
```

**Rust Equivalent**:
```rust
func_index.insert("models", "User", ...);
func_index.insert("models", "User.save", ...);  // Qualified method name
func_index.insert("models", "save", ...);        // Also index bare method name
```

---

## 3. Call Resolution Contract

### 3.1 Input: CallSite

```rust
pub struct CallSite {
    target: String,           // "process", "user.save", "utils.helper"
    call_type: CallType,      // Direct, Attr, Method, etc.
    caller: String,           // "main"
    receiver: Option<String>, // "user" for method calls
}
```

### 3.2 Resolution Algorithm (Python Reference)

#### 3.2.1 Direct Call (`process()`)

```python
# Call: process()
# Import map: {"process": ("helper", "process")}

if call_target in import_map:
    module, orig_name = import_map[call_target]  # ("helper", "process")
    
    # Try simple module first
    simple_module = module.split('.')[-1]  # "helper"
    key = (simple_module, orig_name)       # ("helper", "process")
    if key in func_index:
        dst_file = func_index[key]          # "helper.py"
        add_edge(src_file, caller, dst_file, orig_name)
    
    # Fallback to full module
    else:
        key = (module, orig_name)           # ("helper", "process")
        if key in func_index:
            dst_file = func_index[key]
            add_edge(src_file, caller, dst_file, orig_name)
```

**Rust Must**: Check BOTH simple and full module names.

#### 3.2.2 Attribute Call (`obj.method()`)

```python
# Call: utils.helper()
# Module imports: {"utils": "pkg.utils"}

parts = call_target.split('.', 1)  # ["utils", "helper"]
if len(parts) == 2:
    obj, method = parts
    if obj in module_imports:
        module = module_imports[obj]         # "pkg.utils"
        simple_module = module.split('.')[-1]  # "utils"
        key = (simple_module, method)        # ("utils", "helper")
        if key in func_index:
            dst_file = func_index[key]        # "pkg/utils.py"
            add_edge(src_file, caller, dst_file, method)
```

**Rust Must**: Split on `.` and check if first part is an imported module.

#### 3.2.3 Method Call (`user.save()`)

```python
# Call: user.save()
# Receiver type: User (from type inference or annotation)
# Import map: {"User": ("models", "User")}

# Look up User.save in class index
for (module, name), file_path in func_index.items():
    if name == "User.save":  # Qualified method name
        add_edge(src_file, caller, file_path, "save")
```

**Rust Must**: Support qualified method names (`Class.method`) in func_index.

---

## 4. Edge Cases & Invariants

### 4.1 Star Imports

```python
from utils import *
```

**Python Behavior**: Parse `__all__` from `utils.py` to expand wildcard.

**Rust**: Currently unimplemented. This is acceptable but should log a warning.

### 4.2 Aliased Imports

```python
from utils import process as proc
```

**Behavior**:
- `import_map["proc"]` → `("utils", "process")`
- `import_map["process"]` → `("utils", "process")` (ALSO keep original)

**Rust Bug**: If Rust doesn't map BOTH names, calls using the original name will fail.

### 4.3 Same-File Calls (Intra-File)

```python
def main():
    helper()  # Same file

def helper():
    pass
```

**Resolution**: Add edge `(file, "main", file, "helper")` without import lookup.

**Invariant**: Intra-file calls should never require import resolution.

### 4.4 Module-Level Calls

```python
# At module level (not in any function)
result = process()
```

**Python**: Groups under synthetic `"<module>"` caller.

**Rust**: May skip module-level code. This is a known gap.

### 4.5 Re-Exports

```python
# api.py
from .core import process
```

```python
# main.py
from api import process  # Actually defined in core.py
```

**Expected**: Edge points to `core.py`, not `api.py`.

**Python**: Uses `ReExportTracer` to follow the chain.

**Rust**: Currently doesn't handle re-exports (causes undercounting).

---

## 5. Test Cases (Reference Codebase)

### 5.1 Known Expected Behavior

**Codebase**: `/tmp/llm-tldr-test/tldr`

| Metric | Python (Correct) | Rust (Broken) |
|--------|------------------|---------------|
| Cross-file edges | ~650 | 19 |
| `build_project_call_graph` callers | 3 | 0-1 |
| Total functions indexed | ~1500 | Similar |

### 5.2 Specific Test: `build_project_call_graph`

**Location**: `tldr/cross_file_calls.py:build_project_call_graph`

**Expected Callers** (found by Python):
1. `tldr/api.py` (exports for public API)
2. `tldr/cli.py` (CLI entry point)
3. `tldr/tests/test_cross_file.py` (test suite)

**Rust Behavior**: Finds 0-1 callers (missed due to import resolution bugs).

### 5.3 Test: Relative Import

```python
# tldr/ast_extractor.py
from .workspace import filter_paths

# Expected resolution:
# Module: tldr.ast_extractor
# Import: from . import workspace.filter_paths
# Resolved: tldr.workspace.filter_paths
# File: tldr/workspace.py
```

**Rust Bug**: May resolve to wrong path or fail entirely.

---

## 6. Performance Characteristics

### 6.1 Function Index Size

**Python**: ~3000 entries for a 1500-function codebase (2x for module name duplication).

**Rust**: Should be similar. If Rust index is much smaller, it's under-indexing.

### 6.2 Cache Hit Rate (ImportResolver)

**Expected**: 60-80% cache hit rate for import resolution on repeated imports.

**Rust**: If < 50%, cache key construction may be broken.

### 6.3 Timing

**Python**: ~2-3 seconds for 1500 functions.

**Rust**: Should be faster (~500ms) due to parallelism. If slower, parallelism isn't working.

---

## 7. Common Bugs in Rust Implementation

### 7.1 **Import Map Not Populated**

**Symptom**: All cross-file edges are 0.

**Check**: After parsing imports, log `import_map.len()`. Should be > 0 for any file with imports.

**Fix**: Ensure `ImportResolver::resolve()` is actually called and results stored.

### 7.2 **Module Name Mismatch**

**Symptom**: 90% of edges are missing.

**Check**: Compare Python module derivation (`pkg/core.py` → `pkg.core`) to Rust.

**Fix**: Ensure Rust converts file paths to module names the same way.

### 7.3 **Only Checking Full Module Names**

**Symptom**: 50% of edges missing.

**Check**: When resolving `process()`, Rust may only check `("pkg.helper", "process")` and miss `("helper", "process")`.

**Fix**: Index BOTH forms and check both during resolution.

### 7.4 **Intra-File Edges Not Added**

**Symptom**: Edges are added but count is still low.

**Check**: Are same-file calls being treated as cross-file?

**Fix**: Add intra-file edges directly without import lookup.

### 7.5 **Relative Import Level Off-by-One**

**Symptom**: Relative imports resolve to wrong files.

**Check**: For `from .. import X`, does `level=2` go up 1 directory (Python) or 2 (bug)?

**Fix**: Follow Python's `level - 1` logic for directory traversal.

---

## 8. Acceptance Criteria

### 8.1 Parity Test

```bash
# Python
cd /tmp/llm-tldr-test
python -c "from tldr.cross_file_calls import build_project_call_graph; g = build_project_call_graph('.', 'python'); print(len(g.edges))"
# Output: ~650

# Rust
tldr calls /tmp/llm-tldr-test --format json | jq '.cross_file_edges | length'
# Output: Should be ~650 (±10%)
```

### 8.2 Specific Function Test

```bash
# Python: Find callers of build_project_call_graph
python -c "
from tldr.cross_file_calls import build_project_call_graph
g = build_project_call_graph('/tmp/llm-tldr-test', 'python')
callers = [e for e in g.edges if e[3] == 'build_project_call_graph']
print(f'Callers: {len(callers)}')
for e in callers:
    print(f'  {e[0]}:{e[1]} -> {e[2]}:{e[3]}')
"
# Output: 3 callers

# Rust equivalent
tldr calls /tmp/llm-tldr-test --format json | jq '.edges[] | select(.target == "build_project_call_graph")'
# Output: Should show same 3 callers
```

### 8.3 Regression Tests

After fixing, these should remain true:

- ✅ Intra-file calls still work
- ✅ No duplicate edges (same edge added twice)
- ✅ No self-edges (`main -> main` unless it's actual recursion)
- ✅ Edge file paths are relative to project root
- ✅ Parse errors don't crash (should log and continue)

---

## 9. Implementation Checklist

Use this checklist when fixing the Rust implementation:

- [ ] **Import Resolution**
  - [ ] `ImportDef` parsed correctly from AST
  - [ ] Relative imports use PEP 328 logic
  - [ ] Absolute imports work
  - [ ] Aliases map to BOTH aliased and original names
  - [ ] Module imports (non-`from`) are tracked separately
  
- [ ] **Function Index**
  - [ ] Both simple and full module names indexed
  - [ ] Classes indexed
  - [ ] Class methods indexed as `Class.method`
  - [ ] String keys work (e.g., `"pkg.core.func"`)
  
- [ ] **Call Resolution**
  - [ ] Direct calls check import_map
  - [ ] Attribute calls split on `.` and check module_imports
  - [ ] Method calls use qualified names
  - [ ] Intra-file calls bypass import lookup
  - [ ] Both simple and full module names checked
  
- [ ] **Edge Cases**
  - [ ] Star imports log warning (don't crash)
  - [ ] Re-exports handled (or documented as limitation)
  - [ ] Module-level code handled (or skipped intentionally)
  - [ ] Parse errors logged, not fatal
  
- [ ] **Testing**
  - [ ] Parity test passes (~650 edges)
  - [ ] Specific function test passes (3 callers)
  - [ ] Regression tests pass
  - [ ] No performance regression

---

## 10. Reference Implementation Locations

### Python (Working)
- **Import Parsing**: `cross_file_calls.py:parse_imports()` (line ~220)
- **Index Building**: `cross_file_calls.py:build_function_index()` (line ~1891)
- **Call Resolution**: `cross_file_calls.py:_build_python_call_graph()` (line ~3318)
- **Relative Imports**: `cross_file_calls.py:_resolve_relative_import()` (line ~470)

### Rust (Broken)
- **Import Parsing**: `builder_v2.rs` + language handlers
- **Import Resolution**: `import_resolver.rs:resolve()`
- **Index Building**: `module_index.rs`
- **Call Resolution**: `builder_v2.rs:resolve_cross_file_calls()`

---

## 11. Debugging Guide

### Step 1: Verify Import Parsing

```rust
// Add after import parsing
eprintln!("File: {}, Imports: {}", file, imports.len());
for imp in &imports {
    eprintln!("  {:?}", imp);
}
```

**Expected**: Every Python file with imports should have `imports.len() > 0`.

### Step 2: Verify Import Resolution

```rust
// After building import_map
eprintln!("Import map size: {}", import_map.len());
for (name, (module, orig)) in &import_map {
    eprintln!("  {} -> ({}, {})", name, module, orig);
}
```

**Expected**: For `from helper import process`, should see `process -> (helper, process)`.

### Step 3: Verify Function Index

```rust
// After building func_index
eprintln!("Function index size: {}", func_index.len());
let sample: Vec<_> = func_index.iter().take(10).collect();
for (key, path) in sample {
    eprintln!("  {:?} -> {:?}", key, path);
}
```

**Expected**: For 1500 functions, index size should be ~3000 (double-indexed).

### Step 4: Verify Call Resolution

```rust
// In call resolution loop
if let Some(resolved_file) = resolve_call(&call, &import_map, &func_index) {
    eprintln!("RESOLVED: {} -> {}", call.target, resolved_file);
} else {
    eprintln!("UNRESOLVED: {}", call.target);
}
```

**Expected**: Most calls should resolve. If 90%+ are unresolved, import_map is wrong.

---

## 12. Summary

**The Core Issue**: Rust implementation likely has one or more of:

1. **Import resolution not populating `import_map`** (most likely)
2. **Module name derivation mismatches Python** (very likely)
3. **Only checking full module names, not simple names** (likely)
4. **Intra-file edges not being added** (possible)
5. **Relative imports using wrong algorithm** (possible)

**The Fix**: Follow the Python implementation's logic exactly for:
- Module name derivation from file paths
- Double-indexing (simple + full module names)
- Import map population (including aliases)
- Call resolution (check both simple and full)

**Success Metric**: ~650 cross-file edges on the test codebase.
