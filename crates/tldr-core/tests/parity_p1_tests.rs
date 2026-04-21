//! P1 Parity Tests: Schema Fix and Missing Core Features
//!
//! Tests defined BEFORE implementation to drive TDD.
//! These tests should FAIL initially - that's the point.
//!
//! Contract 1.1: FileTree must serialize with "type" not "node_type"

use std::path::PathBuf;
use tempfile::TempDir;

// =============================================================================
// Contract 1.1: FileTree JSON Schema - "type" field (not "node_type")
// =============================================================================

#[cfg(test)]
mod schema_tests {
    use super::*;
    use tldr_core::{FileTree, NodeType};

    /// Contract 1.1: FileTree must serialize with "type" field for Python parity
    ///
    /// Current behavior: `{"node_type": "dir", ...}`
    /// Required behavior: `{"type": "dir", ...}`
    #[test]
    fn filetree_json_uses_type_field() {
        let tree = FileTree::dir("src", vec![]);
        let json = serde_json::to_string(&tree).unwrap();

        // MUST contain "type" field
        assert!(
            json.contains("\"type\""),
            "FileTree JSON should contain 'type' field, got: {json}"
        );

        // MUST NOT contain "node_type" field
        assert!(
            !json.contains("\"node_type\""),
            "FileTree JSON should NOT contain 'node_type' field, got: {json}"
        );
    }

    /// Contract 1.1: Directory node serializes with type = "dir"
    #[test]
    fn filetree_dir_type_value() {
        let tree = FileTree::dir("mydir", vec![]);
        let json = serde_json::to_string(&tree).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(
            value.get("type").and_then(|v| v.as_str()),
            Some("dir"),
            "Directory node type should be 'dir'"
        );
    }

    /// Contract 1.1: File node serializes with type = "file"
    #[test]
    fn filetree_file_type_value() {
        let tree = FileTree::file("main.rs", PathBuf::from("src/main.rs"));
        let json = serde_json::to_string(&tree).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(
            value.get("type").and_then(|v| v.as_str()),
            Some("file"),
            "File node type should be 'file'"
        );
    }

    /// Contract 1.1: Nested tree preserves "type" field at all levels
    #[test]
    fn filetree_nested_uses_type_field() {
        let tree = FileTree::dir(
            "root",
            vec![
                FileTree::dir(
                    "subdir",
                    vec![FileTree::file(
                        "nested.py",
                        PathBuf::from("root/subdir/nested.py"),
                    )],
                ),
                FileTree::file("top.rs", PathBuf::from("root/top.rs")),
            ],
        );

        let json = serde_json::to_string(&tree).unwrap();

        // Count occurrences of "type" and "node_type"
        let type_count = json.matches("\"type\"").count();
        let node_type_count = json.matches("\"node_type\"").count();

        assert!(
            type_count >= 4,
            "Should have at least 4 'type' fields (root, subdir, nested.py, top.rs), got {type_count}"
        );
        assert_eq!(
            node_type_count, 0,
            "Should have 0 'node_type' fields, got {node_type_count}"
        );
    }

    /// Contract 1.1: Deserialization works with "type" field
    #[test]
    fn filetree_deserialize_type_field() {
        let json = r#"{"name":"test","type":"dir","children":[]}"#;
        let tree: Result<FileTree, _> = serde_json::from_str(json);

        assert!(
            tree.is_ok(),
            "Should deserialize JSON with 'type' field: {:?}",
            tree.err()
        );

        let tree = tree.unwrap();
        assert_eq!(tree.node_type, NodeType::Dir);
    }

    /// Contract 1.1: get_file_tree produces "type" not "node_type"
    #[test]
    fn get_file_tree_output_uses_type_field() {
        let temp = TempDir::new().unwrap();
        std::fs::create_dir(temp.path().join("subdir")).unwrap();
        std::fs::write(temp.path().join("file.py"), "# test").unwrap();

        let tree = tldr_core::get_file_tree(temp.path(), None, true, None).unwrap();
        let json = serde_json::to_string(&tree).unwrap();

        assert!(
            json.contains("\"type\""),
            "get_file_tree output should contain 'type' field"
        );
        assert!(
            !json.contains("\"node_type\""),
            "get_file_tree output should NOT contain 'node_type' field"
        );
    }
}

// =============================================================================
// Contract 1.2: Extract Function - Core Tests
// =============================================================================

#[cfg(test)]
mod extract_tests {
    use super::*;
    use tldr_core::extract_file;

    /// Contract 1.2: extract_file returns ModuleInfo with functions
    #[test]
    fn extract_file_returns_functions() {
        let fixtures_dir =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/python-project");
        let app_py = fixtures_dir.join("app.py");

        let result = extract_file(&app_py, None);
        assert!(result.is_ok(), "extract_file should succeed: {:?}", result);

        let module_info = result.unwrap();

        // Should find functions defined in app.py
        assert!(
            !module_info.functions.is_empty(),
            "Should extract functions from app.py"
        );

        // Should find specific functions
        let fn_names: Vec<&str> = module_info
            .functions
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        assert!(
            fn_names.contains(&"complex_function"),
            "Should find complex_function, got: {:?}",
            fn_names
        );
    }

    /// Contract 1.2: extract_file returns imports
    #[test]
    fn extract_file_returns_imports() {
        let fixtures_dir =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/python-project");
        let app_py = fixtures_dir.join("app.py");

        let result = extract_file(&app_py, None).unwrap();

        assert!(
            !result.imports.is_empty(),
            "Should extract imports from app.py"
        );

        // app.py imports os and subprocess
        let import_modules: Vec<&str> = result.imports.iter().map(|i| i.module.as_str()).collect();
        assert!(
            import_modules.iter().any(|m| m.contains("os")),
            "Should find 'os' import, got: {:?}",
            import_modules
        );
    }

    /// Contract 1.2: extract_file includes call graph
    #[test]
    fn extract_file_returns_call_graph() {
        let fixtures_dir =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/python-project");
        let app_py = fixtures_dir.join("app.py");

        let result = extract_file(&app_py, None).unwrap();

        // ModuleInfo should have call_graph field (it's IntraFileCallGraph, not Option)
        // The call_graph has `calls` and `called_by` HashMaps
        // Just verify it serializes properly
        let json = serde_json::to_string(&result).unwrap();
        assert!(
            json.contains("call_graph"),
            "ModuleInfo should have call_graph field in JSON"
        );
    }

    /// Contract 1.2: extract_file error on missing file
    #[test]
    fn extract_file_error_missing_file() {
        let result = extract_file(&PathBuf::from("/nonexistent/file.py"), None);
        assert!(result.is_err(), "Should error on missing file");

        let err = result.unwrap_err();
        let err_msg = err.to_string();
        assert!(
            err_msg.contains("not found") || err_msg.contains("Path"),
            "Error should mention path not found: {err_msg}"
        );
    }

    /// Contract 1.2: extract_file JSON output matches expected schema
    #[test]
    fn extract_file_json_schema() {
        let fixtures_dir =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/python-project");
        let app_py = fixtures_dir.join("app.py");

        let result = extract_file(&app_py, None).unwrap();
        let json = serde_json::to_string(&result).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        // Required top-level fields per spec
        assert!(
            value.get("file_path").is_some(),
            "Should have 'file_path' field"
        );
        assert!(
            value.get("language").is_some(),
            "Should have 'language' field"
        );
        assert!(
            value.get("imports").is_some(),
            "Should have 'imports' field"
        );
        assert!(
            value.get("functions").is_some(),
            "Should have 'functions' field"
        );
        assert!(
            value.get("classes").is_some(),
            "Should have 'classes' field"
        );
    }
}

// =============================================================================
// Contract 1.3: Imports Function - Core Tests
// =============================================================================

#[cfg(test)]
mod imports_tests {
    use super::*;
    use tldr_core::{get_imports, Language};

    /// Contract 1.3: get_imports returns ImportInfo vector
    #[test]
    fn get_imports_returns_import_info() {
        let fixtures_dir =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/python-project");
        let app_py = fixtures_dir.join("app.py");

        let result = get_imports(&app_py, Language::Python);
        assert!(result.is_ok(), "get_imports should succeed: {:?}", result);

        let imports = result.unwrap();
        assert!(!imports.is_empty(), "Should find imports in app.py");
    }

    /// Contract 1.3: ImportInfo has correct fields
    #[test]
    fn import_info_json_schema() {
        let fixtures_dir =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/python-project");
        let app_py = fixtures_dir.join("app.py");

        let imports = get_imports(&app_py, Language::Python).unwrap();
        let json = serde_json::to_string(&imports).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        // Should be an array
        assert!(value.is_array(), "Imports should be an array");

        if let Some(first) = value.as_array().and_then(|a| a.first()) {
            // Each import should have these fields
            assert!(first.get("module").is_some(), "Import should have 'module'");
            assert!(first.get("names").is_some(), "Import should have 'names'");
            assert!(
                first.get("is_from").is_some(),
                "Import should have 'is_from'"
            );
        }
    }
}

// =============================================================================
// Contract 1.4: Importers Function - Core Tests
// =============================================================================

#[cfg(test)]
mod importers_tests {
    use super::*;
    use tldr_core::{find_importers, Language};

    /// Contract 1.4: find_importers returns ImportersReport
    #[test]
    fn find_importers_returns_report() {
        let fixtures_dir =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/python-project");

        // Search for 'os' module importers in fixtures
        let result = find_importers(&fixtures_dir, "os", Language::Python);
        assert!(
            result.is_ok(),
            "find_importers should succeed: {:?}",
            result
        );

        let report = result.unwrap();
        // app.py imports os, so should find at least 1
        assert!(
            report.total >= 1,
            "Should find at least 1 importer of 'os', got: {}",
            report.total
        );
    }

    /// Contract 1.4: ImportersReport JSON schema
    #[test]
    fn importers_report_json_schema() {
        let fixtures_dir =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/python-project");

        let report = find_importers(&fixtures_dir, "os", Language::Python).unwrap();
        let json = serde_json::to_string(&report).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        // Required fields per spec
        assert!(value.get("module").is_some(), "Should have 'module' field");
        assert!(
            value.get("importers").is_some(),
            "Should have 'importers' field"
        );
        assert!(value.get("total").is_some(), "Should have 'total' field");
    }

    /// Contract 1.4: find_importers returns empty for non-imported module
    #[test]
    fn find_importers_empty_for_unknown_module() {
        let fixtures_dir =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/python-project");

        let report =
            find_importers(&fixtures_dir, "nonexistent_module_xyz", Language::Python).unwrap();

        assert_eq!(report.total, 0, "Should find 0 importers of unknown module");
    }
}

// =============================================================================
// Contract 1.5: Complexity Function - Core Tests
// =============================================================================

#[cfg(test)]
mod complexity_tests {
    use super::*;
    use tldr_core::{calculate_complexity, Language};

    /// Contract 1.5: calculate_complexity returns ComplexityMetrics
    #[test]
    fn calculate_complexity_returns_metrics() {
        let fixtures_dir =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/python-project");
        let app_py = fixtures_dir.join("app.py");

        // complex_function has high complexity (many if/else branches)
        let result = calculate_complexity(
            app_py.to_str().unwrap(),
            "complex_function",
            Language::Python,
        );

        assert!(
            result.is_ok(),
            "calculate_complexity should succeed: {:?}",
            result
        );

        let metrics = result.unwrap();
        // complex_function has many branches, cyclomatic should be > 5
        assert!(
            metrics.cyclomatic >= 5,
            "complex_function should have cyclomatic >= 5, got: {}",
            metrics.cyclomatic
        );
    }

    /// Contract 1.5: ComplexityMetrics JSON schema
    #[test]
    fn complexity_metrics_json_schema() {
        let fixtures_dir =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/python-project");
        let app_py = fixtures_dir.join("app.py");

        let metrics = calculate_complexity(
            app_py.to_str().unwrap(),
            "complex_function",
            Language::Python,
        )
        .unwrap();

        let json = serde_json::to_string(&metrics).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        // Required fields per spec
        assert!(
            value.get("function").is_some() || value.get("cyclomatic").is_some(),
            "Should have complexity fields"
        );
        assert!(
            value.get("cyclomatic").is_some(),
            "Should have 'cyclomatic' field"
        );
        assert!(
            value.get("cognitive").is_some(),
            "Should have 'cognitive' field"
        );
    }

    /// Contract 1.5: Error on function not found
    #[test]
    fn calculate_complexity_error_function_not_found() {
        let fixtures_dir =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/python-project");
        let app_py = fixtures_dir.join("app.py");

        let result = calculate_complexity(
            app_py.to_str().unwrap(),
            "nonexistent_function",
            Language::Python,
        );

        assert!(result.is_err(), "Should error when function not found");

        let err = result.unwrap_err();
        let err_msg = err.to_string();
        assert!(
            err_msg.contains("not found") || err_msg.contains("Function"),
            "Error should mention function not found: {err_msg}"
        );
    }
}
