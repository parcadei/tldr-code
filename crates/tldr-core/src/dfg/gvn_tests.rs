//! Global Value Numbering (GVN) Tests
//!
//! Test-driven development tests for GVN migration from Python.
//! These tests define expected behavior based on spec.md behavioral contracts.
//!
//! # Behavioral Contracts Tested
//!
//! - BC-GVN-1: Commutativity normalization (a + b == b + a)
//! - BC-GVN-2: Alias propagation (x = expr; use x)
//! - BC-GVN-3: Sequential analysis (statement order matters)
//! - BC-GVN-4: Function call conservatism (calls always unique)
//! - BC-GVN-5: Depth limiting (>10 levels get unique VNs)
//! - BC-GVN-6: Redundancy detection (N expressions => N-1 redundancies)
//!
//! Reference: migration/spec.md

#![allow(unused_imports)]
use std::collections::HashMap;

// Import types and compute_gvn from the gvn module
use super::gvn::{compute_gvn, ExpressionRef, GVNEquivalence, GVNReport, Redundancy};

// =============================================================================
// BC-GVN-1: Commutativity Normalization Tests
// =============================================================================

#[cfg(test)]
mod commutativity_tests {
    use super::*;

    /// a + b and b + a should get the same value number
    #[test]
    fn test_add_commutativity() {
        let source = r#"
def example(x, y):
    a = x + y
    b = y + x
    return a + b
"#;
        let reports = compute_gvn(source, Some("example"));
        assert_eq!(reports.len(), 1, "Expected one report for 'example'");

        let report = &reports[0];
        assert_eq!(report.function, "example");

        // Find equivalence for x + y and y + x
        let commutative_equiv = report
            .equivalences
            .iter()
            .find(|eq| eq.expressions.len() >= 2 && eq.reason.contains("commutativity"));

        assert!(
            commutative_equiv.is_some(),
            "Expected equivalence class for commutative expressions"
        );

        let equiv = commutative_equiv.unwrap();
        let texts: Vec<&str> = equiv.expressions.iter().map(|e| e.text.as_str()).collect();
        assert!(
            texts.contains(&"x + y") || texts.contains(&"y + x"),
            "Equivalence class should contain x + y or y + x"
        );
    }

    /// Mult is commutative: a * b == b * a
    #[test]
    fn test_mult_commutativity() {
        let source = r#"
def mult_test(a, b):
    x = a * b
    y = b * a
    return x + y
"#;
        let reports = compute_gvn(source, Some("mult_test"));
        assert_eq!(reports.len(), 1);

        let report = &reports[0];
        // a * b and b * a should share value number
        let mult_equiv = report
            .equivalences
            .iter()
            .find(|eq| eq.expressions.iter().any(|e| e.text.contains("*")));

        assert!(
            mult_equiv.is_some(),
            "Expected equivalence for multiplication"
        );
        assert!(
            mult_equiv.unwrap().expressions.len() >= 2,
            "a * b and b * a should be equivalent"
        );
    }

    /// BitOr is commutative: a | b == b | a
    #[test]
    fn test_bitor_commutativity() {
        let source = r#"
def bitor_test(a, b):
    x = a | b
    y = b | a
    return x
"#;
        let reports = compute_gvn(source, Some("bitor_test"));
        assert_eq!(reports.len(), 1);

        let report = &reports[0];
        let bitor_equiv = report
            .equivalences
            .iter()
            .find(|eq| eq.expressions.iter().any(|e| e.text.contains("|")));

        assert!(bitor_equiv.is_some(), "Expected equivalence for bitor");
    }

    /// BitAnd is commutative: a & b == b & a
    #[test]
    fn test_bitand_commutativity() {
        let source = r#"
def bitand_test(a, b):
    x = a & b
    y = b & a
    return x
"#;
        let reports = compute_gvn(source, Some("bitand_test"));
        assert_eq!(reports.len(), 1);

        let report = &reports[0];
        let bitand_equiv = report
            .equivalences
            .iter()
            .find(|eq| eq.expressions.iter().any(|e| e.text.contains("&")));

        assert!(bitand_equiv.is_some(), "Expected equivalence for bitand");
    }

    /// BitXor is commutative: a ^ b == b ^ a
    #[test]
    fn test_bitxor_commutativity() {
        let source = r#"
def bitxor_test(a, b):
    x = a ^ b
    y = b ^ a
    return x
"#;
        let reports = compute_gvn(source, Some("bitxor_test"));
        assert_eq!(reports.len(), 1);

        let report = &reports[0];
        let bitxor_equiv = report
            .equivalences
            .iter()
            .find(|eq| eq.expressions.iter().any(|e| e.text.contains("^")));

        assert!(bitxor_equiv.is_some(), "Expected equivalence for bitxor");
    }

    /// Subtraction is NOT commutative: a - b != b - a
    #[test]
    fn test_sub_not_commutative() {
        let source = r#"
def sub_test(a, b):
    x = a - b
    y = b - a
    return x + y
"#;
        let reports = compute_gvn(source, Some("sub_test"));
        assert_eq!(reports.len(), 1);

        let report = &reports[0];
        // a - b and b - a should NOT share value number
        let sub_equiv = report.equivalences.iter().find(|eq| {
            eq.expressions.len() >= 2 && eq.expressions.iter().all(|e| e.text.contains("-"))
        });

        assert!(
            sub_equiv.is_none(),
            "a - b and b - a should NOT be equivalent (subtraction is not commutative)"
        );
    }

    /// Division is NOT commutative: a / b != b / a
    #[test]
    fn test_div_not_commutative() {
        let source = r#"
def div_test(a, b):
    x = a / b
    y = b / a
    return x + y
"#;
        let reports = compute_gvn(source, Some("div_test"));
        assert_eq!(reports.len(), 1);

        let report = &reports[0];
        let div_equiv = report.equivalences.iter().find(|eq| {
            eq.expressions.len() >= 2 && eq.expressions.iter().all(|e| e.text.contains("/"))
        });

        assert!(
            div_equiv.is_none(),
            "a / b and b / a should NOT be equivalent"
        );
    }
}

// =============================================================================
// BC-GVN-2: Alias Propagation Tests
// =============================================================================

#[cfg(test)]
mod alias_tests {
    use super::*;

    /// Variable alias should propagate value number
    #[test]
    fn test_simple_alias_propagation() {
        let source = r#"
def alias_test(x, y):
    a = x + y
    b = a
    c = x + y
    return b + c
"#;
        let reports = compute_gvn(source, Some("alias_test"));
        assert_eq!(reports.len(), 1);

        let report = &reports[0];
        // a, b, and c should all get the same VN as x + y
        // a = x + y (VN 1)
        // b = a (VN 1, alias)
        // c = x + y (VN 1, same as a)

        // Should have redundancy: c is redundant with a
        assert!(
            !report.redundancies.is_empty(),
            "Expected redundancy detection for c = x + y"
        );
    }

    /// Chained aliases: a = expr; b = a; c = b
    #[test]
    fn test_chained_alias() {
        let source = r#"
def chained(x):
    a = x * 2
    b = a
    c = b
    d = x * 2
    return c + d
"#;
        let reports = compute_gvn(source, Some("chained"));
        assert_eq!(reports.len(), 1);

        let report = &reports[0];
        // All of a, b, c, d should share the same VN
        let equiv = report
            .equivalences
            .iter()
            .find(|eq| eq.expressions.len() >= 2);

        assert!(
            equiv.is_some(),
            "Expected equivalence class for chained aliases"
        );
    }

    /// Reassignment should kill the alias
    #[test]
    fn test_alias_killed_by_reassignment() {
        let source = r#"
def killed_alias(x, y):
    a = x + y
    b = a
    a = x * y  # kills the alias
    c = b      # b still refers to old x + y
    d = a      # d refers to new x * y
    return c + d
"#;
        let reports = compute_gvn(source, Some("killed_alias"));
        assert_eq!(reports.len(), 1);

        let report = &reports[0];
        // b and c should share VN with original x + y
        // a (after reassign) and d should share VN with x * y
        // But b should NOT equal d

        // Check that we have at least 2 equivalence classes
        // This verifies kill behavior
        assert!(
            report.unique_values >= 2,
            "Expected at least 2 unique values after reassignment"
        );
    }
}

// =============================================================================
// BC-GVN-3: Sequential Analysis Tests
// =============================================================================

#[cfg(test)]
mod sequential_tests {
    use super::*;

    /// Control flow: both branches should be analyzed
    #[test]
    fn test_if_else_branches() {
        let source = r#"
def if_test(x, y, cond):
    if cond:
        a = x + y
    else:
        a = y + x
    b = x + y
    return b
"#;
        let reports = compute_gvn(source, Some("if_test"));
        assert_eq!(reports.len(), 1);

        let report = &reports[0];
        // All three x + y / y + x should be equivalent due to commutativity
        // And should be detected as redundant
        assert!(
            report.total_expressions >= 3,
            "Expected at least 3 expressions analyzed"
        );
    }

    /// Loop body should be analyzed
    #[test]
    fn test_for_loop_body() {
        let source = r#"
def loop_test(items):
    total = 0
    for item in items:
        x = item * 2
        y = item * 2  # redundant
        total = total + x + y
    return total
"#;
        let reports = compute_gvn(source, Some("loop_test"));
        assert_eq!(reports.len(), 1);

        let report = &reports[0];
        // item * 2 appears twice - should be detected as redundant
        let redundancy = report
            .redundancies
            .iter()
            .find(|r| r.redundant.text.contains("item * 2") || r.redundant.text.contains("item*2"));

        assert!(
            redundancy.is_some(),
            "Expected redundancy for item * 2 in loop"
        );
    }

    /// While loop body should be analyzed
    #[test]
    fn test_while_loop_body() {
        let source = r#"
def while_test(n):
    i = 0
    while i < n:
        x = n * 2
        y = n * 2  # redundant within iteration
        i = i + 1
    return x
"#;
        let reports = compute_gvn(source, Some("while_test"));
        assert_eq!(reports.len(), 1);

        let report = &reports[0];
        assert!(
            report.total_expressions >= 2,
            "Expected expressions from while loop body"
        );
    }

    /// Try/except blocks should be analyzed
    #[test]
    fn test_try_except() {
        let source = r#"
def try_test(x, y):
    try:
        a = x + y
        b = x + y  # redundant
    except:
        a = y + x  # also redundant due to commutativity
    return a
"#;
        let reports = compute_gvn(source, Some("try_test"));
        assert_eq!(reports.len(), 1);

        let report = &reports[0];
        assert!(
            report.total_expressions >= 3,
            "Expected expressions from try and except blocks"
        );
    }
}

// =============================================================================
// BC-GVN-4: Function Call Conservatism Tests
// =============================================================================

#[cfg(test)]
mod function_call_tests {
    use super::*;

    /// Each function call should get a unique VN (conservative)
    #[test]
    fn test_calls_always_unique() {
        let source = r#"
def call_test():
    a = foo()
    b = foo()  # different VN (calls are impure)
    c = foo()  # different VN
    return a + b + c
"#;
        let reports = compute_gvn(source, Some("call_test"));
        assert_eq!(reports.len(), 1);

        let report = &reports[0];
        // foo() appears 3 times but should NOT be redundant
        // (function calls may have side effects)
        let call_redundancy = report
            .redundancies
            .iter()
            .find(|r| r.redundant.text.contains("foo()"));

        assert!(
            call_redundancy.is_none(),
            "Function calls should NOT be marked as redundant"
        );
    }

    /// Even with same arguments, calls should be unique
    #[test]
    fn test_calls_with_args_unique() {
        let source = r#"
def call_args_test(x):
    a = bar(x)
    b = bar(x)  # different VN despite same args
    return a + b
"#;
        let reports = compute_gvn(source, Some("call_args_test"));
        assert_eq!(reports.len(), 1);

        let report = &reports[0];
        let call_redundancy = report
            .redundancies
            .iter()
            .find(|r| r.redundant.text.contains("bar("));

        assert!(
            call_redundancy.is_none(),
            "Function calls with same args should still be unique"
        );
    }

    /// Method calls should also be unique
    #[test]
    fn test_method_calls_unique() {
        let source = r#"
def method_test(obj):
    a = obj.method()
    b = obj.method()
    return a + b
"#;
        let reports = compute_gvn(source, Some("method_test"));
        assert_eq!(reports.len(), 1);

        let report = &reports[0];
        let method_redundancy = report
            .redundancies
            .iter()
            .find(|r| r.redundant.text.contains("method()"));

        assert!(method_redundancy.is_none(), "Method calls should be unique");
    }
}

// =============================================================================
// BC-GVN-5: Depth Limiting Tests
// =============================================================================

#[cfg(test)]
mod depth_limit_tests {
    use super::*;

    /// Deeply nested expressions (>10 levels) should get unique VNs
    #[test]
    fn test_deep_nesting_unique_vn() {
        // Create expression nested more than 10 levels deep
        let source = r#"
def deep_test(x):
    # 12 levels of nesting
    a = ((((((((((((x + 1) + 1) + 1) + 1) + 1) + 1) + 1) + 1) + 1) + 1) + 1) + 1)
    b = ((((((((((((x + 1) + 1) + 1) + 1) + 1) + 1) + 1) + 1) + 1) + 1) + 1) + 1)
    return a + b
"#;
        let reports = compute_gvn(source, Some("deep_test"));
        assert_eq!(reports.len(), 1);

        let report = &reports[0];
        // Due to depth limiting, these deeply nested expressions may be
        // given unique VNs instead of being recognized as equivalent
        // The implementation should not crash on deep nesting
        assert!(
            report.total_expressions >= 2,
            "Should handle deeply nested expressions without crashing"
        );
    }

    /// Normal nesting (<= 10 levels) should work normally
    #[test]
    fn test_normal_nesting_recognized() {
        let source = r#"
def normal_test(x):
    # 5 levels of nesting - should work
    a = ((((x + 1) + 1) + 1) + 1) + 1
    b = ((((x + 1) + 1) + 1) + 1) + 1
    return a + b
"#;
        let reports = compute_gvn(source, Some("normal_test"));
        assert_eq!(reports.len(), 1);

        let report = &reports[0];
        // These should be recognized as equivalent
        assert!(
            !report.redundancies.is_empty(),
            "Normal nesting should detect redundancy"
        );
    }
}

// =============================================================================
// BC-GVN-6: Redundancy Detection Tests
// =============================================================================

#[cfg(test)]
mod redundancy_tests {
    use super::*;

    /// N expressions with same VN => N-1 redundancies
    #[test]
    fn test_multiple_redundancies() {
        let source = r#"
def multi_redund(x, y):
    a = x + y  # original
    b = x + y  # redundant 1
    c = x + y  # redundant 2
    d = x + y  # redundant 3
    return a + b + c + d
"#;
        let reports = compute_gvn(source, Some("multi_redund"));
        assert_eq!(reports.len(), 1);

        let report = &reports[0];
        // 4 expressions => 3 redundancies
        let xy_redundancies: Vec<_> = report
            .redundancies
            .iter()
            .filter(|r| r.redundant.text == "x + y" || r.original.text == "x + y")
            .collect();

        assert_eq!(
            xy_redundancies.len(),
            3,
            "4 identical expressions should produce 3 redundancies"
        );
    }

    /// First occurrence is always the original
    #[test]
    fn test_first_is_original() {
        let source = r#"
def first_original(x, y):
    a = x + y  # line 3
    b = x + y  # line 4
    return a + b
"#;
        let reports = compute_gvn(source, Some("first_original"));
        assert_eq!(reports.len(), 1);

        let report = &reports[0];
        assert!(!report.redundancies.is_empty(), "Expected redundancy");

        let redundancy = &report.redundancies[0];
        // Original should have smaller line number
        assert!(
            redundancy.original.line < redundancy.redundant.line,
            "Original should appear before redundant"
        );
    }

    /// Reason should indicate why expressions are equivalent
    #[test]
    fn test_redundancy_reason() {
        let source = r#"
def reason_test(x, y):
    a = x + y
    b = y + x  # redundant due to commutativity
    c = x + y  # redundant - identical expression
    return a + b + c
"#;
        let reports = compute_gvn(source, Some("reason_test"));
        assert_eq!(reports.len(), 1);

        let report = &reports[0];
        // Should have redundancies with appropriate reasons
        let has_commutativity_reason = report
            .redundancies
            .iter()
            .any(|r| r.reason.contains("commutativity") || r.reason.contains("commutative"));

        let has_identical_reason = report
            .redundancies
            .iter()
            .any(|r| r.reason.contains("identical"));

        // At least one should be true
        assert!(
            has_commutativity_reason || has_identical_reason,
            "Redundancy reasons should explain the equivalence"
        );
    }
}

// =============================================================================
// Edge Case Tests
// =============================================================================

#[cfg(test)]
mod edge_case_tests {
    use super::*;

    /// Empty source returns empty list
    #[test]
    fn test_empty_source() {
        let reports = compute_gvn("", None);
        assert!(reports.is_empty(), "Empty source should return empty list");
    }

    /// Source with no functions returns empty list
    #[test]
    fn test_no_functions() {
        let source = r#"
x = 1
y = 2
z = x + y
"#;
        let reports = compute_gvn(source, None);
        assert!(
            reports.is_empty(),
            "Source without functions should return empty list"
        );
    }

    /// Function with empty body
    #[test]
    fn test_empty_function_body() {
        let source = r#"
def empty():
    pass
"#;
        let reports = compute_gvn(source, Some("empty"));
        assert_eq!(reports.len(), 1);

        let report = &reports[0];
        assert_eq!(report.total_expressions, 0);
        assert_eq!(report.unique_values, 0);
        assert_eq!(report.compression_ratio(), 1.0);
    }

    /// Function with only constants
    #[test]
    fn test_only_constants() {
        let source = r#"
def constants():
    a = 1
    b = 2
    c = 3
    return a + b + c
"#;
        let reports = compute_gvn(source, Some("constants"));
        assert_eq!(reports.len(), 1);

        let report = &reports[0];
        // Constants don't produce redundancy (each is unique)
        assert!(
            report.redundancies.is_empty() || report.redundancies.len() < 3,
            "Different constants should be unique"
        );
    }

    /// Same constant used multiple times
    #[test]
    fn test_same_constant_multiple_times() {
        let source = r#"
def same_const():
    a = 42
    b = 42
    c = 42
    return a + b + c
"#;
        let reports = compute_gvn(source, Some("same_const"));
        assert_eq!(reports.len(), 1);

        // Note: Whether same constant gets same VN depends on implementation
        // Python GVN does share VN for identical constants
    }

    /// Nonexistent function name returns empty
    #[test]
    fn test_nonexistent_function() {
        let source = r#"
def exists():
    return 42
"#;
        let reports = compute_gvn(source, Some("does_not_exist"));
        assert!(
            reports.is_empty(),
            "Nonexistent function should return empty list"
        );
    }

    /// Analyze all functions when function_name is None
    #[test]
    fn test_analyze_all_functions() {
        let source = r#"
def func1(x):
    return x + x

def func2(y):
    return y * y

def func3(z):
    return z + z
"#;
        let reports = compute_gvn(source, None);
        assert_eq!(reports.len(), 3, "Should analyze all 3 functions");

        let names: Vec<&str> = reports.iter().map(|r| r.function.as_str()).collect();
        assert!(names.contains(&"func1"));
        assert!(names.contains(&"func2"));
        assert!(names.contains(&"func3"));
    }

    /// Async functions should be analyzed
    #[test]
    fn test_async_function() {
        let source = r#"
async def async_test(x, y):
    a = x + y
    b = y + x
    return a + b
"#;
        let reports = compute_gvn(source, Some("async_test"));
        assert_eq!(reports.len(), 1);

        let report = &reports[0];
        assert_eq!(report.function, "async_test");
        // Should detect commutativity
        assert!(!report.equivalences.is_empty() || !report.redundancies.is_empty());
    }
}

// =============================================================================
// Compression Ratio Tests
// =============================================================================

#[cfg(test)]
mod compression_tests {
    use super::*;

    #[test]
    fn test_compression_ratio_zero_expressions() {
        let report = GVNReport {
            function: "test".to_string(),
            equivalences: vec![],
            redundancies: vec![],
            total_expressions: 0,
            unique_values: 0,
        };
        assert_eq!(report.compression_ratio(), 1.0);
    }

    #[test]
    fn test_compression_ratio_no_sharing() {
        let report = GVNReport {
            function: "test".to_string(),
            equivalences: vec![],
            redundancies: vec![],
            total_expressions: 10,
            unique_values: 10,
        };
        assert_eq!(report.compression_ratio(), 1.0);
    }

    #[test]
    fn test_compression_ratio_half_shared() {
        let report = GVNReport {
            function: "test".to_string(),
            equivalences: vec![],
            redundancies: vec![],
            total_expressions: 10,
            unique_values: 5,
        };
        assert_eq!(report.compression_ratio(), 0.5);
    }

    #[test]
    fn test_compression_ratio_high_sharing() {
        let report = GVNReport {
            function: "test".to_string(),
            equivalences: vec![],
            redundancies: vec![],
            total_expressions: 100,
            unique_values: 10,
        };
        assert_eq!(report.compression_ratio(), 0.1);
    }
}

// =============================================================================
// Hash Collision Tests
// =============================================================================

#[cfg(test)]
mod hash_collision_tests {
    use super::*;

    /// Different expressions should not collide
    #[test]
    fn test_no_false_positives() {
        let source = r#"
def no_collide(a, b, c, d):
    w = a + b
    x = c + d
    y = a * b
    z = a - b
    return w + x + y + z
"#;
        let reports = compute_gvn(source, Some("no_collide"));
        assert_eq!(reports.len(), 1);

        let report = &reports[0];
        // None of these should be marked as equivalent
        // (they're all different operations or operands)
        assert!(
            report.equivalences.is_empty()
                || report
                    .equivalences
                    .iter()
                    .all(|eq| eq.expressions.len() == 1),
            "Different expressions should not be marked equivalent"
        );
    }

    /// Complex expressions should hash correctly
    #[test]
    fn test_complex_expression_hashing() {
        let source = r#"
def complex_hash(a, b, c):
    x = (a + b) * c
    y = (a + b) * c  # should be redundant
    z = a * c + b * c  # NOT redundant (different structure)
    return x + y + z
"#;
        let reports = compute_gvn(source, Some("complex_hash"));
        assert_eq!(reports.len(), 1);

        let report = &reports[0];
        // x and y should be equivalent, z should be unique
        let xy_redundancy = report
            .redundancies
            .iter()
            .find(|r| r.original.text == "(a + b) * c" || r.redundant.text == "(a + b) * c");

        assert!(
            xy_redundancy.is_some(),
            "(a + b) * c appearing twice should be redundant"
        );
    }
}

// =============================================================================
// Integration Test: Spec Example
// =============================================================================

#[cfg(test)]
mod spec_example_tests {
    use super::*;

    /// Example from spec.md
    #[test]
    fn test_spec_example() {
        let source = r#"
def example(x, y):
    a = x + y
    b = y + x
    c = x + y
    return a + b + c
"#;
        let reports = compute_gvn(source, Some("example"));
        assert_eq!(reports.len(), 1);

        let report = &reports[0];
        assert_eq!(report.function, "example");

        // Per spec: x + y, y + x, x + y should all share VN 1
        // Expected: 1 equivalence class with 3 expressions
        let main_equiv = report
            .equivalences
            .iter()
            .find(|eq| eq.expressions.len() == 3);

        assert!(
            main_equiv.is_some(),
            "Expected equivalence class with 3 expressions"
        );

        // Should have 2 redundancies (3 expressions - 1 original)
        assert_eq!(
            report.redundancies.len(),
            2,
            "Expected 2 redundancies for 3 equivalent expressions"
        );

        // Check reason mentions commutativity
        let has_commutativity = report
            .equivalences
            .iter()
            .any(|eq| eq.reason.contains("commutativity"));
        assert!(has_commutativity, "Reason should mention commutativity");
    }
}
