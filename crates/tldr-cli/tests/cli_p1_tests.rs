//! P1 CLI Command Tests: Missing CLI Command Wiring
//!
//! Tests defined BEFORE implementation to drive TDD.
//! These tests should FAIL initially - the commands don't exist yet.
//!
//! Contracts:
//! - 1.2: `extract` command
//! - 1.3: `imports` command
//! - 1.4: `importers` command
//! - 1.5: `complexity` command

use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

/// Get the path to the test binary
fn tldr_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
}

// =============================================================================
// Contract 1.2: `extract` CLI Command
// =============================================================================

#[cfg(test)]
mod extract_command {
    use super::*;

    /// Contract 1.2: extract command exists and shows help
    #[test]
    fn test_extract_command_exists() {
        let mut cmd = tldr_cmd();
        cmd.arg("extract").arg("--help");
        cmd.assert()
            .success()
            .stdout(predicate::str::contains("extract"))
            .stdout(predicate::str::contains("file"));
    }

    /// Contract 1.2: extract returns JSON with ModuleInfo
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
        cmd.assert()
            .success()
            .stdout(predicate::str::contains("\"file_path\""))
            .stdout(predicate::str::contains("\"language\""))
            .stdout(predicate::str::contains("\"functions\""))
            .stdout(predicate::str::contains("\"imports\""))
            .stdout(predicate::str::contains("\"classes\""));
    }

    /// Contract 1.2: extract finds functions in the file
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
        cmd.assert()
            .success()
            .stdout(predicate::str::contains("add"))
            .stdout(predicate::str::contains("multiply"));
    }

    /// Contract 1.2: extract error on missing file (exit code 2)
    #[test]
    fn test_extract_error_missing_file() {
        let mut cmd = tldr_cmd();
        cmd.args(["extract", "/nonexistent/path/file.py", "-q"]);
        cmd.assert()
            .failure()
            .code(2)
            .stderr(predicate::str::contains("not found").or(predicate::str::contains("Path")));
    }

    /// Contract 1.2: extract error on unsupported language
    #[test]
    fn test_extract_error_unsupported_language() {
        let temp = TempDir::new().unwrap();
        let test_file = temp.path().join("test.xyz");
        fs::write(&test_file, "some content").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["extract", test_file.to_str().unwrap(), "-q"]);
        cmd.assert().failure().code(11).stderr(
            predicate::str::contains("Unsupported").or(predicate::str::contains("language")),
        );
    }
}

// =============================================================================
// Contract 1.3: `imports` CLI Command
// =============================================================================

#[cfg(test)]
mod imports_command {
    use super::*;

    /// Contract 1.3: imports command exists and shows help
    #[test]
    fn test_imports_command_exists() {
        let mut cmd = tldr_cmd();
        cmd.arg("imports").arg("--help");
        cmd.assert()
            .success()
            .stdout(predicate::str::contains("imports"))
            .stdout(predicate::str::contains("file"));
    }

    /// Contract 1.3: imports returns array of ImportInfo
    #[test]
    fn test_imports_returns_array() {
        let temp = TempDir::new().unwrap();
        let test_file = temp.path().join("test.py");
        fs::write(
            &test_file,
            r#"
import os
import sys
from typing import List, Dict
from collections import OrderedDict as OD
"#,
        )
        .unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["imports", test_file.to_str().unwrap(), "-q"]);
        cmd.assert()
            .success()
            .stdout(predicate::str::starts_with("["))
            .stdout(predicate::str::contains("\"module\""))
            .stdout(predicate::str::contains("\"names\""))
            .stdout(predicate::str::contains("os"))
            .stdout(predicate::str::contains("sys"));
    }

    /// Contract 1.3: imports with explicit --lang flag
    #[test]
    fn test_imports_with_lang_flag() {
        let temp = TempDir::new().unwrap();
        let test_file = temp.path().join("test.ts");
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
        cmd.assert()
            .success()
            .stdout(predicate::str::contains("\"module\""));
    }

    /// Contract 1.3: imports error on missing file
    #[test]
    fn test_imports_error_missing_file() {
        let mut cmd = tldr_cmd();
        cmd.args(["imports", "/nonexistent/file.py", "-q"]);
        cmd.assert()
            .failure()
            .code(2)
            .stderr(predicate::str::contains("not found").or(predicate::str::contains("Path")));
    }

    /// Contract 1.3: imports auto-detects language from extension
    #[test]
    fn test_imports_auto_detect_language() {
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
        cmd.assert()
            .success()
            .stdout(predicate::str::contains("std::path"));
    }
}

// =============================================================================
// Contract 1.4: `importers` CLI Command
// =============================================================================

#[cfg(test)]
mod importers_command {
    use super::*;

    /// Contract 1.4: importers command exists and shows help
    #[test]
    fn test_importers_command_exists() {
        let mut cmd = tldr_cmd();
        cmd.arg("importers").arg("--help");
        cmd.assert()
            .success()
            .stdout(predicate::str::contains("importers"))
            .stdout(predicate::str::contains("module"))
            .stdout(predicate::str::contains("--lang"));
    }

    /// Contract 1.4: importers returns ImportersReport JSON
    #[test]
    fn test_importers_returns_report() {
        let temp = TempDir::new().unwrap();

        // Create a file that imports 'os'
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
        cmd.assert()
            .success()
            .stdout(predicate::str::contains("\"module\""))
            .stdout(predicate::str::contains("\"importers\""))
            .stdout(predicate::str::contains("\"total\""));
    }

    /// Contract 1.4: importers finds correct files
    #[test]
    fn test_importers_finds_files() {
        let temp = TempDir::new().unwrap();

        let file1 = temp.path().join("uses_pandas.py");
        fs::write(&file1, "import pandas as pd\ndf = pd.DataFrame()\n").unwrap();

        let file2 = temp.path().join("no_pandas.py");
        fs::write(&file2, "import os\nprint('hello')\n").unwrap();

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

        // Should find uses_pandas.py
        assert!(
            stdout.contains("uses_pandas.py"),
            "Should find uses_pandas.py, got: {}",
            stdout
        );
        // Should NOT find no_pandas.py
        assert!(
            !stdout.contains("no_pandas.py"),
            "Should NOT find no_pandas.py"
        );
    }

    /// Contract 1.4: importers returns 0 total for non-imported module
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
        cmd.assert().success().stdout(
            predicate::str::contains("\"total\": 0").or(predicate::str::contains("\"total\":0")),
        );
    }

    /// Contract 1.4: importers error on invalid language
    #[test]
    fn test_importers_error_invalid_language() {
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

    /// Contract 1.4: importers requires --lang flag
    #[test]
    fn test_importers_auto_detects_lang() {
        let temp = TempDir::new().unwrap();
        let file = temp.path().join("test.py");
        fs::write(&file, "import os\n").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["importers", "os", temp.path().to_str().unwrap(), "-q"]);
        // Language is auto-detected from .py files in the directory
        cmd.assert()
            .success()
            .stdout(predicate::str::contains("importers"));
    }
}

// =============================================================================
// Contract 1.5: `complexity` CLI Command
// =============================================================================

#[cfg(test)]
mod complexity_command {
    use super::*;

    /// Contract 1.5: complexity command exists and shows help
    #[test]
    fn test_complexity_command_exists() {
        let mut cmd = tldr_cmd();
        cmd.arg("complexity").arg("--help");
        cmd.assert()
            .success()
            .stdout(predicate::str::contains("complexity"))
            .stdout(predicate::str::contains("file"))
            .stdout(predicate::str::contains("function"));
    }

    /// Contract 1.5: complexity returns ComplexityMetrics JSON
    #[test]
    fn test_complexity_returns_metrics() {
        let temp = TempDir::new().unwrap();
        let test_file = temp.path().join("test.py");
        fs::write(
            &test_file,
            r#"
def simple_function():
    return 42

def complex_function(a, b, c):
    if a > 0:
        if b > 0:
            if c > 0:
                return a + b + c
            else:
                return a + b
        else:
            return a
    elif a < 0:
        return -a
    else:
        return 0
"#,
        )
        .unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "complexity",
            test_file.to_str().unwrap(),
            "complex_function",
            "-q",
        ]);
        cmd.assert()
            .success()
            .stdout(predicate::str::contains("\"cyclomatic\""))
            .stdout(predicate::str::contains("\"cognitive\""));
    }

    /// Contract 1.5: complexity calculates correct cyclomatic value
    #[test]
    fn test_complexity_cyclomatic_value() {
        let temp = TempDir::new().unwrap();
        let test_file = temp.path().join("test.py");
        fs::write(
            &test_file,
            r#"
def branchy(x):
    if x > 10:
        return "big"
    elif x > 5:
        return "medium"
    elif x > 0:
        return "small"
    else:
        return "zero"
"#,
        )
        .unwrap();

        let mut cmd = tldr_cmd();
        let output = cmd
            .args(["complexity", test_file.to_str().unwrap(), "branchy", "-q"])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

        let cyclomatic = json.get("cyclomatic").and_then(|v| v.as_i64()).unwrap_or(0);
        // 3 if/elif branches means cyclomatic should be at least 4
        assert!(
            cyclomatic >= 4,
            "Expected cyclomatic >= 4 for branchy function, got {}",
            cyclomatic
        );
    }

    /// Contract 1.5: complexity with explicit --lang flag
    #[test]
    fn test_complexity_with_lang_flag() {
        let temp = TempDir::new().unwrap();
        let test_file = temp.path().join("test.py");
        fs::write(
            &test_file,
            r#"
def simple():
    return 1
"#,
        )
        .unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "complexity",
            test_file.to_str().unwrap(),
            "simple",
            "--lang",
            "python",
            "-q",
        ]);
        cmd.assert()
            .success()
            .stdout(predicate::str::contains("\"cyclomatic\""));
    }

    /// Contract 1.5: complexity error on function not found (exit code 20)
    #[test]
    fn test_complexity_error_function_not_found() {
        let temp = TempDir::new().unwrap();
        let test_file = temp.path().join("test.py");
        fs::write(&test_file, "def existing(): pass\n").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "complexity",
            test_file.to_str().unwrap(),
            "nonexistent_function",
            "-q",
        ]);
        cmd.assert()
            .failure()
            .code(20)
            .stderr(predicate::str::contains("not found").or(predicate::str::contains("Function")));
    }

    /// Contract 1.5: complexity error on missing file
    #[test]
    fn test_complexity_error_missing_file() {
        let mut cmd = tldr_cmd();
        cmd.args(["complexity", "/nonexistent/file.py", "somefunc", "-q"]);
        cmd.assert()
            .failure()
            .code(2)
            .stderr(predicate::str::contains("not found").or(predicate::str::contains("Path")));
    }

    /// Contract 1.5: complexity includes nesting_depth and lines_of_code
    #[test]
    fn test_complexity_all_fields() {
        let temp = TempDir::new().unwrap();
        let test_file = temp.path().join("test.py");
        fs::write(
            &test_file,
            r#"
def nested(x):
    if x > 0:
        for i in range(x):
            if i % 2 == 0:
                print(i)
    return x
"#,
        )
        .unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["complexity", test_file.to_str().unwrap(), "nested", "-q"]);
        cmd.assert()
            .success()
            .stdout(predicate::str::contains("\"cyclomatic\""))
            .stdout(predicate::str::contains("\"cognitive\""))
            .stdout(predicate::str::contains("\"nesting_depth\""))
            .stdout(predicate::str::contains("\"lines_of_code\""));
    }
}

// =============================================================================
// Contract 1.1: Tree Command Schema (Additional CLI tests)
// =============================================================================

#[cfg(test)]
mod tree_schema_tests {
    use super::*;

    /// Contract 1.1: tree output uses "type" not "node_type"
    #[test]
    fn test_tree_uses_type_field() {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join("subdir")).unwrap();
        fs::write(temp.path().join("file.py"), "# test").unwrap();

        let mut cmd = tldr_cmd();
        let output = cmd
            .args(["tree", temp.path().to_str().unwrap(), "-q"])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);

        // Should contain "type" field
        assert!(
            stdout.contains("\"type\""),
            "tree output should contain 'type' field, got: {}",
            stdout
        );

        // Should NOT contain "node_type" field
        assert!(
            !stdout.contains("\"node_type\""),
            "tree output should NOT contain 'node_type' field, got: {}",
            stdout
        );
    }

    /// Contract 1.1: tree root type is "dir"
    #[test]
    fn test_tree_root_type_dir() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("file.py"), "# test").unwrap();

        let mut cmd = tldr_cmd();
        let output = cmd
            .args(["tree", temp.path().to_str().unwrap(), "-q"])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

        assert_eq!(
            json.get("type").and_then(|v| v.as_str()),
            Some("dir"),
            "Root type should be 'dir'"
        );
    }

    /// Contract 1.1: tree file children have type "file"
    #[test]
    fn test_tree_file_type() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("main.py"), "# test").unwrap();

        let mut cmd = tldr_cmd();
        let output = cmd
            .args(["tree", temp.path().to_str().unwrap(), "-q"])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

        let children = json.get("children").and_then(|c| c.as_array());
        if let Some(children) = children {
            for child in children {
                if child.get("name").and_then(|n| n.as_str()) == Some("main.py") {
                    assert_eq!(
                        child.get("type").and_then(|v| v.as_str()),
                        Some("file"),
                        "File type should be 'file'"
                    );
                }
            }
        }
    }
}
