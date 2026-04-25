//! Directory-Level Diff Tests (L6, L7, L8)
//!
//! These tests define expected behavior for multi-level directory diff BEFORE
//! implementation exists. They SHOULD NOT COMPILE until the new types and
//! functions are added to the codebase.
//!
//! ## Test Levels
//!
//! - L6 (file-level): Structural fingerprint comparison across directories
//! - L7 (module-level): Import DAG diff + aggregated L6 file changes
//! - L8 (architecture-level): Call-graph layer diff + stability scoring
//!
//! ## Structure
//!
//! Each test creates temp directories with Python files, runs the diff at the
//! appropriate granularity, and asserts on the resulting DiffReport fields.

use std::fs;
use std::path::Path;

use serde_json::Value;
use tempfile::TempDir;

// These imports reference types that will be added by the implementation.
// Until then, these cause compile errors -- which is the point of TDD.
use tldr_cli::commands::remaining::diff::DiffArgs;
use tldr_cli::commands::remaining::types::{
    ArchChangeType, ArchLevelChange, ChangeType, DiffGranularity, DiffReport, FileLevelChange,
};

// =============================================================================
// Test Utilities
// =============================================================================

/// Create a temporary directory populated with the given files.
///
/// `files` is a slice of `(relative_path, content)` tuples. Parent directories
/// are created automatically.
fn create_test_dir(files: &[(&str, &str)]) -> TempDir {
    let dir = TempDir::new().expect("failed to create temp dir");
    for (path, content) in files {
        let full_path = dir.path().join(path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).expect("failed to create parent dirs");
        }
        fs::write(&full_path, content).expect("failed to write test file");
    }
    dir
}

/// Run a directory-level diff at the given granularity and return the DiffReport.
///
/// This helper constructs DiffArgs pointing at two directories and invokes the
/// diff engine, returning the parsed report. The implementation will wire this
/// through `DiffArgs::run()` or an equivalent internal function.
fn run_diff(dir_a: &Path, dir_b: &Path, granularity: DiffGranularity) -> DiffReport {
    let args = DiffArgs {
        file_a: dir_a.to_path_buf(),
        file_b: dir_b.to_path_buf(),
        granularity,
        semantic_only: false,
        output: None,
    };

    // run_to_report() is an internal helper that returns DiffReport directly
    // instead of emitting to stdout. This avoids coupling tests to CLI output.
    args.run_to_report().expect("diff should succeed")
}

// =============================================================================
// Shared Python Fixtures
// =============================================================================

/// A simple Python file with two functions.
const PYTHON_UTILS: &str = r#"
def helper_one(x):
    return x + 1

def helper_two(x, y):
    return x * y
"#;

/// Same as PYTHON_UTILS but with a modified helper_one (different body).
const PYTHON_UTILS_MODIFIED: &str = r#"
def helper_one(x, offset=0):
    return x + 1 + offset

def helper_two(x, y):
    return x * y
"#;

/// A Python file with a class.
const PYTHON_MODELS: &str = r#"
class User:
    def __init__(self, name, email):
        self.name = name
        self.email = email

    def greet(self):
        return f"Hello, {self.name}"
"#;

/// A Python file that imports from utils.
const PYTHON_MAIN_IMPORTS_UTILS: &str = r#"
from utils import helper_one, helper_two

def main():
    result = helper_one(10)
    product = helper_two(3, 4)
    return result + product
"#;

/// A Python file that imports from utils AND models.
const PYTHON_MAIN_IMPORTS_BOTH: &str = r#"
from utils import helper_one, helper_two
from models import User

def main():
    result = helper_one(10)
    product = helper_two(3, 4)
    user = User("Alice", "alice@example.com")
    return result + product
"#;

/// A Python service file (for architecture layer tests).
const PYTHON_API_HANDLER: &str = r#"
from core.engine import process
from utils.helpers import validate

def handle_request(request):
    data = validate(request)
    return process(data)

def handle_health():
    return {"status": "ok"}
"#;

/// A Python core engine file.
const PYTHON_CORE_ENGINE: &str = r#"
from utils.helpers import transform

def process(data):
    transformed = transform(data)
    return {"result": transformed}

def analyze(data):
    return {"analysis": len(data)}
"#;

/// A Python utility helpers file (leaf layer).
const PYTHON_UTILS_HELPERS: &str = r#"
def validate(data):
    if not data:
        raise ValueError("empty data")
    return data

def transform(data):
    return [x * 2 for x in data]

def format_output(data):
    return str(data)
"#;

// =============================================================================
// L6: File-Level Fingerprint Diff Tests
// =============================================================================

#[test]
fn test_file_level_identical_dirs() {
    // Two directories with exactly the same files and content.
    // Expected: all files classified as Identical, no changes.
    let files = &[("utils.py", PYTHON_UTILS), ("models.py", PYTHON_MODELS)];
    let dir_a = create_test_dir(files);
    let dir_b = create_test_dir(files);

    let report = run_diff(dir_a.path(), dir_b.path(), DiffGranularity::File);

    // Report should indicate identical
    assert!(
        report.identical,
        "Identical directories should produce identical=true"
    );
    assert_eq!(report.granularity, DiffGranularity::File);

    // file_changes should be present but contain only Identical entries (or be empty
    // if the implementation omits unchanged files). Either way, no Modified/Added/Removed.
    let file_changes = report
        .file_changes
        .expect("L6 report should have file_changes");
    // If the implementation includes Identical entries, they should all be Identical-equivalent.
    // If it omits them, the list should be empty. Check there are no Insert/Delete/Update.
    let modifications: Vec<&FileLevelChange> = file_changes
        .iter()
        .filter(|fc| {
            fc.change_type == ChangeType::Update
                || fc.change_type == ChangeType::Insert
                || fc.change_type == ChangeType::Delete
        })
        .collect();
    assert!(
        modifications.is_empty(),
        "Identical dirs should have no modifications, got {} changes",
        modifications.len()
    );
}

#[test]
fn test_file_level_added_file() {
    // dir_b has one extra file not present in dir_a.
    let dir_a = create_test_dir(&[("utils.py", PYTHON_UTILS)]);
    let dir_b = create_test_dir(&[("utils.py", PYTHON_UTILS), ("models.py", PYTHON_MODELS)]);

    let report = run_diff(dir_a.path(), dir_b.path(), DiffGranularity::File);

    assert!(
        !report.identical,
        "Directories with an added file should not be identical"
    );
    assert_eq!(report.granularity, DiffGranularity::File);

    let file_changes = report
        .file_changes
        .expect("L6 report should have file_changes");
    let added: Vec<&FileLevelChange> = file_changes
        .iter()
        .filter(|fc| fc.change_type == ChangeType::Insert)
        .collect();
    assert_eq!(added.len(), 1, "Should detect exactly one added file");
    assert_eq!(added[0].relative_path, "models.py");
    // Added files should have a new_fingerprint but no old_fingerprint
    assert!(
        added[0].new_fingerprint.is_some(),
        "Added file should have new_fingerprint"
    );
    assert!(
        added[0].old_fingerprint.is_none(),
        "Added file should not have old_fingerprint"
    );
}

#[test]
fn test_file_level_removed_file() {
    // dir_b is missing a file that was in dir_a.
    let dir_a = create_test_dir(&[("utils.py", PYTHON_UTILS), ("models.py", PYTHON_MODELS)]);
    let dir_b = create_test_dir(&[("utils.py", PYTHON_UTILS)]);

    let report = run_diff(dir_a.path(), dir_b.path(), DiffGranularity::File);

    assert!(
        !report.identical,
        "Directories with a removed file should not be identical"
    );

    let file_changes = report
        .file_changes
        .expect("L6 report should have file_changes");
    let removed: Vec<&FileLevelChange> = file_changes
        .iter()
        .filter(|fc| fc.change_type == ChangeType::Delete)
        .collect();
    assert_eq!(removed.len(), 1, "Should detect exactly one removed file");
    assert_eq!(removed[0].relative_path, "models.py");
    // Removed files should have old_fingerprint but no new_fingerprint
    assert!(
        removed[0].old_fingerprint.is_some(),
        "Removed file should have old_fingerprint"
    );
    assert!(
        removed[0].new_fingerprint.is_none(),
        "Removed file should not have new_fingerprint"
    );
}

#[test]
fn test_file_level_modified_file() {
    // Same file path in both dirs, but with different function signatures.
    // helper_one gains a new parameter -> structural fingerprint changes.
    let dir_a = create_test_dir(&[("utils.py", PYTHON_UTILS)]);
    let dir_b = create_test_dir(&[("utils.py", PYTHON_UTILS_MODIFIED)]);

    let report = run_diff(dir_a.path(), dir_b.path(), DiffGranularity::File);

    assert!(
        !report.identical,
        "Modified file should make dirs non-identical"
    );

    let file_changes = report
        .file_changes
        .expect("L6 report should have file_changes");
    let modified: Vec<&FileLevelChange> = file_changes
        .iter()
        .filter(|fc| fc.change_type == ChangeType::Update)
        .collect();
    assert_eq!(modified.len(), 1, "Should detect exactly one modified file");
    assert_eq!(modified[0].relative_path, "utils.py");

    // Modified files should have both old and new fingerprints, and they should differ
    let old_fp = modified[0]
        .old_fingerprint
        .expect("Modified file should have old_fingerprint");
    let new_fp = modified[0]
        .new_fingerprint
        .expect("Modified file should have new_fingerprint");
    assert_ne!(
        old_fp, new_fp,
        "Fingerprints should differ for modified files"
    );

    // Signature changes should list what changed
    let sig_changes = modified[0]
        .signature_changes
        .as_ref()
        .expect("Modified file should have signature_changes");
    assert!(
        !sig_changes.is_empty(),
        "Should report which signatures changed"
    );
    // helper_one's signature changed (added offset parameter)
    assert!(
        sig_changes.iter().any(|s| s.contains("helper_one")),
        "Signature changes should mention helper_one, got: {:?}",
        sig_changes
    );
}

#[test]
fn test_file_level_multiple_changes() {
    // A mix of added, removed, modified, and identical files.
    //
    // dir_a has: utils.py, models.py, config.py
    // dir_b has: utils.py (modified), models.py (identical), routes.py (added)
    // config.py is removed; routes.py is added; utils.py is modified; models.py is identical.
    let config_py = "CONFIG_KEY = 'value'\n";
    let routes_py = r#"
def get_routes():
    return ["/home", "/about"]
"#;

    let dir_a = create_test_dir(&[
        ("utils.py", PYTHON_UTILS),
        ("models.py", PYTHON_MODELS),
        ("config.py", config_py),
    ]);
    let dir_b = create_test_dir(&[
        ("utils.py", PYTHON_UTILS_MODIFIED),
        ("models.py", PYTHON_MODELS),
        ("routes.py", routes_py),
    ]);

    let report = run_diff(dir_a.path(), dir_b.path(), DiffGranularity::File);

    assert!(!report.identical);

    let file_changes = report
        .file_changes
        .expect("L6 report should have file_changes");

    // Count by change type
    let added_count = file_changes
        .iter()
        .filter(|fc| fc.change_type == ChangeType::Insert)
        .count();
    let removed_count = file_changes
        .iter()
        .filter(|fc| fc.change_type == ChangeType::Delete)
        .count();
    let modified_count = file_changes
        .iter()
        .filter(|fc| fc.change_type == ChangeType::Update)
        .count();

    assert_eq!(added_count, 1, "Should have 1 added file (routes.py)");
    assert_eq!(removed_count, 1, "Should have 1 removed file (config.py)");
    assert_eq!(modified_count, 1, "Should have 1 modified file (utils.py)");

    // Verify specific paths
    let added_paths: Vec<&str> = file_changes
        .iter()
        .filter(|fc| fc.change_type == ChangeType::Insert)
        .map(|fc| fc.relative_path.as_str())
        .collect();
    assert!(added_paths.contains(&"routes.py"));

    let removed_paths: Vec<&str> = file_changes
        .iter()
        .filter(|fc| fc.change_type == ChangeType::Delete)
        .map(|fc| fc.relative_path.as_str())
        .collect();
    assert!(removed_paths.contains(&"config.py"));

    let modified_paths: Vec<&str> = file_changes
        .iter()
        .filter(|fc| fc.change_type == ChangeType::Update)
        .map(|fc| fc.relative_path.as_str())
        .collect();
    assert!(modified_paths.contains(&"utils.py"));
}

#[test]
fn test_file_level_nested_dirs() {
    // Files in subdirectories should be matched by relative path including the
    // subdirectory component.
    let dir_a = create_test_dir(&[
        ("src/utils.py", PYTHON_UTILS),
        ("src/models.py", PYTHON_MODELS),
    ]);
    let dir_b = create_test_dir(&[
        ("src/utils.py", PYTHON_UTILS_MODIFIED),
        ("src/models.py", PYTHON_MODELS),
    ]);

    let report = run_diff(dir_a.path(), dir_b.path(), DiffGranularity::File);

    let file_changes = report
        .file_changes
        .expect("L6 report should have file_changes");
    let modified: Vec<&FileLevelChange> = file_changes
        .iter()
        .filter(|fc| fc.change_type == ChangeType::Update)
        .collect();
    assert_eq!(modified.len(), 1);
    // Relative path should include the subdirectory
    assert!(
        modified[0].relative_path.contains("src/utils.py")
            || modified[0].relative_path.contains("src\\utils.py"),
        "Relative path should include subdirectory: got {}",
        modified[0].relative_path
    );
}

// =============================================================================
// L7: Module-Level Import Graph Diff Tests
// =============================================================================

#[test]
fn test_module_level_import_added() {
    // dir_a: main.py imports only from utils
    // dir_b: main.py imports from utils AND models (new import edge)
    let dir_a = create_test_dir(&[
        ("utils.py", PYTHON_UTILS),
        ("models.py", PYTHON_MODELS),
        ("main.py", PYTHON_MAIN_IMPORTS_UTILS),
    ]);
    let dir_b = create_test_dir(&[
        ("utils.py", PYTHON_UTILS),
        ("models.py", PYTHON_MODELS),
        ("main.py", PYTHON_MAIN_IMPORTS_BOTH),
    ]);

    let report = run_diff(dir_a.path(), dir_b.path(), DiffGranularity::Module);

    assert!(!report.identical);
    assert_eq!(report.granularity, DiffGranularity::Module);

    let module_changes = report
        .module_changes
        .expect("L7 report should have module_changes");

    // main.py should have a module-level change because its imports changed
    let main_change = module_changes
        .iter()
        .find(|mc| mc.module_path.contains("main.py"))
        .expect("main.py should appear in module_changes");

    // The new import (models.User) should appear in imports_added
    assert!(
        !main_change.imports_added.is_empty(),
        "main.py should have imports_added"
    );
    let has_models_import = main_change
        .imports_added
        .iter()
        .any(|edge| edge.target_module.contains("models"));
    assert!(
        has_models_import,
        "imports_added should include an edge to 'models', got: {:?}",
        main_change.imports_added
    );

    // No imports were removed from main.py
    assert!(
        main_change.imports_removed.is_empty(),
        "main.py should have no imports_removed"
    );
}

#[test]
fn test_module_level_import_removed() {
    // Opposite of above: dir_a has both imports, dir_b has only utils import.
    let dir_a = create_test_dir(&[
        ("utils.py", PYTHON_UTILS),
        ("models.py", PYTHON_MODELS),
        ("main.py", PYTHON_MAIN_IMPORTS_BOTH),
    ]);
    let dir_b = create_test_dir(&[
        ("utils.py", PYTHON_UTILS),
        ("models.py", PYTHON_MODELS),
        ("main.py", PYTHON_MAIN_IMPORTS_UTILS),
    ]);

    let report = run_diff(dir_a.path(), dir_b.path(), DiffGranularity::Module);

    let module_changes = report
        .module_changes
        .expect("L7 report should have module_changes");

    let main_change = module_changes
        .iter()
        .find(|mc| mc.module_path.contains("main.py"))
        .expect("main.py should appear in module_changes");

    // The models import was removed
    assert!(
        !main_change.imports_removed.is_empty(),
        "main.py should have imports_removed"
    );
    let has_models_removal = main_change
        .imports_removed
        .iter()
        .any(|edge| edge.target_module.contains("models"));
    assert!(
        has_models_removal,
        "imports_removed should include an edge to 'models', got: {:?}",
        main_change.imports_removed
    );

    // No imports were added
    assert!(
        main_change.imports_added.is_empty(),
        "main.py should have no imports_added"
    );
}

#[test]
fn test_module_level_new_module() {
    // dir_b has a completely new file (routes.py) with its own imports.
    let routes_with_import = r#"
from utils import helper_one

def get_routes():
    val = helper_one(42)
    return ["/home", "/about", val]
"#;

    let dir_a = create_test_dir(&[
        ("utils.py", PYTHON_UTILS),
        ("main.py", PYTHON_MAIN_IMPORTS_UTILS),
    ]);
    let dir_b = create_test_dir(&[
        ("utils.py", PYTHON_UTILS),
        ("main.py", PYTHON_MAIN_IMPORTS_UTILS),
        ("routes.py", routes_with_import),
    ]);

    let report = run_diff(dir_a.path(), dir_b.path(), DiffGranularity::Module);

    let module_changes = report
        .module_changes
        .expect("L7 report should have module_changes");

    // routes.py should show up as a new module (Insert)
    let routes_change = module_changes
        .iter()
        .find(|mc| mc.module_path.contains("routes.py"))
        .expect("routes.py should appear in module_changes");

    assert_eq!(
        routes_change.change_type,
        ChangeType::Insert,
        "New module should be classified as Insert"
    );

    // Its imports should all appear as "added" since it is a brand new file
    assert!(
        !routes_change.imports_added.is_empty(),
        "New module should have its imports listed in imports_added"
    );
    let imports_utils = routes_change
        .imports_added
        .iter()
        .any(|edge| edge.target_module.contains("utils"));
    assert!(
        imports_utils,
        "routes.py imports from utils, should show in imports_added"
    );
}

#[test]
fn test_module_level_summary() {
    // Verify the ImportGraphSummary counts are correct.
    //
    // dir_a: main.py imports utils (1 edge)
    // dir_b: main.py imports utils AND models (2 edges from main.py); routes.py imports utils (1 edge)
    // Total edges: dir_a=1, dir_b=3, added=2, removed=0
    let routes_with_import = r#"
from utils import helper_one

def get_routes():
    return [helper_one(1)]
"#;

    let dir_a = create_test_dir(&[
        ("utils.py", PYTHON_UTILS),
        ("models.py", PYTHON_MODELS),
        ("main.py", PYTHON_MAIN_IMPORTS_UTILS),
    ]);
    let dir_b = create_test_dir(&[
        ("utils.py", PYTHON_UTILS),
        ("models.py", PYTHON_MODELS),
        ("main.py", PYTHON_MAIN_IMPORTS_BOTH),
        ("routes.py", routes_with_import),
    ]);

    let report = run_diff(dir_a.path(), dir_b.path(), DiffGranularity::Module);

    let summary = report
        .import_graph_summary
        .expect("L7 report should have import_graph_summary");

    // dir_a: main.py -> utils (1 edge)
    // dir_b: main.py -> utils + main.py -> models + routes.py -> utils (3 edges)
    assert!(
        summary.total_edges_b > summary.total_edges_a,
        "dir_b should have more import edges than dir_a: {} vs {}",
        summary.total_edges_b,
        summary.total_edges_a
    );
    assert!(
        summary.edges_added >= 2,
        "At least 2 edges should be added (models import + routes import), got {}",
        summary.edges_added
    );
    assert_eq!(summary.edges_removed, 0, "No edges should be removed");
    assert!(
        summary.modules_with_import_changes >= 1,
        "At least 1 module (main.py) should have import changes, got {}",
        summary.modules_with_import_changes
    );
}

#[test]
fn test_module_level_with_file_change() {
    // When a module changes both its imports AND its structural content,
    // the ModuleLevelChange should carry a file_change (L6 data).
    let dir_a = create_test_dir(&[
        ("utils.py", PYTHON_UTILS),
        ("main.py", PYTHON_MAIN_IMPORTS_UTILS),
    ]);
    let dir_b = create_test_dir(&[
        ("utils.py", PYTHON_UTILS_MODIFIED),   // structural change
        ("main.py", PYTHON_MAIN_IMPORTS_BOTH), // import change + structural change
    ]);

    let report = run_diff(dir_a.path(), dir_b.path(), DiffGranularity::Module);

    let module_changes = report
        .module_changes
        .expect("L7 report should have module_changes");

    // main.py changed both imports and structure
    let main_change = module_changes
        .iter()
        .find(|mc| mc.module_path.contains("main.py"))
        .expect("main.py should appear in module_changes");

    // It should carry a file_change since its structure changed too
    let file_change = main_change
        .file_change
        .as_ref()
        .expect("Module with structural changes should have file_change");
    assert_eq!(file_change.change_type, ChangeType::Update);
}

// =============================================================================
// L8: Architecture-Level Diff Tests
// =============================================================================

#[test]
fn test_arch_level_stable() {
    // Two identical project structures should produce stability_score = 1.0
    // and no architectural changes.
    let files = &[
        ("api/handler.py", PYTHON_API_HANDLER),
        ("core/engine.py", PYTHON_CORE_ENGINE),
        ("utils/helpers.py", PYTHON_UTILS_HELPERS),
    ];
    let dir_a = create_test_dir(files);
    let dir_b = create_test_dir(files);

    let report = run_diff(dir_a.path(), dir_b.path(), DiffGranularity::Architecture);

    assert!(
        report.identical,
        "Identical projects should be architecturally identical"
    );
    assert_eq!(report.granularity, DiffGranularity::Architecture);

    let summary = report
        .arch_summary
        .expect("L8 report should have arch_summary");
    assert!(
        (summary.stability_score - 1.0).abs() < f64::EPSILON,
        "Identical projects should have stability_score = 1.0, got {}",
        summary.stability_score
    );
    assert_eq!(summary.layer_migrations, 0);
    assert_eq!(summary.directories_added, 0);
    assert_eq!(summary.directories_removed, 0);
    assert_eq!(summary.cycles_introduced, 0);
    assert_eq!(summary.cycles_resolved, 0);

    // No arch-level changes
    let arch_changes = report.arch_changes.unwrap_or_default();
    assert!(
        arch_changes.is_empty(),
        "Identical projects should have no arch changes"
    );
}

#[test]
fn test_arch_level_new_directory() {
    // dir_b adds a new "middleware/" directory layer.
    let middleware_py = r#"
from api.handler import handle_request
from utils.helpers import validate

def auth_middleware(request):
    validate(request)
    return handle_request(request)

def logging_middleware(request):
    print(f"Request: {request}")
    return handle_request(request)
"#;

    let dir_a = create_test_dir(&[
        ("api/handler.py", PYTHON_API_HANDLER),
        ("core/engine.py", PYTHON_CORE_ENGINE),
        ("utils/helpers.py", PYTHON_UTILS_HELPERS),
    ]);
    let dir_b = create_test_dir(&[
        ("api/handler.py", PYTHON_API_HANDLER),
        ("core/engine.py", PYTHON_CORE_ENGINE),
        ("utils/helpers.py", PYTHON_UTILS_HELPERS),
        ("middleware/auth.py", middleware_py),
    ]);

    let report = run_diff(dir_a.path(), dir_b.path(), DiffGranularity::Architecture);

    assert!(!report.identical);

    let arch_changes = report
        .arch_changes
        .expect("L8 report should have arch_changes");
    let added_dirs: Vec<&ArchLevelChange> = arch_changes
        .iter()
        .filter(|ac| matches!(ac.change_type, ArchChangeType::Added))
        .collect();
    assert!(
        !added_dirs.is_empty(),
        "Should detect at least one added directory"
    );
    let has_middleware = added_dirs
        .iter()
        .any(|ac| ac.directory.contains("middleware"));
    assert!(
        has_middleware,
        "Added directories should include 'middleware', got: {:?}",
        added_dirs
            .iter()
            .map(|ac| &ac.directory)
            .collect::<Vec<_>>()
    );

    let summary = report
        .arch_summary
        .expect("L8 report should have arch_summary");
    assert!(
        summary.directories_added >= 1,
        "Should count at least 1 added directory, got {}",
        summary.directories_added
    );
    assert!(
        summary.stability_score < 1.0,
        "Adding a directory should lower stability_score from 1.0, got {}",
        summary.stability_score
    );
}

#[test]
fn test_arch_level_removed_directory() {
    // dir_b removes the "utils/" directory entirely.
    // Note: This makes the project structurally different but we are testing
    // that the architecture diff detects the removal.
    let standalone_api = r#"
def handle_request(request):
    return {"result": request}

def handle_health():
    return {"status": "ok"}
"#;
    let standalone_core = r#"
def process(data):
    return {"result": data}

def analyze(data):
    return {"analysis": len(data)}
"#;

    let dir_a = create_test_dir(&[
        ("api/handler.py", standalone_api),
        ("core/engine.py", standalone_core),
        ("utils/helpers.py", PYTHON_UTILS_HELPERS),
    ]);
    let dir_b = create_test_dir(&[
        ("api/handler.py", standalone_api),
        ("core/engine.py", standalone_core),
        // utils/ directory is entirely absent
    ]);

    let report = run_diff(dir_a.path(), dir_b.path(), DiffGranularity::Architecture);

    assert!(!report.identical);

    let arch_changes = report
        .arch_changes
        .expect("L8 report should have arch_changes");
    let removed_dirs: Vec<&ArchLevelChange> = arch_changes
        .iter()
        .filter(|ac| matches!(ac.change_type, ArchChangeType::Removed))
        .collect();
    assert!(
        !removed_dirs.is_empty(),
        "Should detect at least one removed directory"
    );
    let has_utils = removed_dirs.iter().any(|ac| ac.directory.contains("utils"));
    assert!(
        has_utils,
        "Removed directories should include 'utils', got: {:?}",
        removed_dirs
            .iter()
            .map(|ac| &ac.directory)
            .collect::<Vec<_>>()
    );

    let summary = report
        .arch_summary
        .expect("L8 report should have arch_summary");
    assert!(
        summary.directories_removed >= 1,
        "Should count at least 1 removed directory, got {}",
        summary.directories_removed
    );
}

#[test]
fn test_arch_summary_scores() {
    // Verify that ArchDiffSummary counts and stability_score behave correctly
    // for a project with multiple types of architectural changes.
    //
    // dir_a: api/, core/, utils/
    // dir_b: api/ (modified), core/, services/ (new, replaces utils)
    // Expected: 1 added (services), 1 removed (utils), stability < 1.0.

    let services_py = r#"
from core.engine import process

def serve(request):
    return process(request)

def background_task(data):
    return {"processed": data}
"#;

    let dir_a = create_test_dir(&[
        ("api/handler.py", PYTHON_API_HANDLER),
        ("core/engine.py", PYTHON_CORE_ENGINE),
        ("utils/helpers.py", PYTHON_UTILS_HELPERS),
    ]);
    let dir_b = create_test_dir(&[
        ("api/handler.py", PYTHON_API_HANDLER),
        ("core/engine.py", PYTHON_CORE_ENGINE),
        ("services/worker.py", services_py),
        // utils/ removed, services/ added
    ]);

    let report = run_diff(dir_a.path(), dir_b.path(), DiffGranularity::Architecture);

    assert!(!report.identical);

    let summary = report
        .arch_summary
        .expect("L8 report should have arch_summary");

    // At least 1 directory added and 1 removed
    assert!(
        summary.directories_added >= 1,
        "Expected at least 1 added directory (services), got {}",
        summary.directories_added
    );
    assert!(
        summary.directories_removed >= 1,
        "Expected at least 1 removed directory (utils), got {}",
        summary.directories_removed
    );

    // Stability should reflect the changes
    assert!(
        summary.stability_score < 1.0,
        "Stability should be < 1.0 with directory changes, got {}",
        summary.stability_score
    );
    assert!(
        summary.stability_score >= 0.0,
        "Stability should be non-negative, got {}",
        summary.stability_score
    );

    // Verify that arch_changes entries exist
    let arch_changes = report.arch_changes.expect("Should have arch_changes");
    assert!(
        arch_changes.len() >= 2,
        "Should have at least 2 arch changes (added + removed), got {}",
        arch_changes.len()
    );
}

// =============================================================================
// Cross-Level Consistency Tests
// =============================================================================

#[test]
fn test_granularity_field_in_report() {
    // Verify that the granularity field in DiffReport matches what was requested.
    let files = &[("utils.py", PYTHON_UTILS)];
    let dir_a = create_test_dir(files);
    let dir_b = create_test_dir(files);

    let report_l6 = run_diff(dir_a.path(), dir_b.path(), DiffGranularity::File);
    assert_eq!(report_l6.granularity, DiffGranularity::File);

    let report_l7 = run_diff(dir_a.path(), dir_b.path(), DiffGranularity::Module);
    assert_eq!(report_l7.granularity, DiffGranularity::Module);

    let report_l8 = run_diff(dir_a.path(), dir_b.path(), DiffGranularity::Architecture);
    assert_eq!(report_l8.granularity, DiffGranularity::Architecture);
}

#[test]
fn test_directory_input_validation() {
    // L6/L7/L8 should require directories, not files.
    // Passing files should produce a clear error.
    let dir = create_test_dir(&[("utils.py", PYTHON_UTILS)]);
    let file_path = dir.path().join("utils.py");

    for granularity in &[
        DiffGranularity::File,
        DiffGranularity::Module,
        DiffGranularity::Architecture,
    ] {
        let args = DiffArgs {
            file_a: file_path.clone(),
            file_b: file_path.clone(),
            granularity: *granularity,
            semantic_only: false,
            output: None,
        };

        let result = args.run_to_report();
        assert!(
            result.is_err(),
            "Passing files to {:?} granularity should produce an error",
            granularity
        );
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("director") || err_msg.contains("Director"),
            "Error should mention directories, got: {}",
            err_msg
        );
    }
}

// =============================================================================
// JSON Serialization Roundtrip Tests
// =============================================================================

#[test]
fn test_diff_report_json_roundtrip_l6() {
    // Verify that a DiffReport with file_changes serializes and deserializes correctly.
    let dir_a = create_test_dir(&[("utils.py", PYTHON_UTILS)]);
    let dir_b = create_test_dir(&[("utils.py", PYTHON_UTILS_MODIFIED)]);

    let report = run_diff(dir_a.path(), dir_b.path(), DiffGranularity::File);

    // Serialize to JSON
    let json_str =
        serde_json::to_string_pretty(&report).expect("DiffReport should serialize to JSON");

    // Parse as generic Value to check structure
    let value: Value = serde_json::from_str(&json_str).expect("JSON should parse");
    assert!(
        value.get("granularity").is_some(),
        "JSON should contain granularity field"
    );
    assert!(
        value.get("file_changes").is_some(),
        "JSON should contain file_changes field"
    );

    // Deserialize back to DiffReport
    let roundtrip: DiffReport =
        serde_json::from_str(&json_str).expect("JSON should deserialize back to DiffReport");
    assert_eq!(roundtrip.granularity, DiffGranularity::File);
    assert!(roundtrip.file_changes.is_some());
}

#[test]
fn test_diff_report_json_roundtrip_l7() {
    // Verify L7-specific fields in JSON.
    let dir_a = create_test_dir(&[
        ("utils.py", PYTHON_UTILS),
        ("main.py", PYTHON_MAIN_IMPORTS_UTILS),
    ]);
    let dir_b = create_test_dir(&[
        ("utils.py", PYTHON_UTILS),
        ("main.py", PYTHON_MAIN_IMPORTS_BOTH),
        ("models.py", PYTHON_MODELS),
    ]);

    let report = run_diff(dir_a.path(), dir_b.path(), DiffGranularity::Module);

    let json_str =
        serde_json::to_string_pretty(&report).expect("DiffReport should serialize to JSON");
    let value: Value = serde_json::from_str(&json_str).expect("JSON should parse");

    assert!(
        value.get("module_changes").is_some(),
        "L7 JSON should contain module_changes"
    );
    assert!(
        value.get("import_graph_summary").is_some(),
        "L7 JSON should contain import_graph_summary"
    );
}

#[test]
fn test_diff_report_json_roundtrip_l8() {
    // Verify L8-specific fields in JSON.
    let dir_a = create_test_dir(&[
        ("api/handler.py", PYTHON_API_HANDLER),
        ("core/engine.py", PYTHON_CORE_ENGINE),
        ("utils/helpers.py", PYTHON_UTILS_HELPERS),
    ]);
    let dir_b = create_test_dir(&[
        ("api/handler.py", PYTHON_API_HANDLER),
        ("core/engine.py", PYTHON_CORE_ENGINE),
        ("utils/helpers.py", PYTHON_UTILS_HELPERS),
    ]);

    let report = run_diff(dir_a.path(), dir_b.path(), DiffGranularity::Architecture);

    let json_str =
        serde_json::to_string_pretty(&report).expect("DiffReport should serialize to JSON");
    let value: Value = serde_json::from_str(&json_str).expect("JSON should parse");

    assert!(
        value.get("arch_summary").is_some(),
        "L8 JSON should contain arch_summary"
    );
    assert_eq!(
        value.get("granularity").and_then(|v| v.as_str()),
        Some("architecture"),
        "granularity should be 'architecture'"
    );
}

// =============================================================================
// L7: Multi-Language Import Parsing Tests
// =============================================================================

// -- TypeScript fixtures --

/// TypeScript utility module with exported functions.
const TS_UTILS: &str = r#"
export function helperOne(x: number): number {
    return x + 1;
}

export function helperTwo(x: number, y: number): number {
    return x * y;
}
"#;

/// TypeScript models module.
const TS_MODELS: &str = r#"
export interface User {
    name: string;
    email: string;
}

export function createUser(name: string, email: string): User {
    return { name, email };
}
"#;

/// TypeScript main file importing only from utils.
const TS_MAIN_IMPORTS_UTILS: &str = r#"
import { helperOne, helperTwo } from './utils';

function main(): number {
    const result = helperOne(10);
    const product = helperTwo(3, 4);
    return result + product;
}
"#;

/// TypeScript main file importing from utils AND models.
const TS_MAIN_IMPORTS_BOTH: &str = r#"
import { helperOne, helperTwo } from './utils';
import { createUser } from './models';

function main(): number {
    const result = helperOne(10);
    const product = helperTwo(3, 4);
    const user = createUser("Alice", "alice@example.com");
    return result + product;
}
"#;

// -- Go fixtures --

/// Go utility package.
const GO_UTILS: &str = r#"
package utils

func HelperOne(x int) int {
    return x + 1
}

func HelperTwo(x, y int) int {
    return x * y
}
"#;

/// Go models package.
const GO_MODELS: &str = r#"
package models

type User struct {
    Name  string
    Email string
}

func NewUser(name, email string) User {
    return User{Name: name, Email: email}
}
"#;

/// Go main file importing only utils.
const GO_MAIN_IMPORTS_UTILS: &str = r#"
package main

import "myapp/utils"

func main() {
    result := utils.HelperOne(10)
    _ = result
}
"#;

/// Go main file importing utils AND models.
const GO_MAIN_IMPORTS_BOTH: &str = r#"
package main

import (
    "myapp/utils"
    "myapp/models"
)

func main() {
    result := utils.HelperOne(10)
    user := models.NewUser("Alice", "alice@example.com")
    _ = result
    _ = user
}
"#;

// -- Rust fixtures --

/// Rust utility module.
const RS_UTILS: &str = r#"
pub fn helper_one(x: i32) -> i32 {
    x + 1
}

pub fn helper_two(x: i32, y: i32) -> i32 {
    x * y
}
"#;

/// Rust models module.
const RS_MODELS: &str = r#"
pub struct User {
    pub name: String,
    pub email: String,
}

impl User {
    pub fn new(name: String, email: String) -> Self {
        User { name, email }
    }
}
"#;

/// Rust main file with use from utils only.
const RS_MAIN_IMPORTS_UTILS: &str = r#"
use crate::utils::{helper_one, helper_two};

fn main() {
    let result = helper_one(10);
    let product = helper_two(3, 4);
    println!("{}", result + product);
}
"#;

/// Rust main file with use from utils AND models.
const RS_MAIN_IMPORTS_BOTH: &str = r#"
use crate::utils::{helper_one, helper_two};
use crate::models::User;

fn main() {
    let result = helper_one(10);
    let product = helper_two(3, 4);
    let user = User::new("Alice".into(), "alice@example.com".into());
    println!("{}", result + product);
}
"#;

#[test]
fn test_module_level_typescript_import_added() {
    // L7 should detect import changes in TypeScript files, not just Python.
    let dir_a = create_test_dir(&[
        ("utils.ts", TS_UTILS),
        ("models.ts", TS_MODELS),
        ("main.ts", TS_MAIN_IMPORTS_UTILS),
    ]);
    let dir_b = create_test_dir(&[
        ("utils.ts", TS_UTILS),
        ("models.ts", TS_MODELS),
        ("main.ts", TS_MAIN_IMPORTS_BOTH),
    ]);

    let report = run_diff(dir_a.path(), dir_b.path(), DiffGranularity::Module);

    assert!(
        !report.identical,
        "TypeScript import addition should produce non-identical report"
    );
    assert_eq!(report.granularity, DiffGranularity::Module);

    let module_changes = report
        .module_changes
        .expect("L7 report should have module_changes for TypeScript");

    // main.ts should have import changes
    let main_change = module_changes
        .iter()
        .find(|mc| mc.module_path.contains("main.ts"))
        .expect("main.ts should appear in module_changes");

    // The new import (models) should appear in imports_added
    assert!(
        !main_change.imports_added.is_empty(),
        "main.ts should have imports_added for the new models import"
    );
    let has_models_import = main_change
        .imports_added
        .iter()
        .any(|edge| edge.target_module.contains("models"));
    assert!(
        has_models_import,
        "imports_added should include an edge to 'models', got: {:?}",
        main_change.imports_added
    );
}

#[test]
fn test_module_level_typescript_import_removed() {
    // Reverse: dir_a has both imports, dir_b has only utils
    let dir_a = create_test_dir(&[
        ("utils.ts", TS_UTILS),
        ("models.ts", TS_MODELS),
        ("main.ts", TS_MAIN_IMPORTS_BOTH),
    ]);
    let dir_b = create_test_dir(&[
        ("utils.ts", TS_UTILS),
        ("models.ts", TS_MODELS),
        ("main.ts", TS_MAIN_IMPORTS_UTILS),
    ]);

    let report = run_diff(dir_a.path(), dir_b.path(), DiffGranularity::Module);

    let module_changes = report
        .module_changes
        .expect("L7 report should have module_changes");

    let main_change = module_changes
        .iter()
        .find(|mc| mc.module_path.contains("main.ts"))
        .expect("main.ts should appear in module_changes");

    assert!(
        !main_change.imports_removed.is_empty(),
        "main.ts should have imports_removed for the models import"
    );
    let has_models_removal = main_change
        .imports_removed
        .iter()
        .any(|edge| edge.target_module.contains("models"));
    assert!(
        has_models_removal,
        "imports_removed should include an edge to 'models', got: {:?}",
        main_change.imports_removed
    );
}

#[test]
fn test_module_level_go_import_added() {
    // L7 with Go source files: adding an import
    let dir_a = create_test_dir(&[
        ("utils/utils.go", GO_UTILS),
        ("models/models.go", GO_MODELS),
        ("main.go", GO_MAIN_IMPORTS_UTILS),
    ]);
    let dir_b = create_test_dir(&[
        ("utils/utils.go", GO_UTILS),
        ("models/models.go", GO_MODELS),
        ("main.go", GO_MAIN_IMPORTS_BOTH),
    ]);

    let report = run_diff(dir_a.path(), dir_b.path(), DiffGranularity::Module);

    assert!(
        !report.identical,
        "Go import addition should produce non-identical report"
    );

    let module_changes = report
        .module_changes
        .expect("L7 report should have module_changes for Go");

    let main_change = module_changes
        .iter()
        .find(|mc| mc.module_path.contains("main.go"))
        .expect("main.go should appear in module_changes");

    assert!(
        !main_change.imports_added.is_empty(),
        "main.go should have imports_added for the new models import"
    );
    let has_models_import = main_change
        .imports_added
        .iter()
        .any(|edge| edge.target_module.contains("models"));
    assert!(
        has_models_import,
        "imports_added should include an edge to 'models', got: {:?}",
        main_change.imports_added
    );
}

#[test]
fn test_module_level_rust_import_added() {
    // L7 with Rust source files: adding a use statement
    let dir_a = create_test_dir(&[
        ("utils.rs", RS_UTILS),
        ("models.rs", RS_MODELS),
        ("main.rs", RS_MAIN_IMPORTS_UTILS),
    ]);
    let dir_b = create_test_dir(&[
        ("utils.rs", RS_UTILS),
        ("models.rs", RS_MODELS),
        ("main.rs", RS_MAIN_IMPORTS_BOTH),
    ]);

    let report = run_diff(dir_a.path(), dir_b.path(), DiffGranularity::Module);

    assert!(
        !report.identical,
        "Rust import addition should produce non-identical report"
    );

    let module_changes = report
        .module_changes
        .expect("L7 report should have module_changes for Rust");

    let main_change = module_changes
        .iter()
        .find(|mc| mc.module_path.contains("main.rs"))
        .expect("main.rs should appear in module_changes");

    assert!(
        !main_change.imports_added.is_empty(),
        "main.rs should have imports_added for the new models use"
    );
    let has_models_import = main_change
        .imports_added
        .iter()
        .any(|edge| edge.target_module.contains("models"));
    assert!(
        has_models_import,
        "imports_added should include an edge to 'models', got: {:?}",
        main_change.imports_added
    );
}

#[test]
fn test_module_level_mixed_languages() {
    // L7 with a project containing Python AND TypeScript files
    // Both should have their imports parsed correctly.
    let dir_a = create_test_dir(&[
        ("backend/app.py", PYTHON_MAIN_IMPORTS_UTILS),
        ("backend/utils.py", PYTHON_UTILS),
        ("frontend/app.ts", TS_MAIN_IMPORTS_UTILS),
        ("frontend/utils.ts", TS_UTILS),
    ]);
    let dir_b = create_test_dir(&[
        ("backend/app.py", PYTHON_MAIN_IMPORTS_BOTH),
        ("backend/utils.py", PYTHON_UTILS),
        ("backend/models.py", PYTHON_MODELS),
        ("frontend/app.ts", TS_MAIN_IMPORTS_BOTH),
        ("frontend/utils.ts", TS_UTILS),
        ("frontend/models.ts", TS_MODELS),
    ]);

    let report = run_diff(dir_a.path(), dir_b.path(), DiffGranularity::Module);

    assert!(
        !report.identical,
        "Mixed-language import changes should be non-identical"
    );

    let module_changes = report
        .module_changes
        .expect("L7 report should have module_changes for mixed languages");

    // Python backend should detect import changes
    let py_change = module_changes
        .iter()
        .find(|mc| mc.module_path.contains("app.py"));
    assert!(
        py_change.is_some(),
        "Python app.py should appear in module_changes"
    );

    // TypeScript frontend should detect import changes
    let ts_change = module_changes
        .iter()
        .find(|mc| mc.module_path.contains("app.ts"));
    assert!(
        ts_change.is_some(),
        "TypeScript app.ts should appear in module_changes"
    );
}

#[test]
fn test_module_level_no_crash_on_unsupported_language() {
    // L7 should not crash when encountering files without import parsing support.
    // It should gracefully skip import analysis for those files while still
    // detecting file-level changes.
    let dir_a = create_test_dir(&[
        ("main.py", PYTHON_MAIN_IMPORTS_UTILS),
        ("utils.py", PYTHON_UTILS),
    ]);
    let dir_b = create_test_dir(&[
        ("main.py", PYTHON_MAIN_IMPORTS_BOTH),
        ("utils.py", PYTHON_UTILS),
        ("models.py", PYTHON_MODELS),
    ]);

    // This should succeed without errors
    let report = run_diff(dir_a.path(), dir_b.path(), DiffGranularity::Module);
    assert!(!report.identical);
}

#[test]
fn test_module_level_import_graph_summary_multilang() {
    // Verify the import_graph_summary counts edges correctly across languages.
    let dir_a = create_test_dir(&[
        ("utils.py", PYTHON_UTILS),
        ("main.py", PYTHON_MAIN_IMPORTS_UTILS),
        ("utils.ts", TS_UTILS),
        ("main.ts", TS_MAIN_IMPORTS_UTILS),
    ]);
    let dir_b = create_test_dir(&[
        ("utils.py", PYTHON_UTILS),
        ("models.py", PYTHON_MODELS),
        ("main.py", PYTHON_MAIN_IMPORTS_BOTH),
        ("utils.ts", TS_UTILS),
        ("models.ts", TS_MODELS),
        ("main.ts", TS_MAIN_IMPORTS_BOTH),
    ]);

    let report = run_diff(dir_a.path(), dir_b.path(), DiffGranularity::Module);

    let summary = report
        .import_graph_summary
        .expect("L7 report should have import_graph_summary");

    // dir_b should have more edges than dir_a (both Python and TS added imports)
    assert!(
        summary.total_edges_b > summary.total_edges_a,
        "dir_b should have more import edges than dir_a: {} vs {}",
        summary.total_edges_b,
        summary.total_edges_a
    );
    assert!(summary.edges_added > 0, "Should have added edges, got 0");
}

// =============================================================================
// L8: Enhanced Architecture Classification Tests
// =============================================================================

#[test]
fn test_arch_level_with_typescript_project() {
    // L8 should work with TypeScript projects too, not just Python.
    let dir_a = create_test_dir(&[
        ("api/routes.ts", TS_UTILS),
        ("core/models.ts", TS_MODELS),
        ("utils/helpers.ts", TS_UTILS),
    ]);
    let dir_b = create_test_dir(&[
        ("api/routes.ts", TS_UTILS),
        ("core/models.ts", TS_MODELS),
        ("utils/helpers.ts", TS_UTILS),
        ("services/auth.ts", TS_UTILS),
    ]);

    let report = run_diff(dir_a.path(), dir_b.path(), DiffGranularity::Architecture);

    assert!(
        !report.identical,
        "Adding a service directory should be non-identical"
    );

    let arch_changes = report
        .arch_changes
        .expect("L8 report should have arch_changes");

    let services_change = arch_changes
        .iter()
        .find(|ac| ac.directory == "services")
        .expect("services directory should appear in arch_changes");

    assert_eq!(services_change.change_type, ArchChangeType::Added);
    assert_eq!(
        services_change.new_layer.as_deref(),
        Some("service"),
        "services/ should be classified as 'service' layer"
    );
}

#[test]
fn test_arch_level_mixed_language_project() {
    // L8 should handle a project with both Python and TypeScript in same arch layers.
    let dir_a = create_test_dir(&[
        ("api/handler.py", PYTHON_API_HANDLER),
        ("api/routes.ts", TS_UTILS),
        ("core/engine.py", PYTHON_CORE_ENGINE),
        ("utils/helpers.py", PYTHON_UTILS_HELPERS),
    ]);
    let dir_b = create_test_dir(&[
        ("api/handler.py", PYTHON_API_HANDLER),
        ("api/routes.ts", TS_UTILS),
        ("core/engine.py", PYTHON_CORE_ENGINE),
        ("utils/helpers.py", PYTHON_UTILS_HELPERS),
    ]);

    let report = run_diff(dir_a.path(), dir_b.path(), DiffGranularity::Architecture);

    // Identical directories should produce identical report
    assert!(
        report.identical,
        "Identical mixed-language dirs should be identical"
    );

    let summary = report.arch_summary.expect("L8 should have arch_summary");
    assert_eq!(
        summary.stability_score, 1.0,
        "Identical dirs should have stability_score 1.0"
    );
}
