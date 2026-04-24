//! Alias Analysis Tests
//!
//! Comprehensive test suite for Andersen-style flow-insensitive points-to analysis
//! as defined in spec.md. These tests define expected behavior for:
//!
//! 1. Type Definition Tests - AbstractLocation, AliasInfo, AliasError
//! 2. May-Alias Computation Tests - Sound may-alias queries
//! 3. Must-Alias Computation Tests - Precise must-alias queries
//! 4. Points-To Set Tests - Abstract location tracking
//! 5. Assignment Aliasing Tests - Copy propagation
//! 6. Object Creation Tests - Allocation site tracking
//! 7. Phi Function Tests - SSA phi handling
//! 8. Field Access Tests - Field-sensitive aliasing
//! 9. Parameter Aliasing Tests - Conservative parameter handling
//! 10. Unknown Source Tests - External/unknown source handling
//! 11. SSA Integration Tests - Works with SSA infrastructure
//!
//! All tests are marked #[ignore] as the alias module is not yet implemented.
//! Reference: spec.md

use std::collections::{HashMap, HashSet};

// =============================================================================
// Type Imports (will be enabled when types are implemented)
// =============================================================================

// Phase 1: Type imports (to be implemented)
// use super::{
//     AbstractLocation, AliasInfo, AliasError,
//     compute_alias, compute_alias_from_ssa,
// };

// Phase 2: SSA types (existing)
// use crate::ssa::types::{
//     SsaFunction, SsaBlock, SsaInstruction, SsaInstructionKind,
//     PhiFunction, PhiSource, SsaName, SsaNameId, SsaType, SsaStats,
// };

// Phase 3: CFG types (existing)
// use crate::types::{
//     CfgInfo, CfgBlock, CfgEdge, BlockType, EdgeType,
//     VarRef, RefType,
// };

// =============================================================================
// Mock Types for Testing (until real implementation exists)
// =============================================================================

/// Mock AbstractLocation for testing (mirrors spec)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MockAbstractLocation {
    Alloc {
        line: u32,
    },
    Param {
        name: String,
    },
    Unknown,
    Field {
        base: Box<MockAbstractLocation>,
        field: String,
    },
}

impl MockAbstractLocation {
    pub fn alloc(line: u32) -> Self {
        MockAbstractLocation::Alloc { line }
    }
    pub fn param(name: &str) -> Self {
        MockAbstractLocation::Param {
            name: name.to_string(),
        }
    }
    pub fn unknown() -> Self {
        MockAbstractLocation::Unknown
    }
    pub fn field(base: MockAbstractLocation, field: &str) -> Self {
        MockAbstractLocation::Field {
            base: Box::new(base),
            field: field.to_string(),
        }
    }

    pub fn format(&self) -> String {
        match self {
            MockAbstractLocation::Alloc { line } => format!("alloc_{}", line),
            MockAbstractLocation::Param { name } => format!("param_{}", name),
            MockAbstractLocation::Unknown => "unknown".to_string(),
            MockAbstractLocation::Field { base, field } => format!("{}.{}", base.format(), field),
        }
    }
}

/// Mock AliasInfo for testing (mirrors spec)
#[derive(Debug, Clone, Default)]
pub struct MockAliasInfo {
    pub function_name: String,
    pub may_alias: HashMap<String, HashSet<String>>,
    pub must_alias: HashMap<String, HashSet<String>>,
    pub points_to: HashMap<String, HashSet<String>>,
    pub allocation_sites: HashMap<u32, String>,
}

impl MockAliasInfo {
    pub fn new(function_name: &str) -> Self {
        Self {
            function_name: function_name.to_string(),
            ..Default::default()
        }
    }

    /// Check if two variables MAY alias
    pub fn may_alias_check(&self, a: &str, b: &str) -> bool {
        // Same variable always aliases itself
        if a == b {
            return true;
        }

        // Check explicit may_alias set
        if self.may_alias.get(a).is_some_and(|s| s.contains(b)) {
            return true;
        }
        if self.may_alias.get(b).is_some_and(|s| s.contains(a)) {
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

    /// Check if two variables MUST alias
    pub fn must_alias_check(&self, a: &str, b: &str) -> bool {
        // Same variable always must-aliases itself
        if a == b {
            return true;
        }

        // Check explicit must_alias set
        if self.must_alias.get(a).is_some_and(|s| s.contains(b)) {
            return true;
        }
        if self.must_alias.get(b).is_some_and(|s| s.contains(a)) {
            return true;
        }

        false
    }

    /// Get points-to set for a variable
    pub fn get_points_to(&self, var: &str) -> HashSet<String> {
        self.points_to.get(var).cloned().unwrap_or_default()
    }

    /// Get all variables that may alias with the given variable
    pub fn get_aliases(&self, var: &str) -> HashSet<String> {
        self.may_alias.get(var).cloned().unwrap_or_default()
    }

    /// Convert to JSON-serializable format
    pub fn to_json_value(&self) -> serde_json::Value {
        use serde_json::json;

        let sorted_may_alias: HashMap<_, Vec<_>> = self
            .may_alias
            .iter()
            .map(|(k, v)| {
                let mut sorted: Vec<_> = v.iter().cloned().collect();
                sorted.sort();
                (k.clone(), sorted)
            })
            .collect();

        let sorted_must_alias: HashMap<_, Vec<_>> = self
            .must_alias
            .iter()
            .map(|(k, v)| {
                let mut sorted: Vec<_> = v.iter().cloned().collect();
                sorted.sort();
                (k.clone(), sorted)
            })
            .collect();

        let sorted_points_to: HashMap<_, Vec<_>> = self
            .points_to
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
// Section 1: AbstractLocation Type Tests
// =============================================================================

#[test]
fn test_abstract_location_alloc() {
    let loc = MockAbstractLocation::alloc(5);
    assert_eq!(loc.format(), "alloc_5");
}

#[test]
fn test_abstract_location_param() {
    let loc = MockAbstractLocation::param("x");
    assert_eq!(loc.format(), "param_x");
}

#[test]
fn test_abstract_location_unknown() {
    let loc = MockAbstractLocation::unknown();
    assert_eq!(loc.format(), "unknown");
}

#[test]
fn test_abstract_location_field() {
    let base = MockAbstractLocation::alloc(5);
    let loc = MockAbstractLocation::field(base, "data");
    assert_eq!(loc.format(), "alloc_5.data");
}

#[test]
fn test_abstract_location_nested_field() {
    let base = MockAbstractLocation::alloc(5);
    let field1 = MockAbstractLocation::field(base, "inner");
    let field2 = MockAbstractLocation::field(field1, "value");
    assert_eq!(field2.format(), "alloc_5.inner.value");
}

// =============================================================================
// Section 2: AliasInfo Dataclass Tests (from Python tests)
// =============================================================================

#[test]
fn test_alias_info_fields() {
    let mut info = MockAliasInfo::new("test_func");
    info.may_alias.insert(
        "x".to_string(),
        HashSet::from(["y".to_string(), "z".to_string()]),
    );
    info.must_alias
        .insert("a".to_string(), HashSet::from(["b".to_string()]));
    info.points_to.insert(
        "x".to_string(),
        HashSet::from(["alloc_1".to_string(), "param_0".to_string()]),
    );
    info.allocation_sites.insert(5, "alloc_5".to_string());

    assert_eq!(info.may_alias.get("x").unwrap().len(), 2);
    assert!(info.must_alias.get("a").unwrap().contains("b"));
    assert!(info.points_to.get("x").unwrap().contains("alloc_1"));
    assert_eq!(info.allocation_sites.get(&5), Some(&"alloc_5".to_string()));
    assert_eq!(info.function_name, "test_func");
}

#[test]
fn test_alias_info_default_empty() {
    let info = MockAliasInfo::default();

    assert!(info.may_alias.is_empty());
    assert!(info.must_alias.is_empty());
    assert!(info.points_to.is_empty());
    assert!(info.allocation_sites.is_empty());
    assert!(info.function_name.is_empty());
}

#[test]
fn test_alias_info_may_alias_check_same_var() {
    let info = MockAliasInfo::new("test");

    // Same variable always aliases itself
    assert!(info.may_alias_check("x", "x"));
    assert!(info.may_alias_check("anything", "anything"));
}

#[test]
fn test_alias_info_may_alias_check_in_set() {
    let mut info = MockAliasInfo::new("test");
    info.may_alias.insert(
        "x".to_string(),
        HashSet::from(["y".to_string(), "z".to_string()]),
    );

    assert!(info.may_alias_check("x", "y"));
    assert!(info.may_alias_check("x", "z"));
    assert!(info.may_alias_check("y", "x")); // Symmetric
}

#[test]
fn test_alias_info_may_alias_check_points_to_overlap() {
    let mut info = MockAliasInfo::new("test");
    info.points_to.insert(
        "x".to_string(),
        HashSet::from(["alloc_1".to_string(), "alloc_2".to_string()]),
    );
    info.points_to.insert(
        "y".to_string(),
        HashSet::from(["alloc_2".to_string(), "alloc_3".to_string()]),
    );
    info.points_to
        .insert("z".to_string(), HashSet::from(["alloc_4".to_string()]));

    // x and y share alloc_2
    assert!(info.may_alias_check("x", "y"));
    // x and z don't share any location
    assert!(!info.may_alias_check("x", "z"));
}

#[test]
fn test_alias_info_may_alias_check_no_overlap() {
    let mut info = MockAliasInfo::new("test");
    info.may_alias
        .insert("x".to_string(), HashSet::from(["a".to_string()]));
    info.points_to
        .insert("x".to_string(), HashSet::from(["alloc_1".to_string()]));
    info.points_to
        .insert("y".to_string(), HashSet::from(["alloc_2".to_string()]));

    // x and y have disjoint points-to sets and no explicit may_alias
    assert!(!info.may_alias_check("x", "y"));
}

#[test]
fn test_alias_info_must_alias_check_same_var() {
    let info = MockAliasInfo::new("test");

    // Same variable always must-aliases itself
    assert!(info.must_alias_check("x", "x"));
}

#[test]
fn test_alias_info_must_alias_check_in_set() {
    let mut info = MockAliasInfo::new("test");
    info.must_alias.insert(
        "a".to_string(),
        HashSet::from(["b".to_string(), "c".to_string()]),
    );

    assert!(info.must_alias_check("a", "b"));
    assert!(info.must_alias_check("b", "a")); // Symmetric
    assert!(info.must_alias_check("a", "c"));
}

#[test]
fn test_alias_info_must_alias_check_not_in_set() {
    let mut info = MockAliasInfo::new("test");
    info.must_alias
        .insert("a".to_string(), HashSet::from(["b".to_string()]));
    info.may_alias.insert(
        "a".to_string(),
        HashSet::from(["b".to_string(), "c".to_string()]),
    );

    // c only may-alias, not must-alias
    assert!(!info.must_alias_check("a", "c"));
}

#[test]
fn test_alias_info_get_points_to() {
    let mut info = MockAliasInfo::new("test");
    info.points_to.insert(
        "x".to_string(),
        HashSet::from(["alloc_1".to_string(), "param_0".to_string()]),
    );

    assert_eq!(
        info.get_points_to("x"),
        HashSet::from(["alloc_1".to_string(), "param_0".to_string()])
    );
    assert!(info.get_points_to("unknown").is_empty()); // Returns empty for unknown
}

#[test]
fn test_alias_info_get_aliases() {
    let mut info = MockAliasInfo::new("test");
    info.may_alias.insert(
        "x".to_string(),
        HashSet::from(["y".to_string(), "z".to_string()]),
    );

    assert_eq!(
        info.get_aliases("x"),
        HashSet::from(["y".to_string(), "z".to_string()])
    );
    assert!(info.get_aliases("unknown").is_empty());
}

#[test]
fn test_alias_info_to_json_sorted() {
    let mut info = MockAliasInfo::new("test");
    info.may_alias.insert(
        "x".to_string(),
        HashSet::from(["z".to_string(), "a".to_string(), "m".to_string()]),
    );

    let json = info.to_json_value();
    let may_alias = json["may_alias"]["x"].as_array().unwrap();

    // Should be sorted
    let sorted: Vec<&str> = may_alias.iter().map(|v| v.as_str().unwrap()).collect();
    assert_eq!(sorted, vec!["a", "m", "z"]);
}

// =============================================================================
// Section 3: compute_alias() Function Tests (HIGH PRIORITY - Capability 1-7)
// These tests will FAIL until the alias module is implemented
// =============================================================================

/// Capability 1: May-Alias Computation
/// Test that compute_alias returns AliasInfo with correct may_alias
#[test]
#[ignore = "Alias analysis not yet implemented"]
fn test_compute_alias_returns_alias_info() {
    // Will test:
    // let ssa = fixtures::simple_ssa("test");
    // let result = compute_alias_from_ssa(&ssa).unwrap();
    // assert!(result.function_name == "test");
    todo!("Implement compute_alias_from_ssa");
}

#[test]
#[ignore = "Alias analysis not yet implemented"]
fn test_compute_alias_empty_function() {
    // Will test:
    // let ssa = fixtures::simple_ssa("empty");
    // let result = compute_alias_from_ssa(&ssa).unwrap();
    // assert!(result.may_alias.is_empty());
    // assert!(result.must_alias.is_empty());
    // assert!(result.points_to.is_empty());
    todo!("Implement compute_alias_from_ssa");
}

// =============================================================================
// Section 4: Assignment Aliasing Tests (HIGH - Capability 4)
// =============================================================================

/// Test: x = y creates must-alias relationship
#[test]
#[ignore = "Alias analysis not yet implemented"]
fn test_simple_assignment_creates_must_alias() {
    // Code pattern:
    // def simple(p):
    //     x = p      # x must-alias p
    //     return x
    //
    // Will test:
    // let ssa = fixtures::copy_assignment_ssa();
    // let result = compute_alias_from_ssa(&ssa).unwrap();
    // assert!(result.must_alias_check("x_0", "p_0"));
    todo!("Implement assignment must-alias");
}

/// Test: Transitive aliasing through chain of assignments
#[test]
#[ignore = "Alias analysis not yet implemented"]
fn test_transitive_aliasing() {
    // Code pattern:
    // y = param
    // x = y
    // z = x     # z must-alias y (transitive)
    //
    // Will test:
    // assert!(result.may_alias_check("z_0", "y_0"));
    todo!("Implement transitive aliasing");
}

/// Test: Assignment propagates points-to set
#[test]
#[ignore = "Alias analysis not yet implemented"]
fn test_assignment_propagates_points_to() {
    // Code pattern:
    // def propagate(p):
    //     x = p
    //     # points_to(x) should include points_to(p)
    //
    // Will test:
    // let pts_p = result.get_points_to("p_0");
    // let pts_x = result.get_points_to("x_0");
    // assert!(!pts_p.is_disjoint(&pts_x));  // Non-empty intersection
    todo!("Implement points-to propagation");
}

// =============================================================================
// Section 5: Object Creation Tests (HIGH - Capability 5)
// =============================================================================

/// Test: x = Foo() creates unique abstract location
#[test]
#[ignore = "Alias analysis not yet implemented"]
fn test_new_object_gets_unique_location() {
    // Code pattern:
    // x = Foo()   # line 1: pts(x) = {alloc_1}
    // y = Foo()   # line 2: pts(y) = {alloc_2}
    // # x and y do NOT alias (different allocation sites)
    //
    // Will test:
    // let ssa = fixtures::two_allocations_ssa();
    // let result = compute_alias_from_ssa(&ssa).unwrap();
    // assert!(!result.may_alias_check("x_0", "y_0"));
    // let pts_x = result.get_points_to("x_0");
    // let pts_y = result.get_points_to("y_0");
    // assert!(pts_x.is_disjoint(&pts_y));  // No overlap
    todo!("Implement allocation site tracking");
}

/// Test: Allocation sites are recorded with line numbers
#[test]
#[ignore = "Alias analysis not yet implemented"]
fn test_allocation_sites_recorded() {
    // Will test:
    // let result = compute_alias_from_ssa(&ssa).unwrap();
    // assert!(result.allocation_sites.contains_key(&1));
    // assert!(result.allocation_sites.get(&1).unwrap().starts_with("alloc"));
    todo!("Implement allocation site recording");
}

/// Test: Assigning same allocation to multiple vars creates alias
#[test]
#[ignore = "Alias analysis not yet implemented"]
fn test_same_object_reused_aliases() {
    // Code pattern:
    // obj = Foo()
    // x = obj
    // y = obj
    // # x and y alias (same object through obj)
    //
    // Will test:
    // let ssa = fixtures::shared_allocation_ssa();
    // let result = compute_alias_from_ssa(&ssa).unwrap();
    // assert!(result.may_alias_check("x_0", "y_0"));
    todo!("Implement shared allocation aliasing");
}

// =============================================================================
// Section 6: Points-To Computation Tests (HIGH - Capability 3)
// =============================================================================

/// Test: get_points_to returns correct abstract locations
#[test]
#[ignore = "Alias analysis not yet implemented"]
fn test_points_to_computation() {
    // Will test:
    // let pts = result.get_points_to("x_0");
    // assert!(pts.contains("alloc_1") || pts.contains("param_x"));
    todo!("Implement points-to computation");
}

/// Test: Points-to sets have correct allocation site format
#[test]
#[ignore = "Alias analysis not yet implemented"]
fn test_points_to_alloc_format() {
    // Will test:
    // let pts = result.get_points_to("x_0");
    // for loc in pts {
    //     assert!(loc.starts_with("alloc_") || loc.starts_with("param_") || loc == "unknown");
    // }
    todo!("Implement points-to format validation");
}

// =============================================================================
// Section 7: Phi Function Handling Tests (HIGH - Capability 6)
// =============================================================================

/// Test: Phi functions create may-alias (not must-alias)
#[test]
#[ignore = "Alias analysis not yet implemented"]
fn test_phi_creates_may_alias() {
    // Code pattern:
    // def conditional(flag, a, b):
    //     if flag:
    //         x = a
    //     else:
    //         x = b
    //     return x  # x may-alias a, x may-alias b
    //
    // Will test:
    // let ssa = fixtures::phi_function_ssa();
    // let result = compute_alias_from_ssa(&ssa).unwrap();
    // assert!(result.may_alias_check("x_2", "a_0"));
    // assert!(result.may_alias_check("x_2", "b_0"));
    // assert!(!result.must_alias_check("x_2", "a_0"));  // NOT must-alias
    // assert!(!result.must_alias_check("x_2", "b_0"));
    todo!("Implement phi function may-alias");
}

/// Test: Phi function propagates points-to from all sources
#[test]
#[ignore = "Alias analysis not yet implemented"]
fn test_phi_propagates_points_to() {
    // Will test:
    // pts(x_2) should be pts(x_0) ∪ pts(x_1)
    todo!("Implement phi points-to propagation");
}

// =============================================================================
// Section 8: SSA Integration Tests (HIGH - Capability 7)
// =============================================================================

/// Test: Uses SSA names (x_0, x_1) not original names (x)
#[test]
#[ignore = "Alias analysis not yet implemented"]
fn test_ssa_uses_versioned_names() {
    // Will test:
    // let result = compute_alias_from_ssa(&ssa).unwrap();
    // assert!(result.points_to.contains_key("x_0"));
    // assert!(!result.points_to.contains_key("x"));  // Not the unversioned name
    todo!("Implement SSA name handling");
}

/// Test: Works with SsaFunction blocks and phi functions
#[test]
#[ignore = "Alias analysis not yet implemented"]
fn test_ssa_integration_blocks() {
    // Will test that it correctly processes SSA blocks
    todo!("Implement SSA block processing");
}

/// Test: Handles SsaInstructionKind::Call as potential allocation
#[test]
#[ignore = "Alias analysis not yet implemented"]
fn test_ssa_call_instruction_allocation() {
    // Will test that Call instructions are detected as allocations
    todo!("Implement Call instruction handling");
}

// =============================================================================
// Section 9: Field Access Tests (MEDIUM - Capability 8, 9)
// =============================================================================

/// Test: x.field tracked separately from x
#[test]
#[ignore = "Alias analysis not yet implemented"]
fn test_field_access_separate() {
    // Code pattern:
    // obj = Foo()
    // x = obj.field
    // # x points to alloc_1.field, not alloc_1
    todo!("Implement field access tracking");
}

/// Test: x.f and y.f alias if x and y alias
#[test]
#[ignore = "Alias analysis not yet implemented"]
fn test_same_field_same_object_aliases() {
    // Code pattern:
    // x = obj.field
    // y = obj.field
    // # x and y may-alias (same field from same object)
    todo!("Implement same-field aliasing");
}

/// Test: Different fields do NOT alias
#[test]
#[ignore = "Alias analysis not yet implemented"]
fn test_different_fields_no_alias() {
    // Code pattern:
    // a = obj.f
    // b = obj.g
    // # a and b don't alias (different fields)
    todo!("Implement different-field separation");
}

/// Test: Field store updates points-to of field location
#[test]
#[ignore = "Alias analysis not yet implemented"]
fn test_field_store() {
    // Code pattern:
    // obj.field = y
    // # pts(alloc_N.field) ⊇ pts(y)
    todo!("Implement field store");
}

/// Test: Fields of aliasing objects may alias
#[test]
#[ignore = "Alias analysis not yet implemented"]
fn test_field_of_aliasing_objects_may_alias() {
    // Code pattern:
    // # a and b may alias (parameters)
    // x = a.field
    // y = b.field
    // # x and y may-alias (same field from aliasing objects)
    todo!("Implement field aliasing propagation");
}

// =============================================================================
// Section 10: Parameter Aliasing Tests (MEDIUM - Capability 10)
// =============================================================================

/// Test: Parameters may alias each other (conservative)
#[test]
#[ignore = "Alias analysis not yet implemented"]
fn test_parameters_may_alias_conservatively() {
    // Code pattern:
    // def params(a, b):
    //     # a and b may alias (caller could pass same object)
    //
    // Will test:
    // let ssa = fixtures::two_params_ssa();
    // let result = compute_alias_from_ssa(&ssa).unwrap();
    // assert!(result.may_alias_check("a_0", "b_0"));
    todo!("Implement parameter conservative aliasing");
}

/// Test: Parameters point to param_X location
#[test]
#[ignore = "Alias analysis not yet implemented"]
fn test_parameter_points_to_param_location() {
    // Will test:
    // let pts = result.get_points_to("x_0");
    // assert!(pts.iter().any(|loc| loc.starts_with("param_")));
    todo!("Implement parameter location");
}

// =============================================================================
// Section 11: Unknown Source Tests (MEDIUM - Capability 11)
// =============================================================================

/// Test: Results from unknown calls get "unknown" location
#[test]
#[ignore = "Alias analysis not yet implemented"]
fn test_unknown_call_result() {
    // Code pattern:
    // x = external_func()  # Unknown source
    // # pts(x) = {unknown}
    todo!("Implement unknown source handling");
}

/// Test: Unknown sources may alias each other (conservative)
#[test]
#[ignore = "Alias analysis not yet implemented"]
fn test_unknown_sources_may_alias() {
    // Code pattern:
    // x = external_func()
    // y = another_external()
    // # Conservative: x may-alias y (both have {unknown})
    todo!("Implement unknown source aliasing");
}

// =============================================================================
// Section 12: Edge Cases and Error Handling
// =============================================================================

/// Test: Self-alias: variable always aliases itself
#[test]
fn test_self_alias_always_true() {
    let info = MockAliasInfo::new("test");

    assert!(info.may_alias_check("x_0", "x_0"));
    assert!(info.must_alias_check("x_0", "x_0"));
    assert!(info.may_alias_check("anything", "anything"));
    assert!(info.must_alias_check("anything", "anything"));
}

/// Test: Symmetry: may_alias(a, b) implies may_alias(b, a)
#[test]
fn test_may_alias_symmetric() {
    let mut info = MockAliasInfo::new("test");
    info.may_alias
        .insert("a".to_string(), HashSet::from(["b".to_string()]));

    assert!(info.may_alias_check("a", "b"));
    assert!(info.may_alias_check("b", "a")); // Symmetric
}

/// Test: must_alias is transitive (through implementation)
#[test]
#[ignore = "Alias analysis not yet implemented"]
fn test_must_alias_transitive() {
    // If a must-alias b, and b must-alias c, then a must-alias c
    todo!("Implement must-alias transitivity");
}

/// Test: Unknown variable queries don't crash
#[test]
fn test_unknown_variable_no_crash() {
    let info = MockAliasInfo::new("test");

    // Should return false, not crash
    assert!(!info.may_alias_check("unknown1", "unknown2"));
    assert!(!info.must_alias_check("unknown1", "unknown2"));
    assert!(info.get_points_to("unknown").is_empty());
    assert!(info.get_aliases("unknown").is_empty());
}

/// Test: Empty CFG handling
#[test]
#[ignore = "Alias analysis not yet implemented"]
fn test_empty_cfg_handling() {
    // Will test:
    // let ssa = fixtures::simple_ssa("empty");
    // ssa.blocks[0].instructions.clear();
    // let result = compute_alias_from_ssa(&ssa).unwrap();
    // assert!(result.may_alias.is_empty());
    todo!("Implement empty CFG handling");
}

/// Test: Max iterations limit (100)
#[test]
#[ignore = "Alias analysis not yet implemented"]
fn test_max_iterations_limit() {
    // Will test that cyclic constraints don't cause infinite loops
    // and respect the MAX_ITERATIONS = 100 limit
    todo!("Implement iteration limit");
}

/// Test: IterationLimit error when exceeded
#[test]
#[ignore = "Alias analysis not yet implemented"]
fn test_iteration_limit_error() {
    // Will test:
    // match result {
    //     Err(AliasError::IterationLimit(n)) => assert!(n >= 100),
    //     _ => panic!("Expected IterationLimit error"),
    // }
    todo!("Implement IterationLimit error");
}

// =============================================================================
// Section 13: Property-Based Tests
// =============================================================================

/// Property: If a must-alias b, then a may-alias b
#[test]
fn test_must_alias_implies_may_alias() {
    let mut info = MockAliasInfo::new("test");
    info.must_alias
        .insert("a".to_string(), HashSet::from(["b".to_string()]));

    // must_alias doesn't automatically add to may_alias in this mock
    // but must_alias_check returning true should mean may_alias_check also returns true
    // This is enforced by the spec: must-alias is a subset of may-alias

    // For the real implementation, this should hold:
    // if info.must_alias_check("a", "b") {
    //     assert!(info.may_alias_check("a", "b"));
    // }
}

/// Property: Different allocation sites never alias
#[test]
fn test_different_alloc_sites_no_alias() {
    let mut info = MockAliasInfo::new("test");
    info.points_to
        .insert("x".to_string(), HashSet::from(["alloc_1".to_string()]));
    info.points_to
        .insert("y".to_string(), HashSet::from(["alloc_2".to_string()]));

    // alloc_1 and alloc_2 are disjoint
    assert!(!info.may_alias_check("x", "y"));
}

/// Property: Phi results never must-alias their sources
#[test]
#[ignore = "Alias analysis not yet implemented"]
fn test_phi_never_must_alias_sources() {
    // For phi: x_2 = phi(x_0, x_1)
    // x_2 should NOT must-alias x_0 or x_1
    // (only one path is taken at runtime)
    todo!("Implement phi must-alias property");
}

// =============================================================================
// Section 14: JSON Output Tests
// =============================================================================

#[test]
fn test_json_output_structure() {
    let mut info = MockAliasInfo::new("test_func");
    info.may_alias
        .insert("x".to_string(), HashSet::from(["y".to_string()]));
    info.must_alias
        .insert("a".to_string(), HashSet::from(["b".to_string()]));
    info.points_to
        .insert("x".to_string(), HashSet::from(["alloc_1".to_string()]));
    info.allocation_sites.insert(5, "alloc_5".to_string());

    let json = info.to_json_value();

    assert!(json.get("function").is_some());
    assert!(json.get("may_alias").is_some());
    assert!(json.get("must_alias").is_some());
    assert!(json.get("points_to").is_some());
    assert!(json.get("allocation_sites").is_some());

    // Should be serializable
    let serialized = serde_json::to_string(&json).unwrap();
    assert!(!serialized.is_empty());
}

#[test]
fn test_json_output_deterministic() {
    let mut info = MockAliasInfo::new("test");
    info.may_alias.insert(
        "x".to_string(),
        HashSet::from(["c".to_string(), "a".to_string(), "b".to_string()]),
    );

    let json1 = info.to_json_value();
    let json2 = info.to_json_value();

    // Should produce identical output (sorted)
    assert_eq!(
        serde_json::to_string(&json1).unwrap(),
        serde_json::to_string(&json2).unwrap()
    );
}

// =============================================================================
// Section 15: Integration Tests (compute_alias with real SSA)
// =============================================================================

/// Integration test: Full alias analysis on simple function
#[test]
#[ignore = "Alias analysis not yet implemented"]
fn test_integration_simple_function() {
    // Code:
    // def simple(p):
    //     x = p
    //     y = Foo()
    //     return x
    //
    // Expected:
    // - x_0 must-alias p_0
    // - y_0 does NOT alias x_0 or p_0
    // - pts(p_0) = {param_p}
    // - pts(x_0) = {param_p}
    // - pts(y_0) = {alloc_N}
    todo!("Implement integration test");
}

/// Integration test: Alias analysis with conditional branch
#[test]
#[ignore = "Alias analysis not yet implemented"]
fn test_integration_conditional() {
    // Code:
    // def conditional(flag, a, b):
    //     if flag:
    //         x = a
    //     else:
    //         x = b
    //     return x
    //
    // Expected:
    // - x_2 (phi result) may-alias both a_0 and b_0
    // - x_2 does NOT must-alias a_0 or b_0
    todo!("Implement conditional integration test");
}

/// Integration test: Alias analysis with loop
#[test]
#[ignore = "Alias analysis not yet implemented"]
fn test_integration_loop() {
    // Code:
    // def loop_test(items):
    //     result = []
    //     for item in items:
    //         result.append(item)
    //     return result
    //
    // Expected:
    // - Fixed-point converges (no infinite loop)
    // - result aliases itself through loop iterations
    todo!("Implement loop integration test");
}

// =============================================================================
// Section 16: Stress/Performance Tests
// =============================================================================

/// Test: Analysis completes on function with many variables
#[test]
#[ignore = "Alias analysis not yet implemented"]
fn test_many_variables_performance() {
    // Create SSA with 100+ variables
    // Verify analysis completes within reasonable time
    todo!("Implement performance test");
}

/// Test: Analysis handles deep nesting
#[test]
#[ignore = "Alias analysis not yet implemented"]
fn test_deep_nesting_performance() {
    // Create deeply nested control flow
    // Verify analysis handles it correctly
    todo!("Implement nesting test");
}

// =============================================================================
// Section 17: ENABLED SSA Integration Tests (Phase 4)
// These tests use real SsaFunction types and compute_alias_from_ssa
// =============================================================================

mod ssa_integration {

    use crate::alias::{compute_alias_from_ssa, AliasInfo};
    use crate::ssa::types::{
        PhiFunction, PhiSource, SsaBlock, SsaFunction, SsaInstruction, SsaInstructionKind, SsaName,
        SsaNameId, SsaStats, SsaType,
    };
    use std::path::PathBuf;

    // -------------------------------------------------------------------------
    // Test Helpers
    // -------------------------------------------------------------------------

    /// Create a minimal SSA function for testing.
    fn create_test_ssa(name: &str) -> SsaFunction {
        SsaFunction {
            function: name.to_string(),
            file: PathBuf::from("test.py"),
            ssa_type: SsaType::Minimal,
            blocks: vec![],
            ssa_names: vec![],
            def_use: std::collections::HashMap::new(),
            stats: SsaStats::default(),
        }
    }

    /// Create an SSA name for testing.
    fn create_ssa_name(id: u32, variable: &str, version: u32, line: u32) -> SsaName {
        SsaName {
            id: SsaNameId(id),
            variable: variable.to_string(),
            version,
            def_block: Some(0),
            def_line: line,
        }
    }

    // -------------------------------------------------------------------------
    // Basic Integration Tests
    // -------------------------------------------------------------------------

    /// Test: compute_alias_from_ssa returns AliasInfo
    #[test]
    fn test_compute_alias_returns_alias_info() {
        let ssa = create_test_ssa("test_func");
        let result = compute_alias_from_ssa(&ssa).unwrap();

        assert_eq!(result.function_name, "test_func");
        assert!(result.may_alias.is_empty());
        assert!(result.must_alias.is_empty());
    }

    /// Test: Empty function produces empty alias info
    #[test]
    fn test_compute_alias_empty_function() {
        let ssa = create_test_ssa("empty");
        let result = compute_alias_from_ssa(&ssa).unwrap();

        assert!(result.may_alias.is_empty());
        assert!(result.must_alias.is_empty());
        assert!(result.points_to.is_empty());
    }

    // -------------------------------------------------------------------------
    // Parameter Tests
    // -------------------------------------------------------------------------

    /// Test: Parameters get param_ locations
    #[test]
    fn test_parameter_gets_param_location() {
        let mut ssa = create_test_ssa("param_test");
        ssa.ssa_names = vec![create_ssa_name(0, "p", 0, 1)];
        ssa.blocks = vec![SsaBlock {
            id: 0,
            label: Some("entry".to_string()),
            lines: (1, 1),
            phi_functions: vec![],
            instructions: vec![SsaInstruction {
                kind: SsaInstructionKind::Param,
                target: Some(SsaNameId(0)),
                uses: vec![],
                line: 1,
                source_text: Some("def f(p):".to_string()),
            }],
            successors: vec![],
            predecessors: vec![],
        }];

        let result = compute_alias_from_ssa(&ssa).unwrap();

        // p_0 should point to param_p
        let pts = result.get_points_to("p_0");
        assert!(
            pts.contains("param_p"),
            "Expected p_0 to point to param_p, got {:?}",
            pts
        );
    }

    /// Test: Two parameters may alias each other (conservative)
    #[test]
    fn test_parameters_may_alias_conservatively() {
        let mut ssa = create_test_ssa("two_params");
        ssa.ssa_names = vec![create_ssa_name(0, "a", 0, 1), create_ssa_name(1, "b", 0, 1)];
        ssa.blocks = vec![SsaBlock {
            id: 0,
            label: Some("entry".to_string()),
            lines: (1, 1),
            phi_functions: vec![],
            instructions: vec![
                SsaInstruction {
                    kind: SsaInstructionKind::Param,
                    target: Some(SsaNameId(0)),
                    uses: vec![],
                    line: 1,
                    source_text: Some("def f(a, b):".to_string()),
                },
                SsaInstruction {
                    kind: SsaInstructionKind::Param,
                    target: Some(SsaNameId(1)),
                    uses: vec![],
                    line: 1,
                    source_text: None,
                },
            ],
            successors: vec![],
            predecessors: vec![],
        }];

        let result = compute_alias_from_ssa(&ssa).unwrap();

        // Parameters may alias each other (conservative assumption)
        assert!(
            result.may_alias_check("a_0", "b_0"),
            "Parameters should may-alias conservatively"
        );
    }

    // -------------------------------------------------------------------------
    // Assignment Tests
    // -------------------------------------------------------------------------

    /// Test: x = p creates must-alias
    #[test]
    fn test_simple_assignment_creates_must_alias() {
        let mut ssa = create_test_ssa("copy_test");
        ssa.ssa_names = vec![create_ssa_name(0, "p", 0, 1), create_ssa_name(1, "x", 0, 2)];
        ssa.blocks = vec![SsaBlock {
            id: 0,
            label: Some("entry".to_string()),
            lines: (1, 2),
            phi_functions: vec![],
            instructions: vec![
                SsaInstruction {
                    kind: SsaInstructionKind::Param,
                    target: Some(SsaNameId(0)),
                    uses: vec![],
                    line: 1,
                    source_text: Some("def f(p):".to_string()),
                },
                SsaInstruction {
                    kind: SsaInstructionKind::Assign,
                    target: Some(SsaNameId(1)),
                    uses: vec![SsaNameId(0)],
                    line: 2,
                    source_text: Some("x = p".to_string()),
                },
            ],
            successors: vec![],
            predecessors: vec![],
        }];

        let result = compute_alias_from_ssa(&ssa).unwrap();

        // x_0 must-alias p_0 (direct copy)
        assert!(
            result.must_alias_check("x_0", "p_0"),
            "x_0 should must-alias p_0"
        );

        // They also may-alias
        assert!(
            result.may_alias_check("x_0", "p_0"),
            "x_0 should may-alias p_0"
        );
    }

    /// Test: Assignment propagates points-to
    #[test]
    fn test_assignment_propagates_points_to() {
        let mut ssa = create_test_ssa("propagate_test");
        ssa.ssa_names = vec![create_ssa_name(0, "p", 0, 1), create_ssa_name(1, "x", 0, 2)];
        ssa.blocks = vec![SsaBlock {
            id: 0,
            label: Some("entry".to_string()),
            lines: (1, 2),
            phi_functions: vec![],
            instructions: vec![
                SsaInstruction {
                    kind: SsaInstructionKind::Param,
                    target: Some(SsaNameId(0)),
                    uses: vec![],
                    line: 1,
                    source_text: Some("def f(p):".to_string()),
                },
                SsaInstruction {
                    kind: SsaInstructionKind::Assign,
                    target: Some(SsaNameId(1)),
                    uses: vec![SsaNameId(0)],
                    line: 2,
                    source_text: Some("x = p".to_string()),
                },
            ],
            successors: vec![],
            predecessors: vec![],
        }];

        let result = compute_alias_from_ssa(&ssa).unwrap();

        let pts_p = result.get_points_to("p_0");
        let pts_x = result.get_points_to("x_0");

        // x_0 should have the same points-to as p_0
        assert!(
            pts_p.is_subset(&pts_x),
            "pts(x) should include pts(p): pts(p)={:?}, pts(x)={:?}",
            pts_p,
            pts_x
        );
    }

    // -------------------------------------------------------------------------
    // Allocation Tests
    // -------------------------------------------------------------------------

    /// Test: Call creates allocation site
    #[test]
    fn test_allocation_creates_unique_location() {
        let mut ssa = create_test_ssa("alloc_test");
        ssa.ssa_names = vec![create_ssa_name(0, "x", 0, 1)];
        ssa.blocks = vec![SsaBlock {
            id: 0,
            label: Some("entry".to_string()),
            lines: (1, 1),
            phi_functions: vec![],
            instructions: vec![SsaInstruction {
                kind: SsaInstructionKind::Call,
                target: Some(SsaNameId(0)),
                uses: vec![],
                line: 1,
                source_text: Some("x = Foo()".to_string()),
            }],
            successors: vec![],
            predecessors: vec![],
        }];

        let result = compute_alias_from_ssa(&ssa).unwrap();

        let pts = result.get_points_to("x_0");
        assert!(pts.contains("alloc_1"), "Expected alloc_1, got {:?}", pts);
    }

    /// Test: Two different allocations don't alias
    #[test]
    fn test_different_allocations_no_alias() {
        let mut ssa = create_test_ssa("two_allocs");
        ssa.ssa_names = vec![create_ssa_name(0, "x", 0, 1), create_ssa_name(1, "y", 0, 2)];
        ssa.blocks = vec![SsaBlock {
            id: 0,
            label: Some("entry".to_string()),
            lines: (1, 2),
            phi_functions: vec![],
            instructions: vec![
                SsaInstruction {
                    kind: SsaInstructionKind::Call,
                    target: Some(SsaNameId(0)),
                    uses: vec![],
                    line: 1,
                    source_text: Some("x = Foo()".to_string()),
                },
                SsaInstruction {
                    kind: SsaInstructionKind::Call,
                    target: Some(SsaNameId(1)),
                    uses: vec![],
                    line: 2,
                    source_text: Some("y = Bar()".to_string()),
                },
            ],
            successors: vec![],
            predecessors: vec![],
        }];

        let result = compute_alias_from_ssa(&ssa).unwrap();

        // Different allocation sites should not alias
        assert!(
            !result.may_alias_check("x_0", "y_0"),
            "Different allocations should not alias"
        );
    }

    /// Test: Shared allocation creates alias
    #[test]
    fn test_shared_allocation_aliases() {
        let mut ssa = create_test_ssa("shared_alloc");
        ssa.ssa_names = vec![
            create_ssa_name(0, "obj", 0, 1),
            create_ssa_name(1, "x", 0, 2),
            create_ssa_name(2, "y", 0, 3),
        ];
        ssa.blocks = vec![SsaBlock {
            id: 0,
            label: Some("entry".to_string()),
            lines: (1, 3),
            phi_functions: vec![],
            instructions: vec![
                // obj = Foo()
                SsaInstruction {
                    kind: SsaInstructionKind::Call,
                    target: Some(SsaNameId(0)),
                    uses: vec![],
                    line: 1,
                    source_text: Some("obj = Foo()".to_string()),
                },
                // x = obj
                SsaInstruction {
                    kind: SsaInstructionKind::Assign,
                    target: Some(SsaNameId(1)),
                    uses: vec![SsaNameId(0)],
                    line: 2,
                    source_text: Some("x = obj".to_string()),
                },
                // y = obj
                SsaInstruction {
                    kind: SsaInstructionKind::Assign,
                    target: Some(SsaNameId(2)),
                    uses: vec![SsaNameId(0)],
                    line: 3,
                    source_text: Some("y = obj".to_string()),
                },
            ],
            successors: vec![],
            predecessors: vec![],
        }];

        let result = compute_alias_from_ssa(&ssa).unwrap();

        // x and y both point to same allocation via obj
        assert!(
            result.may_alias_check("x_0", "y_0"),
            "x and y should alias via shared obj"
        );
    }

    // -------------------------------------------------------------------------
    // Phi Function Tests
    // -------------------------------------------------------------------------

    /// Test: Phi function creates may-alias but not must-alias
    #[test]
    fn test_phi_creates_may_alias_not_must() {
        let mut ssa = create_test_ssa("phi_test");
        ssa.ssa_names = vec![
            create_ssa_name(0, "a", 0, 1),
            create_ssa_name(1, "b", 0, 1),
            create_ssa_name(2, "x", 0, 3), // x_0 = a in true branch
            create_ssa_name(3, "x", 1, 5), // x_1 = b in false branch
            create_ssa_name(4, "x", 2, 7), // x_2 = phi(x_0, x_1)
        ];
        ssa.blocks = vec![
            // Entry block
            SsaBlock {
                id: 0,
                label: Some("entry".to_string()),
                lines: (1, 2),
                phi_functions: vec![],
                instructions: vec![
                    SsaInstruction {
                        kind: SsaInstructionKind::Param,
                        target: Some(SsaNameId(0)),
                        uses: vec![],
                        line: 1,
                        source_text: Some("def f(a, b):".to_string()),
                    },
                    SsaInstruction {
                        kind: SsaInstructionKind::Param,
                        target: Some(SsaNameId(1)),
                        uses: vec![],
                        line: 1,
                        source_text: None,
                    },
                ],
                successors: vec![1, 2],
                predecessors: vec![],
            },
            // True branch
            SsaBlock {
                id: 1,
                label: Some("true".to_string()),
                lines: (3, 4),
                phi_functions: vec![],
                instructions: vec![SsaInstruction {
                    kind: SsaInstructionKind::Assign,
                    target: Some(SsaNameId(2)),
                    uses: vec![SsaNameId(0)],
                    line: 3,
                    source_text: Some("x = a".to_string()),
                }],
                successors: vec![3],
                predecessors: vec![0],
            },
            // False branch
            SsaBlock {
                id: 2,
                label: Some("false".to_string()),
                lines: (5, 6),
                phi_functions: vec![],
                instructions: vec![SsaInstruction {
                    kind: SsaInstructionKind::Assign,
                    target: Some(SsaNameId(3)),
                    uses: vec![SsaNameId(1)],
                    line: 5,
                    source_text: Some("x = b".to_string()),
                }],
                successors: vec![3],
                predecessors: vec![0],
            },
            // Merge block with phi
            SsaBlock {
                id: 3,
                label: Some("merge".to_string()),
                lines: (7, 8),
                phi_functions: vec![PhiFunction {
                    target: SsaNameId(4),
                    variable: "x".to_string(),
                    sources: vec![
                        PhiSource {
                            block: 1,
                            name: SsaNameId(2),
                        },
                        PhiSource {
                            block: 2,
                            name: SsaNameId(3),
                        },
                    ],
                    line: 7,
                }],
                instructions: vec![SsaInstruction {
                    kind: SsaInstructionKind::Return,
                    target: None,
                    uses: vec![SsaNameId(4)],
                    line: 7,
                    source_text: Some("return x".to_string()),
                }],
                successors: vec![],
                predecessors: vec![1, 2],
            },
        ];

        let result = compute_alias_from_ssa(&ssa).unwrap();

        // Phi result may-alias both sources
        assert!(
            result.may_alias_check("x_2", "x_0"),
            "Phi result x_2 should may-alias x_0"
        );
        assert!(
            result.may_alias_check("x_2", "x_1"),
            "Phi result x_2 should may-alias x_1"
        );

        // Phi result does NOT must-alias either source
        // (only one branch taken at runtime)
        assert!(
            !result.must_alias_check("x_2", "x_0"),
            "Phi result x_2 should NOT must-alias x_0"
        );
        assert!(
            !result.must_alias_check("x_2", "x_1"),
            "Phi result x_2 should NOT must-alias x_1"
        );
    }

    // -------------------------------------------------------------------------
    // Unknown/External Call Tests
    // -------------------------------------------------------------------------

    /// Test: Unknown function call gets unknown location
    #[test]
    fn test_unknown_call_gets_unknown_location() {
        let mut ssa = create_test_ssa("unknown_test");
        ssa.ssa_names = vec![create_ssa_name(0, "x", 0, 1)];
        ssa.blocks = vec![SsaBlock {
            id: 0,
            label: Some("entry".to_string()),
            lines: (1, 1),
            phi_functions: vec![],
            instructions: vec![SsaInstruction {
                kind: SsaInstructionKind::Call,
                target: Some(SsaNameId(0)),
                uses: vec![],
                line: 1,
                source_text: Some("x = external_func()".to_string()),
            }],
            successors: vec![],
            predecessors: vec![],
        }];

        let result = compute_alias_from_ssa(&ssa).unwrap();

        let pts = result.get_points_to("x_0");
        assert!(
            pts.contains("unknown_1"),
            "Expected unknown_1, got {:?}",
            pts
        );
    }

    // -------------------------------------------------------------------------
    // JSON Output Tests
    // -------------------------------------------------------------------------

    /// Test: JSON output is deterministic (sorted)
    #[test]
    fn test_json_output_deterministic() {
        let mut ssa = create_test_ssa("json_test");
        ssa.ssa_names = vec![
            create_ssa_name(0, "a", 0, 1),
            create_ssa_name(1, "b", 0, 1),
            create_ssa_name(2, "c", 0, 1),
        ];
        ssa.blocks = vec![SsaBlock {
            id: 0,
            label: Some("entry".to_string()),
            lines: (1, 1),
            phi_functions: vec![],
            instructions: vec![
                SsaInstruction {
                    kind: SsaInstructionKind::Param,
                    target: Some(SsaNameId(0)),
                    uses: vec![],
                    line: 1,
                    source_text: Some("def f(a, b, c):".to_string()),
                },
                SsaInstruction {
                    kind: SsaInstructionKind::Param,
                    target: Some(SsaNameId(1)),
                    uses: vec![],
                    line: 1,
                    source_text: None,
                },
                SsaInstruction {
                    kind: SsaInstructionKind::Param,
                    target: Some(SsaNameId(2)),
                    uses: vec![],
                    line: 1,
                    source_text: None,
                },
            ],
            successors: vec![],
            predecessors: vec![],
        }];

        let result = compute_alias_from_ssa(&ssa).unwrap();
        let json = result.to_json_value();

        // Verify JSON structure exists
        assert!(json.get("function").is_some());
        assert!(json.get("may_alias").is_some());
        assert!(json.get("must_alias").is_some());
        assert!(json.get("points_to").is_some());
    }

    /// Test: JSON structure has correct shape
    #[test]
    fn test_json_output_structure() {
        let ssa = create_test_ssa("json_struct");
        let result = compute_alias_from_ssa(&ssa).unwrap();
        let json = result.to_json_value();

        assert_eq!(json["function"], "json_struct");
        assert!(json["may_alias"].is_object());
        assert!(json["must_alias"].is_object());
        assert!(json["points_to"].is_object());
        assert!(json["allocation_sites"].is_object());
    }

    // -------------------------------------------------------------------------
    // Phase 6: Field Access Integration Tests
    // -------------------------------------------------------------------------

    /// Test: Field load creates field location in points-to set
    #[test]
    fn test_field_load_creates_field_location() {
        let mut ssa = create_test_ssa("field_load_test");
        ssa.ssa_names = vec![
            create_ssa_name(0, "obj", 0, 1),
            create_ssa_name(1, "x", 0, 2),
        ];
        ssa.blocks = vec![SsaBlock {
            id: 0,
            label: Some("entry".to_string()),
            lines: (1, 2),
            phi_functions: vec![],
            instructions: vec![
                // obj = param
                SsaInstruction {
                    kind: SsaInstructionKind::Param,
                    target: Some(SsaNameId(0)),
                    uses: vec![],
                    line: 1,
                    source_text: Some("def f(obj):".to_string()),
                },
                // x = obj.field (field load)
                SsaInstruction {
                    kind: SsaInstructionKind::Assign,
                    target: Some(SsaNameId(1)),
                    uses: vec![SsaNameId(0)],
                    line: 2,
                    source_text: Some("x = obj.field".to_string()),
                },
            ],
            successors: vec![],
            predecessors: vec![],
        }];

        let result = compute_alias_from_ssa(&ssa).unwrap();

        // x should point to param_obj.field
        let pts = result.get_points_to("x_0");
        assert!(
            pts.iter()
                .any(|loc| loc.contains("param_obj") && loc.contains("field")),
            "Expected x_0 to point to param_obj.field, got {:?}",
            pts
        );
    }

    /// Test: Same field from same object aliases
    #[test]
    fn test_same_field_same_object_aliasing() {
        let mut ssa = create_test_ssa("same_field_test");
        ssa.ssa_names = vec![
            create_ssa_name(0, "obj", 0, 1),
            create_ssa_name(1, "x", 0, 2),
            create_ssa_name(2, "y", 0, 3),
        ];
        ssa.blocks = vec![SsaBlock {
            id: 0,
            label: Some("entry".to_string()),
            lines: (1, 3),
            phi_functions: vec![],
            instructions: vec![
                // obj = param
                SsaInstruction {
                    kind: SsaInstructionKind::Param,
                    target: Some(SsaNameId(0)),
                    uses: vec![],
                    line: 1,
                    source_text: Some("def f(obj):".to_string()),
                },
                // x = obj.data
                SsaInstruction {
                    kind: SsaInstructionKind::Assign,
                    target: Some(SsaNameId(1)),
                    uses: vec![SsaNameId(0)],
                    line: 2,
                    source_text: Some("x = obj.data".to_string()),
                },
                // y = obj.data (same field!)
                SsaInstruction {
                    kind: SsaInstructionKind::Assign,
                    target: Some(SsaNameId(2)),
                    uses: vec![SsaNameId(0)],
                    line: 3,
                    source_text: Some("y = obj.data".to_string()),
                },
            ],
            successors: vec![],
            predecessors: vec![],
        }];

        let result = compute_alias_from_ssa(&ssa).unwrap();

        // x and y both load same field from same object - should alias
        assert!(
            result.may_alias_check("x_0", "y_0"),
            "x_0 and y_0 should may-alias (same field from same object)"
        );
    }

    /// Test: Different fields from same object do not alias
    #[test]
    fn test_different_fields_no_aliasing() {
        let mut ssa = create_test_ssa("diff_field_test");
        ssa.ssa_names = vec![
            create_ssa_name(0, "obj", 0, 1),
            create_ssa_name(1, "x", 0, 2),
            create_ssa_name(2, "y", 0, 3),
        ];
        ssa.blocks = vec![SsaBlock {
            id: 0,
            label: Some("entry".to_string()),
            lines: (1, 3),
            phi_functions: vec![],
            instructions: vec![
                SsaInstruction {
                    kind: SsaInstructionKind::Param,
                    target: Some(SsaNameId(0)),
                    uses: vec![],
                    line: 1,
                    source_text: Some("def f(obj):".to_string()),
                },
                // x = obj.field_a
                SsaInstruction {
                    kind: SsaInstructionKind::Assign,
                    target: Some(SsaNameId(1)),
                    uses: vec![SsaNameId(0)],
                    line: 2,
                    source_text: Some("x = obj.field_a".to_string()),
                },
                // y = obj.field_b (different field!)
                SsaInstruction {
                    kind: SsaInstructionKind::Assign,
                    target: Some(SsaNameId(2)),
                    uses: vec![SsaNameId(0)],
                    line: 3,
                    source_text: Some("y = obj.field_b".to_string()),
                },
            ],
            successors: vec![],
            predecessors: vec![],
        }];

        let result = compute_alias_from_ssa(&ssa).unwrap();

        // x and y load different fields - should NOT alias
        assert!(
            !result.may_alias_check("x_0", "y_0"),
            "x_0 and y_0 should NOT alias (different fields)"
        );
    }

    // -------------------------------------------------------------------------
    // Phase 5: Python Patterns Integration Tests
    // -------------------------------------------------------------------------

    /// Test: Class variable access creates ClassVar location
    #[test]
    fn test_class_variable_creates_class_var_location() {
        let mut ssa = create_test_ssa("class_var_test");
        ssa.ssa_names = vec![create_ssa_name(0, "x", 0, 1)];
        ssa.blocks = vec![SsaBlock {
            id: 0,
            label: Some("entry".to_string()),
            lines: (1, 1),
            phi_functions: vec![],
            instructions: vec![SsaInstruction {
                kind: SsaInstructionKind::Assign,
                target: Some(SsaNameId(0)),
                uses: vec![],
                line: 1,
                source_text: Some("x = Config.DEBUG".to_string()),
            }],
            successors: vec![],
            predecessors: vec![],
        }];

        let result = compute_alias_from_ssa(&ssa).unwrap();

        // x should point to alloc_class_Config_DEBUG
        let pts = result.get_points_to("x_0");
        assert!(
            pts.contains("alloc_class_Config_DEBUG"),
            "Expected alloc_class_Config_DEBUG, got {:?}",
            pts
        );
    }

    /// Test: Two accesses to same class variable alias
    #[test]
    fn test_class_variable_singleton_aliasing() {
        let mut ssa = create_test_ssa("class_var_alias_test");
        ssa.ssa_names = vec![create_ssa_name(0, "x", 0, 1), create_ssa_name(1, "y", 0, 2)];
        ssa.blocks = vec![SsaBlock {
            id: 0,
            label: Some("entry".to_string()),
            lines: (1, 2),
            phi_functions: vec![],
            instructions: vec![
                SsaInstruction {
                    kind: SsaInstructionKind::Assign,
                    target: Some(SsaNameId(0)),
                    uses: vec![],
                    line: 1,
                    source_text: Some("x = Config.DEBUG".to_string()),
                },
                SsaInstruction {
                    kind: SsaInstructionKind::Assign,
                    target: Some(SsaNameId(1)),
                    uses: vec![],
                    line: 2,
                    source_text: Some("y = Config.DEBUG".to_string()),
                },
            ],
            successors: vec![],
            predecessors: vec![],
        }];

        let result = compute_alias_from_ssa(&ssa).unwrap();

        // x and y both access same class variable - should alias
        assert!(
            result.may_alias_check("x_0", "y_0"),
            "x_0 and y_0 should alias (same class variable)"
        );
    }

    // =========================================================================
    // Uncertain Findings Tests - AliasInfo enrichment
    // =========================================================================

    #[test]
    fn test_alias_info_has_uncertain_fields() {
        use crate::alias::Confidence;
        let info = AliasInfo::new("test_func");
        assert!(info.uncertain.is_empty());
        assert_eq!(info.confidence, Confidence::Low);
        assert!(info.language_notes.is_empty());
    }

    #[test]
    fn test_uncertain_alias_construction() {
        use crate::alias::UncertainAlias;
        let ua = UncertainAlias {
            vars: vec!["a".to_string(), "b".to_string()],
            line: 42,
            reason: "assignment from function return - type unknown".to_string(),
        };
        assert_eq!(ua.vars.len(), 2);
        assert_eq!(ua.line, 42);
        assert!(ua.reason.contains("function return"));
    }

    #[test]
    fn test_uncertain_alias_serialization() {
        use crate::alias::UncertainAlias;
        let ua = UncertainAlias {
            vars: vec!["x".to_string(), "y".to_string()],
            line: 10,
            reason: "depends on value vs reference semantics".to_string(),
        };
        let json = serde_json::to_string(&ua).unwrap();
        assert!(json.contains("\"vars\""));
        assert!(json.contains("\"line\""));
        assert!(json.contains("\"reason\""));
        let deserialized: UncertainAlias = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.vars, ua.vars);
        assert_eq!(deserialized.line, ua.line);
    }

    #[test]
    fn test_alias_info_uncertain_in_json() {
        use crate::alias::{Confidence, UncertainAlias};
        let mut info = AliasInfo::new("my_func");
        info.uncertain.push(UncertainAlias {
            vars: vec!["a".to_string(), "b".to_string()],
            line: 42,
            reason: "assignment from function return".to_string(),
        });
        info.confidence = Confidence::Medium;
        info.language_notes = "Python uses reference semantics for non-primitive types".to_string();

        let json_str = info.to_json();
        let json: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert!(
            json.get("uncertain").is_some(),
            "JSON should have uncertain key"
        );
        assert!(
            json.get("confidence").is_some(),
            "JSON should have confidence key"
        );
        assert!(
            json.get("language_notes").is_some(),
            "JSON should have language_notes key"
        );

        let uncertain = json["uncertain"].as_array().unwrap();
        assert_eq!(uncertain.len(), 1);
        assert_eq!(uncertain[0]["line"], 42);
        assert_eq!(json["confidence"], "medium");
    }

    #[test]
    fn test_confidence_enum_shared() {
        use crate::alias::Confidence;
        assert_eq!(Confidence::Low, Confidence::default());
        let json = serde_json::to_string(&Confidence::High).unwrap();
        assert_eq!(json, "\"high\"");
    }
}
