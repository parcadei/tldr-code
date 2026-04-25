//! GVN CLI Integration Tests
//!
//! Tests for the `tldr gvn` command.
//! These tests define expected CLI behavior BEFORE implementation.
//!
//! Reference: Phase P5 - CLI Integration for GVN

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
// Test Fixtures
// =============================================================================

mod fixtures {
    pub const PYTHON_REDUNDANT: &str = r#"
def example(a, b):
    x = a + b
    y = a + b  # redundant with x
    z = b + a  # commutativity: also redundant
    return x + y + z
"#;

    pub const PYTHON_NO_REDUNDANCY: &str = r#"
def unique(a, b):
    x = a + b
    y = a - b
    z = a * b
    return x + y + z
"#;

    pub const PYTHON_MULTI_FUNC: &str = r#"
def func1(x):
    a = x + 1
    b = x + 1  # redundant
    return a + b

def func2(x):
    a = x * 2
    b = x * 2  # redundant
    return a + b
"#;
}

// =============================================================================
// Basic Command Tests
// =============================================================================

#[test]
fn test_gvn_command_exists() {
    // Test that `tldr gvn` command is recognized
    let mut cmd = tldr_cmd();
    cmd.arg("gvn").arg("--help");
    cmd.assert().success().stdout(
        predicate::str::contains("value numbering")
            .or(predicate::str::contains("GVN"))
            .or(predicate::str::contains("redundant")),
    );
}

#[test]
fn test_gvn_basic_analysis() {
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("test.py");
    fs::write(&test_file, fixtures::PYTHON_REDUNDANT).unwrap();

    let mut cmd = tldr_cmd();
    cmd.arg("gvn").arg(&test_file).arg("-f").arg("json");

    cmd.assert().success().stdout(
        predicate::str::contains("redundancies").or(predicate::str::contains("equivalences")),
    );
}

#[test]
fn test_gvn_with_function_filter() {
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("test.py");
    fs::write(&test_file, fixtures::PYTHON_MULTI_FUNC).unwrap();

    let mut cmd = tldr_cmd();
    cmd.arg("gvn")
        .arg(&test_file)
        .arg("func1")
        .arg("-f")
        .arg("json");

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("func1"));
}

#[test]
fn test_gvn_text_format() {
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("test.py");
    fs::write(&test_file, fixtures::PYTHON_REDUNDANT).unwrap();

    let mut cmd = tldr_cmd();
    cmd.arg("gvn").arg(&test_file).arg("-f").arg("text");

    cmd.assert().success().stdout(
        predicate::str::contains("GVN")
            .or(predicate::str::contains("Value"))
            .or(predicate::str::contains("expression")),
    );
}

#[test]
fn test_gvn_json_format() {
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("test.py");
    fs::write(&test_file, fixtures::PYTHON_REDUNDANT).unwrap();

    let mut cmd = tldr_cmd();
    cmd.arg("gvn").arg(&test_file).arg("-f").arg("json");

    let output = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should be valid JSON
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Should have expected structure
    assert!(json.is_object() || json.is_array());
}

// =============================================================================
// Error Handling Tests
// =============================================================================

#[test]
fn test_gvn_file_not_found() {
    let mut cmd = tldr_cmd();
    cmd.arg("gvn").arg("/nonexistent/path.py");

    cmd.assert().failure().stderr(
        predicate::str::contains("not found")
            .or(predicate::str::contains("No such file"))
            .or(predicate::str::contains("does not exist")),
    );
}

#[test]
fn test_gvn_function_not_found() {
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("test.py");
    fs::write(&test_file, fixtures::PYTHON_REDUNDANT).unwrap();

    let mut cmd = tldr_cmd();
    cmd.arg("gvn").arg(&test_file).arg("nonexistent_function");

    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("not found").or(predicate::str::contains("No function")));
}

// =============================================================================
// Redundancy Detection Tests
// =============================================================================

#[test]
fn test_gvn_detects_exact_duplicates() {
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("test.py");
    fs::write(&test_file, fixtures::PYTHON_REDUNDANT).unwrap();

    let mut cmd = tldr_cmd();
    cmd.arg("gvn").arg(&test_file).arg("-f").arg("json");

    let output = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should detect redundancy
    assert!(stdout.contains("redundan") || stdout.contains("equivalen"));
}

#[test]
fn test_gvn_no_false_positives() {
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("test.py");
    fs::write(&test_file, fixtures::PYTHON_NO_REDUNDANCY).unwrap();

    let mut cmd = tldr_cmd();
    cmd.arg("gvn").arg(&test_file).arg("-f").arg("json");

    let output = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Should have no redundancies or empty redundancies
    if let Some(arr) = json.as_array() {
        for report in arr {
            if let Some(redundancies) = report.get("redundancies") {
                if let Some(arr) = redundancies.as_array() {
                    assert!(
                        arr.is_empty(),
                        "Expected no redundancies in unique expressions"
                    );
                }
            }
        }
    }
}
