//! CLI Basic Commands Test Suite
//!
//! Comprehensive test coverage for basic CLI commands:
//! - tree: File tree command
//! - structure: Structure extraction command  
//! - imports: Imports command
//! - extract: Extract module command
//! - importers: Find importers command
//!
//! Tests cover: CLI args, options, output formats, error handling

use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

/// Get the path to the tldr binary
fn tldr_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
}

// =============================================================================
// Tree Command Tests
// =============================================================================

#[cfg(test)]
mod tree_tests {
    use super::*;

    /// Test tree command help output
    #[test]
    fn test_tree_help() {
        let mut cmd = tldr_cmd();
        cmd.args(["tree", "--help"]);
        cmd.assert()
            .success()
            .stdout(predicate::str::contains("file tree"))
            .stdout(predicate::str::contains("--ext"))
            .stdout(predicate::str::contains("--include-hidden"));
    }

    /// Test tree command with default path (current directory)
    #[test]
    fn test_tree_default_path() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "# test").unwrap();

        let mut cmd = tldr_cmd();
        cmd.current_dir(temp.path());
        cmd.args(["tree", "-q"]);
        cmd.assert()
            .success()
            .stdout(predicate::str::contains("\"type\""))
            .stdout(predicate::str::contains("\"children\""));
    }

    /// Test tree command with explicit path
    #[test]
    fn test_tree_explicit_path() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("main.py"), "# main").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["tree", temp.path().to_str().unwrap(), "-q"]);
        cmd.assert()
            .success()
            .stdout(predicate::str::contains("main.py"));
    }

    /// Test tree command with single extension filter
    #[test]
    fn test_tree_single_extension() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("script.py"), "# python").unwrap();
        fs::write(temp.path().join("main.rs"), "// rust").unwrap();
        fs::write(temp.path().join("readme.md"), "# readme").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["tree", temp.path().to_str().unwrap(), "--ext", ".py", "-q"]);
        let output = cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);

        assert!(stdout.contains("script.py"), "Should contain .py file");
        assert!(!stdout.contains("main.rs"), "Should NOT contain .rs file");
        assert!(!stdout.contains("readme.md"), "Should NOT contain .md file");
    }

    /// Test tree command with multiple extension filters
    #[test]
    fn test_tree_multiple_extensions() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("a.py"), "").unwrap();
        fs::write(temp.path().join("b.rs"), "").unwrap();
        fs::write(temp.path().join("c.js"), "").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "tree",
            temp.path().to_str().unwrap(),
            "--ext",
            ".py",
            "--ext",
            ".rs",
            "-q",
        ]);
        let output = cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);

        assert!(stdout.contains("a.py"), "Should contain .py file");
        assert!(stdout.contains("b.rs"), "Should contain .rs file");
        assert!(!stdout.contains("c.js"), "Should NOT contain .js file");
    }

    /// Test tree command with extension filter without leading dot
    #[test]
    fn test_tree_extension_no_dot() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "").unwrap();
        fs::write(temp.path().join("main.rs"), "").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["tree", temp.path().to_str().unwrap(), "--ext", "py", "-q"]);
        let output = cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);

        assert!(
            stdout.contains("test.py"),
            "Should handle extension without dot"
        );
        assert!(
            !stdout.contains("main.rs"),
            "Should filter other extensions"
        );
    }

    /// Test tree command with include-hidden flag
    #[test]
    fn test_tree_include_hidden() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("visible.py"), "").unwrap();
        fs::write(temp.path().join(".hidden.py"), "").unwrap();

        // Without hidden flag
        let mut cmd = tldr_cmd();
        cmd.args(["tree", temp.path().to_str().unwrap(), "-q"]);
        let output = cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            !stdout.contains(".hidden.py"),
            "Should NOT show hidden files by default"
        );

        // With hidden flag
        let mut cmd = tldr_cmd();
        cmd.args(["tree", temp.path().to_str().unwrap(), "-H", "-q"]);
        let output = cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains(".hidden.py"),
            "Should show hidden files with -H"
        );
    }

    /// Test tree command JSON output format
    #[test]
    fn test_tree_json_format() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["tree", temp.path().to_str().unwrap(), "-f", "json", "-q"]);
        let output = cmd.output().unwrap();

        // Verify valid JSON
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(json.is_object());
        assert!(json.get("type").is_some());
    }

    /// Test tree command text output format
    #[test]
    #[ignore = "BUG: See bugs_cli_basic.md - Issue 5"]
    fn test_tree_text_format() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["tree", temp.path().to_str().unwrap(), "-f", "text"]);
        cmd.assert()
            .success()
            .stdout(predicate::str::contains("[D]").or(predicate::str::contains("[F]")));
    }

    /// Test tree command compact output format
    #[test]
    fn test_tree_compact_format() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["tree", temp.path().to_str().unwrap(), "-f", "compact", "-q"]);
        let output = cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);

        // Compact should be single line
        let lines: Vec<&str> = stdout.lines().collect();
        assert_eq!(lines.len(), 1, "Compact output should be single line");

        // Verify valid JSON
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(json.is_object());
    }

    /// Test tree command with nonexistent path
    #[test]
    fn test_tree_nonexistent_path() {
        let mut cmd = tldr_cmd();
        cmd.args(["tree", "/nonexistent/path/xyz123", "-q"]);
        cmd.assert().failure();
    }

    /// Test tree alias "t"
    #[test]
    fn test_tree_alias() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["t", temp.path().to_str().unwrap(), "-q"]);
        cmd.assert().success();
    }

    /// Test tree with nested directories
    #[test]
    fn test_tree_nested_directories() {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join("level1")).unwrap();
        fs::create_dir(temp.path().join("level1/level2")).unwrap();
        fs::write(temp.path().join("level1/level2/deep.py"), "").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["tree", temp.path().to_str().unwrap(), "-q"]);
        let output = cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);

        assert!(stdout.contains("deep.py"), "Should find nested file");
    }
}

// =============================================================================
// Structure Command Tests
// =============================================================================

#[cfg(test)]
mod structure_tests {
    use super::*;

    /// Test structure command help output
    #[test]
    fn test_structure_help() {
        let mut cmd = tldr_cmd();
        cmd.args(["structure", "--help"]);
        cmd.assert()
            .success()
            .stdout(predicate::str::contains("code structure"))
            .stdout(predicate::str::contains("--lang"))
            .stdout(predicate::str::contains("--max-results"));
    }

    /// Test structure command with default path
    #[test]
    fn test_structure_default_path() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.current_dir(temp.path());
        cmd.args(["structure", "-l", "python", "-q"]);
        cmd.assert()
            .success()
            .stdout(predicate::str::contains("\"functions\""))
            .stdout(predicate::str::contains("\"classes\""));
    }

    /// Test structure command with explicit path
    #[test]
    fn test_structure_explicit_path() {
        let temp = TempDir::new().unwrap();
        fs::write(
            temp.path().join("main.py"),
            r#"
def hello():
    pass

class World:
    pass
"#,
        )
        .unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "structure",
            temp.path().to_str().unwrap(),
            "-l",
            "python",
            "-q",
        ]);
        let output = cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);

        assert!(stdout.contains("hello"), "Should find function");
        assert!(stdout.contains("World"), "Should find class");
    }

    /// Test structure command with language auto-detection
    #[test]
    fn test_structure_auto_detect_language() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("main.py"), "def python_func(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["structure", temp.path().to_str().unwrap(), "-q"]);
        cmd.assert()
            .success()
            .stdout(predicate::str::contains("\"language\""));
    }

    /// Test structure command with explicit language flag
    #[test]
    fn test_structure_explicit_language() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("script.py"), "def func(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "structure",
            temp.path().to_str().unwrap(),
            "--lang",
            "python",
            "-q",
        ]);
        cmd.assert().success();
    }

    /// Test structure command with max-results limit
    #[test]
    fn test_structure_max_results() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("a.py"), "def a(): pass").unwrap();
        fs::write(temp.path().join("b.py"), "def b(): pass").unwrap();
        fs::write(temp.path().join("c.py"), "def c(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "structure",
            temp.path().to_str().unwrap(),
            "-l",
            "python",
            "-m",
            "2",
            "-q",
        ]);
        cmd.assert().success();
    }

    /// Test structure command JSON output
    #[test]
    fn test_structure_json_output() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "structure",
            temp.path().to_str().unwrap(),
            "-f",
            "json",
            "-l",
            "python",
            "-q",
        ]);
        let output = cmd.output().unwrap();

        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(json.is_object());
        assert!(json.get("files").is_some());
        assert!(json.get("language").is_some());
    }

    /// Test structure command text output
    #[test]
    #[ignore = "BUG: See bugs_cli_basic.md - Issue 5"]
    fn test_structure_text_output() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "structure",
            temp.path().to_str().unwrap(),
            "-f",
            "text",
            "-l",
            "python",
        ]);
        cmd.assert()
            .success()
            .stdout(predicate::str::contains("Functions:").or(predicate::str::contains("test.py")));
    }

    /// Test structure command compact output
    #[test]
    fn test_structure_compact_output() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "structure",
            temp.path().to_str().unwrap(),
            "-f",
            "compact",
            "-l",
            "python",
            "-q",
        ]);
        let output = cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);

        let lines: Vec<&str> = stdout.lines().collect();
        assert_eq!(lines.len(), 1, "Compact output should be single line");
    }

    /// Test structure with empty directory
    #[test]
    fn test_structure_empty_directory() {
        let temp = TempDir::new().unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "structure",
            temp.path().to_str().unwrap(),
            "-l",
            "python",
            "-q",
        ]);
        cmd.assert().success();
    }

    /// Test structure alias "s"
    #[test]
    fn test_structure_alias() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["s", temp.path().to_str().unwrap(), "-l", "python", "-q"]);
        cmd.assert().success();
    }
}

// =============================================================================
// Imports Command Tests
// =============================================================================

#[cfg(test)]
mod imports_tests {
    use super::*;

    /// Test imports command help output
    #[test]
    fn test_imports_help() {
        let mut cmd = tldr_cmd();
        cmd.args(["imports", "--help"]);
        cmd.assert()
            .success()
            .stdout(predicate::str::contains("import statements"))
            .stdout(predicate::str::contains("<FILE>"))
            .stdout(predicate::str::contains("--lang"));
    }

    /// Test imports command with Python file
    #[test]
    fn test_imports_python_file() {
        let temp = TempDir::new().unwrap();
        let test_file = temp.path().join("test.py");
        fs::write(
            &test_file,
            r#"
import os
import sys
from typing import List, Dict
from collections import OrderedDict
"#,
        )
        .unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["imports", test_file.to_str().unwrap(), "-q"]);
        let output = cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);

        assert!(stdout.contains("os"), "Should find 'os' import");
        assert!(stdout.contains("sys"), "Should find 'sys' import");
        assert!(stdout.contains("typing"), "Should find 'typing' import");
        assert!(
            stdout.contains("collections"),
            "Should find 'collections' import"
        );
    }

    /// Test imports command returns valid JSON array
    #[test]
    fn test_imports_returns_json_array() {
        let temp = TempDir::new().unwrap();
        let test_file = temp.path().join("test.py");
        fs::write(&test_file, "import os\n").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["imports", test_file.to_str().unwrap(), "-q"]);
        let output = cmd.output().unwrap();

        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(json.is_array(), "Imports output should be a JSON array");
    }

    /// Test imports command with --lang flag
    #[test]
    fn test_imports_with_lang_flag() {
        let temp = TempDir::new().unwrap();
        let test_file = temp.path().join("script.ts");
        fs::write(
            &test_file,
            r#"
import { readFile } from 'fs';
import * as path from 'path';
"#,
        )
        .unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "imports",
            test_file.to_str().unwrap(),
            "--lang",
            "typescript",
            "-q",
        ]);
        cmd.assert().success();
    }

    /// Test imports command auto-detects language from extension
    #[test]
    fn test_imports_auto_detect_python() {
        let temp = TempDir::new().unwrap();
        let test_file = temp.path().join("main.py");
        fs::write(&test_file, "import os\n").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["imports", test_file.to_str().unwrap(), "-q"]);
        cmd.assert()
            .success()
            .stdout(predicate::str::contains("os"));
    }

    /// Test imports command auto-detects Rust from extension
    #[test]
    fn test_imports_auto_detect_rust() {
        let temp = TempDir::new().unwrap();
        let test_file = temp.path().join("main.rs");
        fs::write(
            &test_file,
            r#"
use std::path::PathBuf;
use serde::{Serialize, Deserialize};
"#,
        )
        .unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["imports", test_file.to_str().unwrap(), "-q"]);
        cmd.assert().success();
    }

    /// Test imports command with JSON format
    #[test]
    fn test_imports_json_format() {
        let temp = TempDir::new().unwrap();
        let test_file = temp.path().join("test.py");
        fs::write(&test_file, "import os\n").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["imports", test_file.to_str().unwrap(), "-f", "json", "-q"]);
        let output = cmd.output().unwrap();

        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(json.is_array());
    }

    /// Test imports command with compact format
    #[test]
    fn test_imports_compact_format() {
        let temp = TempDir::new().unwrap();
        let test_file = temp.path().join("test.py");
        fs::write(&test_file, "import os\n").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "imports",
            test_file.to_str().unwrap(),
            "-f",
            "compact",
            "-q",
        ]);
        let output = cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);

        let lines: Vec<&str> = stdout.lines().collect();
        assert_eq!(lines.len(), 1, "Compact output should be single line");
    }

    /// Test imports command error on missing file
    #[test]
    fn test_imports_missing_file() {
        let mut cmd = tldr_cmd();
        cmd.args(["imports", "/nonexistent/path/file.py", "-q"]);
        cmd.assert().failure();
    }

    /// Test imports command with empty file
    #[test]
    fn test_imports_empty_file() {
        let temp = TempDir::new().unwrap();
        let test_file = temp.path().join("empty.py");
        fs::write(&test_file, "\n").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["imports", test_file.to_str().unwrap(), "-q"]);
        cmd.assert()
            .success()
            .stdout(predicate::str::contains("[]"));
    }

    /// Test imports command with file that has no imports
    #[test]
    fn test_imports_no_imports() {
        let temp = TempDir::new().unwrap();
        let test_file = temp.path().join("no_imports.py");
        fs::write(&test_file, "def foo(): pass\n").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["imports", test_file.to_str().unwrap(), "-q"]);
        cmd.assert()
            .success()
            .stdout(predicate::str::contains("[]"));
    }

    /// Test imports command with unsupported language
    #[test]
    #[ignore = "BUG: See bugs_cli_basic.md - Issue 1"]
    fn test_imports_unsupported_language() {
        let temp = TempDir::new().unwrap();
        let test_file = temp.path().join("test.xyz");
        fs::write(&test_file, "some content").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["imports", test_file.to_str().unwrap(), "-q"]);
        cmd.assert().failure();
    }
}

// =============================================================================
// Extract Command Tests
// =============================================================================

#[cfg(test)]
mod extract_tests {
    use super::*;

    /// Test extract command help output
    #[test]
    fn test_extract_help() {
        let mut cmd = tldr_cmd();
        cmd.args(["extract", "--help"]);
        cmd.assert()
            .success()
            .stdout(predicate::str::contains("module info"))
            .stdout(predicate::str::contains("<FILE>"))
            .stdout(predicate::str::contains("--lang"));
    }

    /// Test extract command returns ModuleInfo structure
    #[test]
    fn test_extract_returns_module_info() {
        let temp = TempDir::new().unwrap();
        let test_file = temp.path().join("test.py");
        fs::write(
            &test_file,
            r#"
import os
from typing import List

def hello(name: str) -> str:
    """Say hello."""
    return f"Hello, {name}"

class Greeter:
    def greet(self):
        pass
"#,
        )
        .unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["extract", test_file.to_str().unwrap(), "-q"]);
        let output = cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);

        assert!(
            stdout.contains("file_path"),
            "Should contain file_path field"
        );
        assert!(stdout.contains("language"), "Should contain language field");
        assert!(
            stdout.contains("functions"),
            "Should contain functions field"
        );
        assert!(stdout.contains("imports"), "Should contain imports field");
        assert!(stdout.contains("classes"), "Should contain classes field");
    }

    /// Test extract command finds functions
    #[test]
    fn test_extract_finds_functions() {
        let temp = TempDir::new().unwrap();
        let test_file = temp.path().join("funcs.py");
        fs::write(
            &test_file,
            r#"
def add(a, b):
    return a + b

def multiply(x, y):
    return x * y
"#,
        )
        .unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["extract", test_file.to_str().unwrap(), "-q"]);
        let output = cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);

        assert!(stdout.contains("add"), "Should find 'add' function");
        assert!(
            stdout.contains("multiply"),
            "Should find 'multiply' function"
        );
    }

    /// Test extract command finds classes
    #[test]
    fn test_extract_finds_classes() {
        let temp = TempDir::new().unwrap();
        let test_file = temp.path().join("classes.py");
        fs::write(
            &test_file,
            r#"
class BaseClass:
    pass

class DerivedClass(BaseClass):
    def method(self):
        pass
"#,
        )
        .unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["extract", test_file.to_str().unwrap(), "-q"]);
        let output = cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);

        assert!(stdout.contains("BaseClass"), "Should find 'BaseClass'");
        assert!(
            stdout.contains("DerivedClass"),
            "Should find 'DerivedClass'"
        );
    }

    /// Test extract command with explicit language flag
    #[test]
    fn test_extract_with_lang() {
        let temp = TempDir::new().unwrap();
        let test_file = temp.path().join("script.py");
        fs::write(&test_file, "def foo(): pass\n").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "extract",
            test_file.to_str().unwrap(),
            "--lang",
            "python",
            "-q",
        ]);
        cmd.assert().success();
    }

    /// Test extract command auto-detects language
    #[test]
    fn test_extract_auto_detect() {
        let temp = TempDir::new().unwrap();
        let test_file = temp.path().join("main.py");
        fs::write(&test_file, "def foo(): pass\n").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["extract", test_file.to_str().unwrap(), "-q"]);
        cmd.assert()
            .success()
            .stdout(predicate::str::contains("\"language\""));
    }

    /// Test extract command JSON output
    #[test]
    fn test_extract_json_output() {
        let temp = TempDir::new().unwrap();
        let test_file = temp.path().join("test.py");
        fs::write(&test_file, "def foo(): pass\n").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["extract", test_file.to_str().unwrap(), "-f", "json", "-q"]);
        let output = cmd.output().unwrap();

        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(json.is_object());
    }

    /// Test extract command compact output
    #[test]
    fn test_extract_compact_output() {
        let temp = TempDir::new().unwrap();
        let test_file = temp.path().join("test.py");
        fs::write(&test_file, "def foo(): pass\n").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "extract",
            test_file.to_str().unwrap(),
            "-f",
            "compact",
            "-q",
        ]);
        let output = cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);

        let lines: Vec<&str> = stdout.lines().collect();
        assert_eq!(lines.len(), 1, "Compact output should be single line");
    }

    /// Test extract command error on missing file
    #[test]
    fn test_extract_missing_file() {
        let mut cmd = tldr_cmd();
        cmd.args(["extract", "/nonexistent/path/file.py", "-q"]);
        cmd.assert().failure().code(2);
    }

    /// Test extract command error on unsupported language
    #[test]
    #[ignore = "BUG: See bugs_cli_basic.md - Issue 2"]
    fn test_extract_unsupported_language() {
        let temp = TempDir::new().unwrap();
        let test_file = temp.path().join("test.xyz");
        fs::write(&test_file, "some content").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["extract", test_file.to_str().unwrap(), "-q"]);
        cmd.assert().failure().code(11);
    }

    /// Test extract alias "e"
    #[test]
    fn test_extract_alias() {
        let temp = TempDir::new().unwrap();
        let test_file = temp.path().join("test.py");
        fs::write(&test_file, "def foo(): pass\n").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["e", test_file.to_str().unwrap(), "-q"]);
        cmd.assert().success();
    }

    /// Test extract command with empty file
    #[test]
    fn test_extract_empty_file() {
        let temp = TempDir::new().unwrap();
        let test_file = temp.path().join("empty.py");
        fs::write(&test_file, "\n").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["extract", test_file.to_str().unwrap(), "-q"]);
        cmd.assert().success();
    }
}

// =============================================================================
// Importers Command Tests
// =============================================================================

#[cfg(test)]
mod importers_tests {
    use super::*;

    /// Test importers command help output
    #[test]
    fn test_importers_help() {
        let mut cmd = tldr_cmd();
        cmd.args(["importers", "--help"]);
        cmd.assert()
            .success()
            .stdout(predicate::str::contains("import"))
            .stdout(predicate::str::contains("<MODULE>"))
            .stdout(predicate::str::contains("--lang"));
    }

    /// Test importers command returns ImportersReport
    #[test]
    fn test_importers_returns_report() {
        let temp = TempDir::new().unwrap();
        let file1 = temp.path().join("a.py");
        fs::write(&file1, "import os\n").unwrap();
        let file2 = temp.path().join("b.py");
        fs::write(&file2, "from os import path\n").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "importers",
            "os",
            temp.path().to_str().unwrap(),
            "--lang",
            "python",
            "-q",
        ]);
        let output = cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);

        assert!(stdout.contains("module"), "Should contain module field");
        assert!(
            stdout.contains("importers"),
            "Should contain importers field"
        );
        assert!(stdout.contains("total"), "Should contain total field");
    }

    /// Test importers command finds files that import a module
    #[test]
    fn test_importers_finds_files() {
        let temp = TempDir::new().unwrap();
        let file1 = temp.path().join("uses_pandas.py");
        fs::write(&file1, "import pandas as pd\n").unwrap();
        let file2 = temp.path().join("no_pandas.py");
        fs::write(&file2, "import os\n").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "importers",
            "pandas",
            temp.path().to_str().unwrap(),
            "--lang",
            "python",
            "-q",
        ]);
        let output = cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);

        assert!(
            stdout.contains("uses_pandas.py"),
            "Should find uses_pandas.py"
        );
        assert!(
            !stdout.contains("no_pandas.py"),
            "Should NOT find no_pandas.py"
        );
    }

    /// Test importers with from-import syntax
    #[test]
    fn test_importers_from_import() {
        let temp = TempDir::new().unwrap();
        let file1 = temp.path().join("uses_typing.py");
        fs::write(&file1, "from typing import List\n").unwrap();
        let file2 = temp.path().join("other.py");
        fs::write(&file2, "import os\n").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "importers",
            "typing",
            temp.path().to_str().unwrap(),
            "--lang",
            "python",
            "-q",
        ]);
        let output = cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);

        assert!(
            stdout.contains("uses_typing.py"),
            "Should find file with 'from typing import'"
        );
    }

    /// Test importers returns zero for unknown module
    #[test]
    fn test_importers_zero_for_unknown() {
        let temp = TempDir::new().unwrap();
        let file = temp.path().join("test.py");
        fs::write(&file, "import os\n").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "importers",
            "nonexistent_module_xyz",
            temp.path().to_str().unwrap(),
            "--lang",
            "python",
            "-q",
        ]);
        let output = cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);

        assert!(
            stdout.contains(r#""total":0"#) || stdout.contains(r#""total": 0"#),
            "Should return total: 0 for unknown module, got: {}",
            stdout
        );
    }

    /// Test importers command with explicit path
    #[test]
    fn test_importers_explicit_path() {
        let temp = TempDir::new().unwrap();
        let file = temp.path().join("test.py");
        fs::write(&file, "import sys\n").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "importers",
            "sys",
            temp.path().to_str().unwrap(),
            "--lang",
            "python",
            "-q",
        ]);
        cmd.assert().success();
    }

    /// Test importers command with JSON format
    #[test]
    fn test_importers_json_format() {
        let temp = TempDir::new().unwrap();
        let file = temp.path().join("test.py");
        fs::write(&file, "import os\n").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "importers",
            "os",
            temp.path().to_str().unwrap(),
            "-f",
            "json",
            "--lang",
            "python",
            "-q",
        ]);
        let output = cmd.output().unwrap();

        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(json.is_object());
        assert!(json.get("module").is_some());
        assert!(json.get("importers").is_some());
        assert!(json.get("total").is_some());
    }

    /// Test importers command with compact format
    #[test]
    fn test_importers_compact_format() {
        let temp = TempDir::new().unwrap();
        let file = temp.path().join("test.py");
        fs::write(&file, "import os\n").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "importers",
            "os",
            temp.path().to_str().unwrap(),
            "-f",
            "compact",
            "--lang",
            "python",
            "-q",
        ]);
        let output = cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);

        let lines: Vec<&str> = stdout.lines().collect();
        assert_eq!(lines.len(), 1, "Compact output should be single line");
    }

    /// Test importers command with text format
    #[test]
    #[ignore = "BUG: See bugs_cli_basic.md - Issue 3"]
    fn test_importers_text_format() {
        let temp = TempDir::new().unwrap();
        let file = temp.path().join("test.py");
        fs::write(&file, "import os\n").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "importers",
            "os",
            temp.path().to_str().unwrap(),
            "-f",
            "text",
            "--lang",
            "python",
            "-q",
        ]);
        cmd.assert().success();
    }

    /// Test importers with invalid language
    #[test]
    #[ignore = "BUG: See bugs_cli_basic.md - Issue 4"]
    fn test_importers_invalid_language() {
        let temp = TempDir::new().unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "importers",
            "os",
            temp.path().to_str().unwrap(),
            "--lang",
            "invalid_language",
            "-q",
        ]);
        cmd.assert()
            .failure()
            .stderr(predicate::str::contains("Invalid").or(predicate::str::contains("language")));
    }

    /// Test importers with empty directory
    #[test]
    fn test_importers_empty_directory() {
        let temp = TempDir::new().unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "importers",
            "os",
            temp.path().to_str().unwrap(),
            "--lang",
            "python",
            "-q",
        ]);
        cmd.assert().success().stdout(
            predicate::str::contains(r#""total":0"#).or(predicate::str::contains(r#""total": 0"#)),
        );
    }

    /// Test importers with nonexistent path
    #[test]
    fn test_importers_nonexistent_path() {
        let mut cmd = tldr_cmd();
        cmd.args([
            "importers",
            "os",
            "/nonexistent/path/xyz123",
            "--lang",
            "python",
            "-q",
        ]);
        cmd.assert().failure();
    }
}

// =============================================================================
// Global Options Tests
// =============================================================================

#[cfg(test)]
mod global_options_tests {
    use super::*;

    /// Test --quiet flag suppresses progress output
    #[test]
    fn test_quiet_flag() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "structure",
            temp.path().to_str().unwrap(),
            "-l",
            "python",
            "-q",
        ]);
        let output = cmd.output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(
            !stderr.contains("Extracting"),
            "Quiet flag should suppress progress messages"
        );
    }

    /// Test --verbose flag
    #[test]
    fn test_verbose_flag() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["tree", temp.path().to_str().unwrap(), "-v", "-q"]);
        cmd.assert().success();
    }

    /// Test --format global option with json
    #[test]
    fn test_format_json() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "tree",
            temp.path().to_str().unwrap(),
            "--format",
            "json",
            "-q",
        ]);
        let output = cmd.output().unwrap();

        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(json.is_object());
    }

    /// Test --format global option with text
    #[test]
    fn test_format_text() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "tree",
            temp.path().to_str().unwrap(),
            "--format",
            "text",
            "-q",
        ]);
        cmd.assert().success();
    }

    /// Test --format global option with compact
    #[test]
    fn test_format_compact() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "tree",
            temp.path().to_str().unwrap(),
            "--format",
            "compact",
            "-q",
        ]);
        let output = cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);

        let lines: Vec<&str> = stdout.lines().collect();
        assert_eq!(lines.len(), 1);
    }

    /// Test -f short option for format
    #[test]
    fn test_format_short_option() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["tree", temp.path().to_str().unwrap(), "-f", "text", "-q"]);
        cmd.assert().success();
    }

    /// Test --lang global option
    #[test]
    fn test_lang_option() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "structure",
            temp.path().to_str().unwrap(),
            "--lang",
            "python",
            "-q",
        ]);
        cmd.assert().success();
    }

    /// Test -l short option for language
    #[test]
    fn test_lang_short_option() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "structure",
            temp.path().to_str().unwrap(),
            "-l",
            "python",
            "-q",
        ]);
        cmd.assert().success();
    }
}

// =============================================================================
// Error Handling Tests
// =============================================================================

#[cfg(test)]
mod error_handling_tests {
    use super::*;

    /// Test exit code 2 for missing file/path
    #[test]
    fn test_exit_code_missing_file() {
        let mut cmd = tldr_cmd();
        cmd.args(["extract", "/nonexistent/file.py", "-q"]);
        cmd.assert().failure().code(2);
    }

    /// Test exit code 2 for missing directory
    #[test]
    fn test_exit_code_missing_directory() {
        let mut cmd = tldr_cmd();
        cmd.args(["tree", "/nonexistent/dir", "-q"]);
        cmd.assert().failure().code(2);
    }

    /// Test error message contains helpful context
    #[test]
    fn test_error_message_context() {
        let mut cmd = tldr_cmd();
        cmd.args(["extract", "/nonexistent/file.py", "-q"]);
        cmd.assert()
            .failure()
            .stderr(predicate::str::contains("Error").or(predicate::str::contains("not found")));
    }
}

// =============================================================================
// Output Schema Validation Tests
// =============================================================================

#[cfg(test)]
mod schema_tests {
    use super::*;

    /// Test tree output schema
    #[test]
    fn test_tree_schema() {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join("subdir")).unwrap();
        fs::write(temp.path().join("file.py"), "").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["tree", temp.path().to_str().unwrap(), "-q"]);
        let output = cmd.output().unwrap();

        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

        // Check required fields
        assert!(json.get("name").is_some(), "Should have 'name' field");
        assert!(json.get("type").is_some(), "Should have 'type' field");
        assert!(
            json.get("children").is_some(),
            "Should have 'children' field"
        );
    }

    /// Test structure output schema
    #[test]
    fn test_structure_schema() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "structure",
            temp.path().to_str().unwrap(),
            "-l",
            "python",
            "-q",
        ]);
        let output = cmd.output().unwrap();

        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

        assert!(json.get("root").is_some(), "Should have 'root' field");
        assert!(
            json.get("language").is_some(),
            "Should have 'language' field"
        );
        assert!(json.get("files").is_some(), "Should have 'files' field");
    }

    /// Test imports output schema
    #[test]
    fn test_imports_schema() {
        let temp = TempDir::new().unwrap();
        let test_file = temp.path().join("test.py");
        fs::write(&test_file, "import os\n").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["imports", test_file.to_str().unwrap(), "-q"]);
        let output = cmd.output().unwrap();

        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

        assert!(json.is_array(), "Imports should be an array");
        if let Some(first) = json.as_array().and_then(|arr| arr.first()) {
            assert!(
                first.get("module").is_some(),
                "Import should have 'module' field"
            );
        }
    }

    /// Test extract output schema
    #[test]
    fn test_extract_schema() {
        let temp = TempDir::new().unwrap();
        let test_file = temp.path().join("test.py");
        fs::write(&test_file, "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["extract", test_file.to_str().unwrap(), "-q"]);
        let output = cmd.output().unwrap();

        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

        assert!(
            json.get("file_path").is_some(),
            "Should have 'file_path' field"
        );
        assert!(
            json.get("language").is_some(),
            "Should have 'language' field"
        );
        assert!(
            json.get("functions").is_some(),
            "Should have 'functions' field"
        );
        assert!(json.get("imports").is_some(), "Should have 'imports' field");
        assert!(json.get("classes").is_some(), "Should have 'classes' field");
    }

    /// Test importers output schema
    #[test]
    fn test_importers_schema() {
        let temp = TempDir::new().unwrap();
        let file = temp.path().join("test.py");
        fs::write(&file, "import os\n").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "importers",
            "os",
            temp.path().to_str().unwrap(),
            "--lang",
            "python",
            "-q",
        ]);
        let output = cmd.output().unwrap();

        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

        assert!(json.get("module").is_some(), "Should have 'module' field");
        assert!(
            json.get("importers").is_some(),
            "Should have 'importers' field"
        );
        assert!(json.get("total").is_some(), "Should have 'total' field");
    }
}
