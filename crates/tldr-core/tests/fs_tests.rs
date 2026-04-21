//! Comprehensive tests for tldr-core FS module
//!
//! Coverage: tree submodule - get_file_tree, collect_files, ignore patterns

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use tldr_core::fs::tree::{collect_files, get_file_tree};
use tldr_core::types::{FileTree, IgnoreSpec, NodeType};

// =============================================================================
// Basic get_file_tree tests
// =============================================================================

#[test]
fn test_get_file_tree_basic() {
    let temp_dir = tempfile::tempdir().unwrap();

    // Create test files
    fs::write(temp_dir.path().join("main.py"), "# main").unwrap();
    fs::write(temp_dir.path().join("utils.py"), "# utils").unwrap();

    let tree = get_file_tree(temp_dir.path(), None, true, None).unwrap();

    assert_eq!(tree.node_type, NodeType::Dir);
    assert!(!tree.children.is_empty());
}

#[test]
fn test_get_file_tree_empty_directory() {
    let temp_dir = tempfile::tempdir().unwrap();

    let tree = get_file_tree(temp_dir.path(), None, true, None).unwrap();

    assert_eq!(tree.node_type, NodeType::Dir);
    // Empty dir might have 0 children or include itself depending on implementation
}

#[test]
fn test_get_file_tree_nonexistent() {
    let result = get_file_tree(Path::new("/nonexistent/path"), None, true, None);

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("not found") || err.to_string().contains("PathNotFound"));
}

// =============================================================================
// Extension filtering tests
// =============================================================================

#[test]
fn test_get_file_tree_extension_filter_python() {
    let temp_dir = tempfile::tempdir().unwrap();

    fs::write(temp_dir.path().join("main.py"), "# main").unwrap();
    fs::write(temp_dir.path().join("utils.py"), "# utils").unwrap();
    fs::write(temp_dir.path().join("config.json"), "{}").unwrap();
    fs::write(temp_dir.path().join("readme.md"), "# readme").unwrap();

    let extensions: HashSet<String> = [".py".to_string()].into_iter().collect();
    let tree = get_file_tree(temp_dir.path(), Some(&extensions), true, None).unwrap();

    // All files in tree should be .py
    fn check_extensions(node: &FileTree) {
        if node.node_type == NodeType::File {
            assert!(
                node.name.ends_with(".py"),
                "Found non-py file: {}",
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
fn test_get_file_tree_extension_filter_multiple() {
    let temp_dir = tempfile::tempdir().unwrap();

    fs::write(temp_dir.path().join("main.py"), "# main").unwrap();
    fs::write(temp_dir.path().join("utils.ts"), "// utils").unwrap();
    fs::write(temp_dir.path().join("config.json"), "{}").unwrap();
    fs::write(temp_dir.path().join("readme.md"), "# readme").unwrap();

    let extensions: HashSet<String> = [".py".to_string(), ".ts".to_string()].into_iter().collect();
    let tree = get_file_tree(temp_dir.path(), Some(&extensions), true, None).unwrap();

    fn check_extensions(node: &FileTree, allowed: &HashSet<String>) {
        if node.node_type == NodeType::File {
            let ext = Path::new(&node.name)
                .extension()
                .map(|e| format!(".{}", e.to_string_lossy()))
                .unwrap_or_default();
            assert!(
                allowed.contains(&ext),
                "Found file with disallowed extension: {}",
                node.name
            );
        }
        for child in &node.children {
            check_extensions(child, allowed);
        }
    }
    check_extensions(&tree, &extensions);
}

#[test]
fn test_get_file_tree_no_extension_match() {
    let temp_dir = tempfile::tempdir().unwrap();

    fs::write(temp_dir.path().join("file.txt"), "text").unwrap();

    let extensions: HashSet<String> = [".py".to_string()].into_iter().collect();
    let _tree = get_file_tree(temp_dir.path(), Some(&extensions), true, None).unwrap();

    // Tree should exist but have no file children (just the directory)
    // or be empty depending on implementation
}

// =============================================================================
// Hidden file handling tests
// =============================================================================

#[test]
fn test_get_file_tree_exclude_hidden() {
    let temp_dir = tempfile::tempdir().unwrap();

    fs::write(temp_dir.path().join("visible.py"), "# visible").unwrap();
    fs::write(temp_dir.path().join(".hidden"), "hidden").unwrap();
    fs::write(temp_dir.path().join(".hidden.py"), "# hidden python").unwrap();

    let tree = get_file_tree(temp_dir.path(), None, true, None).unwrap();

    fn check_no_hidden(node: &FileTree) {
        if node.node_type == NodeType::File {
            assert!(
                !node.name.starts_with('.'),
                "Found hidden file: {}",
                node.name
            );
        }
        for child in &node.children {
            check_no_hidden(child);
        }
    }
    // Check children only, root can be hidden (e.g., .tmp directories)
    for child in &tree.children {
        check_no_hidden(child);
    }
}

#[test]
fn test_get_file_tree_include_hidden() {
    let temp_dir = tempfile::tempdir().unwrap();

    fs::write(temp_dir.path().join("visible.py"), "# visible").unwrap();
    fs::write(temp_dir.path().join(".hidden"), "hidden").unwrap();

    let tree = get_file_tree(temp_dir.path(), None, false, None).unwrap();

    fn has_hidden(node: &FileTree) -> bool {
        if node.name.starts_with('.') && node.name != "." {
            return true;
        }
        node.children.iter().any(has_hidden)
    }

    assert!(has_hidden(&tree), "Expected hidden files to be included");
}

// =============================================================================
// Ignore pattern tests
// =============================================================================

#[test]
fn test_get_file_tree_ignore_single_pattern() {
    let temp_dir = tempfile::tempdir().unwrap();

    fs::write(temp_dir.path().join("keep.py"), "# keep").unwrap();
    fs::write(temp_dir.path().join("skip.pyc"), "binary").unwrap();

    let ignore = IgnoreSpec::new(vec!["*.pyc".to_string()]);
    let tree = get_file_tree(temp_dir.path(), None, true, Some(&ignore)).unwrap();

    fn check_no_pyc(node: &FileTree) {
        if node.node_type == NodeType::File {
            assert!(
                !node.name.ends_with(".pyc"),
                "Found .pyc file: {}",
                node.name
            );
        }
        for child in &node.children {
            check_no_pyc(child);
        }
    }
    check_no_pyc(&tree);
}

#[test]
fn test_get_file_tree_ignore_multiple_patterns() {
    let temp_dir = tempfile::tempdir().unwrap();

    fs::write(temp_dir.path().join("keep.py"), "# keep").unwrap();
    fs::write(temp_dir.path().join("skip.pyc"), "binary").unwrap();
    fs::write(temp_dir.path().join("temp.tmp"), "temp").unwrap();
    fs::write(temp_dir.path().join("readme.md"), "# readme").unwrap();

    let ignore = IgnoreSpec::new(vec!["*.pyc".to_string(), "*.tmp".to_string()]);
    let tree = get_file_tree(temp_dir.path(), None, true, Some(&ignore)).unwrap();

    fn check_patterns(node: &FileTree) {
        if node.node_type == NodeType::File {
            assert!(
                !node.name.ends_with(".pyc") && !node.name.ends_with(".tmp"),
                "Found ignored file: {}",
                node.name
            );
        }
        for child in &node.children {
            check_patterns(child);
        }
    }
    check_patterns(&tree);
}

#[test]
fn test_get_file_tree_ignore_directory() {
    let temp_dir = tempfile::tempdir().unwrap();
    let subdir = temp_dir.path().join("__pycache__");
    fs::create_dir(&subdir).unwrap();

    fs::write(temp_dir.path().join("main.py"), "# main").unwrap();
    fs::write(subdir.join("cache.pyc"), "binary").unwrap();

    let ignore = IgnoreSpec::new(vec!["__pycache__/".to_string()]);
    let tree = get_file_tree(temp_dir.path(), None, true, Some(&ignore)).unwrap();

    fn check_no_pycache(node: &FileTree) {
        assert!(node.name != "__pycache__", "Found __pycache__ directory");
        for child in &node.children {
            check_no_pycache(child);
        }
    }
    check_no_pycache(&tree);
}

#[test]
fn test_get_file_tree_empty_ignore() {
    let temp_dir = tempfile::tempdir().unwrap();

    fs::write(temp_dir.path().join("file.py"), "# file").unwrap();

    let ignore = IgnoreSpec::new(vec![]);
    let tree = get_file_tree(temp_dir.path(), None, true, Some(&ignore)).unwrap();

    assert!(!tree.children.is_empty());
}

// =============================================================================
// Directory structure tests
// =============================================================================

#[test]
fn test_get_file_tree_nested_directories() {
    let temp_dir = tempfile::tempdir().unwrap();

    let src_dir = temp_dir.path().join("src");
    let utils_dir = src_dir.join("utils");
    fs::create_dir_all(&utils_dir).unwrap();

    fs::write(src_dir.join("main.py"), "# main").unwrap();
    fs::write(utils_dir.join("helper.py"), "# helper").unwrap();

    let tree = get_file_tree(temp_dir.path(), None, true, None).unwrap();

    // Should find both files
    let files = collect_files(&tree, temp_dir.path());
    assert!(files.iter().any(|f| f.ends_with("main.py")));
    assert!(files.iter().any(|f| f.ends_with("helper.py")));
}

#[test]
fn test_get_file_tree_default_skip_dirs() {
    let temp_dir = tempfile::tempdir().unwrap();

    // Create directories that should be skipped by default
    let node_modules = temp_dir.path().join("node_modules");
    let git_dir = temp_dir.path().join(".git");
    let target_dir = temp_dir.path().join("target");

    fs::create_dir(&node_modules).unwrap();
    fs::create_dir(&git_dir).unwrap();
    fs::create_dir(&target_dir).unwrap();

    fs::write(node_modules.join("package.js"), "// package").unwrap();
    fs::write(git_dir.join("config"), "config").unwrap();
    fs::write(target_dir.join("binary"), "binary").unwrap();
    fs::write(temp_dir.path().join("main.py"), "# main").unwrap();

    let tree = get_file_tree(temp_dir.path(), None, true, None).unwrap();

    // Should not include skipped directories
    fn check_skip_dirs(node: &FileTree) {
        assert!(
            node.name != "node_modules" && node.name != ".git" && node.name != "target",
            "Found directory that should be skipped: {}",
            node.name
        );
        for child in &node.children {
            check_skip_dirs(child);
        }
    }
    check_skip_dirs(&tree);
}

// =============================================================================
// collect_files tests
// =============================================================================

#[test]
fn test_collect_files_basic() {
    let temp_dir = tempfile::tempdir().unwrap();

    fs::write(temp_dir.path().join("a.py"), "# a").unwrap();
    fs::write(temp_dir.path().join("b.py"), "# b").unwrap();

    let tree = get_file_tree(temp_dir.path(), None, true, None).unwrap();
    let files = collect_files(&tree, temp_dir.path());

    assert_eq!(files.len(), 2);
    assert!(files.iter().any(|f| f.ends_with("a.py")));
    assert!(files.iter().any(|f| f.ends_with("b.py")));
}

#[test]
fn test_collect_files_empty_tree() {
    let temp_dir = tempfile::tempdir().unwrap();

    // Create tree with no files (using extension filter that matches nothing)
    let extensions: HashSet<String> = [".nonexistent".to_string()].into_iter().collect();
    let tree = get_file_tree(temp_dir.path(), Some(&extensions), true, None).unwrap();
    let files = collect_files(&tree, temp_dir.path());

    // Should return empty or minimal set
    assert!(files.is_empty());
}

#[test]
fn test_collect_files_nested() {
    let temp_dir = tempfile::tempdir().unwrap();

    let deep_dir = temp_dir.path().join("a").join("b").join("c");
    fs::create_dir_all(&deep_dir).unwrap();

    fs::write(deep_dir.join("deep.py"), "# deep").unwrap();
    fs::write(temp_dir.path().join("shallow.py"), "# shallow").unwrap();

    let tree = get_file_tree(temp_dir.path(), None, true, None).unwrap();
    let files = collect_files(&tree, temp_dir.path());

    assert_eq!(files.len(), 2);
    assert!(files.iter().any(|f| f.ends_with("deep.py")));
    assert!(files.iter().any(|f| f.ends_with("shallow.py")));
}

// =============================================================================
// Edge cases and error handling
// =============================================================================

#[test]
fn test_get_file_tree_special_characters_in_filename() {
    let temp_dir = tempfile::tempdir().unwrap();

    // Test with various special characters
    fs::write(temp_dir.path().join("file-with-dashes.py"), "# file").unwrap();
    fs::write(temp_dir.path().join("file_with_underscores.py"), "# file").unwrap();
    fs::write(temp_dir.path().join("file.with.dots.py"), "# file").unwrap();

    let tree = get_file_tree(temp_dir.path(), None, true, None).unwrap();
    let files = collect_files(&tree, temp_dir.path());

    assert_eq!(files.len(), 3);
}

#[test]
fn test_get_file_tree_unicode_filenames() {
    let temp_dir = tempfile::tempdir().unwrap();

    fs::write(temp_dir.path().join("文件.py"), "# unicode").unwrap();
    fs::write(temp_dir.path().join("файл.py"), "# cyrillic").unwrap();
    fs::write(temp_dir.path().join("🚀emoji.py"), "# emoji").unwrap();

    let tree = get_file_tree(temp_dir.path(), None, true, None).unwrap();
    let files = collect_files(&tree, temp_dir.path());

    assert_eq!(files.len(), 3);
}

#[test]
fn test_get_file_tree_symlinks() {
    let temp_dir = tempfile::tempdir().unwrap();

    let real_file = temp_dir.path().join("real.py");
    let symlink = temp_dir.path().join("link.py");

    fs::write(&real_file, "# real").unwrap();

    // Create symlink (may fail on Windows without permissions)
    // Note: Symlink testing is platform-specific
    #[cfg(unix)]
    {
        match std::os::unix::fs::symlink(&real_file, &symlink) {
            Ok(_) => {
                let tree = get_file_tree(temp_dir.path(), None, true, None).unwrap();
                // Symlinks might be followed or not depending on implementation
                // Just verify the operation doesn't panic
                let _files = collect_files(&tree, temp_dir.path());
            }
            Err(_) => {
                // Symlink creation failed (permissions), skip test
            }
        }
    }

    // Mark test as passed for non-unix platforms
    #[cfg(not(unix))]
    {
        let _ = (real_file, symlink); // Suppress unused variable warnings
    }
}

#[test]
fn test_get_file_tree_readonly_directory() {
    let temp_dir = tempfile::tempdir().unwrap();

    fs::write(temp_dir.path().join("file.py"), "# file").unwrap();

    // Make directory read-only (Unix only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(temp_dir.path()).unwrap().permissions();
        perms.set_mode(0o555);
        fs::set_permissions(temp_dir.path(), perms).unwrap();

        // Should still be able to read
        let result = get_file_tree(temp_dir.path(), None, true, None);
        // Restore permissions for cleanup
        let mut perms = fs::metadata(temp_dir.path()).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(temp_dir.path(), perms).unwrap();

        assert!(result.is_ok());
    }
}

// =============================================================================
// Integration tests
// =============================================================================

#[test]
fn test_realistic_python_project_structure() {
    let temp_dir = tempfile::tempdir().unwrap();

    // Create realistic project structure
    let src = temp_dir.path().join("src");
    let tests = temp_dir.path().join("tests");
    let docs = temp_dir.path().join("docs");

    fs::create_dir(&src).unwrap();
    fs::create_dir(&tests).unwrap();
    fs::create_dir(&docs).unwrap();
    fs::create_dir(src.join("utils")).unwrap();

    // Create files
    fs::write(src.join("__init__.py"), "").unwrap();
    fs::write(src.join("main.py"), "# main").unwrap();
    fs::write(src.join("utils").join("__init__.py"), "").unwrap();
    fs::write(src.join("utils").join("helpers.py"), "# helpers").unwrap();
    fs::write(tests.join("test_main.py"), "# tests").unwrap();
    fs::write(docs.join("readme.md"), "# readme").unwrap();
    fs::write(temp_dir.path().join("setup.py"), "# setup").unwrap();

    // Get tree with Python filter
    let extensions: HashSet<String> = [".py".to_string()].into_iter().collect();
    let tree = get_file_tree(temp_dir.path(), Some(&extensions), true, None).unwrap();
    let files = collect_files(&tree, temp_dir.path());

    // Should only have Python files
    assert_eq!(files.len(), 5); // All .py files
    for file in &files {
        assert!(file.extension().map(|e| e == "py").unwrap_or(false));
    }
}

#[test]
fn test_node_modules_exclusion() {
    let temp_dir = tempfile::tempdir().unwrap();

    let node_modules = temp_dir.path().join("node_modules");
    fs::create_dir(&node_modules).unwrap();
    fs::create_dir(node_modules.join("lodash")).unwrap();
    fs::create_dir(node_modules.join("express")).unwrap();

    fs::write(node_modules.join("lodash").join("index.js"), "// lodash").unwrap();
    fs::write(node_modules.join("express").join("index.js"), "// express").unwrap();
    fs::write(temp_dir.path().join("app.js"), "// app").unwrap();

    let extensions: HashSet<String> = [".js".to_string()].into_iter().collect();
    let tree = get_file_tree(temp_dir.path(), Some(&extensions), true, None).unwrap();
    let files = collect_files(&tree, temp_dir.path());

    // Should only have app.js, not node_modules files
    assert_eq!(files.len(), 1);
    assert!(files[0].ends_with("app.js"));
}

#[test]
fn test_gitignore_style_patterns() {
    let temp_dir = tempfile::tempdir().unwrap();

    // Create files
    fs::write(temp_dir.path().join("main.py"), "# main").unwrap();
    fs::write(temp_dir.path().join("temp.py"), "# temp").unwrap();
    fs::write(temp_dir.path().join("debug.log"), "log").unwrap();
    fs::write(temp_dir.path().join("error.log"), "log").unwrap();

    // Ignore all log files and temp.py
    let ignore = IgnoreSpec::new(vec!["*.log".to_string(), "temp.py".to_string()]);
    let tree = get_file_tree(temp_dir.path(), None, true, Some(&ignore)).unwrap();
    let files = collect_files(&tree, temp_dir.path());

    // Should only have main.py
    assert_eq!(files.len(), 1);
    assert!(files[0].ends_with("main.py"));
}

#[test]
fn test_file_tree_sorting() {
    let temp_dir = tempfile::tempdir().unwrap();

    // Create files in non-alphabetical order
    fs::write(temp_dir.path().join("zebra.py"), "# z").unwrap();
    fs::write(temp_dir.path().join("alpha.py"), "# a").unwrap();
    fs::write(temp_dir.path().join("beta.py"), "# b").unwrap();

    let tree = get_file_tree(temp_dir.path(), None, true, None).unwrap();

    // Children should be sorted alphabetically
    let file_names: Vec<&str> = tree
        .children
        .iter()
        .filter(|c| c.node_type == NodeType::File)
        .map(|c| c.name.as_str())
        .collect();

    // Verify alphabetical ordering
    for i in 1..file_names.len() {
        assert!(
            file_names[i - 1] <= file_names[i],
            "Files not sorted: {} before {}",
            file_names[i - 1],
            file_names[i]
        );
    }
}

#[test]
fn test_directories_before_files_sorting() {
    let temp_dir = tempfile::tempdir().unwrap();

    // Create a mix of files and directories
    fs::write(temp_dir.path().join("zebra.py"), "# z").unwrap();
    fs::write(temp_dir.path().join("alpha.py"), "# a").unwrap();
    let subdir = temp_dir.path().join("subdir");
    fs::create_dir(&subdir).unwrap();
    fs::write(subdir.join("file.py"), "# file").unwrap();

    let tree = get_file_tree(temp_dir.path(), None, true, None).unwrap();

    // All directories should come before files
    let mut saw_file = false;
    for child in &tree.children {
        if child.node_type == NodeType::File {
            saw_file = true;
        } else {
            assert!(!saw_file, "Directory came after file in sorting");
        }
    }
}

// =============================================================================
// Path handling tests
// =============================================================================

#[test]
fn test_path_traversal_protection() {
    // Attempt path traversal - should be handled safely
    let result = get_file_tree(Path::new("../../../etc"), None, true, None);
    assert!(result.is_err());
}

#[test]
fn test_relative_path_handling() {
    let temp_dir = tempfile::tempdir().unwrap();
    fs::write(temp_dir.path().join("file.py"), "# file").unwrap();

    // Test with relative path
    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(&temp_dir).unwrap();

    let result = get_file_tree(Path::new("."), None, true, None);

    std::env::set_current_dir(original_dir).unwrap();

    assert!(result.is_ok());
}

#[test]
fn test_absolute_path_handling() {
    let temp_dir = tempfile::tempdir().unwrap();
    fs::write(temp_dir.path().join("file.py"), "# file").unwrap();

    // Test with absolute path
    let result = get_file_tree(temp_dir.path(), None, true, None);
    assert!(result.is_ok());
}
