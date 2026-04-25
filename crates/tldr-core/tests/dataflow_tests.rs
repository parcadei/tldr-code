//! Test coverage for tldr-core dataflow module
//!
//! Tests all public functions and types from:
//! - crates/tldr-core/src/dataflow/mod.rs
//! - crates/tldr-core/src/dataflow/types.rs
//! - crates/tldr-core/src/dataflow/available.rs
//! - crates/tldr-core/src/dataflow/abstract_interp.rs
//!
//! Coverage areas:
//! - Abstract interpretation (range tracking, nullability)
//! - Available expressions (CSE detection)
//! - Dataflow types and helpers (BlockId, predecessors, successors, back edges)

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

// Import from tldr_core
use tldr_core::dataflow::abstract_interp::{
    compute_abstract_interp, AbstractInterpInfo, AbstractState, AbstractValue, ConstantValue,
    Nullability,
};
use tldr_core::dataflow::available::{
    compute_available_exprs, is_function_call, normalize_expression, parse_expression_from_line,
    AvailableExprsInfo, BlockExpressions, Confidence, ExprInstance, Expression, UncertainFinding,
    COMMUTATIVE_OPS,
};
use tldr_core::dataflow::*;
use tldr_core::types::{BlockType, CfgBlock, CfgEdge, CfgInfo, DfgInfo, EdgeType, RefType, VarRef};

// =============================================================================
// Dataflow Types Tests (BlockId, DataflowError)
// =============================================================================

#[test]
fn test_block_id_is_usize_alias() {
    // BlockId should be a type alias for usize
    let block_id: BlockId = 42;
    let as_usize: usize = block_id;
    assert_eq!(as_usize, 42);
}

#[test]
fn test_dataflow_error_no_cfg_display() {
    let err = DataflowError::NoCfg;
    assert!(err.to_string().contains("CFG"));
}

#[test]
fn test_dataflow_error_no_dfg_display() {
    let err = DataflowError::NoDfg;
    assert!(err.to_string().contains("DFG"));
}

#[test]
fn test_dataflow_error_too_many_blocks() {
    let err = DataflowError::TooManyBlocks { count: 15000 };
    assert!(err.to_string().contains("15000"));
    assert!(err.to_string().contains("10000"));
}

#[test]
fn test_dataflow_error_iteration_limit() {
    let err = DataflowError::IterationLimit { iterations: 500 };
    assert!(err.to_string().contains("500"));
}

#[test]
fn test_dataflow_error_unsupported_pattern() {
    let err = DataflowError::UnsupportedCfgPattern {
        pattern: "exception edges".to_string(),
    };
    assert!(err.to_string().contains("exception edges"));
}

#[test]
fn test_max_blocks_constant() {
    assert_eq!(MAX_BLOCKS, 10_000);
}

#[test]
fn test_max_iterations_constant() {
    assert_eq!(MAX_ITERATIONS, 100);
}

// =============================================================================
// CFG Helper Functions Tests
// =============================================================================

fn make_test_cfg(
    blocks: Vec<(usize, BlockType)>,
    edges: Vec<(usize, usize)>,
    entry: usize,
) -> CfgInfo {
    CfgInfo {
        function: "test".to_string(),
        blocks: blocks
            .into_iter()
            .map(|(id, block_type)| CfgBlock {
                id,
                block_type,
                lines: (1, 10),
                calls: vec![],
            })
            .collect(),
        edges: edges
            .into_iter()
            .map(|(from, to)| CfgEdge {
                from,
                to,
                edge_type: EdgeType::Unconditional,
                condition: None,
            })
            .collect(),
        entry_block: entry,
        exit_blocks: vec![],
        cyclomatic_complexity: 1,
        nested_functions: HashMap::new(),
    }
}

#[test]
fn test_build_predecessors_empty_cfg() {
    let cfg = make_test_cfg(vec![], vec![], 0);
    let preds = build_predecessors(&cfg);
    assert!(preds.is_empty());
}

#[test]
fn test_build_predecessors_single_block() {
    let cfg = make_test_cfg(vec![(0, BlockType::Entry)], vec![], 0);
    let preds = build_predecessors(&cfg);
    assert_eq!(preds.len(), 1);
    assert!(preds.get(&0).unwrap().is_empty());
}

#[test]
fn test_build_predecessors_linear_cfg() {
    // 0 -> 1 -> 2
    let cfg = make_test_cfg(
        vec![
            (0, BlockType::Entry),
            (1, BlockType::Body),
            (2, BlockType::Exit),
        ],
        vec![(0, 1), (1, 2)],
        0,
    );
    let preds = build_predecessors(&cfg);

    assert!(preds.get(&0).unwrap().is_empty());
    assert_eq!(preds.get(&1).unwrap(), &vec![0]);
    assert_eq!(preds.get(&2).unwrap(), &vec![1]);
}

#[test]
fn test_build_predecessors_diamond_cfg() {
    //     0
    //    / \
    //   1   2
    //    \ /
    //     3
    let cfg = make_test_cfg(
        vec![
            (0, BlockType::Entry),
            (1, BlockType::Body),
            (2, BlockType::Body),
            (3, BlockType::Exit),
        ],
        vec![(0, 1), (0, 2), (1, 3), (2, 3)],
        0,
    );
    let preds = build_predecessors(&cfg);

    assert!(preds.get(&0).unwrap().is_empty());
    assert_eq!(preds.get(&1).unwrap(), &vec![0]);
    assert_eq!(preds.get(&2).unwrap(), &vec![0]);

    let preds_3 = preds.get(&3).unwrap();
    assert_eq!(preds_3.len(), 2);
    assert!(preds_3.contains(&1));
    assert!(preds_3.contains(&2));
}

#[test]
fn test_build_successors_linear_cfg() {
    // 0 -> 1 -> 2
    let cfg = make_test_cfg(
        vec![
            (0, BlockType::Entry),
            (1, BlockType::Body),
            (2, BlockType::Exit),
        ],
        vec![(0, 1), (1, 2)],
        0,
    );
    let succs = build_successors(&cfg);

    assert_eq!(succs.get(&0).unwrap(), &vec![1]);
    assert_eq!(succs.get(&1).unwrap(), &vec![2]);
    assert!(succs.get(&2).unwrap().is_empty());
}

#[test]
fn test_find_back_edges_empty_cfg() {
    let cfg = make_test_cfg(vec![], vec![], 0);
    let headers = find_back_edges(&cfg);
    assert!(headers.is_empty());
}

#[test]
fn test_find_back_edges_no_loops() {
    // 0 -> 1 -> 2 (no back edges)
    let cfg = make_test_cfg(
        vec![
            (0, BlockType::Entry),
            (1, BlockType::Body),
            (2, BlockType::Exit),
        ],
        vec![(0, 1), (1, 2)],
        0,
    );
    let headers = find_back_edges(&cfg);
    assert!(headers.is_empty());
}

#[test]
fn test_find_back_edges_simple_loop() {
    // 0 -> 1 -> 2
    //      ^    |
    //      +----+
    // Back edge: 2 -> 1, so 1 is loop header
    let cfg = make_test_cfg(
        vec![
            (0, BlockType::Entry),
            (1, BlockType::LoopHeader),
            (2, BlockType::LoopBody),
        ],
        vec![(0, 1), (1, 2), (2, 1)],
        0,
    );
    let headers = find_back_edges(&cfg);
    assert_eq!(headers.len(), 1);
    assert!(headers.contains(&1));
}

#[test]
fn test_find_back_edges_self_loop() {
    // 0 -> 1
    //      ^|
    //      +
    // Self-loop: 1 -> 1, so 1 is loop header
    let cfg = make_test_cfg(
        vec![(0, BlockType::Entry), (1, BlockType::LoopHeader)],
        vec![(0, 1), (1, 1)],
        0,
    );
    let headers = find_back_edges(&cfg);
    assert_eq!(headers.len(), 1);
    assert!(headers.contains(&1));
}

#[test]
fn test_reverse_postorder_empty_cfg() {
    let cfg = make_test_cfg(vec![], vec![], 0);
    let order = reverse_postorder(&cfg);
    assert!(order.is_empty());
}

#[test]
fn test_reverse_postorder_single_block() {
    let cfg = make_test_cfg(vec![(0, BlockType::Entry)], vec![], 0);
    let order = reverse_postorder(&cfg);
    assert_eq!(order, vec![0]);
}

#[test]
fn test_reverse_postorder_linear_cfg() {
    // 0 -> 1 -> 2
    let cfg = make_test_cfg(
        vec![
            (0, BlockType::Entry),
            (1, BlockType::Body),
            (2, BlockType::Exit),
        ],
        vec![(0, 1), (1, 2)],
        0,
    );
    let order = reverse_postorder(&cfg);
    assert_eq!(order, vec![0, 1, 2]);
}

#[test]
fn test_reverse_postorder_entry_first() {
    // Entry block should always be first in reverse postorder
    let cfg = make_test_cfg(
        vec![
            (0, BlockType::Entry),
            (1, BlockType::Body),
            (2, BlockType::Exit),
        ],
        vec![(0, 1), (0, 2)],
        0,
    );
    let order = reverse_postorder(&cfg);
    assert_eq!(order[0], cfg.entry_block);
}

#[test]
fn test_validate_cfg_empty() {
    let cfg = make_test_cfg(vec![], vec![], 0);
    let result = validate_cfg(&cfg);
    assert_eq!(result, Err(DataflowError::NoCfg));
}

#[test]
fn test_validate_cfg_valid() {
    let cfg = make_test_cfg(vec![(0, BlockType::Entry)], vec![], 0);
    let result = validate_cfg(&cfg);
    assert!(result.is_ok());
}

#[test]
fn test_reachable_blocks_empty_cfg() {
    let cfg = make_test_cfg(vec![], vec![], 0);
    let reachable = reachable_blocks(&cfg);
    assert!(reachable.is_empty());
}

#[test]
fn test_reachable_blocks_all_reachable() {
    let cfg = make_test_cfg(
        vec![
            (0, BlockType::Entry),
            (1, BlockType::Body),
            (2, BlockType::Exit),
        ],
        vec![(0, 1), (1, 2)],
        0,
    );
    let reachable = reachable_blocks(&cfg);
    assert_eq!(reachable.len(), 3);
    assert!(reachable.contains(&0));
    assert!(reachable.contains(&1));
    assert!(reachable.contains(&2));
}

#[test]
fn test_reachable_blocks_with_unreachable() {
    // Block 2 is not reachable from entry
    let cfg = make_test_cfg(
        vec![
            (0, BlockType::Entry),
            (1, BlockType::Exit),
            (2, BlockType::Body), // Unreachable
        ],
        vec![(0, 1)],
        0,
    );
    let reachable = reachable_blocks(&cfg);
    assert_eq!(reachable.len(), 2);
    assert!(reachable.contains(&0));
    assert!(reachable.contains(&1));
    assert!(!reachable.contains(&2));
}

// =============================================================================
// Available Expressions - Expression Struct Tests
// =============================================================================

#[test]
fn test_expression_creation() {
    let expr = Expression::new("a + b", vec!["a", "b"], 1);
    assert_eq!(expr.text, "a + b");
    assert_eq!(expr.line, 1);
}

#[test]
fn test_expression_equality_based_on_text_only() {
    let expr1 = Expression::new("a + b", vec!["a", "b"], 1);
    let expr2 = Expression::new("a + b", vec!["a", "b"], 100);

    assert_eq!(
        expr1, expr2,
        "Equality should be based on text only, not line"
    );
}

#[test]
fn test_expression_hash_based_on_text_only() {
    let expr1 = Expression::new("x * y", vec!["x", "y"], 1);
    let expr2 = Expression::new("x * y", vec!["x", "y"], 999);

    let mut hasher1 = DefaultHasher::new();
    let mut hasher2 = DefaultHasher::new();
    expr1.hash(&mut hasher1);
    expr2.hash(&mut hasher2);

    assert_eq!(
        hasher1.finish(),
        hasher2.finish(),
        "Hash should be based on text only"
    );
}

#[test]
fn test_expression_different_text_not_equal() {
    let expr1 = Expression::new("a + b", vec!["a", "b"], 1);
    let expr2 = Expression::new("a - b", vec!["a", "b"], 1);

    assert_ne!(
        expr1, expr2,
        "Different text should mean different expressions"
    );
}

#[test]
fn test_expression_is_killed_by_operand() {
    let expr = Expression::new("a + b", vec!["a", "b"], 1);

    assert!(
        expr.is_killed_by("a"),
        "Expression should be killed by redefining 'a'"
    );
    assert!(
        expr.is_killed_by("b"),
        "Expression should be killed by redefining 'b'"
    );
}

#[test]
fn test_expression_not_killed_by_non_operand() {
    let expr = Expression::new("a + b", vec!["a", "b"], 1);

    assert!(
        !expr.is_killed_by("c"),
        "Expression should NOT be killed by unrelated variable"
    );
    assert!(
        !expr.is_killed_by("x"),
        "Expression should NOT be killed by unrelated variable"
    );
}

// =============================================================================
// Available Expressions - Commutative Normalization Tests
// =============================================================================

#[test]
fn test_commutative_ops_constant() {
    assert!(COMMUTATIVE_OPS.contains(&"+"));
    assert!(COMMUTATIVE_OPS.contains(&"*"));
    assert!(COMMUTATIVE_OPS.contains(&"=="));
    assert!(COMMUTATIVE_OPS.contains(&"!="));
}

#[test]
fn test_normalize_expression_commutative_add() {
    // "a + b" and "b + a" should normalize to same form
    let norm1 = normalize_expression("a", "+", "b");
    let norm2 = normalize_expression("b", "+", "a");

    assert_eq!(
        norm1, norm2,
        "Commutative addition should normalize identically"
    );
}

#[test]
fn test_normalize_expression_commutative_mult() {
    let norm1 = normalize_expression("x", "*", "y");
    let norm2 = normalize_expression("y", "*", "x");

    assert_eq!(
        norm1, norm2,
        "Commutative multiplication should normalize identically"
    );
}

#[test]
fn test_normalize_expression_commutative_eq() {
    let norm1 = normalize_expression("foo", "==", "bar");
    let norm2 = normalize_expression("bar", "==", "foo");

    assert_eq!(
        norm1, norm2,
        "Commutative equality should normalize identically"
    );
}

#[test]
fn test_normalize_expression_non_commutative_sub() {
    // Subtraction is NOT commutative
    let norm1 = normalize_expression("a", "-", "b");
    let norm2 = normalize_expression("b", "-", "a");

    assert_ne!(
        norm1, norm2,
        "Non-commutative subtraction should preserve order"
    );
    assert_eq!(norm1, "a - b");
    assert_eq!(norm2, "b - a");
}

#[test]
fn test_normalize_expression_non_commutative_div() {
    let norm1 = normalize_expression("x", "/", "y");
    let norm2 = normalize_expression("y", "/", "x");

    assert_ne!(
        norm1, norm2,
        "Non-commutative division should preserve order"
    );
}

// =============================================================================
// Available Expressions - AvailableExprsInfo Tests
// =============================================================================

#[test]
fn test_available_exprs_info_new() {
    let info = AvailableExprsInfo::new(0);

    assert!(info.avail_in.is_empty());
    assert!(info.avail_out.is_empty());
    assert!(info.all_exprs.is_empty());
    assert_eq!(info.entry_block, 0);
    assert!(info.expr_instances.is_empty());
}

#[test]
fn test_available_exprs_info_is_available() {
    let mut info = AvailableExprsInfo::new(0);
    let expr = Expression::new("a + b", vec!["a", "b"], 1);

    let mut block_exprs = HashSet::new();
    block_exprs.insert(expr.clone());
    info.avail_in.insert(1, block_exprs);

    assert!(
        info.is_available(1, &expr),
        "is_available should return true when expr in avail_in"
    );
}

#[test]
fn test_available_exprs_info_is_available_not_found() {
    let info = AvailableExprsInfo::new(0);
    let expr = Expression::new("a + b", vec!["a", "b"], 1);

    assert!(
        !info.is_available(1, &expr),
        "is_available should return false when expr not in avail_in"
    );
}

#[test]
fn test_available_exprs_info_is_available_unknown_block() {
    let info = AvailableExprsInfo::new(0);
    let expr = Expression::new("a + b", vec!["a", "b"], 1);

    assert!(
        !info.is_available(999, &expr),
        "is_available should return false for unknown block"
    );
}

#[test]
fn test_available_exprs_info_is_available_at_exit() {
    let mut info = AvailableExprsInfo::new(0);
    let expr = Expression::new("x * y", vec!["x", "y"], 5);

    let mut block_exprs = HashSet::new();
    block_exprs.insert(expr.clone());
    info.avail_out.insert(0, block_exprs);

    assert!(
        info.is_available_at_exit(0, &expr),
        "is_available_at_exit should check avail_out"
    );
}

#[test]
fn test_available_exprs_info_redundant_computations() {
    let mut info = AvailableExprsInfo::new(0);

    // Add same expression twice at different lines
    info.expr_instances
        .push(Expression::new("a + b", vec!["a", "b"], 2));
    info.expr_instances
        .push(Expression::new("a + b", vec!["a", "b"], 5));

    let redundant = info.redundant_computations();

    // Should detect one redundant computation
    assert_eq!(redundant.len(), 1);
    assert_eq!(redundant[0].0, "a + b");
    assert_eq!(redundant[0].1, 2); // first_at
    assert_eq!(redundant[0].2, 5); // redundant_at
}

#[test]
fn test_available_exprs_info_redundant_sorted() {
    let mut info = AvailableExprsInfo::new(0);

    // Add expressions in non-sorted order
    info.expr_instances
        .push(Expression::new("a + b", vec!["a", "b"], 2));
    info.expr_instances
        .push(Expression::new("x * y", vec!["x", "y"], 3));
    info.expr_instances
        .push(Expression::new("a + b", vec!["a", "b"], 10));
    info.expr_instances
        .push(Expression::new("x * y", vec!["x", "y"], 8));

    let redundant = info.redundant_computations();

    // Should be sorted by redundant_line
    for window in redundant.windows(2) {
        assert!(
            window[0].2 <= window[1].2,
            "redundant_computations should be sorted by line"
        );
    }
}

#[test]
fn test_available_exprs_info_to_json() {
    let mut info = AvailableExprsInfo::new(0);
    let expr = Expression::new("a + b", vec!["a", "b"], 2);

    let mut block_exprs = HashSet::new();
    block_exprs.insert(expr.clone());
    info.avail_in.insert(0, HashSet::new());
    info.avail_out.insert(0, block_exprs.clone());
    info.all_exprs.insert(expr);

    let json = info.to_json();

    // Verify it's valid JSON with expected fields
    assert!(json.get("avail_in").is_some());
    assert!(json.get("avail_out").is_some());
    assert!(json.get("all_expressions").is_some());
    assert!(json.get("entry_block").is_some());
    assert!(json.get("redundant_computations").is_some());
}

// =============================================================================
// Available Expressions - Helper Functions Tests
// =============================================================================

#[test]
fn test_is_function_call_detection() {
    assert!(is_function_call("foo()"));
    assert!(is_function_call("bar(x, y)"));
    assert!(is_function_call("obj.method()"));
    assert!(is_function_call("  foo()  ")); // with whitespace
}

#[test]
fn test_is_function_call_not_call() {
    assert!(!is_function_call("a + b"));
    assert!(!is_function_call("x * y"));
    assert!(!is_function_call("obj.field"));
    assert!(!is_function_call("x = 5"));
}

#[test]
fn test_parse_expression_from_line_binary() {
    let result = parse_expression_from_line("x = a + b");
    assert!(result.is_some());
    let (left, op, right) = result.unwrap();
    assert_eq!(left, "a");
    assert_eq!(op, "+");
    assert_eq!(right, "b");
}

#[test]
fn test_parse_expression_from_line_function_call() {
    // Function calls should be excluded
    let result = parse_expression_from_line("x = foo()");
    assert!(result.is_none());
}

#[test]
fn test_parse_expression_from_line_method_call() {
    let result = parse_expression_from_line("y = bar.baz()");
    assert!(result.is_none());
}

// =============================================================================
// Available Expressions - Confidence & UncertainFinding Tests
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

#[test]
fn test_uncertain_finding_construction() {
    let uf = UncertainFinding {
        expr: "foo() + x".to_string(),
        line: 42,
        reason: "contains function call - purity unknown".to_string(),
    };
    assert_eq!(uf.expr, "foo() + x");
    assert_eq!(uf.line, 42);
    assert_eq!(uf.reason, "contains function call - purity unknown");
}

#[test]
fn test_uncertain_finding_serialization() {
    let uf = UncertainFinding {
        expr: "obj.method() + y".to_string(),
        line: 15,
        reason: "method access - purity unknown".to_string(),
    };
    let json = serde_json::to_string(&uf).unwrap();
    assert!(json.contains("\"expr\""));
    assert!(json.contains("\"line\""));
    assert!(json.contains("\"reason\""));
    let deserialized: UncertainFinding = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.expr, uf.expr);
    assert_eq!(deserialized.line, uf.line);
}

// =============================================================================
// Abstract Interpretation - Nullability Tests
// =============================================================================

#[test]
fn test_nullability_enum_values() {
    let never = Nullability::Never;
    let _maybe = Nullability::Maybe;
    let always = Nullability::Always;

    // Just verify they can be created
    assert_ne!(format!("{:?}", never), format!("{:?}", always));
}

#[test]
fn test_nullability_default_is_maybe() {
    let default: Nullability = Default::default();
    assert_eq!(default, Nullability::Maybe);
}

// =============================================================================
// Abstract Interpretation - AbstractValue Tests
// =============================================================================

#[test]
fn test_abstract_value_top() {
    let top = AbstractValue::top();

    assert!(top.type_.is_none(), "top() should have None type");
    assert!(top.range_.is_none(), "top() should have None range");
    assert_eq!(
        top.nullable,
        Nullability::Maybe,
        "top() should have MAYBE nullable"
    );
    assert!(top.constant.is_none(), "top() should have None constant");
}

#[test]
fn test_abstract_value_bottom() {
    let bottom = AbstractValue::bottom();

    assert_eq!(bottom.type_, Some("<bottom>".to_string()));
}

#[test]
fn test_abstract_value_from_constant_int() {
    let val = AbstractValue::from_constant(ConstantValue::Int(42));

    assert_eq!(val.type_, Some("int".to_string()));
    assert_eq!(val.range_, Some((Some(42), Some(42))));
    assert_eq!(val.nullable, Nullability::Never);
    assert!(val.is_constant());
}

#[test]
fn test_abstract_value_from_constant_negative_int() {
    let val = AbstractValue::from_constant(ConstantValue::Int(-5));

    assert_eq!(val.type_, Some("int".to_string()));
    assert_eq!(val.range_, Some((Some(-5), Some(-5))));
}

#[test]
fn test_abstract_value_from_constant_string() {
    let val = AbstractValue::from_constant(ConstantValue::String("hello".to_string()));

    assert_eq!(val.type_, Some("str".to_string()));
    assert_eq!(val.nullable, Nullability::Never);
    assert!(val.is_constant());
}

#[test]
fn test_abstract_value_from_constant_null() {
    let val = AbstractValue::from_constant(ConstantValue::Null);

    assert_eq!(val.type_, Some("NoneType".to_string()));
    assert_eq!(val.nullable, Nullability::Always);
    assert!(val.range_.is_none());
}

#[test]
fn test_abstract_value_from_constant_bool() {
    let val_true = AbstractValue::from_constant(ConstantValue::Bool(true));
    let val_false = AbstractValue::from_constant(ConstantValue::Bool(false));

    assert_eq!(val_true.type_, Some("bool".to_string()));
    assert_eq!(val_false.type_, Some("bool".to_string()));
}

#[test]
fn test_abstract_value_may_be_zero_range_including_zero() {
    let val = AbstractValue {
        type_: Some("int".to_string()),
        range_: Some((Some(-5), Some(5))),
        nullable: Nullability::Never,
        constant: None,
    };

    assert!(val.may_be_zero(), "Range [-5, 5] should may_be_zero");
}

#[test]
fn test_abstract_value_may_be_zero_range_excluding_zero() {
    let val = AbstractValue {
        type_: Some("int".to_string()),
        range_: Some((Some(5), Some(10))),
        nullable: Nullability::Never,
        constant: None,
    };

    assert!(!val.may_be_zero(), "Range [5, 10] should not may_be_zero");
}

#[test]
fn test_abstract_value_may_be_zero_unknown_range() {
    let val = AbstractValue::top();

    assert!(
        val.may_be_zero(),
        "Unknown range should conservatively may_be_zero"
    );
}

#[test]
fn test_abstract_value_may_be_null_maybe() {
    let val = AbstractValue {
        type_: None,
        range_: None,
        nullable: Nullability::Maybe,
        constant: None,
    };

    assert!(val.may_be_null(), "MAYBE should may_be_null");
}

#[test]
fn test_abstract_value_may_be_null_never() {
    let val = AbstractValue::from_constant(ConstantValue::Int(5));

    assert!(!val.may_be_null(), "NEVER should not may_be_null");
}

#[test]
fn test_abstract_value_may_be_null_always() {
    let val = AbstractValue::from_constant(ConstantValue::Null);

    assert!(val.may_be_null(), "ALWAYS should may_be_null");
}

#[test]
fn test_abstract_value_is_constant_true() {
    let val = AbstractValue::from_constant(ConstantValue::Int(42));
    assert!(val.is_constant());
}

#[test]
fn test_abstract_value_is_constant_false() {
    let val = AbstractValue::top();
    assert!(!val.is_constant());
}

// =============================================================================
// Abstract Interpretation - AbstractState Tests
// =============================================================================

#[test]
fn test_abstract_state_empty() {
    let state = AbstractState::new();
    assert!(state.values.is_empty());
}

#[test]
fn test_abstract_state_get_existing() {
    let mut state = AbstractState::new();
    let val = AbstractValue::from_constant(ConstantValue::Int(10));
    state.values.insert("x".to_string(), val.clone());

    let retrieved = state.get("x");
    assert_eq!(retrieved, val);
}

#[test]
fn test_abstract_state_get_missing_returns_top() {
    let state = AbstractState::new();

    let retrieved = state.get("unknown");
    assert_eq!(retrieved, AbstractValue::top());
}

#[test]
fn test_abstract_state_set() {
    let state = AbstractState::new();
    let val = AbstractValue::from_constant(ConstantValue::Int(5));

    let new_state = state.set("x", val.clone());

    assert!(
        state.values.is_empty(),
        "Original state should be unchanged"
    );
    assert!(
        new_state.values.contains_key("x"),
        "New state should have x"
    );
}

// =============================================================================
// Abstract Interpretation - AbstractInterpInfo Tests
// =============================================================================

#[test]
fn test_abstract_interp_info_new() {
    let info = AbstractInterpInfo::new("test_func");

    assert_eq!(info.function_name, "test_func");
    assert!(info.state_in.is_empty());
    assert!(info.state_out.is_empty());
    assert!(info.potential_div_zero.is_empty());
    assert!(info.potential_null_deref.is_empty());
}

#[test]
fn test_abstract_interp_info_value_at() {
    let mut info = AbstractInterpInfo::new("test");
    let val = AbstractValue::from_constant(ConstantValue::Int(42));
    let state = AbstractState::new().set("x", val.clone());
    info.state_in.insert(0, state);

    let retrieved = info.value_at(0, "x");
    assert_eq!(retrieved, val);
}

#[test]
fn test_abstract_interp_info_value_at_missing_block() {
    let info = AbstractInterpInfo::new("test");

    let retrieved = info.value_at(999, "x");
    assert_eq!(retrieved, AbstractValue::top());
}

#[test]
fn test_abstract_interp_info_value_at_exit() {
    let mut info = AbstractInterpInfo::new("test");
    let val = AbstractValue::from_constant(ConstantValue::Int(100));
    let state = AbstractState::new().set("result", val.clone());
    info.state_out.insert(0, state);

    let retrieved = info.value_at_exit(0, "result");
    assert_eq!(retrieved, val);
}

#[test]
fn test_abstract_interp_info_range_at() {
    let mut info = AbstractInterpInfo::new("test");
    let val = AbstractValue {
        type_: Some("int".to_string()),
        range_: Some((Some(1), Some(10))),
        nullable: Nullability::Never,
        constant: None,
    };
    let state = AbstractState::new().set("x", val);
    info.state_in.insert(0, state);

    let range = info.range_at(0, "x");
    assert_eq!(range, Some((Some(1), Some(10))));
}

#[test]
fn test_abstract_interp_info_type_at() {
    let mut info = AbstractInterpInfo::new("test");
    let val = AbstractValue::from_constant(ConstantValue::String("hello".to_string()));
    let state = AbstractState::new().set("s", val);
    info.state_in.insert(0, state);

    let typ = info.type_at(0, "s");
    assert_eq!(typ, Some("str".to_string()));
}

#[test]
fn test_abstract_interp_info_is_definitely_not_null() {
    let mut info = AbstractInterpInfo::new("test");
    let val = AbstractValue::from_constant(ConstantValue::Int(5));
    let state = AbstractState::new().set("x", val);
    info.state_in.insert(0, state);

    assert!(info.is_definitely_not_null(0, "x"));
}

#[test]
fn test_abstract_interp_info_is_definitely_not_null_maybe() {
    let mut info = AbstractInterpInfo::new("test");
    let val = AbstractValue::top(); // MAYBE nullable
    let state = AbstractState::new().set("x", val);
    info.state_in.insert(0, state);

    assert!(!info.is_definitely_not_null(0, "x"));
}

#[test]
fn test_abstract_interp_info_to_json() {
    let mut info = AbstractInterpInfo::new("example");
    let val = AbstractValue::from_constant(ConstantValue::Int(5));
    let state = AbstractState::new().set("x", val);
    info.state_in.insert(0, AbstractState::new());
    info.state_out.insert(0, state);
    info.potential_div_zero.push((10, "divisor".to_string()));

    let json = info.to_json();

    assert!(json.get("function").is_some());
    assert!(json.get("state_in").is_some());
    assert!(json.get("state_out").is_some());
    assert!(json.get("potential_div_zero").is_some());
    assert!(json.get("potential_null_deref").is_some());
}

// =============================================================================
// Abstract Interpretation - ConstantValue Tests
// =============================================================================

#[test]
fn test_constant_value_int() {
    let val = ConstantValue::Int(42);
    assert!(matches!(val, ConstantValue::Int(42)));
}

#[test]
fn test_constant_value_float() {
    let val = ConstantValue::Float(std::f64::consts::PI);
    assert!(
        matches!(val, ConstantValue::Float(v) if (v - std::f64::consts::PI).abs() < f64::EPSILON)
    );
}

#[test]
fn test_constant_value_string() {
    let val = ConstantValue::String("hello".to_string());
    assert!(matches!(val, ConstantValue::String(s) if s == "hello"));
}

#[test]
fn test_constant_value_bool() {
    let val_true = ConstantValue::Bool(true);
    let val_false = ConstantValue::Bool(false);
    assert!(matches!(val_true, ConstantValue::Bool(true)));
    assert!(matches!(val_false, ConstantValue::Bool(false)));
}

#[test]
fn test_constant_value_null() {
    let val = ConstantValue::Null;
    assert!(matches!(val, ConstantValue::Null));
}

// =============================================================================
// ExprInstance Tests
// =============================================================================

#[test]
fn test_expr_instance_new() {
    let expr = Expression::new("a + b", vec!["a", "b"], 1);
    let instance = ExprInstance::new(expr, 0);
    assert_eq!(instance.block_id, 0);
    assert_eq!(instance.expr.text, "a + b");
}

// =============================================================================
// BlockExpressions Tests
// =============================================================================

#[test]
fn test_block_expressions_new() {
    let block_expr = BlockExpressions::new();
    assert!(block_expr.gen.is_empty());
    assert!(block_expr.kill.is_empty());
}

// =============================================================================
// Integration Tests (compute functions)
// =============================================================================

fn make_test_cfg_for_phase3(
    blocks: Vec<(usize, BlockType, (u32, u32))>,
    edges: Vec<(usize, usize)>,
    entry: usize,
) -> CfgInfo {
    CfgInfo {
        function: "test".to_string(),
        blocks: blocks
            .into_iter()
            .map(|(id, block_type, lines)| CfgBlock {
                id,
                block_type,
                lines,
                calls: vec![],
            })
            .collect(),
        edges: edges
            .into_iter()
            .map(|(from, to)| CfgEdge {
                from,
                to,
                edge_type: EdgeType::Unconditional,
                condition: None,
            })
            .collect(),
        entry_block: entry,
        exit_blocks: vec![],
        cyclomatic_complexity: 1,
        nested_functions: HashMap::new(),
    }
}

fn make_empty_dfg_for_phase3() -> DfgInfo {
    DfgInfo {
        function: "test".to_string(),
        refs: vec![],
        edges: vec![],
        variables: vec![],
    }
}

fn make_var_ref_for_phase3(name: &str, line: u32, ref_type: RefType) -> VarRef {
    VarRef {
        name: name.to_string(),
        ref_type,
        line,
        column: 0,
        context: None,
        group_id: None,
    }
}

fn make_dfg_with_refs_for_phase3(refs: Vec<VarRef>) -> DfgInfo {
    let variables: Vec<String> = refs.iter().map(|r| r.name.clone()).collect();
    DfgInfo {
        function: "test".to_string(),
        refs,
        edges: vec![],
        variables,
    }
}

#[test]
fn test_compute_available_exprs_empty_function() {
    let cfg = make_test_cfg_for_phase3(vec![(0, BlockType::Entry, (1, 1))], vec![], 0);
    let dfg = make_empty_dfg_for_phase3();

    let result = compute_available_exprs(&cfg, &dfg);
    assert!(
        result.is_ok(),
        "Empty function should not crash: {:?}",
        result.err()
    );

    let info = result.unwrap();
    assert!(info.avail_in.contains_key(&0));
    assert!(info.avail_out.contains_key(&0));
}

#[test]
fn test_compute_available_exprs_linear_cfg() {
    let cfg = make_test_cfg_for_phase3(
        vec![
            (0, BlockType::Entry, (1, 2)),
            (1, BlockType::Body, (3, 4)),
            (2, BlockType::Exit, (5, 6)),
        ],
        vec![(0, 1), (1, 2)],
        0,
    );

    let dfg = make_dfg_with_refs_for_phase3(vec![
        make_var_ref_for_phase3("x", 2, RefType::Definition),
        make_var_ref_for_phase3("a", 2, RefType::Use),
        make_var_ref_for_phase3("b", 2, RefType::Use),
    ]);

    let result = compute_available_exprs(&cfg, &dfg);
    assert!(result.is_ok(), "Linear CFG should work: {:?}", result.err());
}

#[test]
fn test_compute_available_exprs_entry_has_nothing_available() {
    let cfg = make_test_cfg_for_phase3(
        vec![(0, BlockType::Entry, (1, 5)), (1, BlockType::Exit, (6, 10))],
        vec![(0, 1)],
        0,
    );
    let dfg = make_empty_dfg_for_phase3();

    let result = compute_available_exprs(&cfg, &dfg).unwrap();

    // Entry block avail_in should always be empty
    assert!(
        result.avail_in.get(&0).unwrap().is_empty(),
        "Entry block should have nothing available at its entry"
    );
}

#[test]
fn test_compute_available_exprs_loop_cfg() {
    // Loop CFG: 0 -> 1 (header) <-> 2 (body) -> 3 (exit)
    let cfg = make_test_cfg_for_phase3(
        vec![
            (0, BlockType::Entry, (1, 1)),
            (1, BlockType::LoopHeader, (2, 2)),
            (2, BlockType::LoopBody, (3, 3)),
            (3, BlockType::Exit, (4, 4)),
        ],
        vec![(0, 1), (1, 2), (2, 1), (1, 3)],
        0,
    );
    let dfg = make_empty_dfg_for_phase3();

    // Should not crash and should terminate
    let result = compute_available_exprs(&cfg, &dfg);
    assert!(result.is_ok(), "Loop CFG should be handled without crash");
}

#[test]
fn test_compute_available_exprs_unreachable_block() {
    let cfg = make_test_cfg_for_phase3(
        vec![
            (0, BlockType::Entry, (1, 1)),
            (1, BlockType::Exit, (2, 2)),
            (2, BlockType::Body, (3, 3)), // No edges to this block
        ],
        vec![(0, 1)],
        0,
    );
    let dfg = make_empty_dfg_for_phase3();

    let result = compute_available_exprs(&cfg, &dfg);
    assert!(
        result.is_ok(),
        "Unreachable block should be handled without crash"
    );

    let info = result.unwrap();
    // Unreachable block should have empty avail_in (no predecessors)
    assert!(
        info.avail_in.get(&2).unwrap().is_empty(),
        "Unreachable block should have nothing available"
    );
}

#[test]
fn test_compute_available_exprs_self_loop() {
    // CFG with self-loop: 0 -> 1 -> 1 (self-loop)
    let cfg = make_test_cfg_for_phase3(
        vec![
            (0, BlockType::Entry, (1, 1)),
            (1, BlockType::LoopHeader, (2, 3)),
        ],
        vec![(0, 1), (1, 1)],
        0,
    );
    let dfg = make_empty_dfg_for_phase3();

    // Should terminate (fixpoint)
    let result = compute_available_exprs(&cfg, &dfg);
    assert!(result.is_ok(), "Self-loop CFG should terminate at fixpoint");
}

#[test]
fn test_compute_available_exprs_pathological_cfg_limited() {
    // Create a CFG with MAX_BLOCKS + 1 blocks
    let block_count = MAX_BLOCKS + 1; // 10001 blocks

    let blocks: Vec<CfgBlock> = (0..block_count)
        .map(|id| CfgBlock {
            id,
            block_type: if id == 0 {
                BlockType::Entry
            } else {
                BlockType::Body
            },
            lines: (id as u32, id as u32),
            calls: vec![],
        })
        .collect();

    let edges: Vec<CfgEdge> = (0..block_count - 1)
        .map(|i| CfgEdge {
            from: i,
            to: i + 1,
            edge_type: EdgeType::Unconditional,
            condition: None,
        })
        .collect();

    let cfg = CfgInfo {
        function: "pathological".to_string(),
        blocks,
        edges,
        entry_block: 0,
        exit_blocks: vec![block_count - 1],
        cyclomatic_complexity: 1,
        nested_functions: HashMap::new(),
    };

    let dfg = DfgInfo {
        function: "pathological".to_string(),
        refs: vec![],
        edges: vec![],
        variables: vec![],
    };

    let result = compute_available_exprs(&cfg, &dfg);

    // Should return TooManyBlocks error
    match result {
        Err(DataflowError::TooManyBlocks { count }) => {
            assert_eq!(count, block_count, "Error should report actual block count");
        }
        Ok(_) => {
            panic!(
                "Analysis should reject CFG with {} blocks (exceeds MAX_BLOCKS={})",
                block_count, MAX_BLOCKS
            );
        }
        Err(other) => {
            panic!("Expected TooManyBlocks error, got: {:?}", other);
        }
    }
}

// =============================================================================
// Abstract Interpretation Integration Tests
// =============================================================================

#[test]
fn test_compute_abstract_interp_empty_function() {
    let cfg = make_test_cfg_for_phase3(vec![(0, BlockType::Entry, (1, 1))], vec![], 0);
    let dfg = make_empty_dfg_for_phase3();

    let result = compute_abstract_interp(&cfg, &dfg, None, "python");
    assert!(
        result.is_ok(),
        "Empty function should not crash: {:?}",
        result.err()
    );

    let info = result.unwrap();
    assert_eq!(info.function_name, "test");
}

#[test]
fn test_compute_abstract_interp_loop_cfg() {
    let cfg = make_test_cfg_for_phase3(
        vec![
            (0, BlockType::Entry, (1, 1)),
            (1, BlockType::LoopHeader, (2, 2)),
            (2, BlockType::LoopBody, (3, 3)),
            (3, BlockType::Exit, (4, 4)),
        ],
        vec![(0, 1), (1, 2), (2, 1), (1, 3)],
        0,
    );
    let dfg = make_empty_dfg_for_phase3();

    let result = compute_abstract_interp(&cfg, &dfg, None, "python");
    assert!(result.is_ok(), "Loop CFG should terminate via widening");
}

// =============================================================================
// Summary
// =============================================================================
// Total tests:
// - Dataflow types (BlockId, DataflowError): 8 tests
// - CFG helpers (predecessors, successors, back edges, etc.): 15 tests
// - Available expressions (Expression, normalization, AvailableExprsInfo): 20 tests
// - Confidence & UncertainFinding: 5 tests
// - Abstract interpretation (Nullability, AbstractValue, AbstractState): 20 tests
// - AbstractInterpInfo: 10 tests
// - ConstantValue: 5 tests
// - BlockExpressions & ExprInstance: 4 tests
// - Integration tests (compute_available_exprs, compute_abstract_interp): 8 tests
//
// Total: ~95 tests covering the public API of dataflow module
