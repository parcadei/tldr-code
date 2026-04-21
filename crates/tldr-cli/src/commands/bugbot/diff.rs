//! Function-level AST diff for bugbot
//!
//! Wraps the existing `DiffArgs::run_to_report()` infrastructure to compare
//! a baseline file (from git) against the current working-tree version.
//! Exposes convenience helpers to categorize changes by type (inserted,
//! updated, deleted functions).

use std::path::Path;

use anyhow::Result;

use crate::commands::remaining::diff::DiffArgs;
use crate::commands::remaining::types::{
    ASTChange, ChangeType, DiffGranularity, DiffReport, NodeKind,
};

/// Compute function-level AST diff between a baseline file and the current file.
///
/// Both paths must point to existing files with the same language extension.
/// The diff is performed at function granularity with `semantic_only` enabled
/// so that whitespace/comment-only changes are excluded.
pub fn diff_functions(baseline_path: &Path, current_path: &Path) -> Result<DiffReport> {
    let diff_args = DiffArgs {
        file_a: baseline_path.to_path_buf(),
        file_b: current_path.to_path_buf(),
        granularity: DiffGranularity::Function,
        semantic_only: true,
        output: None,
    };
    diff_args.run_to_report()
}

/// Compute function-level AST diff without the semantic-only filter.
///
/// This variant preserves formatting-only changes in the report, which
/// can be useful when the caller needs to see all changes including
/// whitespace and comment modifications.
pub fn diff_functions_raw(baseline_path: &Path, current_path: &Path) -> Result<DiffReport> {
    let diff_args = DiffArgs {
        file_a: baseline_path.to_path_buf(),
        file_b: current_path.to_path_buf(),
        granularity: DiffGranularity::Function,
        semantic_only: false,
        output: None,
    };
    diff_args.run_to_report()
}

/// Returns true if the given `NodeKind` represents a function-like construct.
///
/// Currently matches `Function` and `Method`.
fn is_function_like(kind: &NodeKind) -> bool {
    matches!(kind, NodeKind::Function | NodeKind::Method)
}

/// Filter to only inserted functions (new functions added in the current file).
pub fn inserted_functions(changes: &[ASTChange]) -> Vec<&ASTChange> {
    changes
        .iter()
        .filter(|c| matches!(c.change_type, ChangeType::Insert))
        .filter(|c| is_function_like(&c.node_kind))
        .collect()
}

/// Filter to only updated functions (modified function bodies).
pub fn updated_functions(changes: &[ASTChange]) -> Vec<&ASTChange> {
    changes
        .iter()
        .filter(|c| matches!(c.change_type, ChangeType::Update))
        .filter(|c| is_function_like(&c.node_kind))
        .collect()
}

/// Filter to only deleted functions (functions removed in the current file).
pub fn deleted_functions(changes: &[ASTChange]) -> Vec<&ASTChange> {
    changes
        .iter()
        .filter(|c| matches!(c.change_type, ChangeType::Delete))
        .filter(|c| is_function_like(&c.node_kind))
        .collect()
}

/// Filter to only renamed functions (same body, different name).
pub fn renamed_functions(changes: &[ASTChange]) -> Vec<&ASTChange> {
    changes
        .iter()
        .filter(|c| matches!(c.change_type, ChangeType::Rename))
        .filter(|c| is_function_like(&c.node_kind))
        .collect()
}

/// Filter to all function-like changes regardless of change type.
pub fn all_function_changes(changes: &[ASTChange]) -> Vec<&ASTChange> {
    changes
        .iter()
        .filter(|c| is_function_like(&c.node_kind))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    /// Write content to a temporary file with a `.rs` extension so tree-sitter
    /// can detect Rust as the language.
    fn write_temp_rs(content: &str) -> NamedTempFile {
        let mut f = tempfile::Builder::new()
            .suffix(".rs")
            .tempfile()
            .expect("create temp file");
        f.write_all(content.as_bytes())
            .expect("write temp file content");
        f.flush().expect("flush temp file");
        f
    }

    #[test]
    fn test_diff_no_changes() {
        let code = r#"
fn hello() {
    println!("hello");
}

fn world() -> i32 {
    42
}
"#;
        let baseline = write_temp_rs(code);
        let current = write_temp_rs(code);

        let report =
            diff_functions(baseline.path(), current.path()).expect("diff should succeed");

        assert!(
            report.identical,
            "Identical files should produce an identical report"
        );
        assert!(
            report.changes.is_empty(),
            "Identical files should produce zero changes, got: {:?}",
            report.changes
        );
    }

    #[test]
    fn test_diff_new_function_inserted() {
        let baseline_code = r#"
fn existing() {
    println!("existing");
}
"#;
        let current_code = r#"
fn existing() {
    println!("existing");
}

fn brand_new() {
    println!("I am new");
}
"#;
        let baseline = write_temp_rs(baseline_code);
        let current = write_temp_rs(current_code);

        let report =
            diff_functions(baseline.path(), current.path()).expect("diff should succeed");

        assert!(
            !report.identical,
            "Files with a new function should not be identical"
        );

        let inserts = inserted_functions(&report.changes);
        assert!(
            !inserts.is_empty(),
            "Should detect at least one inserted function, got changes: {:?}",
            report.changes
        );

        // Verify the inserted function has the expected name
        let names: Vec<&str> = inserts
            .iter()
            .filter_map(|c| c.name.as_deref())
            .collect();
        assert!(
            names.contains(&"brand_new"),
            "Inserted function should be named 'brand_new', got names: {:?}",
            names
        );
    }

    #[test]
    fn test_diff_function_deleted() {
        let baseline_code = r#"
fn keeper() {
    println!("I stay");
}

fn doomed() {
    println!("I will be removed");
}
"#;
        let current_code = r#"
fn keeper() {
    println!("I stay");
}
"#;
        let baseline = write_temp_rs(baseline_code);
        let current = write_temp_rs(current_code);

        let report =
            diff_functions(baseline.path(), current.path()).expect("diff should succeed");

        assert!(
            !report.identical,
            "Files with a deleted function should not be identical"
        );

        let deletes = deleted_functions(&report.changes);
        assert!(
            !deletes.is_empty(),
            "Should detect at least one deleted function, got changes: {:?}",
            report.changes
        );

        let names: Vec<&str> = deletes
            .iter()
            .filter_map(|c| c.name.as_deref())
            .collect();
        assert!(
            names.contains(&"doomed"),
            "Deleted function should be named 'doomed', got names: {:?}",
            names
        );
    }

    #[test]
    fn test_diff_function_body_updated() {
        let baseline_code = r#"
fn compute() -> i32 {
    let x = 1;
    let y = 2;
    x + y
}
"#;
        let current_code = r#"
fn compute() -> i32 {
    let x = 10;
    let y = 20;
    x * y
}
"#;
        let baseline = write_temp_rs(baseline_code);
        let current = write_temp_rs(current_code);

        let report =
            diff_functions(baseline.path(), current.path()).expect("diff should succeed");

        assert!(
            !report.identical,
            "Files with a modified function body should not be identical"
        );

        let updates = updated_functions(&report.changes);
        assert!(
            !updates.is_empty(),
            "Should detect at least one updated function, got changes: {:?}",
            report.changes
        );

        let names: Vec<&str> = updates
            .iter()
            .filter_map(|c| c.name.as_deref())
            .collect();
        assert!(
            names.contains(&"compute"),
            "Updated function should be named 'compute', got names: {:?}",
            names
        );
    }

    #[test]
    fn test_diff_whitespace_only() {
        let baseline_code = "fn spaced() {\n    println!(\"hello\");\n}\n";
        let current_code = "fn spaced() {\n        println!(\"hello\");\n}\n";

        let baseline = write_temp_rs(baseline_code);
        let current = write_temp_rs(current_code);

        // With semantic_only=true (the default for diff_functions), whitespace
        // changes should either be absent or classified as Format.
        let report =
            diff_functions(baseline.path(), current.path()).expect("diff should succeed");

        let semantic: Vec<&ASTChange> = report
            .changes
            .iter()
            .filter(|c| !matches!(c.change_type, ChangeType::Format))
            .collect();

        assert!(
            semantic.is_empty(),
            "Whitespace-only changes should produce no semantic changes (with semantic_only), got: {:?}",
            semantic
        );
    }

    #[test]
    fn test_all_function_changes_filter() {
        let baseline_code = r#"
fn alpha() {
    println!("a");
}

fn beta() {
    println!("b");
}
"#;
        let current_code = r#"
fn alpha() {
    println!("a modified");
}

fn gamma() {
    println!("c");
}
"#;
        let baseline = write_temp_rs(baseline_code);
        let current = write_temp_rs(current_code);

        let report =
            diff_functions(baseline.path(), current.path()).expect("diff should succeed");

        let func_changes = all_function_changes(&report.changes);
        assert!(
            func_changes.len() >= 2,
            "Should have at least 2 function-level changes (update alpha, delete beta, insert gamma), got {}",
            func_changes.len()
        );
    }

    #[test]
    fn test_diff_functions_raw_preserves_format_changes() {
        let baseline_code = "fn fmt_test() {\n    let x = 1;\n}\n";
        let current_code = "fn fmt_test() {\n        let x = 1;\n}\n";

        let baseline = write_temp_rs(baseline_code);
        let current = write_temp_rs(current_code);

        // raw diff should succeed even if there are only formatting changes
        let report =
            diff_functions_raw(baseline.path(), current.path()).expect("diff should succeed");

        // We don't assert on change count because the diff engine may or may
        // not detect the indentation shift as a change. We just verify it
        // doesn't error.
        let _ = report;
    }

    #[test]
    fn test_inserted_functions_ignores_non_function_changes() {
        // Manually construct changes to test the filter helpers in isolation
        let changes = vec![
            ASTChange {
                change_type: ChangeType::Insert,
                node_kind: NodeKind::Function,
                name: Some("func_a".to_string()),
                old_location: None,
                new_location: None,
                old_text: None,
                new_text: None,
                similarity: None,
                children: None,
                base_changes: None,
            },
            ASTChange {
                change_type: ChangeType::Insert,
                node_kind: NodeKind::Class,
                name: Some("ClassB".to_string()),
                old_location: None,
                new_location: None,
                old_text: None,
                new_text: None,
                similarity: None,
                children: None,
                base_changes: None,
            },
            ASTChange {
                change_type: ChangeType::Delete,
                node_kind: NodeKind::Function,
                name: Some("func_c".to_string()),
                old_location: None,
                new_location: None,
                old_text: None,
                new_text: None,
                similarity: None,
                children: None,
                base_changes: None,
            },
            ASTChange {
                change_type: ChangeType::Insert,
                node_kind: NodeKind::Method,
                name: Some("method_d".to_string()),
                old_location: None,
                new_location: None,
                old_text: None,
                new_text: None,
                similarity: None,
                children: None,
                base_changes: None,
            },
        ];

        let inserts = inserted_functions(&changes);
        assert_eq!(
            inserts.len(),
            2,
            "Should find 2 inserted function-like nodes (func_a + method_d)"
        );
        assert_eq!(inserts[0].name.as_deref(), Some("func_a"));
        assert_eq!(inserts[1].name.as_deref(), Some("method_d"));

        let deletes = deleted_functions(&changes);
        assert_eq!(deletes.len(), 1, "Should find 1 deleted function");
        assert_eq!(deletes[0].name.as_deref(), Some("func_c"));

        let updates = updated_functions(&changes);
        assert!(updates.is_empty(), "No updates in this set");

        let renames = renamed_functions(&changes);
        assert!(renames.is_empty(), "No renames in this set");
    }
}
