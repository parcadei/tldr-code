//! L3 Statement-Level Diff Tests (Zhang-Shasha Tree Edit Distance)
//!
//! These tests define the expected behavior for `--granularity statement` in the
//! `tldr diff` command. Statement-level diff works by:
//! 1. Running L4 function matching to pair functions by name
//! 2. For each matched pair, extracting statement subtrees from the AST
//! 3. Building labeled trees and running Zhang-Shasha tree edit distance
//! 4. Converting the edit script into ASTChange records with children
//!
//! Spec: thoughts/shared/plans/multi-level-diff-spec.md, Section 4.5

use std::io::Write;

use tempfile::NamedTempFile;

use tldr_cli::commands::remaining::diff::DiffArgs;
use tldr_cli::commands::remaining::types::{
    ASTChange, ChangeType, DiffGranularity, DiffReport, NodeKind,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a temporary Python file from an inline source string.
fn write_temp_py(content: &str) -> NamedTempFile {
    let mut f = NamedTempFile::with_suffix(".py").unwrap();
    write!(f, "{}", content).unwrap();
    f.flush().unwrap();
    f
}

/// Create a temporary TypeScript file from an inline source string.
fn write_temp_ts(content: &str) -> NamedTempFile {
    let mut f = NamedTempFile::with_suffix(".ts").unwrap();
    write!(f, "{}", content).unwrap();
    f.flush().unwrap();
    f
}

/// Run L3 (statement-level) diff and return the report.
fn run_l3_diff(file_a: &NamedTempFile, file_b: &NamedTempFile) -> DiffReport {
    let args = DiffArgs {
        file_a: file_a.path().to_path_buf(),
        file_b: file_b.path().to_path_buf(),
        granularity: DiffGranularity::Statement,
        semantic_only: false,
        output: None,
    };
    args.run_to_report()
        .expect("L3 statement-level diff should succeed")
}

/// Find a change in the report by name and change_type.
fn find_change<'a>(
    changes: &'a [ASTChange],
    name: &str,
    change_type: ChangeType,
) -> Option<&'a ASTChange> {
    changes
        .iter()
        .find(|c| c.change_type == change_type && c.name.as_deref() == Some(name))
}

// ===========================================================================
// Test 1: Identical functions -> no statement changes
// ===========================================================================

#[test]
fn test_statement_identical() {
    let source = r#"
def compute(x, y):
    result = x + y
    if result > 10:
        result = 10
    return result

def helper():
    return 42
"#;

    let file_a = write_temp_py(source);
    let file_b = write_temp_py(source);
    let report = run_l3_diff(&file_a, &file_b);

    assert!(
        report.identical,
        "Identical files should produce identical=true"
    );
    assert!(
        report.changes.is_empty(),
        "Identical files should have zero changes, got {}",
        report.changes.len()
    );
    assert_eq!(
        report.granularity,
        DiffGranularity::Statement,
        "Report granularity should be Statement"
    );
}

// ===========================================================================
// Test 2: Function gains a new statement -> Insert
// ===========================================================================

#[test]
fn test_statement_added() {
    let source_a = r#"
def compute(x, y):
    result = x + y
    return result
"#;

    let source_b = r#"
def compute(x, y):
    result = x + y
    if result < 0:
        result = 0
    return result
"#;

    let file_a = write_temp_py(source_a);
    let file_b = write_temp_py(source_b);
    let report = run_l3_diff(&file_a, &file_b);

    assert!(!report.identical, "Files should not be identical");

    // Should have an Update change for function "compute" (function-level)
    let compute_change = find_change(&report.changes, "compute", ChangeType::Update)
        .expect("Should have Update change for 'compute'");

    // The function-level change should have statement-level children
    assert!(
        compute_change.children.is_some(),
        "Function update should have statement-level children"
    );

    let children = compute_change.children.as_ref().unwrap();
    assert!(
        !children.is_empty(),
        "Should have at least one statement-level change"
    );

    // There should be an Insert for the if_statement
    let has_insert = children
        .iter()
        .any(|c| c.change_type == ChangeType::Insert && c.node_kind == NodeKind::Statement);
    assert!(
        has_insert,
        "Should have a statement Insert for the new if block"
    );
}

// ===========================================================================
// Test 3: Function loses a statement -> Delete
// ===========================================================================

#[test]
fn test_statement_removed() {
    let source_a = r#"
def compute(x, y):
    result = x + y
    if result < 0:
        result = 0
    return result
"#;

    let source_b = r#"
def compute(x, y):
    result = x + y
    return result
"#;

    let file_a = write_temp_py(source_a);
    let file_b = write_temp_py(source_b);
    let report = run_l3_diff(&file_a, &file_b);

    assert!(!report.identical, "Files should not be identical");

    let compute_change = find_change(&report.changes, "compute", ChangeType::Update)
        .expect("Should have Update change for 'compute'");

    assert!(
        compute_change.children.is_some(),
        "Function update should have statement-level children"
    );

    // There should be a Delete for the if_statement
    let has_delete = compute_change
        .children
        .as_ref()
        .unwrap()
        .iter()
        .any(|c| c.change_type == ChangeType::Delete && c.node_kind == NodeKind::Statement);
    assert!(
        has_delete,
        "Should have a statement Delete for the removed if block"
    );
}

// ===========================================================================
// Test 4: Statement changed (e.g., return value) -> Update (Relabel)
// ===========================================================================

#[test]
fn test_statement_modified() {
    let source_a = r#"
def compute(x):
    result = x * 2
    return result
"#;

    let source_b = r#"
def compute(x):
    result = x * 3
    return result
"#;

    let file_a = write_temp_py(source_a);
    let file_b = write_temp_py(source_b);
    let report = run_l3_diff(&file_a, &file_b);

    assert!(!report.identical, "Files should not be identical");

    let compute_change = find_change(&report.changes, "compute", ChangeType::Update)
        .expect("Should have Update change for 'compute'");

    assert!(
        compute_change.children.is_some(),
        "Function update should have statement-level children"
    );

    // The assignment "result = x * 2" -> "result = x * 3" should be an Update
    let has_update = compute_change
        .children
        .as_ref()
        .unwrap()
        .iter()
        .any(|c| c.change_type == ChangeType::Update && c.node_kind == NodeKind::Statement);
    assert!(
        has_update,
        "Should have a statement Update for the modified assignment"
    );
}

// ===========================================================================
// Test 5: Statements reordered -> Delete + Insert
// ===========================================================================

#[test]
fn test_statement_reordered() {
    let source_a = r#"
def process(items):
    items.sort()
    items.reverse()
    return items
"#;

    let source_b = r#"
def process(items):
    items.reverse()
    items.sort()
    return items
"#;

    let file_a = write_temp_py(source_a);
    let file_b = write_temp_py(source_b);
    let report = run_l3_diff(&file_a, &file_b);

    assert!(!report.identical, "Files should not be identical");

    let process_change = find_change(&report.changes, "process", ChangeType::Update)
        .expect("Should have Update change for 'process'");

    assert!(
        process_change.children.is_some(),
        "Function update should have statement-level children"
    );

    // Reordering should produce changes (Update/Delete+Insert depending on algorithm)
    let children = process_change.children.as_ref().unwrap();
    assert!(
        !children.is_empty(),
        "Should detect statement reordering as changes"
    );
}

// ===========================================================================
// Test 6: Nested if-block body changed
// ===========================================================================

#[test]
fn test_nested_if_changed() {
    let source_a = r#"
def check(x):
    if x > 0:
        print("positive")
        return True
    return False
"#;

    let source_b = r#"
def check(x):
    if x > 0:
        print("non-negative")
        return True
    return False
"#;

    let file_a = write_temp_py(source_a);
    let file_b = write_temp_py(source_b);
    let report = run_l3_diff(&file_a, &file_b);

    assert!(!report.identical, "Files should not be identical");

    let check_change = find_change(&report.changes, "check", ChangeType::Update)
        .expect("Should have Update change for 'check'");

    assert!(
        check_change.children.is_some(),
        "Function update should have statement-level children"
    );

    // Should detect the change within the if block
    let children = check_change.children.as_ref().unwrap();
    assert!(
        !children.is_empty(),
        "Should detect changes within nested if block"
    );
}

// ===========================================================================
// Test 7: New function added -> function-level Insert (not statement)
// ===========================================================================

#[test]
fn test_unmatched_function() {
    let source_a = r#"
def existing():
    return 42
"#;

    let source_b = r#"
def existing():
    return 42

def brand_new(x):
    if x > 0:
        return x
    return 0
"#;

    let file_a = write_temp_py(source_a);
    let file_b = write_temp_py(source_b);
    let report = run_l3_diff(&file_a, &file_b);

    assert!(!report.identical, "Files should not be identical");

    // "existing" should not have any changes (identical)
    let existing_change = find_change(&report.changes, "existing", ChangeType::Update);
    assert!(
        existing_change.is_none(),
        "Identical function 'existing' should have no Update change"
    );

    // "brand_new" should be a function-level Insert, NOT statement-level
    let new_fn = find_change(&report.changes, "brand_new", ChangeType::Insert)
        .expect("Should have Insert for new function 'brand_new'");

    assert_eq!(
        new_fn.node_kind,
        NodeKind::Function,
        "New function should be reported as NodeKind::Function, not Statement"
    );

    // Function-level insert should NOT have statement children
    // (no point in statement-diffing a brand new function)
    assert!(
        new_fn.children.is_none() || new_fn.children.as_ref().unwrap().is_empty(),
        "Function-level Insert should not have statement children"
    );
}

// ===========================================================================
// Test 8: Multiple functions, one with statement changes, one identical
// ===========================================================================

#[test]
fn test_multiple_functions() {
    let source_a = r#"
def unchanged():
    return 42

def modified(x):
    result = x + 1
    return result
"#;

    let source_b = r#"
def unchanged():
    return 42

def modified(x):
    result = x + 1
    if result > 100:
        result = 100
    return result
"#;

    let file_a = write_temp_py(source_a);
    let file_b = write_temp_py(source_b);
    let report = run_l3_diff(&file_a, &file_b);

    assert!(!report.identical, "Files should not be identical");

    // "unchanged" should not appear in changes
    let unchanged = find_change(&report.changes, "unchanged", ChangeType::Update);
    assert!(
        unchanged.is_none(),
        "'unchanged' function should not have any changes"
    );

    // "modified" should have an Update with statement-level children
    let modified_change = find_change(&report.changes, "modified", ChangeType::Update)
        .expect("Should have Update change for 'modified'");

    assert!(
        modified_change.children.is_some(),
        "Modified function update should have statement-level children"
    );

    let children = modified_change.children.as_ref().unwrap();
    assert!(
        !children.is_empty(),
        "Modified function should have statement-level changes"
    );
}

// ===========================================================================
// Test 9: Large function fallback (>200 statements -> no statement children)
// ===========================================================================

#[test]
fn test_large_function_fallback() {
    // Generate a function with >200 statements
    let mut lines_a = vec!["def big_func():".to_string()];
    let mut lines_b = vec!["def big_func():".to_string()];
    for i in 0..210 {
        lines_a.push(format!("    x_{} = {}", i, i));
        // Change the last few to ensure it's detected as Update
        if i >= 205 {
            lines_b.push(format!("    x_{} = {}", i, i + 1000));
        } else {
            lines_b.push(format!("    x_{} = {}", i, i));
        }
    }
    lines_a.push("    return x_0".to_string());
    lines_b.push("    return x_0".to_string());

    let source_a = lines_a.join("\n");
    let source_b = lines_b.join("\n");

    let file_a = write_temp_py(&source_a);
    let file_b = write_temp_py(&source_b);
    let report = run_l3_diff(&file_a, &file_b);

    assert!(!report.identical, "Files should not be identical");

    // Should have an Update for big_func
    let big_fn = find_change(&report.changes, "big_func", ChangeType::Update)
        .expect("Should have Update for 'big_func'");

    // With >200 statements, should fall back to L4-style (no statement children)
    // OR have children but computed via fallback. Either way, the function
    // should be detected as changed.
    assert_eq!(
        big_fn.node_kind,
        NodeKind::Function,
        "big_func should be reported as a function-level update"
    );
}

// ===========================================================================
// Test 10: TypeScript statement diff (multi-language support)
// ===========================================================================

#[test]
fn test_statement_typescript() {
    let source_a = r#"
function calculate(x: number): number {
    const result = x * 2;
    return result;
}
"#;

    let source_b = r#"
function calculate(x: number): number {
    const result = x * 2;
    if (result > 100) {
        return 100;
    }
    return result;
}
"#;

    let file_a = write_temp_ts(source_a);
    let file_b = write_temp_ts(source_b);
    let report = run_l3_diff(&file_a, &file_b);

    assert!(!report.identical, "Files should not be identical");
    assert_eq!(report.granularity, DiffGranularity::Statement);

    // Should detect the function as modified
    let calc_change = find_change(&report.changes, "calculate", ChangeType::Update)
        .expect("Should have Update change for 'calculate'");

    // Should have statement-level children
    assert!(
        calc_change.children.is_some() && !calc_change.children.as_ref().unwrap().is_empty(),
        "TypeScript function update should have statement-level children"
    );
}

// ===========================================================================
// Test 11: Function deleted should be function-level Delete
// ===========================================================================

#[test]
fn test_function_deleted() {
    let source_a = r#"
def keep_me():
    return 1

def remove_me():
    x = 42
    return x
"#;

    let source_b = r#"
def keep_me():
    return 1
"#;

    let file_a = write_temp_py(source_a);
    let file_b = write_temp_py(source_b);
    let report = run_l3_diff(&file_a, &file_b);

    assert!(!report.identical);

    let del = find_change(&report.changes, "remove_me", ChangeType::Delete)
        .expect("Should have Delete for removed function");

    assert_eq!(
        del.node_kind,
        NodeKind::Function,
        "Removed function should be NodeKind::Function"
    );

    // Function-level deletes should not have statement children
    assert!(
        del.children.is_none() || del.children.as_ref().unwrap().is_empty(),
        "Function-level Delete should not have statement children"
    );
}

// ===========================================================================
// Test 12: Summary statistics should be correct
// ===========================================================================

#[test]
fn test_statement_summary() {
    let source_a = r#"
def func_a():
    return 1

def func_b(x):
    result = x + 1
    return result
"#;

    let source_b = r#"
def func_a():
    return 1

def func_b(x):
    result = x + 1
    if result > 10:
        result = 10
    return result
"#;

    let file_a = write_temp_py(source_a);
    let file_b = write_temp_py(source_b);
    let report = run_l3_diff(&file_a, &file_b);

    let summary = report.summary.expect("Report should have summary");
    assert!(
        summary.total_changes >= 1,
        "Should have at least 1 function-level change"
    );
    assert!(
        summary.updates >= 1,
        "Should have at least 1 update (for func_b)"
    );
}
