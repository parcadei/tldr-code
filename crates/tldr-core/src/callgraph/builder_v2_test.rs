//! Tests for Phase 14: Builder V2
//!
//! These tests verify the V2 call graph builder implementation per
//! `migration/spec/phases-14-16-spec.md` Section 14.
//!
//! All tests are designed to fail initially (red phase of TDD) since
//! the implementation does not exist yet. They will pass once the
//! `builder_v2` module is implemented.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

// Types that will be created in builder_v2.rs
use super::builder_v2::{build_project_call_graph_v2, BuildConfig, BuildError};
use super::cross_file_types::CallType;

// =============================================================================
// Test Fixtures
// =============================================================================

/// Creates a temporary directory with Python test files.
fn create_python_project() -> TempDir {
    let dir = TempDir::new().unwrap();

    // main.py - calls helper.process()
    let main_py = r#"
from helper import process

def main():
    process()

if __name__ == "__main__":
    main()
"#;
    fs::write(dir.path().join("main.py"), main_py).unwrap();

    // helper.py - defines process()
    let helper_py = r#"
def process():
    print("processing")
"#;
    fs::write(dir.path().join("helper.py"), helper_py).unwrap();

    dir
}

/// Creates a project with local (intra-file) calls.
fn create_intra_file_project() -> TempDir {
    let dir = TempDir::new().unwrap();

    let code = r#"
def foo():
    bar()

def bar():
    baz()

def baz():
    pass
"#;
    fs::write(dir.path().join("module.py"), code).unwrap();

    dir
}

/// Creates a project with method calls requiring type resolution.
fn create_method_call_project() -> TempDir {
    let dir = TempDir::new().unwrap();

    // models.py
    let models = r#"
class User:
    def save(self):
        pass

    def delete(self):
        pass
"#;
    fs::write(dir.path().join("models.py"), models).unwrap();

    // service.py
    let service = r#"
from models import User

def create_user():
    user = User()
    user.save()

def remove_user(user: User):
    user.delete()
"#;
    fs::write(dir.path().join("service.py"), service).unwrap();

    dir
}

/// Creates a large project for memory/performance testing.
fn create_large_project(num_files: usize) -> TempDir {
    let dir = TempDir::new().unwrap();

    for i in 0..num_files {
        let code = format!(
            r#"
def func_{i}():
    pass

def caller_{i}():
    func_{i}()
"#,
            i = i
        );
        fs::write(dir.path().join(format!("module_{}.py", i)), code).unwrap();
    }

    dir
}

/// Creates a project with symlinks that could cause cycles.
fn create_symlink_project() -> TempDir {
    let dir = TempDir::new().unwrap();

    // Create a subdirectory
    let subdir = dir.path().join("subdir");
    fs::create_dir(&subdir).unwrap();

    // Create a file
    let code = "def foo(): pass";
    fs::write(subdir.join("module.py"), code).unwrap();

    // Create a symlink that points back to parent (potential cycle)
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let _ = symlink(dir.path(), subdir.join("parent_link"));
    }

    dir
}

/// Creates a project with non-UTF8 file (using Latin-1 encoding).
fn create_non_utf8_project() -> TempDir {
    let dir = TempDir::new().unwrap();

    // Write a file with Latin-1 encoded content (invalid UTF-8)
    // This simulates legacy code files with non-UTF8 encoding
    let latin1_bytes: Vec<u8> = vec![
        0x64, 0x65, 0x66, 0x20, 0x66, 0x6f, 0x6f, 0x28, 0x29, 0x3a, 0x0a, // def foo():
        0x20, 0x20, 0x20, 0x20, 0x70, 0x61, 0x73, 0x73, 0x0a, // pass
        0x0a, 0x23, 0x20, 0xe9, 0xe8, 0xe0, // # with some Latin-1 chars
    ];
    fs::write(dir.path().join("legacy.py"), latin1_bytes).unwrap();

    dir
}

// =============================================================================
// Phase 14.2: Main Entry Point Tests
// =============================================================================

mod main_entry_point {
    use super::*;

    /// Test: Build call graph for an empty project.
    /// Spec: "Empty Projects - Return empty CallGraphIR with no files"
    #[test]
    fn test_build_empty_project() {
        let dir = TempDir::new().unwrap();
        let config = BuildConfig {
            language: "python".to_string(),
            ..Default::default()
        };

        let result = build_project_call_graph_v2(dir.path(), config);

        assert!(result.is_ok(), "Empty project should succeed");
        let ir = result.unwrap();
        assert_eq!(ir.file_count(), 0, "Empty project should have 0 files");
        assert_eq!(
            ir.function_count(),
            0,
            "Empty project should have 0 functions"
        );
    }

    /// Test: Build call graph for a single file.
    #[test]
    fn test_build_single_file() {
        let dir = create_intra_file_project();
        let config = BuildConfig {
            language: "python".to_string(),
            ..Default::default()
        };

        let result = build_project_call_graph_v2(dir.path(), config);

        assert!(result.is_ok(), "Single file project should succeed");
        let ir = result.unwrap();
        assert_eq!(ir.file_count(), 1, "Should have 1 file");
        assert!(
            ir.function_count() >= 3,
            "Should have at least 3 functions (foo, bar, baz)"
        );
    }

    /// Test: Build call graph with imports across files.
    #[test]
    fn test_build_with_imports() {
        let dir = create_python_project();
        let config = BuildConfig {
            language: "python".to_string(),
            ..Default::default()
        };

        let result = build_project_call_graph_v2(dir.path(), config);

        assert!(result.is_ok(), "Project with imports should succeed");
        let ir = result.unwrap();
        assert_eq!(
            ir.file_count(),
            2,
            "Should have 2 files (main.py, helper.py)"
        );
    }

    /// Test: Cross-file call resolution.
    /// Spec Section 14.4: "For each call site, resolve callee via imports"
    #[test]
    fn test_build_cross_file_calls() {
        let dir = create_python_project();
        let config = BuildConfig {
            language: "python".to_string(),
            ..Default::default()
        };

        let ir = build_project_call_graph_v2(dir.path(), config).unwrap();

        // main.py calls helper.process()
        // We should have an edge from main.py:main -> helper.py:process
        let main_file = ir.get_file("main.py");
        assert!(main_file.is_some(), "main.py should be in IR");

        let main_ir = main_file.unwrap();
        let calls: Vec<_> = main_ir
            .calls
            .values()
            .flatten()
            .filter(|c| c.target == "process")
            .collect();

        assert!(!calls.is_empty(), "Should have call to 'process'");
    }

    /// Test: Method resolution with type information.
    /// Spec Section 14.6: "Type-aware method resolution"
    #[test]
    fn test_build_method_resolution() {
        let dir = create_method_call_project();
        let config = BuildConfig {
            language: "python".to_string(),
            use_type_resolution: true,
            ..Default::default()
        };

        let ir = build_project_call_graph_v2(dir.path(), config).unwrap();

        // service.py calls user.save() and user.delete()
        let service_file = ir.get_file("service.py");
        assert!(service_file.is_some(), "service.py should be in IR");

        let service_ir = service_file.unwrap();

        // With type resolution enabled, we should resolve user.save() -> User.save
        let method_calls: Vec<_> = service_ir
            .calls
            .values()
            .flatten()
            .filter(|c| c.call_type == CallType::Method)
            .collect();

        assert!(
            method_calls.len() >= 2,
            "Should have at least 2 method calls"
        );
    }
}

// =============================================================================
// Phase 14.5: Parallel Processing Tests
// =============================================================================

mod parallel_processing {
    use super::*;

    /// Test: Parallel processing produces deterministic results.
    /// Spec Section 14.11.2: "Deterministic - Same input -> same output"
    #[test]
    fn test_parallel_processing() {
        let dir = create_large_project(100);
        let config = BuildConfig {
            language: "python".to_string(),
            parallelism: 4, // Force 4 threads
            ..Default::default()
        };

        // Run twice and compare
        let ir1 = build_project_call_graph_v2(dir.path(), config.clone()).unwrap();
        let ir2 = build_project_call_graph_v2(dir.path(), config).unwrap();

        // Results must be identical
        assert_eq!(
            ir1.file_count(),
            ir2.file_count(),
            "File counts should match"
        );
        assert_eq!(
            ir1.function_count(),
            ir2.function_count(),
            "Function counts should match"
        );

        // Edge sets should be identical (order doesn't matter)
        let edges1: HashSet<String> = ir1
            .files
            .values()
            .flat_map(|f| f.calls.values().flatten())
            .map(|c| format!("{}:{}", c.caller, c.target))
            .collect();
        let edges2: HashSet<String> = ir2
            .files
            .values()
            .flat_map(|f| f.calls.values().flatten())
            .map(|c| format!("{}:{}", c.caller, c.target))
            .collect();

        assert_eq!(edges1, edges2, "Edge sets should be identical across runs");
    }

    /// Test: Automatic parallelism detection (0 = auto).
    #[test]
    fn test_parallelism_auto_detect() {
        let dir = create_intra_file_project();
        let config = BuildConfig {
            language: "python".to_string(),
            parallelism: 0, // Auto-detect
            ..Default::default()
        };

        let result = build_project_call_graph_v2(dir.path(), config);
        assert!(result.is_ok(), "Auto parallelism should work");
    }
}

// =============================================================================
// Phase 14.8: Memory Management Tests
// =============================================================================

mod memory_management {
    use super::*;

    /// Test: Memory usage stays bounded for large codebases.
    /// Spec Section 14.8: "Peak memory < 500MB with interning"
    /// Note: This is a heuristic test - we check relative memory, not absolute.
    #[test]
    fn test_memory_bounded() {
        let dir = create_large_project(500);
        let config = BuildConfig {
            language: "python".to_string(),
            ..Default::default()
        };

        // This test verifies the build completes without memory issues
        // A proper memory test would need external instrumentation
        let result = build_project_call_graph_v2(dir.path(), config);
        assert!(result.is_ok(), "Large project should complete without OOM");

        let ir = result.unwrap();
        assert_eq!(ir.file_count(), 500, "Should process all 500 files");
    }

    /// Test: String interning deduplicates paths.
    /// Spec Section 14.8: "String Interning - Expected reduction: 1.9GB -> ~80MB"
    #[test]
    fn test_string_interning_dedup() {
        let dir = create_large_project(100);
        let config = BuildConfig {
            language: "python".to_string(),
            ..Default::default()
        };

        let ir = build_project_call_graph_v2(dir.path(), config).unwrap();

        // Verify we have many files but string storage is efficient
        assert_eq!(ir.file_count(), 100);

        // The interner should show deduplication
        // This would require access to interner stats - placeholder assertion
        // In real impl: assert!(interner.stats().dedup_ratio() > 0.0);
    }
}

// =============================================================================
// Phase 14: Edge Cases
// =============================================================================

mod edge_cases {
    use super::*;

    /// Test: Graceful handling of non-UTF8 files.
    /// Spec: "Non-UTF8 Files - Try UTF-8, fallback to latin-1"
    #[test]
    fn test_non_utf8_fallback() {
        let dir = create_non_utf8_project();
        let config = BuildConfig {
            language: "python".to_string(),
            ..Default::default()
        };

        let result = build_project_call_graph_v2(dir.path(), config);

        // Should not fail - should either parse or skip with warning
        assert!(result.is_ok(), "Non-UTF8 file should not cause failure");
    }

    /// Test: Symlink cycle detection.
    /// Spec: "Symlink Cycles - Break cycle by not following symlink if target already visited"
    #[test]
    #[cfg(unix)]
    fn test_symlink_cycle_handling() {
        let dir = create_symlink_project();
        let config = BuildConfig {
            language: "python".to_string(),
            ..Default::default()
        };

        let result = build_project_call_graph_v2(dir.path(), config);

        // Should complete without infinite loop
        assert!(
            result.is_ok(),
            "Symlink cycles should not cause infinite loop"
        );
    }

    /// Test: Project root validation.
    /// Spec Section 14.10: "RootNotFound if root doesn't exist"
    #[test]
    fn test_root_not_found() {
        let config = BuildConfig {
            language: "python".to_string(),
            ..Default::default()
        };

        let result = build_project_call_graph_v2(Path::new("/nonexistent/path"), config);

        assert!(result.is_err(), "Nonexistent root should fail");
        assert!(matches!(result.unwrap_err(), BuildError::RootNotFound(_)));
    }

    /// Test: Unsupported language handling.
    /// Spec Section 14.10: "UnsupportedLanguage if language not in registry"
    #[test]
    fn test_unsupported_language() {
        let dir = TempDir::new().unwrap();
        let config = BuildConfig {
            language: "brainfuck".to_string(), // Not supported
            ..Default::default()
        };

        let result = build_project_call_graph_v2(dir.path(), config);

        assert!(result.is_err(), "Unsupported language should fail");
        assert!(matches!(
            result.unwrap_err(),
            BuildError::UnsupportedLanguage(_)
        ));
    }
}

// =============================================================================
// Phase 14.3: BuildConfig Tests
// =============================================================================

mod build_config {
    use super::*;

    /// Test: BuildConfig default values.
    #[test]
    fn test_build_config_defaults() {
        let config = BuildConfig::default();

        assert!(config.language.is_empty() || config.language == "python");
        assert!(!config.use_workspace_config);
        assert!(config.workspace_roots.is_empty());
        assert!(!config.use_type_resolution);
        assert!(config.respect_ignore);
        assert_eq!(config.parallelism, 0); // Auto-detect
        assert!(!config.verbose);
    }

    /// Test: Workspace config filtering.
    /// Spec Section 14.4: "scan_project(root, config.language, config)"
    #[test]
    fn test_workspace_config_filtering() {
        let dir = TempDir::new().unwrap();

        // Create multi-package structure
        let pkg1 = dir.path().join("pkg1");
        let pkg2 = dir.path().join("pkg2");
        fs::create_dir(&pkg1).unwrap();
        fs::create_dir(&pkg2).unwrap();

        fs::write(pkg1.join("module.py"), "def foo(): pass").unwrap();
        fs::write(pkg2.join("module.py"), "def bar(): pass").unwrap();

        // Without workspace config - should scan all
        let config_all = BuildConfig {
            language: "python".to_string(),
            use_workspace_config: false,
            ..Default::default()
        };

        let ir_all = build_project_call_graph_v2(dir.path(), config_all).unwrap();
        assert_eq!(
            ir_all.file_count(),
            2,
            "Should find both packages without filtering"
        );

        // With workspace config - should only scan pkg1
        let config_filtered = BuildConfig {
            language: "python".to_string(),
            use_workspace_config: true,
            workspace_roots: vec![PathBuf::from("pkg1")],
            ..Default::default()
        };

        let ir_filtered = build_project_call_graph_v2(dir.path(), config_filtered).unwrap();
        assert_eq!(ir_filtered.file_count(), 1, "Should only scan pkg1");
        assert!(ir_filtered.get_file("pkg1/module.py").is_some());
        assert!(ir_filtered.get_file("pkg2/module.py").is_none());
    }

    /// Test: Respect .tldrignore patterns.
    #[test]
    fn test_respect_ignore_patterns() {
        let dir = TempDir::new().unwrap();

        // Create files
        fs::write(dir.path().join("included.py"), "def foo(): pass").unwrap();
        fs::write(dir.path().join("excluded.py"), "def bar(): pass").unwrap();

        // Create .tldrignore
        fs::write(dir.path().join(".tldrignore"), "excluded.py").unwrap();

        let config = BuildConfig {
            language: "python".to_string(),
            respect_ignore: true,
            ..Default::default()
        };

        let ir = build_project_call_graph_v2(dir.path(), config).unwrap();

        assert!(
            ir.get_file("included.py").is_some(),
            "included.py should be present"
        );
        assert!(
            ir.get_file("excluded.py").is_none(),
            "excluded.py should be ignored"
        );
    }
}
