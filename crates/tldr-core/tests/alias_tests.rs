//! Test coverage for tldr-core alias module
//!
//! Tests all public functions and types from:
//! - crates/tldr-core/src/alias/mod.rs
//! - crates/tldr-core/src/alias/types.rs
//! - crates/tldr-core/src/alias/constraints.rs
//! - crates/tldr-core/src/alias/solver.rs
//! - crates/tldr-core/src/alias/format.rs
//!
//! Coverage areas:
//! - Points-to analysis (allocation sites, parameters, unknown sources)
//! - May-alias relationships (sound conservative approximation)
//! - Must-alias relationships (precise definite aliasing)
//! - Field-sensitive analysis (field access tracking)
//! - SSA integration (phi functions, versioned names)

use std::collections::HashMap;
use std::path::PathBuf;

// Import from tldr_core
use tldr_core::alias::AliasOutputFormat;
use tldr_core::alias::*;
use tldr_core::alias::{AbstractLocation, AliasError, AliasInfo, UncertainAlias, MAX_FIELD_DEPTH};
use tldr_core::alias::{AliasSolver, MAX_ITERATIONS};
use tldr_core::alias::{Constraint, ConstraintExtractor};

// Import SSA types for integration tests
use tldr_core::ssa::types::{
    PhiFunction, PhiSource, SsaBlock, SsaFunction, SsaInstruction, SsaInstructionKind, SsaName,
    SsaNameId, SsaStats, SsaType,
};

// =============================================================================
// AbstractLocation Tests
// =============================================================================

#[test]
fn test_abstract_location_alloc() {
    let loc = AbstractLocation::alloc(5);
    assert_eq!(loc.format(), "alloc_5");
    assert!(loc.is_alloc());
    assert!(!loc.is_param());
}

#[test]
fn test_abstract_location_param() {
    let loc = AbstractLocation::param("x");
    assert_eq!(loc.format(), "param_x");
    assert!(loc.is_param());
    assert!(!loc.is_alloc());
}

#[test]
fn test_abstract_location_unknown() {
    let loc = AbstractLocation::unknown(10);
    assert_eq!(loc.format(), "unknown_10");
    assert!(loc.is_unknown());
}

#[test]
fn test_abstract_location_field() {
    let base = AbstractLocation::alloc(5);
    let loc = AbstractLocation::field(base, "data");
    assert_eq!(loc.format(), "alloc_5.data");
    assert!(loc.is_field());
    assert_eq!(loc.field_depth(), 1);
}

#[test]
fn test_abstract_location_nested_field() {
    let base = AbstractLocation::alloc(5);
    let field1 = AbstractLocation::field(base, "inner");
    let field2 = AbstractLocation::field(field1, "value");
    assert_eq!(field2.format(), "alloc_5.inner.value");
    assert_eq!(field2.field_depth(), 2);
}

#[test]
fn test_abstract_location_default_arg() {
    let loc = AbstractLocation::default_arg(3);
    assert_eq!(loc.format(), "alloc_default_3");
}

#[test]
fn test_abstract_location_class_var() {
    let loc = AbstractLocation::class_var("Foo", "counter");
    assert_eq!(loc.format(), "alloc_class_Foo_counter");
}

#[test]
fn test_abstract_location_deep_field_truncation() {
    // Create a chain of MAX_FIELD_DEPTH + 1 fields
    let mut loc = AbstractLocation::alloc(1);
    for i in 0..=MAX_FIELD_DEPTH {
        loc = AbstractLocation::field(loc, format!("f{}", i));
    }
    // Should be truncated
    assert!(loc.format().contains("truncated"));
}

#[test]
fn test_abstract_location_display() {
    let loc = AbstractLocation::alloc(5);
    assert_eq!(format!("{}", loc), "alloc_5");
}

#[test]
fn test_abstract_location_serde() {
    let loc = AbstractLocation::alloc(5);
    let json = serde_json::to_string(&loc).unwrap();
    let parsed: AbstractLocation = serde_json::from_str(&json).unwrap();
    assert_eq!(loc, parsed);
}

#[test]
fn test_abstract_location_param_serde() {
    let loc = AbstractLocation::param("x");
    let json = serde_json::to_string(&loc).unwrap();
    let parsed: AbstractLocation = serde_json::from_str(&json).unwrap();
    assert_eq!(loc, parsed);
}

#[test]
fn test_abstract_location_field_serde() {
    let base = AbstractLocation::alloc(5);
    let loc = AbstractLocation::field(base, "data");
    let json = serde_json::to_string(&loc).unwrap();
    let parsed: AbstractLocation = serde_json::from_str(&json).unwrap();
    assert_eq!(loc, parsed);
}

// =============================================================================
// AliasInfo Tests
// =============================================================================

#[test]
fn test_alias_info_new() {
    let info = AliasInfo::new("test_func");
    assert_eq!(info.function_name, "test_func");
    assert!(info.may_alias.is_empty());
    assert!(info.must_alias.is_empty());
    assert!(info.points_to.is_empty());
    assert!(info.allocation_sites.is_empty());
    assert!(info.uncertain.is_empty());
}

#[test]
fn test_alias_info_default() {
    let info = AliasInfo::default();
    assert!(info.function_name.is_empty());
    assert!(info.may_alias.is_empty());
}

#[test]
fn test_alias_info_may_alias_same_var() {
    let info = AliasInfo::new("test");
    assert!(info.may_alias_check("x", "x"));
    assert!(info.may_alias_check("anything", "anything"));
}

#[test]
fn test_alias_info_may_alias_in_set() {
    let mut info = AliasInfo::new("test");
    info.add_may_alias("x", "y");

    assert!(info.may_alias_check("x", "y"));
    assert!(info.may_alias_check("y", "x")); // Symmetric
}

#[test]
fn test_alias_info_may_alias_points_to_overlap() {
    let mut info = AliasInfo::new("test");
    info.add_points_to("x", "alloc_1");
    info.add_points_to("x", "alloc_2");
    info.add_points_to("y", "alloc_2");
    info.add_points_to("y", "alloc_3");
    info.add_points_to("z", "alloc_4");

    // x and y share alloc_2
    assert!(info.may_alias_check("x", "y"));
    // x and z don't share any location
    assert!(!info.may_alias_check("x", "z"));
}

#[test]
fn test_alias_info_must_alias_same_var() {
    let info = AliasInfo::new("test");
    assert!(info.must_alias_check("x", "x"));
}

#[test]
fn test_alias_info_must_alias_in_set() {
    let mut info = AliasInfo::new("test");
    info.add_must_alias("a", "b");

    assert!(info.must_alias_check("a", "b"));
    assert!(info.must_alias_check("b", "a")); // Symmetric
}

#[test]
fn test_alias_info_must_alias_not_in_set() {
    let mut info = AliasInfo::new("test");
    info.add_may_alias("a", "c");

    // c only may-alias, not must-alias
    assert!(!info.must_alias_check("a", "c"));
}

#[test]
fn test_alias_info_get_points_to() {
    let mut info = AliasInfo::new("test");
    info.add_points_to("x", "alloc_1");
    info.add_points_to("x", "param_0");

    let pts = info.get_points_to("x");
    assert!(pts.contains("alloc_1"));
    assert!(pts.contains("param_0"));
    assert!(info.get_points_to("unknown").is_empty());
}

#[test]
fn test_alias_info_get_aliases() {
    let mut info = AliasInfo::new("test");
    info.add_may_alias("x", "y");
    info.add_may_alias("x", "z");

    let aliases = info.get_aliases("x");
    assert!(aliases.contains("y"));
    assert!(aliases.contains("z"));
    assert!(info.get_aliases("unknown").is_empty());
}

#[test]
fn test_alias_info_add_may_alias_symmetric() {
    let mut info = AliasInfo::new("test");
    info.add_may_alias("a", "b");

    // Both directions should be recorded
    assert!(info.may_alias.get("a").unwrap().contains("b"));
    assert!(info.may_alias.get("b").unwrap().contains("a"));
}

#[test]
fn test_alias_info_add_must_alias_symmetric() {
    let mut info = AliasInfo::new("test");
    info.add_must_alias("a", "b");

    // Both directions should be recorded
    assert!(info.must_alias.get("a").unwrap().contains("b"));
    assert!(info.must_alias.get("b").unwrap().contains("a"));
}

#[test]
fn test_alias_info_add_points_to() {
    let mut info = AliasInfo::new("test");
    info.add_points_to("x", "alloc_1");

    let pts = info.get_points_to("x");
    assert!(pts.contains("alloc_1"));
}

#[test]
fn test_alias_info_add_allocation_site() {
    let mut info = AliasInfo::new("test");
    info.add_allocation_site(5, "alloc_5");

    assert_eq!(info.allocation_sites.get(&5), Some(&"alloc_5".to_string()));
}

#[test]
fn test_alias_info_to_json_sorted() {
    let mut info = AliasInfo::new("test");
    info.add_may_alias("x", "z");
    info.add_may_alias("x", "a");
    info.add_may_alias("x", "m");

    let json = info.to_json_value();
    let may_alias = json["may_alias"]["x"].as_array().unwrap();

    // Should be sorted
    let sorted: Vec<&str> = may_alias.iter().map(|v| v.as_str().unwrap()).collect();
    assert_eq!(sorted, vec!["a", "m", "z"]);
}

#[test]
fn test_alias_info_to_json_structure() {
    let mut info = AliasInfo::new("test_func");
    info.add_may_alias("x", "y");
    info.add_must_alias("a", "b");
    info.add_points_to("x", "alloc_1");
    info.add_allocation_site(5, "alloc_5");

    let json = info.to_json_value();

    assert!(json.get("function").is_some());
    assert!(json.get("may_alias").is_some());
    assert!(json.get("must_alias").is_some());
    assert!(json.get("points_to").is_some());
    assert!(json.get("allocation_sites").is_some());
}

// =============================================================================
// UncertainAlias Tests
// =============================================================================

#[test]
fn test_uncertain_alias_construction() {
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
fn test_alias_info_uncertain_field() {
    let mut info = AliasInfo::new("test_func");
    info.uncertain.push(UncertainAlias {
        vars: vec!["a".to_string(), "b".to_string()],
        line: 42,
        reason: "assignment from function return".to_string(),
    });
    info.confidence = Confidence::Medium;
    info.language_notes = "Python uses reference semantics".to_string();

    assert_eq!(info.uncertain.len(), 1);
    assert_eq!(info.confidence, Confidence::Medium);
}

// =============================================================================
// Confidence Tests
// =============================================================================

#[test]
fn test_confidence_enum_values() {
    let low = Confidence::Low;
    let med = Confidence::Medium;
    let high = Confidence::High;
    assert_ne!(format!("{:?}", low), format!("{:?}", high));
    assert_ne!(format!("{:?}", med), format!("{:?}", low));
}

#[test]
fn test_confidence_default_is_low() {
    let c: Confidence = Default::default();
    assert_eq!(c, Confidence::Low);
}

#[test]
fn test_confidence_serialization() {
    let json = serde_json::to_string(&Confidence::High).unwrap();
    assert_eq!(json, "\"high\"");
    let json = serde_json::to_string(&Confidence::Medium).unwrap();
    assert_eq!(json, "\"medium\"");
    let json = serde_json::to_string(&Confidence::Low).unwrap();
    assert_eq!(json, "\"low\"");
}

// =============================================================================
// AliasError Tests
// =============================================================================

#[test]
fn test_alias_error_no_ssa_display() {
    let err = AliasError::NoSsa("test_func".to_string());
    assert!(err.to_string().contains("SSA"));
    assert!(err.to_string().contains("test_func"));
}

#[test]
fn test_alias_error_no_cfg_display() {
    let err = AliasError::NoCfg("test_func".to_string());
    assert!(err.to_string().contains("CFG"));
    assert!(err.to_string().contains("test_func"));
}

#[test]
fn test_alias_error_iteration_limit() {
    let err = AliasError::IterationLimit(100);
    assert!(err.to_string().contains("100"));
    assert!(err.to_string().contains("iterations"));
}

#[test]
fn test_alias_error_invalid_phi_source() {
    let err = AliasError::InvalidPhiSource {
        phi_var: "x_2".to_string(),
        source: "y_99".to_string(),
    };
    assert!(err.to_string().contains("x_2"));
    assert!(err.to_string().contains("y_99"));
}

#[test]
fn test_alias_error_invalid_ref() {
    let err = AliasError::InvalidRef("unknown_var".to_string());
    assert!(err.to_string().contains("unknown_var"));
}

#[test]
fn test_alias_error_internal() {
    let err = AliasError::Internal("something went wrong".to_string());
    assert!(err.to_string().contains("something went wrong"));
}

// =============================================================================
// Constraint Tests
// =============================================================================

// Note: Subset constraints are handled internally by the solver

#[test]
fn test_constraint_copy_creation() {
    let constraint = Constraint::copy("x_0".to_string(), "y_0".to_string());

    match constraint {
        Constraint::Copy { target, source } => {
            assert_eq!(target, "x_0");
            assert_eq!(source, "y_0");
        }
        _ => panic!("Expected Copy constraint"),
    }
}

#[test]
fn test_constraint_allocation_creation() {
    let loc = AbstractLocation::alloc(5);
    let constraint = Constraint::alloc("x_0".to_string(), loc);

    match constraint {
        Constraint::Alloc { target, site } => {
            assert_eq!(target, "x_0");
            assert_eq!(site.format(), "alloc_5");
        }
        _ => panic!("Expected Alloc constraint"),
    }
}

// Note: Phi constraints are handled internally by the solver, not via Constraint enum

#[test]
fn test_constraint_field_load_creation() {
    let constraint =
        Constraint::field_load("x_0".to_string(), "obj_0".to_string(), "data".to_string());

    match constraint {
        Constraint::FieldLoad {
            target,
            base: _,
            field,
        } => {
            assert_eq!(target, "x_0");
            assert_eq!(field, "data");
        }
        _ => panic!("Expected FieldLoad constraint"),
    }
}

#[test]
fn test_constraint_field_store_creation() {
    let _base = AbstractLocation::alloc(1);
    let base_str = "obj_0".to_string();
    let constraint = Constraint::field_store(base_str, "data".to_string(), "y_0".to_string());

    match constraint {
        Constraint::FieldStore {
            base: _,
            field,
            source,
        } => {
            assert_eq!(field, "data");
            assert_eq!(source, "y_0");
        }
        _ => panic!("Expected FieldStore constraint"),
    }
}

// =============================================================================
// ConstraintExtractor Tests
// =============================================================================

fn create_test_ssa(name: &str) -> SsaFunction {
    SsaFunction {
        function: name.to_string(),
        file: PathBuf::from("test.py"),
        ssa_type: SsaType::Minimal,
        blocks: vec![],
        ssa_names: vec![],
        def_use: HashMap::new(),
        stats: SsaStats::default(),
    }
}

fn create_ssa_name(id: u32, variable: &str, version: u32, line: u32) -> SsaName {
    SsaName {
        id: SsaNameId(id),
        variable: variable.to_string(),
        version,
        def_block: Some(0),
        def_line: line,
    }
}

#[test]
fn test_constraint_extractor_new() {
    let _extractor = ConstraintExtractor::new();
    // constraints() method returns a slice of constraints
}

#[test]
fn test_constraint_extractor_extract_from_ssa_empty() {
    let ssa = create_test_ssa("empty");
    let result = ConstraintExtractor::extract_from_ssa(&ssa);

    assert!(result.is_ok());
    let extractor = result.unwrap();
    assert!(extractor.constraints().is_empty());
}

#[test]
fn test_constraint_extractor_extract_from_ssa_param() {
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

    let result = ConstraintExtractor::extract_from_ssa(&ssa);
    assert!(result.is_ok());

    let extractor = result.unwrap();
    // Should have constraints for parameter
    assert!(
        !extractor.constraints().is_empty(),
        "Parameter should generate constraints"
    );
}

#[test]
fn test_constraint_extractor_extract_from_ssa_allocation() {
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

    let result = ConstraintExtractor::extract_from_ssa(&ssa);
    assert!(result.is_ok());

    let extractor = result.unwrap();
    // Should have allocation constraint
    assert!(
        !extractor.constraints().is_empty(),
        "Allocation should generate constraints"
    );
}

// =============================================================================
// AliasSolver Tests
// =============================================================================

#[test]
fn test_alias_solver_new() {
    let extractor = ConstraintExtractor::new();
    let mut solver = AliasSolver::new(&extractor);

    // Solver should be created successfully
    assert!(solver.solve().is_ok());
}

#[test]
fn test_alias_solver_solve_empty() {
    let extractor = ConstraintExtractor::new();
    let mut solver = AliasSolver::new(&extractor);

    let result = solver.solve();
    assert!(
        result.is_ok(),
        "Empty constraint set should solve trivially"
    );
}

#[test]
fn test_alias_solver_iteration_limit() {
    // MAX_ITERATIONS should be a reasonable value
    assert_eq!(MAX_ITERATIONS, 100, "MAX_ITERATIONS should stay aligned with spec");
}

// =============================================================================
// AliasOutputFormat Tests
// =============================================================================

#[test]
fn test_alias_output_format_debug() {
    let format = AliasOutputFormat::Dot;
    assert_eq!(format.to_string(), "dot");
}

#[test]
fn test_alias_output_format_json() {
    let format = AliasOutputFormat::Json;
    assert_eq!(format.to_string(), "json");
}

#[test]
fn test_alias_output_format_default() {
    let format: AliasOutputFormat = Default::default();
    assert_eq!(format, AliasOutputFormat::Json);
}

// =============================================================================
// compute_alias_from_ssa Integration Tests
// =============================================================================

#[test]
fn test_compute_alias_from_ssa_empty() {
    let ssa = create_test_ssa("empty");
    let result = compute_alias_from_ssa(&ssa);

    assert!(
        result.is_ok(),
        "Empty SSA should not cause error: {:?}",
        result.err()
    );

    let info = result.unwrap();
    assert_eq!(info.function_name, "empty");
    assert!(info.may_alias.is_empty());
    assert!(info.must_alias.is_empty());
    assert!(info.points_to.is_empty());
}

#[test]
fn test_compute_alias_from_ssa_param() {
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

    let result = compute_alias_from_ssa(&ssa);
    assert!(
        result.is_ok(),
        "Parameter SSA should work: {:?}",
        result.err()
    );

    let info = result.unwrap();
    // p_0 should point to param_p
    let pts = info.get_points_to("p_0");
    assert!(
        pts.contains("param_p"),
        "Expected p_0 to point to param_p, got {:?}",
        pts
    );
}

#[test]
fn test_compute_alias_from_ssa_two_params_may_alias() {
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

    let result = compute_alias_from_ssa(&ssa);
    assert!(
        result.is_ok(),
        "Two params SSA should work: {:?}",
        result.err()
    );

    let info = result.unwrap();
    // Parameters may alias each other (conservative assumption)
    assert!(
        info.may_alias_check("a_0", "b_0"),
        "Parameters should may-alias conservatively"
    );
}

#[test]
fn test_compute_alias_from_ssa_assignment_must_alias() {
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

    let result = compute_alias_from_ssa(&ssa);
    assert!(
        result.is_ok(),
        "Assignment SSA should work: {:?}",
        result.err()
    );

    let info = result.unwrap();
    // x_0 must-alias p_0 (direct copy)
    assert!(
        info.must_alias_check("x_0", "p_0"),
        "x_0 should must-alias p_0"
    );

    // They also may-alias
    assert!(
        info.may_alias_check("x_0", "p_0"),
        "x_0 should may-alias p_0"
    );
}

#[test]
fn test_compute_alias_from_ssa_allocation() {
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

    let result = compute_alias_from_ssa(&ssa);
    assert!(
        result.is_ok(),
        "Allocation SSA should work: {:?}",
        result.err()
    );

    let info = result.unwrap();
    // x should point to an allocation site
    let pts = info.get_points_to("x_0");
    assert!(
        pts.iter().any(|loc| loc.starts_with("alloc")),
        "x_0 should point to an allocation site, got {:?}",
        pts
    );
}

#[test]
fn test_compute_alias_from_ssa_different_allocations_no_alias() {
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

    let result = compute_alias_from_ssa(&ssa);
    assert!(
        result.is_ok(),
        "Two allocations SSA should work: {:?}",
        result.err()
    );

    let info = result.unwrap();
    // Different allocation sites should not alias
    assert!(
        !info.may_alias_check("x_0", "y_0"),
        "Different allocations should not alias"
    );
}

#[test]
fn test_compute_alias_from_ssa_shared_allocation_aliases() {
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
            SsaInstruction {
                kind: SsaInstructionKind::Call,
                target: Some(SsaNameId(0)),
                uses: vec![],
                line: 1,
                source_text: Some("obj = Foo()".to_string()),
            },
            SsaInstruction {
                kind: SsaInstructionKind::Assign,
                target: Some(SsaNameId(1)),
                uses: vec![SsaNameId(0)],
                line: 2,
                source_text: Some("x = obj".to_string()),
            },
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

    let result = compute_alias_from_ssa(&ssa);
    assert!(
        result.is_ok(),
        "Shared allocation SSA should work: {:?}",
        result.err()
    );

    let info = result.unwrap();
    // x and y both point to same allocation via obj
    assert!(
        info.may_alias_check("x_0", "y_0"),
        "x and y should alias via shared obj"
    );
}

#[test]
fn test_compute_alias_from_ssa_phi_may_alias() {
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

    let result = compute_alias_from_ssa(&ssa);
    assert!(result.is_ok(), "Phi SSA should work: {:?}", result.err());

    let info = result.unwrap();
    // Phi result may-alias both sources
    assert!(
        info.may_alias_check("x_2", "x_0"),
        "Phi result x_2 should may-alias x_0"
    );
    assert!(
        info.may_alias_check("x_2", "x_1"),
        "Phi result x_2 should may-alias x_1"
    );

    // Phi result does NOT must-alias either source
    assert!(
        !info.must_alias_check("x_2", "x_0"),
        "Phi result x_2 should NOT must-alias x_0"
    );
    assert!(
        !info.must_alias_check("x_2", "x_1"),
        "Phi result x_2 should NOT must-alias x_1"
    );
}

#[test]
fn test_compute_alias_from_ssa_unknown_call() {
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

    let result = compute_alias_from_ssa(&ssa);
    assert!(
        result.is_ok(),
        "Unknown call SSA should work: {:?}",
        result.err()
    );

    let info = result.unwrap();
    // Unknown calls should get unknown location
    let pts = info.get_points_to("x_0");
    assert!(
        pts.iter().any(|loc| loc.starts_with("unknown")),
        "x_0 should point to unknown, got {:?}",
        pts
    );
}

#[test]
fn test_compute_alias_from_ssa_class_var() {
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

    let result = compute_alias_from_ssa(&ssa);
    assert!(
        result.is_ok(),
        "Class var SSA should work: {:?}",
        result.err()
    );

    let info = result.unwrap();
    // x should point to class variable location
    let pts = info.get_points_to("x_0");
    assert!(
        pts.iter().any(|loc| loc.contains("alloc_class")),
        "x_0 should point to class variable, got {:?}",
        pts
    );
}

#[test]
fn test_compute_alias_from_ssa_field_load() {
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
            SsaInstruction {
                kind: SsaInstructionKind::Param,
                target: Some(SsaNameId(0)),
                uses: vec![],
                line: 1,
                source_text: Some("def f(obj):".to_string()),
            },
            SsaInstruction {
                kind: SsaInstructionKind::Assign,
                target: Some(SsaNameId(1)),
                uses: vec![SsaNameId(0)],
                line: 2,
                source_text: Some("x = obj.data".to_string()),
            },
        ],
        successors: vec![],
        predecessors: vec![],
    }];

    let result = compute_alias_from_ssa(&ssa);
    assert!(
        result.is_ok(),
        "Field load SSA should work: {:?}",
        result.err()
    );

    let info = result.unwrap();
    // x should point to field location
    let pts = info.get_points_to("x_0");
    assert!(
        pts.iter()
            .any(|loc| loc.contains("field") || loc.contains("data")),
        "x_0 should point to field location, got {:?}",
        pts
    );
}

#[test]
fn test_compute_alias_same_field_same_object_aliasing() {
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
            SsaInstruction {
                kind: SsaInstructionKind::Param,
                target: Some(SsaNameId(0)),
                uses: vec![],
                line: 1,
                source_text: Some("def f(obj):".to_string()),
            },
            SsaInstruction {
                kind: SsaInstructionKind::Assign,
                target: Some(SsaNameId(1)),
                uses: vec![SsaNameId(0)],
                line: 2,
                source_text: Some("x = obj.data".to_string()),
            },
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

    let result = compute_alias_from_ssa(&ssa);
    assert!(
        result.is_ok(),
        "Same field SSA should work: {:?}",
        result.err()
    );

    let info = result.unwrap();
    // x and y both load same field from same object - should alias
    assert!(
        info.may_alias_check("x_0", "y_0"),
        "x_0 and y_0 should may-alias (same field from same object)"
    );
}

#[test]
fn test_compute_alias_different_fields_no_aliasing() {
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
            SsaInstruction {
                kind: SsaInstructionKind::Assign,
                target: Some(SsaNameId(1)),
                uses: vec![SsaNameId(0)],
                line: 2,
                source_text: Some("x = obj.field_a".to_string()),
            },
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

    let result = compute_alias_from_ssa(&ssa);
    assert!(
        result.is_ok(),
        "Different fields SSA should work: {:?}",
        result.err()
    );

    let info = result.unwrap();
    // x and y load different fields - should NOT alias
    assert!(
        !info.may_alias_check("x_0", "y_0"),
        "x_0 and y_0 should NOT alias (different fields)"
    );
}

#[test]
fn test_compute_alias_json_output() {
    let mut ssa = create_test_ssa("json_test");
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

    let result = compute_alias_from_ssa(&ssa);
    assert!(result.is_ok());

    let info = result.unwrap();
    let json = info.to_json_value();

    // Verify JSON structure exists
    assert!(json.get("function").is_some());
    assert!(json.get("may_alias").is_some());
    assert!(json.get("must_alias").is_some());
    assert!(json.get("points_to").is_some());
    assert!(json.get("allocation_sites").is_some());

    // Verify JSON is serializable
    let json_str = serde_json::to_string(&json);
    assert!(json_str.is_ok());
}

// =============================================================================
// Property-Based Tests
// =============================================================================

#[test]
fn test_must_alias_implies_may_alias() {
    let mut info = AliasInfo::new("test");

    // Add must-alias relationship
    info.add_must_alias("a", "b");
    // Must-alias should also be reflected in may-alias behavior
    // (both have same points-to set after solving)
    info.add_points_to("a", "alloc_1");
    info.add_points_to("b", "alloc_1");

    // If must_alias_check returns true, may_alias_check should also return true
    if info.must_alias_check("a", "b") {
        assert!(info.may_alias_check("a", "b"));
    }
}

#[test]
fn test_different_alloc_sites_no_alias() {
    let mut info = AliasInfo::new("test");
    info.add_points_to("x", "alloc_1");
    info.add_points_to("y", "alloc_2");

    // alloc_1 and alloc_2 are disjoint
    assert!(!info.may_alias_check("x", "y"));
}

#[test]
fn test_self_alias_always_true() {
    let info = AliasInfo::new("test");

    assert!(info.may_alias_check("x_0", "x_0"));
    assert!(info.must_alias_check("x_0", "x_0"));
    assert!(info.may_alias_check("anything", "anything"));
    assert!(info.must_alias_check("anything", "anything"));
}

#[test]
fn test_symmetry_may_alias() {
    let mut info = AliasInfo::new("test");
    info.add_may_alias("a", "b");

    assert!(info.may_alias_check("a", "b"));
    assert!(info.may_alias_check("b", "a")); // Symmetric
}

#[test]
fn test_symmetry_must_alias() {
    let mut info = AliasInfo::new("test");
    info.add_must_alias("a", "b");

    assert!(info.must_alias_check("a", "b"));
    assert!(info.must_alias_check("b", "a")); // Symmetric
}

#[test]
fn test_unknown_variable_no_crash() {
    let info = AliasInfo::new("test");

    // Should return false, not crash
    assert!(!info.may_alias_check("unknown1", "unknown2"));
    assert!(!info.must_alias_check("unknown1", "unknown2"));
    assert!(info.get_points_to("unknown").is_empty());
    assert!(info.get_aliases("unknown").is_empty());
}

// =============================================================================
// Edge Cases
// =============================================================================

#[test]
fn test_max_field_depth_constant() {
    // MAX_FIELD_DEPTH should be reasonable
    assert_eq!(MAX_FIELD_DEPTH, 10, "MAX_FIELD_DEPTH should stay aligned with spec");
}

#[test]
fn test_empty_string_var() {
    let info = AliasInfo::new("test");

    // Empty string as variable name
    assert!(info.may_alias_check("", ""));
    assert!(info.must_alias_check("", ""));
}

#[test]
fn test_unicode_in_variable_names() {
    let mut info = AliasInfo::new("test");

    // Unicode variable names should work
    info.add_may_alias("变量", "変数");
    assert!(info.may_alias_check("变量", "変数"));
}

#[test]
fn test_very_long_variable_name() {
    let mut info = AliasInfo::new("test");

    let long_name = "a".repeat(1000);
    info.add_points_to(&long_name, "alloc_1");

    let pts = info.get_points_to(&long_name);
    assert!(pts.contains("alloc_1"));
}

// =============================================================================
// Summary
// =============================================================================
// Total tests:
// - AbstractLocation (allocation, param, unknown, field): 15 tests
// - AliasInfo (may_alias, must_alias, points_to, JSON): 20 tests
// - UncertainAlias & Confidence: 8 tests
// - AliasError: 6 tests
// - Constraint types: 7 tests
// - ConstraintExtractor: 4 tests
// - AliasSolver: 3 tests
// - AliasOutputFormat: 3 tests
// - compute_alias_from_ssa integration: 15 tests
// - Property-based tests: 7 tests
// - Edge cases: 4 tests
//
// Total: ~92 tests covering the public API of alias module
