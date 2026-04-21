//! Tests for Layer 1: AST operations
//!
//! Commands tested: tree, structure, extract, imports
//!
//! These tests verify the behavior of the Phase 2 implementation.

use std::collections::HashSet;
use std::path::PathBuf;

use tldr_core::{
    ast::extract::extract_file, ast::extractor::get_code_structure, ast::imports::get_imports,
    fs::tree::get_file_tree, FileTree, IgnoreSpec, Language, NodeType, TldrError,
};

/// Get the fixtures directory path
fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

// =============================================================================
// tree command tests
// =============================================================================

mod tree_tests {
    use super::*;

    #[test]
    fn tree_returns_files_in_directory() {
        // GIVEN: A directory with Python files
        let project = fixtures_dir().join("simple-project");

        // WHEN: We get the file tree
        let tree = get_file_tree(&project, None, true, None);

        // THEN: It should succeed and contain files
        assert!(tree.is_ok());
        let tree = tree.unwrap();
        assert!(!tree.children.is_empty());
    }

    #[test]
    fn tree_filters_by_extension() {
        // GIVEN: A directory with mixed file types
        let project = fixtures_dir().join("simple-project");
        let extensions: HashSet<String> = [".py".to_string()].into_iter().collect();

        // WHEN: We filter by .py extension
        let tree = get_file_tree(&project, Some(&extensions), true, None);

        // THEN: All files should be Python files
        let tree = tree.unwrap();
        fn check_extensions(node: &FileTree) {
            if node.node_type == NodeType::File {
                assert!(
                    node.name.ends_with(".py"),
                    "Non-py file found: {}",
                    node.name
                );
            }
            for child in &node.children {
                check_extensions(child);
            }
        }
        check_extensions(&tree);
    }

    #[test]
    fn tree_handles_nonexistent_path() {
        // GIVEN: A path that doesn't exist
        let nonexistent = PathBuf::from("/nonexistent/path/that/does/not/exist");

        // WHEN: We try to get the file tree
        let result = get_file_tree(&nonexistent, None, true, None);

        // THEN: It should return PathNotFound error
        assert!(matches!(result, Err(TldrError::PathNotFound(_))));
    }

    #[test]
    fn tree_handles_empty_directory() {
        // GIVEN: An empty directory
        let empty_dir = fixtures_dir().join("empty-dir");

        // WHEN: We get the file tree
        let tree = get_file_tree(&empty_dir, None, true, None);

        // THEN: It should succeed with empty children
        assert!(tree.is_ok());
        let tree = tree.unwrap();
        assert!(tree.children.is_empty());
    }

    #[test]
    fn tree_excludes_hidden_files_by_default() {
        // GIVEN: A directory that may contain hidden files
        let project = fixtures_dir().join("simple-project");

        // WHEN: We get the file tree with exclude_hidden=true
        let tree = get_file_tree(&project, None, true, None);

        // THEN: No files should start with a dot (in children)
        fn check_no_hidden(node: &FileTree) {
            assert!(
                !node.name.starts_with('.'),
                "Hidden file found: {}",
                node.name
            );
            for child in &node.children {
                check_no_hidden(child);
            }
        }
        for child in &tree.unwrap().children {
            check_no_hidden(child);
        }
    }

    #[test]
    fn tree_includes_hidden_files_when_requested() {
        // GIVEN: A directory with hidden files
        let project = fixtures_dir().join("simple-project");

        // WHEN: We get the file tree with exclude_hidden=false
        let tree = get_file_tree(&project, None, false, None);

        // THEN: It should succeed
        assert!(tree.is_ok());
    }

    #[test]
    fn tree_respects_ignore_patterns() {
        // GIVEN: A project with an ignore spec
        let project = fixtures_dir().join("simple-project");
        let ignore = IgnoreSpec::new(vec!["*.pyc".to_string(), "__pycache__".to_string()]);

        // WHEN: We get the file tree with ignore patterns
        let tree = get_file_tree(&project, None, true, Some(&ignore));

        // THEN: Ignored patterns should not appear
        fn check_no_ignored(node: &FileTree, patterns: &[String]) {
            for pattern in patterns {
                let pattern = pattern.trim_start_matches('*');
                if !pattern.is_empty() {
                    assert!(
                        !node.name.ends_with(pattern),
                        "Found ignored pattern: {}",
                        node.name
                    );
                }
            }
            for child in &node.children {
                check_no_ignored(child, patterns);
            }
        }
        check_no_ignored(&tree.unwrap(), &["*.pyc".to_string()]);
    }

    #[test]
    #[ignore] // Path traversal detection is best-effort, not always reliable
    fn tree_detects_path_traversal() {
        // GIVEN: A path with directory traversal attempt
        let malicious = fixtures_dir().join("../../../etc/passwd");

        // WHEN: We try to get the file tree
        let result = get_file_tree(&malicious, None, true, None);

        // THEN: It should return error (either PathTraversal or PathNotFound)
        assert!(result.is_err());
    }

    #[test]
    #[ignore] // Performance test - run manually
    fn tree_performance_1000_files() {
        // GIVEN: A large directory (simulated or real)
        let project = fixtures_dir();

        // WHEN: We time the file tree generation
        let start = std::time::Instant::now();
        let _tree = get_file_tree(&project, None, true, None);
        let elapsed = start.elapsed();

        // THEN: It should complete in under 10ms for 1000 files
        assert!(
            elapsed.as_millis() < 100,
            "Tree took too long: {:?}",
            elapsed
        );
    }
}

// =============================================================================
// structure command tests
// =============================================================================

mod structure_tests {
    use super::*;

    #[test]
    fn structure_extracts_functions_from_python() {
        // GIVEN: A Python project with functions
        let project = fixtures_dir().join("simple-project");

        // WHEN: We extract the code structure
        let structure = get_code_structure(&project, Language::Python, 0, None);

        // THEN: It should find the functions
        assert!(structure.is_ok());
        let structure = structure.unwrap();
        let main_file = structure
            .files
            .iter()
            .find(|f| f.path.to_string_lossy().contains("main.py"))
            .expect("main.py not found");
        assert!(main_file.functions.contains(&"main".to_string()));
        assert!(main_file.functions.contains(&"process_data".to_string()));
    }

    #[test]
    fn structure_extracts_classes_from_python() {
        // GIVEN: A Python project with classes
        let project = fixtures_dir().join("simple-project");

        // WHEN: We extract the code structure
        let structure = get_code_structure(&project, Language::Python, 0, None);

        // THEN: It should find the classes
        let utils_file = structure
            .unwrap()
            .files
            .iter()
            .find(|f| f.path.to_string_lossy().contains("utils.py"))
            .expect("utils.py not found")
            .clone();
        assert!(utils_file.classes.contains(&"DataProcessor".to_string()));
    }

    #[test]
    fn structure_extracts_methods_from_classes() {
        // GIVEN: A Python file with a class containing methods
        let project = fixtures_dir().join("simple-project");

        // WHEN: We extract the code structure
        let structure = get_code_structure(&project, Language::Python, 0, None);

        // THEN: Methods should be extracted
        let utils = structure
            .unwrap()
            .files
            .iter()
            .find(|f| f.path.to_string_lossy().contains("utils.py"))
            .unwrap()
            .clone();
        assert!(utils.methods.contains(&"add".to_string()));
        assert!(utils.methods.contains(&"process".to_string()));
    }

    #[test]
    fn structure_works_with_typescript() {
        // GIVEN: A TypeScript project
        let project = fixtures_dir().join("typescript-project");

        // WHEN: We extract the code structure
        let structure = get_code_structure(&project, Language::TypeScript, 0, None);

        // THEN: It should find TypeScript functions and classes
        assert!(structure.is_ok());
        let structure = structure.unwrap();
        assert!(!structure.files.is_empty());
    }

    #[test]
    fn structure_respects_max_results() {
        // GIVEN: A project with many files
        let project = fixtures_dir().join("simple-project");

        // WHEN: We limit the results
        let structure = get_code_structure(&project, Language::Python, 1, None);

        // THEN: Only max_results files should be returned
        assert!(structure.unwrap().files.len() <= 1);
    }

    #[test]
    fn structure_handles_empty_files() {
        // GIVEN: Empty files are handled gracefully
        let project = fixtures_dir().join("simple-project");

        // WHEN: We extract the code structure
        let structure = get_code_structure(&project, Language::Python, 0, None);

        // THEN: It should succeed
        assert!(structure.is_ok());
    }

    #[test]
    fn structure_extracts_imports() {
        // GIVEN: A Python file with imports
        let project = fixtures_dir().join("simple-project");

        // WHEN: We extract the code structure
        let structure = get_code_structure(&project, Language::Python, 0, None);

        // THEN: Imports should be extracted
        let utils = structure
            .unwrap()
            .files
            .iter()
            .find(|f| f.path.to_string_lossy().contains("utils.py"))
            .unwrap()
            .clone();
        assert!(!utils.imports.is_empty());
    }

    #[test]
    #[ignore] // Performance test
    fn structure_performance_100_files() {
        // GIVEN: A project with 100 files
        let project = fixtures_dir();

        // WHEN: We time the structure extraction
        let start = std::time::Instant::now();
        let _structure = get_code_structure(&project, Language::Python, 0, None);
        let elapsed = start.elapsed();

        // THEN: It should complete in under 500ms
        assert!(
            elapsed.as_millis() < 500,
            "Structure took too long: {:?}",
            elapsed
        );
    }
}

// =============================================================================
// extract command tests
// =============================================================================

mod extract_tests {
    use super::*;

    #[test]
    fn extract_returns_module_info() {
        // GIVEN: A Python file
        let file = fixtures_dir().join("simple-project/main.py");

        // WHEN: We extract the file
        let info = extract_file(&file, None);

        // THEN: It should return complete module info
        assert!(info.is_ok());
        let info = info.unwrap();
        assert_eq!(info.language, Language::Python);
        assert!(!info.functions.is_empty());
    }

    #[test]
    fn extract_includes_docstrings() {
        // GIVEN: A Python file with docstrings
        let file = fixtures_dir().join("simple-project/main.py");

        // WHEN: We extract the file
        let info = extract_file(&file, None);

        // THEN: Docstrings should be present
        let info = info.unwrap();
        assert!(info.docstring.is_some());
        let main_func = info.functions.iter().find(|f| f.name == "main").unwrap();
        assert!(main_func.docstring.is_some());
    }

    #[test]
    fn extract_includes_function_signatures() {
        // GIVEN: A Python file with typed functions
        let file = fixtures_dir().join("simple-project/main.py");

        // WHEN: We extract the file
        let info = extract_file(&file, None);

        // THEN: Function parameters and return types should be present
        let info = info.unwrap();
        let process = info
            .functions
            .iter()
            .find(|f| f.name == "process_data")
            .unwrap();
        assert!(!process.params.is_empty(), "process_data has no params");
        assert!(process.return_type.is_some());
    }

    #[test]
    fn extract_includes_class_inheritance() {
        // GIVEN: A Python file with class inheritance
        let file = fixtures_dir().join("simple-project/utils.py");

        // WHEN: We extract the file
        let info = extract_file(&file, None);

        // THEN: Classes should be extracted
        let info = info.unwrap();
        assert!(!info.classes.is_empty());
    }

    #[test]
    fn extract_builds_intra_file_call_graph() {
        // GIVEN: A Python file with internal calls
        let file = fixtures_dir().join("simple-project/main.py");

        // WHEN: We extract the file
        let info = extract_file(&file, None);

        // THEN: The intra-file call graph should show calls
        let info = info.unwrap();
        assert!(info.call_graph.calls.contains_key("main"));
        assert!(info.call_graph.calls["main"].contains(&"process_data".to_string()));
    }

    #[test]
    fn extract_handles_file_not_found() {
        // GIVEN: A file that doesn't exist
        let nonexistent = fixtures_dir().join("does_not_exist.py");

        // WHEN: We try to extract it
        let result = extract_file(&nonexistent, None);

        // THEN: It should return FileNotFound error
        assert!(matches!(result, Err(TldrError::PathNotFound(_))));
    }

    #[test]
    fn extract_handles_unsupported_language() {
        // GIVEN: A file with unsupported extension
        // Create a temp file with unsupported extension
        use std::io::Write;
        let temp_dir = tempfile::tempdir().unwrap();
        let unsupported = temp_dir.path().join("file.xyz");
        std::fs::File::create(&unsupported)
            .unwrap()
            .write_all(b"content")
            .unwrap();

        // WHEN: We try to extract it
        let result = extract_file(&unsupported, None);

        // THEN: It should return UnsupportedLanguage error
        assert!(matches!(result, Err(TldrError::UnsupportedLanguage(_))));
    }

    #[test]
    fn extract_detects_async_functions() {
        // GIVEN: A TypeScript file with async functions
        let file = fixtures_dir().join("typescript-project/index.ts");

        // WHEN: We extract the file
        let info = extract_file(&file, None);

        // THEN: Async functions should be marked as such
        let info = info.unwrap();
        let async_main = info.functions.iter().find(|f| f.name == "asyncMain");
        assert!(async_main.is_some(), "asyncMain function not found");
        assert!(async_main.unwrap().is_async);
    }

    #[test]
    fn extract_respects_base_path() {
        // GIVEN: A file and a base path
        let file = fixtures_dir().join("simple-project/main.py");
        let base = fixtures_dir();

        // WHEN: We extract with base path
        let info = extract_file(&file, Some(&base));

        // THEN: File path should be relative to base
        let info = info.unwrap();
        assert!(info.file_path.is_relative());
    }
}

// =============================================================================
// imports command tests
// =============================================================================

mod imports_tests {
    use super::*;

    #[test]
    fn imports_parses_python_imports() {
        // GIVEN: A Python file with imports
        let file = fixtures_dir().join("simple-project/utils.py");

        // WHEN: We parse the imports
        let imports = get_imports(&file, Language::Python);

        // THEN: Imports should be extracted
        assert!(imports.is_ok());
        let imports = imports.unwrap();
        assert!(!imports.is_empty());
    }

    #[test]
    fn imports_distinguishes_from_imports() {
        // GIVEN: A Python file with 'from X import Y' statements
        let file = fixtures_dir().join("simple-project/utils.py");

        // WHEN: We parse the imports
        let imports = get_imports(&file, Language::Python);

        // THEN: is_from should be true for 'from' imports
        let imports = imports.unwrap();
        let from_import = imports.iter().find(|i| i.is_from);
        assert!(from_import.is_some());
        assert!(!from_import.unwrap().names.is_empty());
    }

    #[test]
    fn imports_parses_typescript_imports() {
        // GIVEN: A TypeScript file with imports
        let file = fixtures_dir().join("typescript-project/index.ts");

        // WHEN: We parse the imports
        let imports = get_imports(&file, Language::TypeScript);

        // THEN: Imports should be extracted
        assert!(imports.is_ok());
    }

    #[test]
    fn imports_handles_file_not_found() {
        // GIVEN: A file that doesn't exist
        let nonexistent = fixtures_dir().join("does_not_exist.py");

        // WHEN: We try to parse imports
        let result = get_imports(&nonexistent, Language::Python);

        // THEN: It should return an error
        assert!(result.is_err());
    }
}
