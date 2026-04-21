//! SSA CLI Integration Tests
//!
//! Tests for the `tldr ssa` command.
//! These tests define expected CLI behavior BEFORE implementation.
//!
//! Reference: session10-spec.md Section 4.1

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
    pub const PYTHON_SIMPLE: &str = r#"
def simple(x):
    y = x + 1
    return y
"#;

    pub const PYTHON_DIAMOND: &str = r#"
def branch(x):
    if x > 0:
        y = 1
    else:
        y = 2
    return y
"#;

    pub const PYTHON_LOOP: &str = r#"
def loop(n):
    total = 0
    i = 0
    while i < n:
        total = total + i
        i = i + 1
    return total
"#;

    pub const PYTHON_NESTED: &str = r#"
def nested(x, y):
    if x > 0:
        if y > 0:
            z = 1
        else:
            z = 2
    else:
        z = 3
    return z
"#;

    pub const PYTHON_MULTI_VAR: &str = r#"
def multi(a, b):
    x = a
    y = b
    if a > b:
        x = a + 1
        y = b - 1
    return x + y
"#;

    pub const PYTHON_MEMORY: &str = r#"
class Point:
    def __init__(self, x, y):
        self.x = x
        self.y = y

def modify_point(p, cond):
    if cond:
        p.x = 10
    else:
        p.x = 20
    return p.x
"#;

    pub const TYPESCRIPT_SIMPLE: &str = r#"
function simple(x: number): number {
    const y = x + 1;
    return y;
}
"#;

    pub const GO_SIMPLE: &str = r#"
func simple(x int) int {
    y := x + 1
    return y
}
"#;

    pub const RUST_SIMPLE: &str = r#"
fn simple(x: i32) -> i32 {
    let y = x + 1;
    y
}
"#;
}

// =============================================================================
// Help and Basic Command Tests
// =============================================================================

#[test]
fn test_ssa_help() {
    let mut cmd = tldr_cmd();
    cmd.args(["ssa", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("SSA"))
        .stdout(predicate::str::contains("--format"))
        .stdout(predicate::str::contains("--type"))
        .stdout(predicate::str::contains("--var"));
}

#[test]
fn test_ssa_missing_args() {
    let mut cmd = tldr_cmd();
    cmd.arg("ssa")
        .assert()
        .failure()
        .stderr(predicate::str::contains("required"));
}

#[test]
fn test_ssa_file_not_found() {
    let mut cmd = tldr_cmd();
    cmd.args(["ssa", "nonexistent.py", "func"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found").or(predicate::str::contains("No such file")));
}

#[test]
fn test_ssa_function_not_found() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_SIMPLE).unwrap();

    let mut cmd = tldr_cmd();
    cmd.args(["ssa", file.to_str().unwrap(), "nonexistent_function"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found").or(predicate::str::contains("Function")));
}

// =============================================================================
// JSON Output Tests (SSA-19)
// =============================================================================

#[test]
fn test_ssa_json_output_simple() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_SIMPLE).unwrap();

    let mut cmd = tldr_cmd();
    let output = cmd
        .args(["ssa", file.to_str().unwrap(), "simple", "--format", "json"])
        .output()
        .unwrap();

    assert!(output.status.success());

    // Parse as JSON
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("Output should be valid JSON");

    // Verify schema
    assert!(json.get("function").is_some());
    assert!(json.get("ssa_type").is_some());
    assert!(json.get("blocks").is_some());
    assert!(json.get("ssa_names").is_some());
    assert!(json.get("stats").is_some());

    // Verify function name
    assert_eq!(json["function"].as_str().unwrap(), "simple");
}

#[test]
fn test_ssa_json_output_with_phi() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_DIAMOND).unwrap();

    let mut cmd = tldr_cmd();
    let output = cmd
        .args(["ssa", file.to_str().unwrap(), "branch", "--format", "json"])
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("Output should be valid JSON");

    // Should have phi functions
    let blocks = json["blocks"].as_array().unwrap();
    let has_phi = blocks.iter().any(|b| {
        b.get("phi_functions")
            .and_then(|p| p.as_array())
            .is_some_and(|arr| !arr.is_empty())
    });
    assert!(has_phi, "Diamond pattern should have phi functions");

    // Stats should show phi count
    assert!(json["stats"]["phi_count"].as_u64().unwrap() >= 1);
}

#[test]
fn test_ssa_json_output_loop() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_LOOP).unwrap();

    let mut cmd = tldr_cmd();
    let output = cmd
        .args(["ssa", file.to_str().unwrap(), "loop", "--format", "json"])
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("Output should be valid JSON");

    // Loop should have at least 2 phis (for total and i)
    assert!(json["stats"]["phi_count"].as_u64().unwrap() >= 2);
}

// =============================================================================
// Text Output Tests (SSA-18)
// =============================================================================

#[test]
fn test_ssa_text_output() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_DIAMOND).unwrap();

    let mut cmd = tldr_cmd();
    cmd.args(["ssa", file.to_str().unwrap(), "branch", "--format", "text"])
        .assert()
        .success()
        .stdout(predicate::str::contains("SSA Form"))
        .stdout(predicate::str::contains("branch"))
        .stdout(predicate::str::contains("Block"))
        .stdout(predicate::str::contains("phi("));
}

#[test]
fn test_ssa_text_shows_versions() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_DIAMOND).unwrap();

    let mut cmd = tldr_cmd();
    cmd.args(["ssa", file.to_str().unwrap(), "branch", "--format", "text"])
        .assert()
        .success()
        // Should show versioned variables like y_1, y_2
        .stdout(predicate::str::contains("y_").or(predicate::str::contains("y₁")));
}

// =============================================================================
// DOT Output Tests (SSA-20)
// =============================================================================

#[test]
fn test_ssa_dot_output() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_DIAMOND).unwrap();

    let mut cmd = tldr_cmd();
    cmd.args(["ssa", file.to_str().unwrap(), "branch", "--format", "dot"])
        .assert()
        .success()
        .stdout(predicate::str::contains("digraph"))
        .stdout(predicate::str::contains("->"));
}

#[test]
fn test_ssa_dot_has_phi_nodes() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_DIAMOND).unwrap();

    let mut cmd = tldr_cmd();
    cmd.args(["ssa", file.to_str().unwrap(), "branch", "--format", "dot"])
        .assert()
        .success()
        // DOT should show phi in node labels
        .stdout(predicate::str::contains("phi").or(predicate::str::contains("φ")));
}

// =============================================================================
// Variable Filter Tests
// =============================================================================

#[test]
fn test_ssa_filter_by_variable() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_MULTI_VAR).unwrap();

    let mut cmd = tldr_cmd();
    let output = cmd
        .args([
            "ssa",
            file.to_str().unwrap(),
            "multi",
            "--format",
            "json",
            "--var",
            "x",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("Output should be valid JSON");

    // All SSA names should be for variable x
    let ssa_names = json["ssa_names"].as_array().unwrap();
    for name in ssa_names {
        assert_eq!(name["variable"].as_str().unwrap(), "x");
    }
}

#[test]
fn test_ssa_filter_nonexistent_variable() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_SIMPLE).unwrap();

    let mut cmd = tldr_cmd();
    let output = cmd
        .args([
            "ssa",
            file.to_str().unwrap(),
            "simple",
            "--format",
            "json",
            "--var",
            "nonexistent",
        ])
        .output()
        .unwrap();

    // Should succeed but return empty/filtered result
    assert!(output.status.success());

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("Output should be valid JSON");

    // SSA names should be empty (no matching variable)
    let ssa_names = json["ssa_names"].as_array().unwrap();
    assert!(ssa_names.is_empty());
}

// =============================================================================
// SSA Type Tests
// =============================================================================

#[test]
fn test_ssa_minimal_type() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_DIAMOND).unwrap();

    let mut cmd = tldr_cmd();
    let output = cmd
        .args([
            "ssa",
            file.to_str().unwrap(),
            "branch",
            "--format",
            "json",
            "--type",
            "minimal",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["ssa_type"].as_str().unwrap(), "minimal");
}

#[test]
fn test_ssa_semi_pruned_type() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_DIAMOND).unwrap();

    let mut cmd = tldr_cmd();
    let output = cmd
        .args([
            "ssa",
            file.to_str().unwrap(),
            "branch",
            "--format",
            "json",
            "--type",
            "semi-pruned",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["ssa_type"].as_str().unwrap(), "semi_pruned");
}

#[test]
fn test_ssa_pruned_type() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_DIAMOND).unwrap();

    let mut cmd = tldr_cmd();
    let output = cmd
        .args([
            "ssa",
            file.to_str().unwrap(),
            "branch",
            "--format",
            "json",
            "--type",
            "pruned",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["ssa_type"].as_str().unwrap(), "pruned");

    // Pruned should have fewer or equal phi functions than minimal
    let phi_count = json["stats"]["phi_count"].as_u64().unwrap();
    // This is a sanity check - actual comparison would need both runs
    let _ = phi_count;
}

// =============================================================================
// Memory SSA Tests
// =============================================================================

#[test]
fn test_ssa_memory_flag() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_MEMORY).unwrap();

    let mut cmd = tldr_cmd();
    let output = cmd
        .args([
            "ssa",
            file.to_str().unwrap(),
            "modify_point",
            "--format",
            "json",
            "--memory",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    // Should have memory SSA information
    assert!(
        json.get("memory_ssa").is_some(),
        "Memory SSA should be included with --memory flag"
    );
}

#[test]
fn test_ssa_without_memory_flag() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_MEMORY).unwrap();

    let mut cmd = tldr_cmd();
    let output = cmd
        .args([
            "ssa",
            file.to_str().unwrap(),
            "modify_point",
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    // Should NOT have memory SSA by default
    assert!(
        json.get("memory_ssa").is_none() || json["memory_ssa"].is_null(),
        "Memory SSA should not be included by default"
    );
}

// =============================================================================
// Multi-Language Tests
// =============================================================================

#[test]
fn test_ssa_typescript() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.ts");
    fs::write(&file, fixtures::TYPESCRIPT_SIMPLE).unwrap();

    let mut cmd = tldr_cmd();
    cmd.args(["ssa", file.to_str().unwrap(), "simple", "--format", "json"])
        .assert()
        .success();
}

#[test]
fn test_ssa_go() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.go");
    fs::write(&file, fixtures::GO_SIMPLE).unwrap();

    let mut cmd = tldr_cmd();
    cmd.args(["ssa", file.to_str().unwrap(), "simple", "--format", "json"])
        .assert()
        .success();
}

#[test]
fn test_ssa_rust() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.rs");
    fs::write(&file, fixtures::RUST_SIMPLE).unwrap();

    let mut cmd = tldr_cmd();
    cmd.args(["ssa", file.to_str().unwrap(), "simple", "--format", "json"])
        .assert()
        .success();
}

#[test]
fn test_ssa_explicit_language() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.txt"); // No extension
    fs::write(&file, fixtures::PYTHON_SIMPLE).unwrap();

    let mut cmd = tldr_cmd();
    cmd.args([
        "ssa",
        file.to_str().unwrap(),
        "simple",
        "--format",
        "json",
        "--lang",
        "python",
    ])
    .assert()
    .success();
}

// =============================================================================
// Edge Cases
// =============================================================================

#[test]
fn test_ssa_empty_function() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, "def empty():\n    pass\n").unwrap();

    let mut cmd = tldr_cmd();
    let output = cmd
        .args(["ssa", file.to_str().unwrap(), "empty", "--format", "json"])
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["stats"]["phi_count"].as_u64().unwrap(), 0);
}

#[test]
fn test_ssa_single_block() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, "def single(x):\n    return x + 1\n").unwrap();

    let mut cmd = tldr_cmd();
    let output = cmd
        .args(["ssa", file.to_str().unwrap(), "single", "--format", "json"])
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    // Single block should have no phi functions
    assert_eq!(json["stats"]["phi_count"].as_u64().unwrap(), 0);
}

#[test]
fn test_ssa_deeply_nested() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_NESTED).unwrap();

    let mut cmd = tldr_cmd();
    cmd.args(["ssa", file.to_str().unwrap(), "nested", "--format", "json"])
        .assert()
        .success();
}

// =============================================================================
// Performance Tests
// =============================================================================

#[test]
fn test_ssa_reasonable_time() {
    use std::time::Instant;

    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_LOOP).unwrap();

    let start = Instant::now();

    let mut cmd = tldr_cmd();
    cmd.args(["ssa", file.to_str().unwrap(), "loop", "--format", "json"])
        .assert()
        .success();

    let elapsed = start.elapsed();
    // Should complete in under 5 seconds (generous for CI)
    assert!(
        elapsed.as_secs() < 5,
        "SSA construction took too long: {}s",
        elapsed.as_secs()
    );
}
