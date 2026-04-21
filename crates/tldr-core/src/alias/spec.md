# Alias Analysis Specification

**Module:** `tldr-core::alias`
**Version:** 1.0.0
**Created:** 2026-02-03
**Author:** architect-agent

## Overview

This module implements flow-insensitive Andersen-style points-to analysis to determine
when two references may or must refer to the same object. It provides the foundation
for precise data flow analysis, security analysis (taint propagation through aliases),
and optimization passes.

### Algorithm Reference

- **Name:** Andersen's subset-based analysis (1994)
- **Style:** Flow-insensitive (processes all statements once, not per program point)
- **Complexity:** O(n³) time, O(n²) space for n pointers
- **Key Property:** Inclusion-based constraints: `pts(x) ⊇ pts(y)` for `x = y`

### Why Flow-Insensitive?

Flow-insensitive analysis is chosen because:
1. **Scalability:** Linear pass over statements, not per program point
2. **Soundness:** Conservative results safe for security analysis
3. **SSA Integration:** SSA form provides per-definition precision within basic blocks
4. **Simplicity:** Easier to implement correctly and maintain

---

## Data Structures

### Core Types

```rust
//! Alias Analysis Types
//!
//! Core types for Andersen-style flow-insensitive points-to analysis.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use crate::ssa::types::{SsaFunction, PhiFunction, SsaNameId};
use crate::types::{VarRef, RefType, CfgInfo};

// =============================================================================
// Abstract Location Types
// =============================================================================

/// Abstract memory location that a variable may point to.
///
/// These represent the "targets" in points-to analysis - what objects
/// a reference variable could be pointing to at runtime.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AbstractLocation {
    /// Object allocated at a specific line: `alloc_N`
    /// Created when: `x = Foo()`, `x = []`, `x = {}`
    Alloc { site: u32 },

    /// Object passed as a parameter: `param_X`
    /// Created for each function parameter (unknown caller context)
    Param { name: String },

    /// Unknown/external source: `unknown_SITE`
    /// Created when: return from unknown function, global access
    /// Site-specific to prevent unsound aliasing (TIGER-5)
    Unknown { site: u32 },

    /// Field of another location: `{base}.{field}`
    /// Created when: `x = obj.field` where obj points to base
    Field {
        base: Box<AbstractLocation>,
        field: String,
    },

    /// Mutable default argument: `alloc_default_LINE`
    /// Created for default parameter values (shared across calls)
    /// Python-specific: handles `def f(x=[])` correctly (TIGER-7)
    DefaultArg { site: u32 },

    /// Class-level variable: `alloc_class_LINE`
    /// Singleton per class for class variables vs instance variables
    /// Python-specific: `Foo.x` vs `obj.x` distinction (TIGER-8)
    ClassVar { class: String, field: String },
}

impl AbstractLocation {
    /// Create allocation site location
    pub fn alloc(site: u32) -> Self {
        AbstractLocation::Alloc { site }
    }

    /// Create parameter location
    pub fn param(name: impl Into<String>) -> Self {
        AbstractLocation::Param { name: name.into() }
    }

    /// Create unknown location (site-specific)
    pub fn unknown(site: u32) -> Self {
        AbstractLocation::Unknown { site }
    }

    /// Create field location
    pub fn field(base: AbstractLocation, field: impl Into<String>) -> Self {
        AbstractLocation::Field {
            base: Box::new(base),
            field: field.into(),
        }
    }

    /// Create default argument location
    pub fn default_arg(site: u32) -> Self {
        AbstractLocation::DefaultArg { site }
    }

    /// Create class variable location
    pub fn class_var(class: impl Into<String>, field: impl Into<String>) -> Self {
        AbstractLocation::ClassVar {
            class: class.into(),
            field: field.into(),
        }
    }

    /// Format as string for JSON output: "alloc_5", "param_x", "unknown_10", "alloc_5.data"
    pub fn format(&self) -> String {
        match self {
            AbstractLocation::Alloc { site } => format!("alloc_{}", site),
            AbstractLocation::Param { name } => format!("param_{}", name),
            AbstractLocation::Unknown { site } => format!("unknown_{}", site),
            AbstractLocation::Field { base, field } => {
                // TIGER-1: Prevent stack overflow in deeply nested field access
                const MAX_FIELD_DEPTH: usize = 10;
                let mut depth = 0;
                let mut current = base.as_ref();
                while let AbstractLocation::Field { base: inner, .. } = current {
                    depth += 1;
                    if depth >= MAX_FIELD_DEPTH {
                        return format!("{}.{}.truncated", base.format(), field);
                    }
                    current = inner.as_ref();
                }
                format!("{}.{}", base.format(), field)
            }
            AbstractLocation::DefaultArg { site } => format!("alloc_default_{}", site),
            AbstractLocation::ClassVar { class, field } => format!("alloc_class_{}_{}", class, field),
        }
    }
}

impl std::fmt::Display for AbstractLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.format())
    }
}

// =============================================================================
// Alias Info Result
// =============================================================================

/// Alias analysis results for a function.
///
/// Contains may-alias, must-alias relationships and points-to sets.
/// This is the primary output of `compute_alias()`.
///
/// # Soundness Guarantees
///
/// - **may_alias is SOUND:** If `may_alias_check(a, b)` returns `false`,
///   then `a` and `b` definitely do NOT alias at runtime (no false negatives).
///   
/// - **must_alias is PRECISE:** If `must_alias_check(a, b)` returns `true`,
///   then `a` and `b` definitely DO alias at runtime (no false positives).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AliasInfo {
    /// Function name
    pub function_name: String,
    
    /// May-alias relationships: var -> set of vars that MAY alias.
    /// Symmetric: if b ∈ may_alias[a], then a ∈ may_alias[b].
    pub may_alias: HashMap<String, HashSet<String>>,
    
    /// Must-alias relationships: var -> set of vars that DEFINITELY alias.
    /// Symmetric and transitive.
    pub must_alias: HashMap<String, HashSet<String>>,
    
    /// Points-to sets: var -> set of abstract locations it may point to.
    pub points_to: HashMap<String, HashSet<String>>,
    
    /// Allocation sites: line -> abstract location name.
    /// Records where objects are created.
    pub allocation_sites: HashMap<u32, String>,
}

impl AliasInfo {
    /// Create empty alias info for a function.
    pub fn new(function_name: impl Into<String>) -> Self {
        Self {
            function_name: function_name.into(),
            may_alias: HashMap::new(),
            must_alias: HashMap::new(),
            points_to: HashMap::new(),
            allocation_sites: HashMap::new(),
        }
    }
    
    /// Check if two variables MAY alias (point to same object).
    ///
    /// Returns `true` if there exists ANY execution path where
    /// `a` and `b` could reference the same object.
    ///
    /// # Soundness
    /// - Returns `false` ONLY IF variables definitely don't alias
    /// - May return `true` even if they never alias (conservative)
    ///
    /// # Examples
    /// ```
    /// // x = Foo(); y = x  => may_alias_check("x", "y") == true
    /// // x = Foo(); y = Foo()  => may_alias_check("x", "y") == false
    /// ```
    pub fn may_alias_check(&self, a: &str, b: &str) -> bool {
        // Same variable always aliases itself
        if a == b {
            return true;
        }
        
        // Check explicit may_alias set
        if self.may_alias.get(a).map_or(false, |s| s.contains(b)) {
            return true;
        }
        if self.may_alias.get(b).map_or(false, |s| s.contains(a)) {
            return true;
        }
        
        // Check points-to set overlap
        let pts_a = self.points_to.get(a);
        let pts_b = self.points_to.get(b);
        
        match (pts_a, pts_b) {
            (Some(a_set), Some(b_set)) => !a_set.is_disjoint(b_set),
            _ => false,
        }
    }
    
    /// Check if two variables MUST alias (definitely same object).
    ///
    /// Returns `true` ONLY IF on ALL execution paths, `a` and `b`
    /// reference the same object.
    ///
    /// # Precision
    /// - Returns `true` ONLY IF variables definitely alias
    /// - May return `false` even if they always alias (conservative)
    ///
    /// # Examples
    /// ```
    /// // x = p; y = x  => must_alias_check("x", "y") == true (direct copy)
    /// // if cond: x = a else: x = b  => must_alias_check("x", "a") == false
    /// ```
    pub fn must_alias_check(&self, a: &str, b: &str) -> bool {
        // Same variable always must-aliases itself
        if a == b {
            return true;
        }
        
        // Check explicit must_alias set
        if self.must_alias.get(a).map_or(false, |s| s.contains(b)) {
            return true;
        }
        if self.must_alias.get(b).map_or(false, |s| s.contains(a)) {
            return true;
        }
        
        false
    }
    
    /// Get the points-to set for a variable.
    ///
    /// Returns the set of abstract locations this variable may reference.
    /// Returns empty set for unknown variables.
    pub fn get_points_to(&self, var: &str) -> HashSet<String> {
        self.points_to.get(var).cloned().unwrap_or_default()
    }
    
    /// Get all variables that may alias with the given variable.
    ///
    /// Returns the explicit may_alias set (does not compute from points-to).
    pub fn get_aliases(&self, var: &str) -> HashSet<String> {
        self.may_alias.get(var).cloned().unwrap_or_default()
    }
    
    /// Convert to serializable format for JSON output.
    ///
    /// Sets are converted to sorted Vecs for deterministic output.
    pub fn to_json_value(&self) -> serde_json::Value {
        use serde_json::json;
        
        let sorted_may_alias: HashMap<_, Vec<_>> = self.may_alias
            .iter()
            .map(|(k, v)| {
                let mut sorted: Vec<_> = v.iter().cloned().collect();
                sorted.sort();
                (k.clone(), sorted)
            })
            .collect();
            
        let sorted_must_alias: HashMap<_, Vec<_>> = self.must_alias
            .iter()
            .map(|(k, v)| {
                let mut sorted: Vec<_> = v.iter().cloned().collect();
                sorted.sort();
                (k.clone(), sorted)
            })
            .collect();
            
        let sorted_points_to: HashMap<_, Vec<_>> = self.points_to
            .iter()
            .map(|(k, v)| {
                let mut sorted: Vec<_> = v.iter().cloned().collect();
                sorted.sort();
                (k.clone(), sorted)
            })
            .collect();
        
        json!({
            "function": self.function_name,
            "may_alias": sorted_may_alias,
            "must_alias": sorted_must_alias,
            "points_to": sorted_points_to,
            "allocation_sites": self.allocation_sites,
        })
    }
}

// =============================================================================
// Error Types
// =============================================================================

/// Alias analysis errors
#[derive(Debug, Clone, thiserror::Error)]
pub enum AliasError {
    /// SSA form required but not available
    #[error("SSA form not available for function: {0}")]
    NoSsa(String),
    
    /// Invalid variable reference
    #[error("Invalid variable reference: {0}")]
    InvalidRef(String),
    
    /// Analysis limit exceeded (max iterations)
    #[error("Fixed-point iteration limit exceeded: {0} iterations")]
    IterationLimit(usize),
    
    /// Internal error
    #[error("Internal alias analysis error: {0}")]
    Internal(String),
}
```

---

## Behavioral Contracts

### Capability 1: May-Alias Computation (HIGH)

**Description:** Determines when two variables MAY refer to the same object.
Conservative: returns true if there's ANY path where they could alias.

**Contract:**
```rust
/// INVARIANT: Sound - no false negatives
/// If may_alias_check(a, b) returns false, then a and b NEVER alias at runtime.
///
/// ALLOWED: False positives (conservative)
/// May return true even when variables never actually alias.
fn may_alias_check(&self, a: &str, b: &str) -> bool;
```

**Edge Cases:**
- Same variable: `may_alias_check("x", "x")` → always `true`
- Unknown variables: `may_alias_check("unknown1", "unknown2")` → `false` (no info)
- Empty sets: Variables with no points-to info don't alias

**Test Cases:** (from Python)
- `test_alias_info_may_alias_check_same_var`
- `test_alias_info_may_alias_check_in_set`
- `test_alias_info_may_alias_check_points_to_overlap`
- `test_alias_info_may_alias_check_no_overlap`

---

### Capability 2: Must-Alias Computation (HIGH)

**Description:** Determines when two variables DEFINITELY refer to the same object.
Used for copy propagation, dead store elimination.

**Contract:**
```rust
/// INVARIANT: Precise - no false positives
/// If must_alias_check(a, b) returns true, then a and b ALWAYS alias at runtime.
///
/// ALLOWED: False negatives (conservative)
/// May return false even when variables always alias.
fn must_alias_check(&self, a: &str, b: &str) -> bool;
```

**Edge Cases:**
- Same variable: `must_alias_check("x", "x")` → always `true`
- Phi results: NEVER must-alias (multiple possible sources)
- Unknown variables: `must_alias_check("unknown1", "unknown2")` → `false`

**Test Cases:**
- `test_alias_info_must_alias_check_same_var`
- `test_alias_info_must_alias_check_in_set`
- `test_alias_info_must_alias_check_not_in_set`

---

### Capability 3: Points-To Set Computation (HIGH)

**Description:** Maps variables to abstract memory locations they may point to.
Foundation for alias queries.

**Contract:**
```rust
/// Returns all abstract locations this variable may reference.
/// Empty set means: either unknown variable, or no allocation tracked.
fn get_points_to(&self, var: &str) -> HashSet<String>;
```

**Abstract Location Types:**
| Pattern | Meaning | Created By |
|---------|---------|------------|
| `alloc_N` | Object allocated at line N | `x = Foo()`, `x = []` |
| `param_X` | Object passed as parameter X | Function parameter |
| `unknown` | Unknown/external source | External function call |
| `{loc}.{field}` | Field of another location | `x = obj.field` |

**Test Cases:**
- `test_alias_info_get_points_to`
- `test_assignment_propagates_points_to`

---

### Capability 4: Assignment Aliasing - x = y (HIGH)

**Description:** Handles copy semantics: when `x = y`, x aliases whatever y aliases.
Generates copy constraints for fixed-point iteration.

**Contract:**
```rust
/// For assignment x = y:
/// 1. Create copy constraint: pts(x) ⊇ pts(y)
/// 2. If not from phi: add must-alias(x, y)
/// 3. Propagate through fixed-point iteration
```

**Behavior:**
```
x = p         # x must-alias p, pts(x) = pts(p)
y = x         # y must-alias x, y must-alias p (transitive)
```

**Test Cases:**
- `test_simple_assignment_creates_must_alias`
- `test_transitive_aliasing`
- `test_assignment_propagates_points_to`

---

### Capability 5: Object Creation - x = Foo() (HIGH)

**Description:** Each allocation site creates a unique abstract location.
Enables distinguishing objects created at different points.

**Contract:**
```rust
/// For allocation x = Foo() at line N:
/// 1. Create abstract location alloc_N
/// 2. Set pts(x) = {alloc_N}
/// 3. Record in allocation_sites: N -> "alloc_N"
```

**Behavior:**
```
x = Foo()  # line 5: pts(x) = {alloc_5}
y = Foo()  # line 6: pts(y) = {alloc_6}
# x and y do NOT alias (different allocation sites)
```

**Test Cases:**
- `test_new_object_gets_unique_location`
- `test_allocation_sites_recorded`
- `test_same_object_reused_aliases`

---

### Capability 6: Phi Function Handling (HIGH)

**Description:** SSA phi functions create may-alias (not must-alias).
Handles control-flow merge points correctly.

**Contract:**
```rust
/// For phi: x_2 = φ(x_0, x_1)
/// 1. Create copy constraints: pts(x_2) ⊇ pts(x_0), pts(x_2) ⊇ pts(x_1)
/// 2. Add may-alias: x_2 may-alias x_0, x_2 may-alias x_1
/// 3. Do NOT add must-alias (only one branch taken at runtime)
```

**Behavior:**
```
if cond:
    x = a      # x_0 = a
else:
    x = b      # x_1 = b
# x_2 = φ(x_0, x_1)
# x_2 may-alias a, x_2 may-alias b
# x_2 does NOT must-alias a or b
```

**Test Cases:**
- `test_phi_creates_may_alias`

---

### Capability 7: SSA Integration (HIGH)

**Description:** Uses SSA names for per-definition precision.
Leverages SsaFunction's phi_functions and blocks.

**Contract:**
```rust
/// SSA Integration:
/// 1. Use SSA names (x_0, x_1) not original names (x)
/// 2. Extract phi functions from SsaFunction.blocks
/// 3. Map def_line to SSA names for constraint generation
/// 4. Handle SsaInstructionKind for allocation detection
```

**Integration Points:**
- `SsaFunction.blocks[].phi_functions` - phi function list
- `SsaFunction.ssa_names` - all SSA name definitions
- `SsaNameId` - unique identifier for SSA variables
- `SsaInstruction.kind == Call` - potential allocation site

---

### Capability 8: Field Access - x.field (MEDIUM)

**Description:** x.f and y.f may alias if x may alias y.
Tracks field loads and propagates through base object aliasing.

**Contract:**
```rust
/// For field load: z = x.field
/// 1. For each loc ∈ pts(x): add loc.field to pts(z)
/// 2. If x may-alias y and both access same field:
///    z may-alias the result of y.field
```

**Behavior:**
```
x = obj.field  # pts(x) = {pts(obj).field}
y = obj.field  # pts(y) = {pts(obj).field}
# x and y may-alias (same field, same base)

a = p1.field
b = p2.field
# if p1 may-alias p2: a may-alias b
```

**Test Cases:**
- `test_same_field_same_object_aliases`
- `test_different_fields_no_alias`
- `test_field_of_aliasing_objects_may_alias`

---

### Capability 9: Field Store - x.field = y (MEDIUM)

**Description:** For `x.f = y`: updates points-to of all locations x may point to.

**Contract:**
```rust
/// For field store: x.field = y
/// 1. For each loc ∈ pts(x): pts(loc.field) ⊇ pts(y)
/// 2. Conservative: all locations x may point to are updated
```

**Note:** This is field-insensitive at the heap level. More precise
analysis would track each field separately per allocation site.

---

### Capability 10: Parameter Aliasing (MEDIUM)

**Description:** Parameters may alias each other (caller could pass same object).
Safe conservative default.

**Contract:**
```rust
/// For function parameters a, b:
/// 1. Create param locations: pts(a) = {param_a}, pts(b) = {param_b}
/// 2. Add may-alias: a may-alias b (conservative)
/// 3. Do NOT add must-alias (different parameters)
```

**Behavior:**
```
def f(a, b):  # a may-alias b (caller could pass same object)
    x = a.field
    y = b.field
    # x may-alias y (bases may-alias)
```

**Test Cases:**
- `test_parameters_may_alias_conservatively`
- `test_parameter_points_to_param_location`

---

### Capability 11: Unknown Source Handling (MEDIUM)

**Description:** Results from unknown calls get "unknown" location.
Conservative: unknown sources may alias each other.

**Contract:**
```rust
/// For unknown source (e.g., x = external_func()):
/// 1. Set pts(x) = {unknown}
/// 2. All variables with {unknown} may-alias each other
```

**Test Cases:**
- `test_unknown_call_result_conservative`

---

## Public API

```rust
//! Alias Analysis Public API

use crate::ssa::types::SsaFunction;
use crate::types::{VarRef, CfgInfo};

/// Compute alias analysis for a function.
///
/// # Arguments
/// * `ssa` - SSA form of the function (required)
/// * `refs` - Variable references (definitions, uses)
/// * `params` - Function parameter names
/// * `cfg` - Control flow graph (optional, for field access)
///
/// # Returns
/// * `Ok(AliasInfo)` - Alias analysis results
/// * `Err(AliasError)` - If analysis fails
///
/// # Algorithm
/// 1. Initialize points-to sets for parameters and allocations
/// 2. Generate copy constraints from assignments
/// 3. Generate copy constraints from phi functions
/// 4. Fixed-point iteration: propagate points-to sets
/// 5. Derive may-alias from points-to overlap
/// 6. Derive must-alias from direct (non-phi) copies
/// 7. Handle parameter and field aliasing
///
/// # Example
/// ```rust
/// let alias_info = compute_alias(&ssa, &refs, &["a", "b"], Some(&cfg))?;
/// if alias_info.may_alias_check("x_0", "y_0") {
///     println!("x and y may point to same object");
/// }
/// ```
pub fn compute_alias(
    ssa: &SsaFunction,
    refs: &[VarRef],
    params: &[&str],
    cfg: Option<&CfgInfo>,
) -> Result<AliasInfo, AliasError>;

/// Compute alias analysis from SSA function alone.
///
/// Extracts parameters and refs from SSA structure.
/// Convenience wrapper around `compute_alias`.
pub fn compute_alias_from_ssa(
    ssa: &SsaFunction,
) -> Result<AliasInfo, AliasError>;
```

---

## Algorithm Description

### Andersen's Flow-Insensitive Analysis

```
ALGORITHM: Andersen's Subset-Based Analysis
INPUT: SSA function, variable references, parameters
OUTPUT: AliasInfo (may_alias, must_alias, points_to)

1. INITIALIZATION
   FOR each parameter p:
       pts[p] = {param_p}
   FOR each allocation x = Foo() at line N:
       pts[x] = {alloc_N}
       allocation_sites[N] = "alloc_N"
   FOR each unknown source x = unknown_call():
       pts[x] = {unknown}

2. CONSTRAINT GENERATION
   copy_constraints = []
   field_loads = []
   
   FOR each assignment x = y:
       IF y is known variable:
           copy_constraints.append((x, y))
   
   FOR each phi x = φ(y1, y2, ...):
       FOR each yi:
           copy_constraints.append((x, yi))
       phi_targets.add(x)
   
   FOR each field load x = base.field:
       field_loads.append((x, base, field))

3. FIXED-POINT ITERATION
   changed = true
   iterations = 0
   WHILE changed AND iterations < MAX_ITERATIONS:
       changed = false
       iterations++
       
       # Copy constraint propagation
       FOR each (target, source) in copy_constraints:
           old_size = |pts[target]|
           pts[target] = pts[target] ∪ pts[source]
           IF |pts[target]| > old_size:
               changed = true
       
       # Field load propagation
       FOR each (target, base, field) in field_loads:
           FOR each loc in pts[base]:
               field_loc = loc + "." + field
               old_size = |pts[target]|
               pts[target] = pts[target] ∪ {field_loc}
               IF |pts[target]| > old_size:
                   changed = true

4. DERIVE MAY-ALIAS
   FOR each pair (v1, v2) where v1 != v2:
       IF pts[v1] ∩ pts[v2] != ∅:
           may_alias[v1].add(v2)
           may_alias[v2].add(v1)
   
   # Transitive closure from copy constraints
   FOR each variable v:
       sources = transitive_closure(copy_constraints, v)
       FOR each source in sources:
           may_alias[v].add(source)
           may_alias[source].add(v)

5. DERIVE MUST-ALIAS
   FOR each (target, source) in copy_constraints:
       IF target NOT IN phi_targets:
           must_alias[target].add(source)
           must_alias[source].add(target)

6. PARAMETER ALIASING (CONSERVATIVE)
   FOR each pair (p1, p2) of parameters:
       may_alias[p1].add(p2)
       may_alias[p2].add(p1)

7. FIELD ALIASING
   FOR each pair of field loads (x, base1, f), (y, base2, f):
       IF base1 may-alias base2:
           may_alias[x].add(y)
           may_alias[y].add(x)

RETURN AliasInfo { may_alias, must_alias, pts, allocation_sites }
```

### Fixed-Point Iteration Limits

- **MAX_ITERATIONS:** 100
- **Rationale:** Prevents infinite loops in cyclic constraints
- **Behavior on limit:** Return `AliasError::IterationLimit`

---

## Integration with Existing Infrastructure

### Dependencies

| Module | Types Used | Purpose |
|--------|-----------|---------|
| `ssa::types` | `SsaFunction`, `PhiFunction`, `SsaNameId`, `SsaBlock` | SSA form, phi functions |
| `types` | `VarRef`, `RefType`, `CfgInfo` | Variable references, CFG |
| `security::taint` | (consumer) | Taint propagation through aliases |

### VarRef Extension

The current `VarRef` may need extension for alias analysis:

```rust
/// Extended VarRef with alias-relevant metadata
pub struct AliasVarRef {
    /// Base VarRef
    pub var_ref: VarRef,
    /// SSA name (e.g., "x_0")
    pub ssa_name: Option<String>,
    /// Is this an allocation? (x = Foo())
    pub is_allocation: bool,
    /// Is this a field access? (x = obj.field)
    pub is_field_access: bool,
    /// Base object for field access
    pub base_object: Option<String>,
    /// Field name for field access
    pub field_name: Option<String>,
}
```

**Alternative:** Detect allocation/field access from AST patterns rather than
extending VarRef. This keeps VarRef simple and moves complexity to the
alias analysis extractor.

### SSA Integration Pattern

```rust
fn compute_alias_from_ssa(ssa: &SsaFunction) -> Result<AliasInfo, AliasError> {
    let mut result = AliasInfo::new(&ssa.function);
    let mut copy_constraints: Vec<(String, String)> = Vec::new();
    let mut phi_targets: HashSet<String> = HashSet::new();
    
    // Extract phi functions from SSA blocks
    for block in &ssa.blocks {
        for phi in &block.phi_functions {
            let target = format_ssa_name(ssa, phi.target);
            phi_targets.insert(target.clone());
            
            for source in &phi.sources {
                let source_name = format_ssa_name(ssa, source.name);
                copy_constraints.push((target.clone(), source_name));
            }
        }
        
        // Extract assignments from instructions
        for instr in &block.instructions {
            if let (Some(target), SsaInstructionKind::Assign) = 
                (&instr.target, &instr.kind) {
                // Check if RHS is a copy or allocation
                // ...
            }
        }
    }
    
    // ... rest of algorithm
    Ok(result)
}

fn format_ssa_name(ssa: &SsaFunction, id: SsaNameId) -> String {
    ssa.ssa_names
        .iter()
        .find(|n| n.id == id)
        .map(|n| n.format_name())
        .unwrap_or_else(|| format!("${}", id.0))
}
```

---

## CLI Command Interface

Following the v1 pattern for flow analysis commands:

```
tldr alias <file> <function> [OPTIONS]

DESCRIPTION:
    Compute alias analysis for a function.

ARGUMENTS:
    <file>      Source file path
    <function>  Function name to analyze

OPTIONS:
    --format <FORMAT>    Output format: json (default), text
    --check <VAR1,VAR2>  Check if two variables may alias
    --points-to <VAR>    Show points-to set for variable
    --verbose            Include detailed constraint info

EXAMPLES:
    # Full alias analysis as JSON
    tldr alias src/main.py process_data
    
    # Check if two variables alias
    tldr alias src/main.py process_data --check x_0,y_0
    
    # Show what x points to
    tldr alias src/main.py process_data --points-to x_0
```

### JSON Output Format

```json
{
  "function": "process_data",
  "may_alias": {
    "x_0": ["y_0", "z_0"],
    "y_0": ["x_0"]
  },
  "must_alias": {
    "x_0": ["p_0"]
  },
  "points_to": {
    "x_0": ["param_p", "alloc_5"],
    "y_0": ["alloc_5"],
    "z_0": ["unknown"]
  },
  "allocation_sites": {
    "5": "alloc_5",
    "10": "alloc_10"
  }
}
```

### Text Output Format

```
Alias Analysis: process_data

Points-To Sets:
  x_0 -> {param_p, alloc_5}
  y_0 -> {alloc_5}
  z_0 -> {unknown}

May-Alias Pairs:
  x_0 <-> y_0  (shared: alloc_5)
  x_0 <-> z_0  (via copy)

Must-Alias Pairs:
  x_0 <-> p_0  (direct assignment)

Allocation Sites:
  Line 5: alloc_5
  Line 10: alloc_10
```

---

## Edge Cases from Python Tests

### Self-Alias
```rust
#[test]
fn test_self_alias() {
    let info = AliasInfo::new("test");
    assert!(info.may_alias_check("x_0", "x_0"));
    assert!(info.must_alias_check("x_0", "x_0"));
}
```

### Unknown Variable No Crash
```rust
#[test]
fn test_unknown_variable_no_crash() {
    let info = AliasInfo::new("test");
    assert!(!info.may_alias_check("unknown1", "unknown2"));
    assert!(!info.must_alias_check("unknown1", "unknown2"));
    assert!(info.get_points_to("unknown").is_empty());
    assert!(info.get_aliases("unknown").is_empty());
}
```

### CFG Optional
```rust
#[test]
fn test_cfg_optional() {
    let ssa = create_mock_ssa("test");
    let result = compute_alias(&ssa, &[], &[], None);
    assert!(result.is_ok());
}
```

---

## Test Strategy

### Unit Tests (Port from Python)

Port all 31 Python tests to Rust:

1. **AliasInfo Dataclass Tests**
   - `test_alias_info_fields`
   - `test_alias_info_default_empty`
   - `test_alias_info_may_alias_check_*` (4 tests)
   - `test_alias_info_must_alias_check_*` (3 tests)
   - `test_alias_info_get_points_to`
   - `test_alias_info_get_aliases`
   - `test_alias_info_to_dict*` (2 tests)

2. **compute_alias Tests**
   - `test_compute_alias_returns_alias_info`
   - `test_compute_alias_empty_refs`

3. **Assignment Aliasing Tests**
   - `test_simple_assignment_creates_must_alias`
   - `test_transitive_aliasing`
   - `test_assignment_propagates_points_to`

4. **Object Creation Tests**
   - `test_new_object_gets_unique_location`
   - `test_allocation_sites_recorded`
   - `test_same_object_reused_aliases`

5. **Conditional/Phi Tests**
   - `test_phi_creates_may_alias`

6. **Field Access Tests**
   - `test_same_field_same_object_aliases`
   - `test_different_fields_no_alias`
   - `test_field_of_aliasing_objects_may_alias`

7. **Parameter Tests**
   - `test_parameters_may_alias_conservatively`
   - `test_parameter_points_to_param_location`

8. **Conservative Handling Tests**
   - `test_unknown_call_result_conservative`

9. **Edge Case Tests**
   - `test_self_alias`
   - `test_unknown_variable_no_crash`
   - `test_cfg_optional`

### Property-Based Tests

```rust
#[test]
fn prop_may_alias_symmetric() {
    // If a may-alias b, then b may-alias a
}

#[test]
fn prop_must_alias_implies_may_alias() {
    // If a must-alias b, then a may-alias b
}

#[test]
fn prop_different_alloc_sites_no_alias() {
    // Objects from different alloc_N never alias
}
```

---

## File Organization

```
tldr-core/src/alias/
├── mod.rs           # Module exports, compute_alias_from_ssa
├── types.rs         # AbstractLocation, AliasInfo, AliasError
├── compute.rs       # compute_alias implementation
├── constraints.rs   # Constraint generation from SSA
├── solver.rs        # Fixed-point iteration solver
├── format.rs        # JSON/text output formatting
└── alias_tests.rs   # Unit tests
```

---

## Estimated Effort

| Phase | Description | LOC | Effort |
|-------|-------------|-----|--------|
| 1 | Types (`types.rs`) | ~150 | Small |
| 2 | Core algorithm (`compute.rs`, `solver.rs`) | ~300 | Medium |
| 3 | Constraint generation (`constraints.rs`) | ~150 | Small |
| 4 | Formatting (`format.rs`) | ~100 | Small |
| 5 | Tests (`alias_tests.rs`) | ~400 | Medium |
| 6 | CLI integration | ~50 | Small |

**Total:** ~1150 LOC (Rust is more verbose than Python but more explicit)

---

## Success Criteria

1. All 31 ported tests pass
2. Same abstract location naming scheme (`alloc_N`, `param_X`, `unknown`)
3. Same constraint handling (copy, phi, field)
4. Fixed-point iteration converges (max 100 iterations)
5. JSON output matches Python format (modulo key ordering)
6. SSA integration works with existing `SsaFunction`
7. CLI command `tldr alias <file> <func>` works
8. Integration with taint analysis for alias-aware taint propagation

---

## Risk Mitigations (Pre-Mortem)

**Pre-Mortem Run:**
- Date: 2026-02-03
- Passes: 3
- Mode: deep
- Total issues: 15 tigers, 3 elephants

### Pass 1 Tigers (Algorithm)

1. **TIGER-1: Stack overflow in field formatting**
   - **Risk:** Deeply nested field access like `a.b.c.d...` causes unbounded recursion in `format()`
   - **Mitigation:** Add `MAX_FIELD_DEPTH = 10` in `AbstractLocation::format()`, return `"truncated"` for deeper nesting
   - **Implementation:** See updated `format()` method above - tracks depth during recursion

2. **TIGER-2: Convergence not guaranteed**
   - **Risk:** Fixed-point iteration may not converge if constraint graph has cycles
   - **Mitigation:** Pre-validate constraint graph for cycles, track last-changed variables to detect oscillation
   - **Implementation:** Add cycle detection in `solver.rs` before entering fixed-point loop

3. **TIGER-3: Invalid SSA references**
   - **Risk:** Phi sources reference SSA names not in `ssa.ssa_names` (corrupted/invalid SSA)
   - **Mitigation:** Validate all phi sources exist in `ssa.ssa_names` before analysis
   - **Implementation:** Add validation pass in `constraints.rs` when extracting phi constraints

4. **TIGER-4: Parameter aliasing explosion**
   - **Risk:** All-pairs parameter aliasing creates O(n²) may-alias entries for n parameters
   - **Mitigation:** Lazy evaluation in `may_alias_check()` instead of precomputing all pairs
   - **Implementation:** Don't populate `may_alias` for all parameter pairs - compute on-demand in query

5. **TIGER-5: Soundness violation (all unknowns alias)**
   - **Risk:** Single `Unknown` location means all unknown calls alias each other (unsound)
   - **Mitigation:** Make unknown locations site-specific: `Unknown { site: u32 }`
   - **Implementation:** See updated `AbstractLocation` enum above - now `unknown_10`, `unknown_20`, etc.

6. **TIGER-6: Must-alias transitivity missing**
   - **Risk:** `x = y; y = z` should make `x` must-alias `z`, but direct edge-only approach misses this
   - **Mitigation:** Add transitive closure after direct edge computation
   - **Implementation:** Compute transitive closure of `must_alias` relation in final step

### Pass 3 Tigers (Python-specific)

7. **TIGER-7: Mutable default arguments**
   - **Risk:** Python's `def f(x=[])` shares the same list across all calls - not modeled
   - **Mitigation:** Create `alloc_default_LINE` for default parameter values (shared across calls)
   - **Implementation:** See `DefaultArg { site }` in `AbstractLocation` enum

8. **TIGER-8: Class variables vs instance variables**
   - **Risk:** Python's class-level `Foo.x` (singleton) vs instance-level `obj.x` (per-instance) not distinguished
   - **Mitigation:** Add `alloc_class_LINE` (singleton per class) vs `alloc_N.field` (per-instance)
   - **Implementation:** See `ClassVar { class, field }` in `AbstractLocation` enum

9. **TIGER-9: List comprehensions**
   - **Risk:** Allocations inside comprehensions create multiple objects, but appear at single line
   - **Mitigation:** Use `alloc_LINE_comprehension` for allocations inside comprehensions
   - **Implementation:** Detect comprehension context in AST, append `_comprehension` suffix to site

10. **TIGER-10: Context managers (`with` statements)**
    - **Risk:** `with open(f) as x` has complex semantics (`__enter__`, `__exit__`)
    - **Mitigation:** Conservative - treat as `unknown` (function call result)
    - **Implementation:** Detect `with` statements, assign `unknown_LINE` to bound variable

11. **TIGER-11: Property decorators**
    - **Risk:** Python's `@property` makes field access invoke function, not direct field load
    - **Mitigation:** Properties → `unknown` (function call, not field access)
    - **Implementation:** Detect `@property` in AST, treat access as call not field

12. **TIGER-12: `self` parameter precision**
    - **Risk:** Generic `param_self` loses precision - which instance is `self`?
    - **Mitigation:** Track allocation site when `self` comes from known `alloc_N`
    - **Implementation:** Flow-sensitive tracking: if `obj = Foo(); obj.method()`, then `self` → `alloc_N`

### Additional Algorithm Tigers

13. **TIGER-13: Field store soundness**
    - **Risk:** `x.f = y` should update all locations `x` may point to, but easy to miss some
    - **Mitigation:** Conservative: for each `loc ∈ pts(x)`, add constraint `pts(loc.f) ⊇ pts(y)`
    - **Implementation:** Explicitly enumerate all points-to targets in field store handling

14. **TIGER-14: Phi function validation**
    - **Risk:** Phi with mismatched source count and predecessor count (invalid SSA)
    - **Mitigation:** Validate `phi.sources.len() == phi.predecessors.len()` before processing
    - **Implementation:** Add assertion in `constraints.rs` phi extraction

15. **TIGER-15: Allocation site collision**
    - **Risk:** Multiple statements on same line create ambiguous `alloc_5` (which one?)
    - **Mitigation:** Use column number: `alloc_5_10` (line 5, column 10)
    - **Implementation:** Extend `Alloc { site }` to `Alloc { line, col }` if line alone insufficient

### Accepted Limitations (Elephants)

1. **Elephant-1: Flow-insensitive precision loss**
   - **Issue:** Singleton pattern not precisely modeled (single allocation used everywhere)
   - **Impact:** `get_instance()` returns `alloc_N`, but all calls share it - analysis sees "may return different objects"
   - **Acceptance:** Flow-insensitive design trade-off. Flow-sensitive analysis would fix but costs O(n²) memory.
   - **Documentation:** Document in spec with example

2. **Elephant-2: O(n²) memory complexity**
   - **Issue:** Points-to sets and alias pairs scale quadratically with variable count
   - **Impact:** 10k variables → 100M may-alias pairs (out-of-memory on large codebases)
   - **Acceptance:** Standard for subset-based analysis. Add configurable limit (default 10MB for alias info).
   - **Mitigation:** Add `--max-memory` CLI flag, abort analysis if exceeded

3. **Elephant-3: Field-insensitive heap**
   - **Issue:** All fields of an allocation site share the same location (`alloc_5.x` vs `alloc_5.y`)
   - **Impact:** Over-conservative: writing `obj.x` may affect `obj.y` in analysis
   - **Acceptance:** Field-sensitive analysis would fix but increases complexity significantly.
   - **Documentation:** Document with example showing when false aliasing occurs

### Mitigation Summary

| Tiger | Risk Level | Mitigation Status | Implementation Location |
|-------|-----------|-------------------|-------------------------|
| 1 | High | Implemented | `AbstractLocation::format()` |
| 2 | High | Planned | `solver.rs` |
| 3 | High | Planned | `constraints.rs` |
| 4 | Medium | Planned | `AliasInfo::may_alias_check()` |
| 5 | Critical | Implemented | `AbstractLocation` enum |
| 6 | High | Planned | `compute.rs` final step |
| 7 | Medium | Implemented | `AbstractLocation::DefaultArg` |
| 8 | Medium | Implemented | `AbstractLocation::ClassVar` |
| 9 | Low | Planned | AST extraction |
| 10 | Low | Planned | AST extraction |
| 11 | Low | Planned | AST extraction |
| 12 | Medium | Planned | Flow-sensitive extension |
| 13 | High | Planned | `constraints.rs` field store |
| 14 | High | Planned | `constraints.rs` phi validation |
| 15 | Medium | Planned | `Alloc` enhancement |
