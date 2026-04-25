//! L5 Class-Level Diff Tests
//!
//! These tests define the expected behavior for `--granularity class` in the
//! `tldr diff` command. They reference types and functions that DO NOT EXIST
//! yet (DiffGranularity, BaseChanges, ASTChange.children, NodeKind::Field,
//! run_class_diff). They are designed to fail at compilation until the L5
//! implementation is complete.
//!
//! Spec: thoughts/shared/plans/multi-level-diff-spec.md, Section 4.1
//!
//! Algorithm summary:
//! 1. Parse both files, extract ClassInfo (via ModuleInfo)
//! 2. Match classes by name (exact -> rename detection -> insert/delete)
//! 3. For matched pairs: diff methods, diff fields, diff bases
//! 4. Unmatched classes = Insert/Delete
//! 5. Output: DiffReport with nested ASTChange children for member changes

use std::io::Write;
use std::path::PathBuf;

use tempfile::NamedTempFile;

// Existing types
use tldr_cli::commands::remaining::types::{
    ASTChange, ChangeType, DiffReport, DiffSummary, Location, NodeKind,
};

// New types that will be added for L5 (these imports will fail until implemented)
use tldr_cli::commands::remaining::types::{BaseChanges, DiffGranularity};

// The class-level diff entry point (will be added to diff module)
use tldr_cli::commands::remaining::diff::run_class_diff;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a temporary Python file from an inline source string.
fn write_temp_py(content: &str) -> NamedTempFile {
    let mut f = NamedTempFile::with_suffix(".py").unwrap();
    write!(f, "{}", content).unwrap();
    f
}

/// Convenience: run class-level diff on two inline Python source strings.
/// Returns the DiffReport for assertion.
fn diff_classes(source_a: &str, source_b: &str) -> DiffReport {
    let file_a = write_temp_py(source_a);
    let file_b = write_temp_py(source_b);
    run_class_diff(
        &PathBuf::from(file_a.path()),
        &PathBuf::from(file_b.path()),
        false, // semantic_only
    )
    .expect("run_class_diff should succeed")
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

/// Find a child change within a parent ASTChange by name and change_type.
fn find_child_change<'a>(
    parent: &'a ASTChange,
    name: &str,
    change_type: ChangeType,
) -> Option<&'a ASTChange> {
    parent.children.as_ref().and_then(|children| {
        children
            .iter()
            .find(|c| c.change_type == change_type && c.name.as_deref() == Some(name))
    })
}

// ===========================================================================
// Test 1: Identical classes
// ===========================================================================

#[test]
fn test_class_identical() {
    let source = r#"
class Calculator:
    """A simple calculator."""

    def __init__(self, value=0):
        self.value = value

    def add(self, x):
        self.value += x
        return self

    def result(self):
        return self.value
"#;

    let report = diff_classes(source, source);

    assert!(
        report.identical,
        "Identical class files should produce identical=true"
    );
    assert!(
        report.changes.is_empty(),
        "Identical class files should have zero changes, got {}",
        report.changes.len()
    );
    assert_eq!(
        report.granularity,
        DiffGranularity::Class,
        "Report granularity should be Class"
    );
}

// ===========================================================================
// Test 2: Class with a method added
// ===========================================================================

#[test]
fn test_class_method_added() {
    let source_a = r#"
class Calculator:
    def __init__(self, value=0):
        self.value = value

    def add(self, x):
        self.value += x
        return self
"#;

    let source_b = r#"
class Calculator:
    def __init__(self, value=0):
        self.value = value

    def add(self, x):
        self.value += x
        return self

    def subtract(self, x):
        self.value -= x
        return self
"#;

    let report = diff_classes(source_a, source_b);

    assert!(!report.identical, "Files should not be identical");

    // Top-level: Calculator class is Updated (its members changed)
    let calc_change = find_change(&report.changes, "Calculator", ChangeType::Update)
        .expect("Calculator should appear as an Update change");
    assert_eq!(calc_change.node_kind, NodeKind::Class);

    // Children: subtract method was inserted
    let children = calc_change
        .children
        .as_ref()
        .expect("Updated class should have children vec");
    let subtract_insert = find_child_change(calc_change, "subtract", ChangeType::Insert)
        .expect("subtract method should be an Insert child");
    assert_eq!(subtract_insert.node_kind, NodeKind::Method);

    // Existing methods (__init__, add) should NOT appear as changes
    assert!(
        find_child_change(calc_change, "__init__", ChangeType::Update).is_none(),
        "__init__ did not change, should not appear in children"
    );
    assert!(
        find_child_change(calc_change, "add", ChangeType::Update).is_none(),
        "add did not change, should not appear in children"
    );
}

// ===========================================================================
// Test 3: Class with a method removed
// ===========================================================================

#[test]
fn test_class_method_removed() {
    let source_a = r#"
class Logger:
    def info(self, msg):
        print(f"INFO: {msg}")

    def debug(self, msg):
        print(f"DEBUG: {msg}")

    def trace(self, msg):
        print(f"TRACE: {msg}")
"#;

    let source_b = r#"
class Logger:
    def info(self, msg):
        print(f"INFO: {msg}")

    def debug(self, msg):
        print(f"DEBUG: {msg}")
"#;

    let report = diff_classes(source_a, source_b);

    assert!(!report.identical);

    let logger_change = find_change(&report.changes, "Logger", ChangeType::Update)
        .expect("Logger should appear as Update");
    assert_eq!(logger_change.node_kind, NodeKind::Class);

    let trace_delete = find_child_change(logger_change, "trace", ChangeType::Delete)
        .expect("trace method should be a Delete child");
    assert_eq!(trace_delete.node_kind, NodeKind::Method);

    // info and debug unchanged
    assert!(find_child_change(logger_change, "info", ChangeType::Update).is_none());
    assert!(find_child_change(logger_change, "debug", ChangeType::Update).is_none());
}

// ===========================================================================
// Test 4: Class with a method body updated
// ===========================================================================

#[test]
fn test_class_method_updated() {
    let source_a = r#"
class Formatter:
    def format(self, text):
        return text.strip()

    def validate(self, text):
        return len(text) > 0
"#;

    let source_b = r#"
class Formatter:
    def format(self, text):
        return text.strip().lower()

    def validate(self, text):
        return len(text) > 0
"#;

    let report = diff_classes(source_a, source_b);

    assert!(!report.identical);

    let fmt_change = find_change(&report.changes, "Formatter", ChangeType::Update)
        .expect("Formatter should appear as Update");

    let format_update = find_child_change(fmt_change, "format", ChangeType::Update)
        .expect("format method should be an Update child");
    assert_eq!(format_update.node_kind, NodeKind::Method);

    // The updated method should have a similarity score between 0.0 and 1.0
    let sim = format_update
        .similarity
        .expect("Updated method should have a similarity score");
    assert!(
        sim > 0.0 && sim < 1.0,
        "Similarity for modified method should be between 0 and 1, got {}",
        sim
    );

    // validate was not changed
    assert!(find_child_change(fmt_change, "validate", ChangeType::Update).is_none());
}

// ===========================================================================
// Test 5: Class with a field added
// ===========================================================================

#[test]
fn test_class_field_added() {
    let source_a = r#"
class Config:
    debug = False

    def __init__(self):
        self.name = "default"
"#;

    let source_b = r#"
class Config:
    debug = False
    verbose = True

    def __init__(self):
        self.name = "default"
"#;

    let report = diff_classes(source_a, source_b);

    assert!(!report.identical);

    let config_change = find_change(&report.changes, "Config", ChangeType::Update)
        .expect("Config should appear as Update");

    let verbose_insert = find_child_change(config_change, "verbose", ChangeType::Insert)
        .expect("verbose field should be an Insert child");
    assert_eq!(
        verbose_insert.node_kind,
        NodeKind::Field,
        "Inserted field should have NodeKind::Field"
    );
}

// ===========================================================================
// Test 6: Class with a field removed
// ===========================================================================

#[test]
fn test_class_field_removed() {
    let source_a = r#"
class Settings:
    timeout = 30
    retries = 3
    verbose = False
"#;

    let source_b = r#"
class Settings:
    timeout = 30
    retries = 3
"#;

    let report = diff_classes(source_a, source_b);

    assert!(!report.identical);

    let settings_change = find_change(&report.changes, "Settings", ChangeType::Update)
        .expect("Settings should appear as Update");

    let verbose_delete = find_child_change(settings_change, "verbose", ChangeType::Delete)
        .expect("verbose field should be a Delete child");
    assert_eq!(verbose_delete.node_kind, NodeKind::Field);
}

// ===========================================================================
// Test 7: Class base classes changed
// ===========================================================================

#[test]
fn test_class_base_changed() {
    let source_a = r#"
class MyWidget(BaseWidget, Serializable):
    def render(self):
        pass
"#;

    let source_b = r#"
class MyWidget(BaseWidget, Cacheable):
    def render(self):
        pass
"#;

    let report = diff_classes(source_a, source_b);

    assert!(!report.identical);

    let widget_change = find_change(&report.changes, "MyWidget", ChangeType::Update)
        .expect("MyWidget should appear as Update (bases changed)");

    let base_changes = widget_change
        .base_changes
        .as_ref()
        .expect("Updated class with base changes should have base_changes field");

    assert!(
        base_changes.removed.contains(&"Serializable".to_string()),
        "Serializable should be in removed bases, got: {:?}",
        base_changes.removed
    );
    assert!(
        base_changes.added.contains(&"Cacheable".to_string()),
        "Cacheable should be in added bases, got: {:?}",
        base_changes.added
    );

    // BaseWidget is in both, should not appear in changes
    assert!(
        !base_changes.removed.contains(&"BaseWidget".to_string()),
        "BaseWidget is unchanged, should not be in removed"
    );
    assert!(
        !base_changes.added.contains(&"BaseWidget".to_string()),
        "BaseWidget is unchanged, should not be in added"
    );
}

// ===========================================================================
// Test 8: New class inserted
// ===========================================================================

#[test]
fn test_class_inserted() {
    let source_a = r#"
class Alpha:
    def run(self):
        return 1
"#;

    let source_b = r#"
class Alpha:
    def run(self):
        return 1

class Beta:
    def run(self):
        return 2
"#;

    let report = diff_classes(source_a, source_b);

    assert!(!report.identical);

    // Alpha should be unchanged (no entry or identical)
    assert!(
        find_change(&report.changes, "Alpha", ChangeType::Update).is_none(),
        "Alpha is unchanged, should not appear as Update"
    );
    assert!(
        find_change(&report.changes, "Alpha", ChangeType::Delete).is_none(),
        "Alpha exists in both, should not be Delete"
    );

    // Beta is new
    let beta_insert = find_change(&report.changes, "Beta", ChangeType::Insert)
        .expect("Beta should appear as Insert");
    assert_eq!(beta_insert.node_kind, NodeKind::Class);
    assert!(
        beta_insert.new_location.is_some(),
        "Inserted class should have new_location"
    );
    assert!(
        beta_insert.old_location.is_none(),
        "Inserted class should not have old_location"
    );
}

// ===========================================================================
// Test 9: Class deleted
// ===========================================================================

#[test]
fn test_class_deleted() {
    let source_a = r#"
class Keeper:
    def keep(self):
        return True

class Disposable:
    def dispose(self):
        return False
"#;

    let source_b = r#"
class Keeper:
    def keep(self):
        return True
"#;

    let report = diff_classes(source_a, source_b);

    assert!(!report.identical);

    // Keeper unchanged
    assert!(find_change(&report.changes, "Keeper", ChangeType::Update).is_none());

    // Disposable deleted
    let disposable_delete = find_change(&report.changes, "Disposable", ChangeType::Delete)
        .expect("Disposable should appear as Delete");
    assert_eq!(disposable_delete.node_kind, NodeKind::Class);
    assert!(
        disposable_delete.old_location.is_some(),
        "Deleted class should have old_location"
    );
    assert!(
        disposable_delete.new_location.is_none(),
        "Deleted class should not have new_location"
    );
}

// ===========================================================================
// Test 10: Class renamed (body similar, name different)
// ===========================================================================

#[test]
fn test_class_renamed() {
    let source_a = r#"
class OldProcessor:
    def __init__(self):
        self.count = 0

    def process(self, item):
        self.count += 1
        return item.upper()

    def get_count(self):
        return self.count
"#;

    let source_b = r#"
class NewProcessor:
    def __init__(self):
        self.count = 0

    def process(self, item):
        self.count += 1
        return item.upper()

    def get_count(self):
        return self.count
"#;

    let report = diff_classes(source_a, source_b);

    assert!(!report.identical);

    // Should be detected as a rename, not as delete+insert
    let rename_change = find_change(&report.changes, "OldProcessor", ChangeType::Rename)
        .expect("OldProcessor should appear as Rename");
    assert_eq!(rename_change.node_kind, NodeKind::Class);

    // old_text should be the old name, new_text should be the new name
    assert_eq!(
        rename_change.old_text.as_deref(),
        Some("OldProcessor"),
        "old_text should be the old class name"
    );
    assert_eq!(
        rename_change.new_text.as_deref(),
        Some("NewProcessor"),
        "new_text should be the new class name"
    );

    // Similarity should be high (>= 0.8) since only the name changed
    let sim = rename_change
        .similarity
        .expect("Renamed class should have similarity score");
    assert!(
        sim >= 0.8,
        "Rename similarity should be >= 0.8 for identical bodies, got {}",
        sim
    );

    // Should NOT also appear as separate delete + insert
    assert!(
        find_change(&report.changes, "OldProcessor", ChangeType::Delete).is_none(),
        "Renamed class should not also appear as Delete"
    );
    assert!(
        find_change(&report.changes, "NewProcessor", ChangeType::Insert).is_none(),
        "Renamed class should not also appear as Insert"
    );
}

// ===========================================================================
// Test 11: Multiple classes - mixed changes
// ===========================================================================

#[test]
fn test_multiple_classes() {
    let source_a = r#"
class Unchanged:
    def method_a(self):
        return "a"

class Modified:
    def old_method(self):
        return "old"

    def shared_method(self):
        return "shared"

class Removed:
    def goodbye(self):
        return "bye"
"#;

    let source_b = r#"
class Unchanged:
    def method_a(self):
        return "a"

class Modified:
    def new_method(self):
        return "new"

    def shared_method(self):
        return "shared"

class Added:
    def hello(self):
        return "hi"
"#;

    let report = diff_classes(source_a, source_b);

    assert!(!report.identical);

    // 1. Unchanged should NOT appear in changes
    assert!(
        find_change(&report.changes, "Unchanged", ChangeType::Update).is_none(),
        "Unchanged class should not appear in changes"
    );
    assert!(
        find_change(&report.changes, "Unchanged", ChangeType::Insert).is_none(),
        "Unchanged class should not appear as Insert"
    );
    assert!(
        find_change(&report.changes, "Unchanged", ChangeType::Delete).is_none(),
        "Unchanged class should not appear as Delete"
    );

    // 2. Modified should appear as Update with method changes
    let modified_change = find_change(&report.changes, "Modified", ChangeType::Update)
        .expect("Modified should appear as Update");
    assert_eq!(modified_change.node_kind, NodeKind::Class);

    let children = modified_change
        .children
        .as_ref()
        .expect("Modified class should have children");

    // old_method was deleted (only in source_a)
    let old_method_delete = find_child_change(modified_change, "old_method", ChangeType::Delete);
    // new_method was inserted (only in source_b)
    let new_method_insert = find_child_change(modified_change, "new_method", ChangeType::Insert);

    // Either old_method is deleted and new_method is inserted,
    // OR old_method was renamed to new_method (body differs, so likely delete+insert)
    // The algorithm should detect at least one of these patterns
    let has_delete_insert = old_method_delete.is_some() && new_method_insert.is_some();
    let has_rename = children.iter().any(|c| c.change_type == ChangeType::Rename);
    assert!(
        has_delete_insert || has_rename,
        "old_method->new_method should be detected as either delete+insert or rename"
    );

    // shared_method is unchanged within Modified
    assert!(
        find_child_change(modified_change, "shared_method", ChangeType::Update).is_none(),
        "shared_method did not change, should not appear"
    );

    // 3. Removed class was deleted
    let removed_delete = find_change(&report.changes, "Removed", ChangeType::Delete)
        .expect("Removed should appear as Delete");
    assert_eq!(removed_delete.node_kind, NodeKind::Class);

    // 4. Added class was inserted
    let added_insert = find_change(&report.changes, "Added", ChangeType::Insert)
        .expect("Added should appear as Insert");
    assert_eq!(added_insert.node_kind, NodeKind::Class);

    // Verify summary counts make sense
    if let Some(ref summary) = report.summary {
        assert!(
            summary.total_changes >= 3,
            "Should have at least 3 changes (Modified update, Removed delete, Added insert), got {}",
            summary.total_changes
        );
    }
}

// ===========================================================================
// Test 12: DiffReport has correct granularity field
// ===========================================================================

#[test]
fn test_report_granularity_field() {
    let source = r#"
class Foo:
    pass
"#;
    let report = diff_classes(source, source);

    assert_eq!(
        report.granularity,
        DiffGranularity::Class,
        "Class diff report should have granularity=Class"
    );
}

// ===========================================================================
// Test 13: Class with both method and field changes
// ===========================================================================

#[test]
fn test_class_method_and_field_changes() {
    let source_a = r#"
class Service:
    timeout = 30

    def connect(self):
        return True
"#;

    let source_b = r#"
class Service:
    timeout = 60
    max_retries = 3

    def connect(self):
        return True

    def disconnect(self):
        return False
"#;

    let report = diff_classes(source_a, source_b);

    assert!(!report.identical);

    let service_change = find_change(&report.changes, "Service", ChangeType::Update)
        .expect("Service should appear as Update");

    let children = service_change
        .children
        .as_ref()
        .expect("Service should have children");

    // Should have field changes (max_retries added, timeout possibly updated)
    let has_field_changes = children.iter().any(|c| c.node_kind == NodeKind::Field);
    assert!(
        has_field_changes,
        "Should detect field-level changes in children"
    );

    // max_retries is a new field
    let max_retries_insert = find_child_change(service_change, "max_retries", ChangeType::Insert);
    assert!(
        max_retries_insert.is_some(),
        "max_retries field should be detected as Insert"
    );
    if let Some(field_change) = max_retries_insert {
        assert_eq!(field_change.node_kind, NodeKind::Field);
    }

    // disconnect is a new method
    let disconnect_insert = find_child_change(service_change, "disconnect", ChangeType::Insert);
    assert!(
        disconnect_insert.is_some(),
        "disconnect method should be detected as Insert"
    );
    if let Some(method_change) = disconnect_insert {
        assert_eq!(method_change.node_kind, NodeKind::Method);
    }
}

// ===========================================================================
// Test 14: Empty classes
// ===========================================================================

#[test]
fn test_empty_class_to_populated() {
    let source_a = r#"
class Stub:
    pass
"#;

    let source_b = r#"
class Stub:
    value = 42

    def compute(self):
        return self.value * 2
"#;

    let report = diff_classes(source_a, source_b);

    assert!(!report.identical);

    let stub_change = find_change(&report.changes, "Stub", ChangeType::Update)
        .expect("Stub should appear as Update");

    let children = stub_change
        .children
        .as_ref()
        .expect("Stub should have children for added members");

    // Should have at least a field insert and a method insert
    let field_inserts: Vec<_> = children
        .iter()
        .filter(|c| c.change_type == ChangeType::Insert && c.node_kind == NodeKind::Field)
        .collect();
    let method_inserts: Vec<_> = children
        .iter()
        .filter(|c| c.change_type == ChangeType::Insert && c.node_kind == NodeKind::Method)
        .collect();

    assert!(
        !field_inserts.is_empty(),
        "Should detect field insertion (value)"
    );
    assert!(
        !method_inserts.is_empty(),
        "Should detect method insertion (compute)"
    );
}

// ===========================================================================
// Test 15: Base changes only (no method/field changes)
// ===========================================================================

#[test]
fn test_base_change_only() {
    let source_a = r#"
class Handler(BaseHandler):
    def handle(self):
        return "handled"
"#;

    let source_b = r#"
class Handler(BaseHandler, Loggable):
    def handle(self):
        return "handled"
"#;

    let report = diff_classes(source_a, source_b);

    assert!(!report.identical);

    let handler_change = find_change(&report.changes, "Handler", ChangeType::Update)
        .expect("Handler should appear as Update (base changed)");

    let base_changes = handler_change
        .base_changes
        .as_ref()
        .expect("Handler should have base_changes");

    assert!(
        base_changes.added.contains(&"Loggable".to_string()),
        "Loggable should be in added bases"
    );
    assert!(
        base_changes.removed.is_empty(),
        "No bases were removed, got: {:?}",
        base_changes.removed
    );

    // Methods are unchanged, so children should be None or empty
    let has_method_changes = handler_change
        .children
        .as_ref()
        .map(|c| !c.is_empty())
        .unwrap_or(false);
    assert!(
        !has_method_changes,
        "No method/field changes, children should be empty or None"
    );
}
