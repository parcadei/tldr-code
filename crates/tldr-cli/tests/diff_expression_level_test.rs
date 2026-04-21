//! L2 Expression-Level Diff Tests
//!
//! Defines expected behavior for `DiffGranularity::Expression` (L2).
//! These tests WILL FAIL until the difftastic-based L2 implementation is complete.
//!
//! L2 sits between L1 (token) and L3 (statement):
//! - L1: every individual token change is a separate DiffChange
//! - L2: token changes GROUPED into expression-level DiffChange entries
//! - L3: changes grouped at statement level
//!
//! For example, if `x + 1` changes to `x + 2`:
//! - L1 reports the `1` -> `2` token change
//! - L2 reports the whole `x + 1` -> `x + 2` expression change
//! - L3 reports `return x + 1` -> `return x + 2` statement change

use std::io::Write;

use tempfile::NamedTempFile;

use tldr_cli::commands::remaining::diff::DiffArgs;
use tldr_cli::commands::remaining::types::{
    ASTChange, ChangeType, DiffGranularity, DiffReport, NodeKind,
};

// =============================================================================
// Helpers
// =============================================================================

/// Create a temp file with given content and file extension suffix.
fn write_temp(content: &str, suffix: &str) -> NamedTempFile {
    let mut f = NamedTempFile::with_suffix(suffix).unwrap();
    write!(f, "{}", content).unwrap();
    f.flush().unwrap();
    f
}

/// Run L2 (expression-level) diff and return the report.
fn run_l2_diff(file_a: &NamedTempFile, file_b: &NamedTempFile) -> DiffReport {
    let args = DiffArgs {
        file_a: file_a.path().to_path_buf(),
        file_b: file_b.path().to_path_buf(),
        granularity: DiffGranularity::Expression,
        semantic_only: false,
        output: None,
    };
    args.run_to_report().expect("L2 diff should succeed")
}

/// Assert that a report has at least one change of the given type.
#[allow(dead_code)]
fn assert_has_change(report: &DiffReport, change_type: ChangeType, context: &str) {
    let found = report.changes.iter().any(|c| c.change_type == change_type);
    assert!(
        found,
        "{}: expected at least one {:?} change, but found none. Changes: {:?}",
        context,
        change_type,
        report
            .changes
            .iter()
            .map(|c| format!(
                "{:?}:{:?}:{}",
                c.change_type,
                c.node_kind,
                c.name.as_deref().unwrap_or("<none>")
            ))
            .collect::<Vec<_>>()
    );
}

/// Assert that a report has at least one change of the given type with the given node kind.
#[allow(dead_code)]
fn assert_has_change_with_kind(
    report: &DiffReport,
    change_type: ChangeType,
    node_kind: NodeKind,
    context: &str,
) {
    let found = report
        .changes
        .iter()
        .any(|c| c.change_type == change_type && c.node_kind == node_kind);
    assert!(
        found,
        "{}: expected {:?} change with {:?} kind, but found none. Changes: {:?}",
        context,
        change_type,
        node_kind,
        report
            .changes
            .iter()
            .map(|c| format!(
                "{:?}:{:?}:{}",
                c.change_type,
                c.node_kind,
                c.name.as_deref().unwrap_or("<none>")
            ))
            .collect::<Vec<_>>()
    );
}

/// Count changes of a specific type in a report.
#[allow(dead_code)]
fn count_changes(report: &DiffReport, change_type: ChangeType) -> usize {
    report
        .changes
        .iter()
        .filter(|c| c.change_type == change_type)
        .count()
}

/// Count all top-level changes (excluding children).
fn count_top_level_changes(report: &DiffReport) -> usize {
    report.changes.len()
}

/// Find the first change of a specific type.
#[allow(dead_code)]
fn find_change(report: &DiffReport, change_type: ChangeType) -> Option<&ASTChange> {
    report.changes.iter().find(|c| c.change_type == change_type)
}

/// Collect all changes including nested children into a flat list.
fn flatten_changes(report: &DiffReport) -> Vec<&ASTChange> {
    let mut result = Vec::new();
    for change in &report.changes {
        result.push(change);
        if let Some(children) = &change.children {
            for child in children {
                result.push(child);
            }
        }
    }
    result
}

/// Format changes for debug output.
fn format_changes(report: &DiffReport) -> String {
    report
        .changes
        .iter()
        .map(|c| {
            let children_info = if let Some(children) = &c.children {
                format!(
                    " [children: {}]",
                    children
                        .iter()
                        .map(|ch| format!(
                            "{:?}:{}",
                            ch.change_type,
                            ch.name.as_deref().unwrap_or("<none>")
                        ))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            } else {
                String::new()
            };
            format!(
                "{:?}:{:?}:{}{}",
                c.change_type,
                c.node_kind,
                c.name.as_deref().unwrap_or("<none>"),
                children_info
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

// =============================================================================
// Core Expression Tests
// =============================================================================

mod core_expression {
    use super::*;

    /// Test 1: Two identical files produce identical=true with no changes.
    #[test]
    fn identical_files_expression_level() {
        let content = "def foo(x):\n    return x + 1\n\ndef bar(y):\n    return y * 2\n";
        let a = write_temp(content, ".py");
        let b = write_temp(content, ".py");

        let report = run_l2_diff(&a, &b);

        assert!(
            report.identical,
            "Identical files should produce identical=true. Changes: {}",
            format_changes(&report)
        );
        assert!(
            report.changes.is_empty(),
            "Identical files should produce no changes. Changes: {}",
            format_changes(&report)
        );
        assert_eq!(
            report.granularity,
            DiffGranularity::Expression,
            "Granularity should be Expression"
        );
    }

    /// Test 2: One expression changes (e.g., `x + 1` -> `x + 2`) produces
    /// one Update at expression level.
    #[test]
    fn single_expression_change() {
        let a = write_temp("def foo(x):\n    return x + 1\n", ".py");
        let b = write_temp("def foo(x):\n    return x + 2\n", ".py");

        let report = run_l2_diff(&a, &b);

        assert!(
            !report.identical,
            "Files with expression change should not be identical"
        );
        assert_eq!(
            report.granularity,
            DiffGranularity::Expression,
            "Granularity should be Expression"
        );

        // There should be at least one change that captures the expression update
        assert!(
            !report.changes.is_empty(),
            "Should detect at least one expression-level change"
        );

        // The change should reflect the expression-level update (not just a single token)
        let all_changes = flatten_changes(&report);
        let has_update_or_insert_delete = all_changes.iter().any(|c| {
            c.change_type == ChangeType::Update
                || c.change_type == ChangeType::Insert
                || c.change_type == ChangeType::Delete
        });
        assert!(
            has_update_or_insert_delete,
            "Should have Update, Insert, or Delete changes for the expression. Changes: {}",
            format_changes(&report)
        );
    }

    /// Test 3: New expression added produces Insert change.
    #[test]
    fn expression_insert() {
        let a = write_temp("def foo(x):\n    y = x + 1\n    return y\n", ".py");
        let b = write_temp(
            "def foo(x):\n    y = x + 1\n    z = y * 2\n    return z\n",
            ".py",
        );

        let report = run_l2_diff(&a, &b);

        assert!(
            !report.identical,
            "Files with added expression should not be identical"
        );

        // Should detect an Insert for the new `z = y * 2` expression
        let all_changes = flatten_changes(&report);
        let has_insert = all_changes
            .iter()
            .any(|c| c.change_type == ChangeType::Insert);
        assert!(
            has_insert,
            "Should detect Insert for added expression. Changes: {}",
            format_changes(&report)
        );
    }

    /// Test 4: Expression removed produces Delete change.
    #[test]
    fn expression_delete() {
        let a = write_temp(
            "def foo(x):\n    y = x + 1\n    z = y * 2\n    return z\n",
            ".py",
        );
        let b = write_temp("def foo(x):\n    y = x + 1\n    return y\n", ".py");

        let report = run_l2_diff(&a, &b);

        assert!(
            !report.identical,
            "Files with removed expression should not be identical"
        );

        // Should detect a Delete for the removed `z = y * 2` expression
        let all_changes = flatten_changes(&report);
        let has_delete = all_changes
            .iter()
            .any(|c| c.change_type == ChangeType::Delete);
        assert!(
            has_delete,
            "Should detect Delete for removed expression. Changes: {}",
            format_changes(&report)
        );
    }

    /// Test 5: Inner expression changes within outer expression are reported
    /// at the appropriate expression level.
    #[test]
    fn nested_expression_change() {
        let a = write_temp("def foo(x):\n    return (x + 1) * (x - 1)\n", ".py");
        let b = write_temp("def foo(x):\n    return (x + 2) * (x - 1)\n", ".py");

        let report = run_l2_diff(&a, &b);

        assert!(
            !report.identical,
            "Files with nested expression change should not be identical"
        );

        // The change should be reported -- whether as a single expression-level
        // Update on the outer expression with children, or as a change on the
        // inner `(x + 1)` -> `(x + 2)` expression, depends on grouping.
        // Either way, there must be changes detected.
        assert!(
            !report.changes.is_empty(),
            "Should detect expression-level change for nested expression. Changes: {}",
            format_changes(&report)
        );

        // The number of top-level changes should be small -- the change in `1` -> `2`
        // should NOT produce many scattered token-level changes at the top level.
        // At L2, it should be grouped into expression-level changes.
        let total = count_top_level_changes(&report);
        assert!(
            total <= 3,
            "Nested expression change should produce at most 3 top-level changes (got {}). \
             L2 groups tokens into expressions. Changes: {}",
            total,
            format_changes(&report)
        );
    }

    /// Test 6: Several expressions changed are each reported separately.
    #[test]
    fn multiple_expression_changes() {
        let a = write_temp(
            "def foo(x):\n    a = x + 1\n    b = x * 2\n    c = x - 3\n    return a + b + c\n",
            ".py",
        );
        let b = write_temp(
            "def foo(x):\n    a = x + 10\n    b = x * 20\n    c = x - 3\n    return a + b + c\n",
            ".py",
        );

        let report = run_l2_diff(&a, &b);

        assert!(
            !report.identical,
            "Files with multiple expression changes should not be identical"
        );

        // Should detect at least 2 changes (for `a = x + 1` -> `a = x + 10`
        // and `b = x * 2` -> `b = x * 20`). The unchanged `c = x - 3` should
        // NOT appear as a change.
        let all_changes = flatten_changes(&report);
        let change_count = all_changes
            .iter()
            .filter(|c| c.change_type != ChangeType::Format)
            .count();
        assert!(
            change_count >= 2,
            "Should detect at least 2 expression-level changes (got {}). Changes: {}",
            change_count,
            format_changes(&report)
        );
    }
}

// =============================================================================
// Expression vs Token Distinction
// =============================================================================

mod expression_vs_token {
    use super::*;

    /// Test 7: A change that spans multiple tokens within one expression is
    /// reported as ONE expression-level change (not individual tokens like L1).
    #[test]
    fn groups_tokens_into_expressions() {
        // Change `x + 1` to `y + 2` -- two token changes (x->y, 1->2) within
        // one expression. L2 should group them as a single expression change.
        let a = write_temp("def foo():\n    return x + 1\n", ".py");
        let b = write_temp("def foo():\n    return y + 2\n", ".py");

        let report = run_l2_diff(&a, &b);

        assert!(
            !report.identical,
            "Files with expression change should not be identical"
        );

        // At L2, the changes within `x + 1` -> `y + 2` should be GROUPED.
        // We should NOT see individual token-level changes at the top level for
        // `x`->`y` and `1`->`2` separately. Instead, we should see the expression
        // grouped together (either as a single Update with children, or as an
        // expression-level change).
        //
        // The key distinction from L1: L1 would report each token separately.
        // L2 groups them.
        let top_level_semantic = report
            .changes
            .iter()
            .filter(|c| c.change_type != ChangeType::Format)
            .count();

        // At most we expect 1-2 top-level expression changes, not 4+ token changes
        assert!(
            top_level_semantic <= 2,
            "L2 should group token changes within an expression. Got {} top-level changes \
             (expected <= 2). If this were L1, each token change would be separate. Changes: {}",
            top_level_semantic,
            format_changes(&report)
        );
    }

    /// Test 8: Changes in two different expressions are reported as TWO separate
    /// expression-level changes.
    #[test]
    fn separates_different_expressions() {
        let a = write_temp("x = 1\ny = 2\n", ".py");
        let b = write_temp("x = 10\ny = 20\n", ".py");

        let report = run_l2_diff(&a, &b);

        assert!(
            !report.identical,
            "Files with two expression changes should not be identical"
        );

        // Two different assignment expressions changed -> two separate changes at L2.
        // Each assignment is its own expression, so they should be separate entries.
        let all_changes = flatten_changes(&report);
        let semantic_changes = all_changes
            .iter()
            .filter(|c| c.change_type != ChangeType::Format)
            .count();
        assert!(
            semantic_changes >= 2,
            "Two different expressions changed should produce at least 2 changes (got {}). \
             Changes: {}",
            semantic_changes,
            format_changes(&report)
        );
    }

    /// Test 9: Function call argument change `foo(x, y)` -> `foo(x, z)` is a
    /// single expression-level update on the call/argument expression.
    #[test]
    fn function_call_arg_change() {
        let a = write_temp("def main():\n    result = foo(x, y)\n", ".py");
        let b = write_temp("def main():\n    result = foo(x, z)\n", ".py");

        let report = run_l2_diff(&a, &b);

        assert!(
            !report.identical,
            "Files with call arg change should not be identical"
        );

        // The change `y` -> `z` within `foo(x, y)` -> `foo(x, z)` should be
        // grouped at the expression level. At L2, the call expression is the
        // grouping unit.
        assert!(
            !report.changes.is_empty(),
            "Should detect expression-level change for call arg. Changes: {}",
            format_changes(&report)
        );

        // Should NOT produce many separate top-level changes -- the arg change
        // is within one expression.
        let top_level = count_top_level_changes(&report);
        assert!(
            top_level <= 3,
            "Call arg change should be grouped in expression (got {} top-level, expected <= 3). \
             Changes: {}",
            top_level,
            format_changes(&report)
        );
    }
}

// =============================================================================
// Language-Specific Tests
// =============================================================================

mod language_specific {
    use super::*;

    /// Test 10: Python with changed conditional expression produces
    /// expression-level change.
    #[test]
    fn python_expression_diff() {
        let a = write_temp(
            "def check(x):\n    result = x if x > 0 else -x\n    return result\n",
            ".py",
        );
        let b = write_temp(
            "def check(x):\n    result = x if x > 10 else -x\n    return result\n",
            ".py",
        );

        let report = run_l2_diff(&a, &b);

        assert!(
            !report.identical,
            "Python conditional expression change should not be identical"
        );
        assert_eq!(
            report.granularity,
            DiffGranularity::Expression,
            "Granularity should be Expression"
        );

        // The change from `x > 0` to `x > 10` should be detected as an
        // expression-level change, not as isolated token changes.
        assert!(
            !report.changes.is_empty(),
            "Should detect expression change in Python conditional. Changes: {}",
            format_changes(&report)
        );
    }

    /// Test 11: Rust with changed match arm produces expression-level change.
    #[test]
    fn rust_expression_diff() {
        let a = write_temp(
            r#"fn classify(x: i32) -> &'static str {
    match x {
        0 => "zero",
        1 => "one",
        _ => "other",
    }
}
"#,
            ".rs",
        );
        let b = write_temp(
            r#"fn classify(x: i32) -> &'static str {
    match x {
        0 => "zero",
        1 => "one",
        2 => "two",
        _ => "other",
    }
}
"#,
            ".rs",
        );

        let report = run_l2_diff(&a, &b);

        assert!(
            !report.identical,
            "Rust match arm change should not be identical"
        );
        assert_eq!(
            report.granularity,
            DiffGranularity::Expression,
            "Granularity should be Expression"
        );

        // Should detect the added match arm `2 => "two"` as an expression-level
        // Insert or as part of an Update to the match expression.
        let all_changes = flatten_changes(&report);
        let has_change = all_changes
            .iter()
            .any(|c| c.change_type == ChangeType::Insert || c.change_type == ChangeType::Update);
        assert!(
            has_change,
            "Should detect Insert or Update for added Rust match arm. Changes: {}",
            format_changes(&report)
        );
    }

    /// Test 12: TypeScript with changed ternary produces expression-level change.
    #[test]
    fn typescript_expression_diff() {
        let a = write_temp(
            "function check(x: number): number {\n    return x > 0 ? x : -x;\n}\n",
            ".ts",
        );
        let b = write_temp(
            "function check(x: number): number {\n    return x > 10 ? x : -x;\n}\n",
            ".ts",
        );

        let report = run_l2_diff(&a, &b);

        assert!(
            !report.identical,
            "TypeScript ternary change should not be identical"
        );
        assert_eq!(
            report.granularity,
            DiffGranularity::Expression,
            "Granularity should be Expression"
        );

        // The change from `x > 0` to `x > 10` within the ternary should be
        // reported at the expression level.
        assert!(
            !report.changes.is_empty(),
            "Should detect expression change in TypeScript ternary. Changes: {}",
            format_changes(&report)
        );
    }
}

// =============================================================================
// Edge Cases
// =============================================================================

mod edge_cases {
    use super::*;

    /// Test 13: Empty function body is handled properly.
    #[test]
    fn empty_expression() {
        let a = write_temp("def foo():\n    pass\n", ".py");
        let b = write_temp("def foo():\n    return 1\n", ".py");

        let report = run_l2_diff(&a, &b);

        assert!(
            !report.identical,
            "Empty body to non-empty body should not be identical"
        );
        assert_eq!(
            report.granularity,
            DiffGranularity::Expression,
            "Granularity should be Expression"
        );

        // Should detect the change from `pass` to `return 1`
        assert!(
            !report.changes.is_empty(),
            "Should detect changes from pass to return. Changes: {}",
            format_changes(&report)
        );
    }

    /// Test 14: Deeply nested expressions (5+ levels) report changes at the
    /// correct expression level.
    #[test]
    fn deeply_nested_expressions() {
        let a = write_temp("def foo(x):\n    return ((((x + 1))))\n", ".py");
        let b = write_temp("def foo(x):\n    return ((((x + 2))))\n", ".py");

        let report = run_l2_diff(&a, &b);

        assert!(
            !report.identical,
            "Deeply nested expression change should not be identical"
        );

        // Even with 5+ levels of parenthesization, the change should bubble up
        // to a reasonable expression level -- not produce a separate change for
        // each nesting level.
        assert!(
            !report.changes.is_empty(),
            "Should detect deeply nested expression change. Changes: {}",
            format_changes(&report)
        );

        // The number of top-level changes should be small (the nesting should
        // be grouped, not each paren reported separately).
        let total = count_top_level_changes(&report);
        assert!(
            total <= 3,
            "Deeply nested change should produce at most 3 top-level changes (got {}). \
             L2 should not report each nesting level separately. Changes: {}",
            total,
            format_changes(&report)
        );
    }

    /// Test 15: Verify report.granularity field is DiffGranularity::Expression.
    #[test]
    fn granularity_is_expression() {
        let a = write_temp("x = 1\n", ".py");
        let b = write_temp("x = 2\n", ".py");

        let report = run_l2_diff(&a, &b);

        assert_eq!(
            report.granularity,
            DiffGranularity::Expression,
            "DiffReport granularity must be Expression for L2 diff. Got: {:?}",
            report.granularity
        );
    }
}

// =============================================================================
// Expression-Level Children Tests
// =============================================================================

mod children {
    use super::*;

    /// When an expression is Updated (delimiters unchanged but internals changed),
    /// the top-level ASTChange should have children containing the token-level
    /// changes within that expression.
    #[test]
    fn update_has_children() {
        let a = write_temp("def foo(x):\n    return x + 1\n", ".py");
        let b = write_temp("def foo(x):\n    return x + 2\n", ".py");

        let report = run_l2_diff(&a, &b);

        assert!(!report.identical, "Files should not be identical");

        // Find expression-level Update changes
        let updates: Vec<&ASTChange> = report
            .changes
            .iter()
            .filter(|c| c.change_type == ChangeType::Update)
            .collect();

        // If there are Update changes, they should have children per the spec
        // (L2 Update = expression delimiters unchanged but children changed)
        if !updates.is_empty() {
            let has_children = updates.iter().any(|u| {
                u.children
                    .as_ref()
                    .map(|ch| !ch.is_empty())
                    .unwrap_or(false)
            });
            assert!(
                has_children,
                "Expression-level Update changes should have children containing \
                 token-level changes. Updates: {:?}",
                updates
                    .iter()
                    .map(|u| format!(
                        "name={}, children={:?}",
                        u.name.as_deref().unwrap_or("<none>"),
                        u.children.as_ref().map(|c| c.len())
                    ))
                    .collect::<Vec<_>>()
            );
        }
        // If no Updates, the changes might be reported as paired Insert/Delete --
        // which is also valid for L2 (entire expression novel on each side).
        // In that case, at minimum we need some changes detected.
        assert!(!report.changes.is_empty(), "Should have changes detected");
    }

    /// When an entire expression is inserted (novel on RHS), it should NOT have
    /// children -- it is a single expression-level Insert.
    #[test]
    fn insert_no_children() {
        let a = write_temp("def foo(x):\n    return x\n", ".py");
        let b = write_temp("def foo(x):\n    y = x * 2\n    return y\n", ".py");

        let report = run_l2_diff(&a, &b);

        assert!(!report.identical, "Files should not be identical");

        // Find Insert changes
        let inserts: Vec<&ASTChange> = report
            .changes
            .iter()
            .filter(|c| c.change_type == ChangeType::Insert)
            .collect();

        // Inserted expressions should NOT have children -- the whole expression
        // is novel, so it is a flat Insert.
        for ins in &inserts {
            let child_count = ins.children.as_ref().map(|c| c.len()).unwrap_or(0);
            assert_eq!(
                child_count,
                0,
                "Expression-level Insert should not have children (got {}). \
                 Name: {}",
                child_count,
                ins.name.as_deref().unwrap_or("<none>")
            );
        }
    }
}

// =============================================================================
// Summary Statistics Tests
// =============================================================================

mod summary {
    use super::*;

    /// The summary counts should accurately reflect expression-level changes.
    #[test]
    fn summary_counts_accurate() {
        let a = write_temp("x = 1\ny = 2\n", ".py");
        let b = write_temp("x = 10\ny = 2\nz = 3\n", ".py");

        let report = run_l2_diff(&a, &b);

        assert!(!report.identical, "Files should not be identical");

        // Verify summary exists and has non-zero counts
        let summary = report
            .summary
            .as_ref()
            .expect("L2 report should have a summary");

        assert!(
            summary.total_changes > 0,
            "Summary should have non-zero total_changes. Summary: {:?}",
            summary
        );

        // The total should match the actual number of changes
        let actual_count = report.changes.len() as u32;
        assert_eq!(
            summary.total_changes, actual_count,
            "Summary total_changes ({}) should match actual changes count ({}). Summary: {:?}",
            summary.total_changes, actual_count, summary
        );
    }
}

// =============================================================================
// Multi-Language Expression Tests (additional coverage)
// =============================================================================

mod multilang_expression {
    use super::*;

    /// JavaScript arrow function expression change.
    #[test]
    fn javascript_arrow_expression() {
        let a = write_temp(
            "const add = (x, y) => x + y;\nconst sub = (x, y) => x - y;\n",
            ".js",
        );
        let b = write_temp(
            "const add = (x, y) => x + y + 1;\nconst sub = (x, y) => x - y;\n",
            ".js",
        );

        let report = run_l2_diff(&a, &b);

        assert!(
            !report.identical,
            "JavaScript arrow expression change should not be identical"
        );
        assert_eq!(
            report.granularity,
            DiffGranularity::Expression,
            "Granularity should be Expression"
        );
        assert!(
            !report.changes.is_empty(),
            "Should detect expression change in JS arrow function. Changes: {}",
            format_changes(&report)
        );
    }

    /// Go expression-level diff with changed return expression.
    #[test]
    fn go_expression_diff() {
        let a = write_temp(
            "package main\n\nfunc foo(x int) int {\n\treturn x + 1\n}\n",
            ".go",
        );
        let b = write_temp(
            "package main\n\nfunc foo(x int) int {\n\treturn x + 2\n}\n",
            ".go",
        );

        let report = run_l2_diff(&a, &b);

        assert!(
            !report.identical,
            "Go expression change should not be identical"
        );
        assert_eq!(
            report.granularity,
            DiffGranularity::Expression,
            "Granularity should be Expression"
        );
        assert!(
            !report.changes.is_empty(),
            "Should detect expression change in Go. Changes: {}",
            format_changes(&report)
        );
    }

    /// Java expression-level diff with changed method call argument.
    #[test]
    fn java_expression_diff() {
        let a = write_temp(
            "public class Main {\n    public static int foo(int x) {\n        return Math.max(x, 0);\n    }\n}\n",
            ".java",
        );
        let b = write_temp(
            "public class Main {\n    public static int foo(int x) {\n        return Math.max(x, 10);\n    }\n}\n",
            ".java",
        );

        let report = run_l2_diff(&a, &b);

        assert!(
            !report.identical,
            "Java expression change should not be identical"
        );
        assert_eq!(
            report.granularity,
            DiffGranularity::Expression,
            "Granularity should be Expression"
        );
        assert!(
            !report.changes.is_empty(),
            "Should detect expression change in Java method call. Changes: {}",
            format_changes(&report)
        );
    }

    /// Ruby expression-level diff.
    #[test]
    fn ruby_expression_diff() {
        let a = write_temp("def foo(x)\n  x + 1\nend\n", ".rb");
        let b = write_temp("def foo(x)\n  x + 2\nend\n", ".rb");

        let report = run_l2_diff(&a, &b);

        assert!(
            !report.identical,
            "Ruby expression change should not be identical"
        );
        assert_eq!(
            report.granularity,
            DiffGranularity::Expression,
            "Granularity should be Expression"
        );
        assert!(
            !report.changes.is_empty(),
            "Should detect expression change in Ruby. Changes: {}",
            format_changes(&report)
        );
    }

    /// C expression-level diff.
    #[test]
    fn c_expression_diff() {
        let a = write_temp("int foo(int x) {\n    return x + 1;\n}\n", ".c");
        let b = write_temp("int foo(int x) {\n    return x + 2;\n}\n", ".c");

        let report = run_l2_diff(&a, &b);

        assert!(
            !report.identical,
            "C expression change should not be identical"
        );
        assert_eq!(
            report.granularity,
            DiffGranularity::Expression,
            "Granularity should be Expression"
        );
        assert!(
            !report.changes.is_empty(),
            "Should detect expression change in C. Changes: {}",
            format_changes(&report)
        );
    }

    /// C++ expression-level diff with changed return value.
    #[test]
    fn cpp_expression_diff() {
        let a = write_temp(
            "#include <iostream>\nint main() {\n    std::cout << \"hello\";\n    return 0;\n}\n",
            ".cpp",
        );
        let b = write_temp(
            "#include <iostream>\nint main() {\n    std::cout << \"hello\";\n    return 1;\n}\n",
            ".cpp",
        );

        let report = run_l2_diff(&a, &b);

        assert!(
            !report.identical,
            "C++ expression change should not be identical"
        );
        assert_eq!(
            report.granularity,
            DiffGranularity::Expression,
            "Granularity should be Expression"
        );
        assert!(
            !report.changes.is_empty(),
            "Should detect expression change in C++. Changes: {}",
            format_changes(&report)
        );
    }

    /// Kotlin expression-level diff with changed argument.
    /// NOTE: Kotlin uses tree-sitter-kotlin-ng which may have different node kinds
    /// than difftastic's expectations.
    #[test]
    fn kotlin_expression_diff() {
        let a = write_temp("fun main() {\n    val x = 1\n    println(x)\n}\n", ".kt");
        let b = write_temp("fun main() {\n    val x = 2\n    println(x)\n}\n", ".kt");

        let report = run_l2_diff(&a, &b);

        assert!(
            !report.identical,
            "Kotlin expression change should not be identical"
        );
        assert_eq!(
            report.granularity,
            DiffGranularity::Expression,
            "Granularity should be Expression"
        );
        assert!(
            !report.changes.is_empty(),
            "Should detect expression change in Kotlin. Changes: {}",
            format_changes(&report)
        );
    }

    /// Swift expression-level diff with changed value.
    #[test]
    fn swift_expression_diff() {
        let a = write_temp(
            "func compute() -> Int {\n    let x = 1\n    return x + 1\n}\n",
            ".swift",
        );
        let b = write_temp(
            "func compute() -> Int {\n    let x = 1\n    return x + 2\n}\n",
            ".swift",
        );

        let report = run_l2_diff(&a, &b);

        assert!(
            !report.identical,
            "Swift expression change should not be identical"
        );
        assert_eq!(
            report.granularity,
            DiffGranularity::Expression,
            "Granularity should be Expression"
        );
        assert!(
            !report.changes.is_empty(),
            "Should detect expression change in Swift. Changes: {}",
            format_changes(&report)
        );
    }

    /// C# expression-level diff with changed method argument.
    #[test]
    fn csharp_expression_diff() {
        let a = write_temp(
            "using System;\nclass Program {\n    static void Main() {\n        Console.WriteLine(42);\n    }\n}\n",
            ".cs",
        );
        let b = write_temp(
            "using System;\nclass Program {\n    static void Main() {\n        Console.WriteLine(99);\n    }\n}\n",
            ".cs",
        );

        let report = run_l2_diff(&a, &b);

        assert!(
            !report.identical,
            "C# expression change should not be identical"
        );
        assert_eq!(
            report.granularity,
            DiffGranularity::Expression,
            "Granularity should be Expression"
        );
        assert!(
            !report.changes.is_empty(),
            "Should detect expression change in C#. Changes: {}",
            format_changes(&report)
        );
    }

    /// Scala expression-level diff with changed return value.
    #[test]
    fn scala_expression_diff() {
        let a = write_temp(
            "object Main {\n  def compute(x: Int): Int = {\n    x + 1\n  }\n}\n",
            ".scala",
        );
        let b = write_temp(
            "object Main {\n  def compute(x: Int): Int = {\n    x + 2\n  }\n}\n",
            ".scala",
        );

        let report = run_l2_diff(&a, &b);

        assert!(
            !report.identical,
            "Scala expression change should not be identical"
        );
        assert_eq!(
            report.granularity,
            DiffGranularity::Expression,
            "Granularity should be Expression"
        );
        assert!(
            !report.changes.is_empty(),
            "Should detect expression change in Scala. Changes: {}",
            format_changes(&report)
        );
    }

    /// PHP expression-level diff with changed return value.
    #[test]
    fn php_expression_diff() {
        let a = write_temp(
            "<?php\nfunction compute($x) {\n    return $x + 1;\n}\n",
            ".php",
        );
        let b = write_temp(
            "<?php\nfunction compute($x) {\n    return $x + 2;\n}\n",
            ".php",
        );

        let report = run_l2_diff(&a, &b);

        assert!(
            !report.identical,
            "PHP expression change should not be identical"
        );
        assert_eq!(
            report.granularity,
            DiffGranularity::Expression,
            "Granularity should be Expression"
        );
        assert!(
            !report.changes.is_empty(),
            "Should detect expression change in PHP. Changes: {}",
            format_changes(&report)
        );
    }

    /// Lua expression-level diff with changed value.
    #[test]
    fn lua_expression_diff() {
        let a = write_temp("function compute(x)\n    return x + 1\nend\n", ".lua");
        let b = write_temp("function compute(x)\n    return x + 2\nend\n", ".lua");

        let report = run_l2_diff(&a, &b);

        assert!(
            !report.identical,
            "Lua expression change should not be identical"
        );
        assert_eq!(
            report.granularity,
            DiffGranularity::Expression,
            "Granularity should be Expression"
        );
        assert!(
            !report.changes.is_empty(),
            "Should detect expression change in Lua. Changes: {}",
            format_changes(&report)
        );
    }

    /// Luau expression-level diff with changed value.
    #[test]
    fn luau_expression_diff() {
        let a = write_temp(
            "local function compute(x: number): number\n    return x + 1\nend\n",
            ".luau",
        );
        let b = write_temp(
            "local function compute(x: number): number\n    return x + 2\nend\n",
            ".luau",
        );

        let report = run_l2_diff(&a, &b);

        assert!(
            !report.identical,
            "Luau expression change should not be identical"
        );
        assert_eq!(
            report.granularity,
            DiffGranularity::Expression,
            "Granularity should be Expression"
        );
        assert!(
            !report.changes.is_empty(),
            "Should detect expression change in Luau. Changes: {}",
            format_changes(&report)
        );
    }

    /// Elixir expression-level diff with changed value.
    #[test]
    fn elixir_expression_diff() {
        let a = write_temp(
            "defmodule Math do\n  def add(x) do\n    x + 1\n  end\nend\n",
            ".ex",
        );
        let b = write_temp(
            "defmodule Math do\n  def add(x) do\n    x + 2\n  end\nend\n",
            ".ex",
        );

        let report = run_l2_diff(&a, &b);

        assert!(
            !report.identical,
            "Elixir expression change should not be identical"
        );
        assert_eq!(
            report.granularity,
            DiffGranularity::Expression,
            "Granularity should be Expression"
        );
        assert!(
            !report.changes.is_empty(),
            "Should detect expression change in Elixir. Changes: {}",
            format_changes(&report)
        );
    }

    /// OCaml expression-level diff with changed value.
    #[test]
    fn ocaml_expression_diff() {
        let a = write_temp("let compute x =\n  x + 1\n", ".ml");
        let b = write_temp("let compute x =\n  x + 2\n", ".ml");

        let report = run_l2_diff(&a, &b);

        assert!(
            !report.identical,
            "OCaml expression change should not be identical"
        );
        assert_eq!(
            report.granularity,
            DiffGranularity::Expression,
            "Granularity should be Expression"
        );
        assert!(
            !report.changes.is_empty(),
            "Should detect expression change in OCaml. Changes: {}",
            format_changes(&report)
        );
    }
}
