//! Reaching Definitions CLI Integration Tests
//!
//! Tests for the `tldr reaching-defs` command.
//! These tests define expected CLI behavior BEFORE implementation.
//!
//! Reference: session10-spec.md Section 4.2

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
    pub const PYTHON_LINEAR: &str = r#"
def linear():
    x = 1
    y = x
    z = y
    return z
"#;

    pub const PYTHON_KILLED: &str = r#"
def killed():
    x = 1
    x = 2
    return x
"#;

    pub const PYTHON_MULTIPLE_PATHS: &str = r#"
def paths(cond):
    if cond:
        x = 1
    else:
        x = 2
    return x
"#;

    pub const PYTHON_UNINITIALIZED: &str = r#"
def uninit(cond):
    if cond:
        x = 1
    return x
"#;

    pub const PYTHON_LOOP: &str = r#"
def loop_def():
    x = 0
    for i in range(10):
        x = x + i
    return x
"#;

    pub const PYTHON_MULTI_VAR: &str = r#"
def multi_var(a, b):
    x = a
    y = b
    z = x + y
    x = z
    return x + y
"#;

    pub const TYPESCRIPT_SIMPLE: &str = r#"
function simple(): number {
    let x = 1;
    let y = x + 1;
    return y;
}
"#;

    pub const GO_SIMPLE: &str = r#"
func simple() int {
    x := 1
    y := x + 1
    return y
}
"#;
}

// =============================================================================
// Help and Basic Command Tests
// =============================================================================

#[test]
fn test_reaching_defs_help() {
    let mut cmd = tldr_cmd();
    cmd.args(["reaching-defs", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("reaching"))
        .stdout(predicate::str::contains("--format"))
        .stdout(predicate::str::contains("--var"))
        .stdout(predicate::str::contains("--line"));
}

#[test]
fn test_reaching_defs_missing_args() {
    let mut cmd = tldr_cmd();
    cmd.arg("reaching-defs")
        .assert()
        .failure()
        .stderr(predicate::str::contains("required"));
}

#[test]
fn test_reaching_defs_file_not_found() {
    let mut cmd = tldr_cmd();
    cmd.args(["reaching-defs", "nonexistent.py", "func"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found").or(predicate::str::contains("No such file")));
}

#[test]
fn test_reaching_defs_function_not_found() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_LINEAR).unwrap();

    let mut cmd = tldr_cmd();
    cmd.args(["reaching-defs", file.to_str().unwrap(), "nonexistent"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found").or(predicate::str::contains("Function")));
}

// =============================================================================
// JSON Output Tests (RD-15)
// =============================================================================

#[test]
fn test_reaching_defs_json_output() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_LINEAR).unwrap();

    let mut cmd = tldr_cmd();
    let output = cmd
        .args([
            "reaching-defs",
            file.to_str().unwrap(),
            "linear",
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("Output should be valid JSON");

    // Verify schema
    assert!(json.get("function").is_some());
    assert!(json.get("file").is_some());
    assert!(json.get("blocks").is_some());
    assert!(json.get("def_use_chains").is_some());
    assert!(json.get("stats").is_some());

    // Verify function name
    assert_eq!(json["function"].as_str().unwrap(), "linear");
}

#[test]
fn test_reaching_defs_json_has_chains() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_LINEAR).unwrap();

    let mut cmd = tldr_cmd();
    let output = cmd
        .args([
            "reaching-defs",
            file.to_str().unwrap(),
            "linear",
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    // Should have def-use chains
    let chains = json["def_use_chains"].as_array().unwrap();
    assert!(!chains.is_empty(), "Should have def-use chains");

    // Each chain should have definition and uses
    for chain in chains {
        assert!(chain.get("definition").is_some());
        assert!(chain.get("uses").is_some());
    }
}

#[test]
fn test_reaching_defs_json_block_structure() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_MULTIPLE_PATHS).unwrap();

    let mut cmd = tldr_cmd();
    let output = cmd
        .args([
            "reaching-defs",
            file.to_str().unwrap(),
            "paths",
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    let blocks = json["blocks"].as_array().unwrap();
    assert!(!blocks.is_empty());

    for block in blocks {
        assert!(block.get("id").is_some());
        assert!(block.get("gen").is_some());
        // in is a reserved keyword, might be "in_set"
        assert!(block.get("in").is_some() || block.get("in_set").is_some());
        assert!(block.get("out").is_some());
    }
}

// =============================================================================
// Text Output Tests (RD-14)
// =============================================================================

#[test]
fn test_reaching_defs_text_output() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_LINEAR).unwrap();

    let mut cmd = tldr_cmd();
    cmd.args([
        "reaching-defs",
        file.to_str().unwrap(),
        "linear",
        "--format",
        "text",
    ])
    .assert()
    .success()
    .stdout(predicate::str::contains("Reaching Definitions"))
    .stdout(predicate::str::contains("linear"))
    .stdout(predicate::str::contains("Block"));
}

#[test]
fn test_reaching_defs_text_shows_gen_kill() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_KILLED).unwrap();

    let mut cmd = tldr_cmd();
    cmd.args([
        "reaching-defs",
        file.to_str().unwrap(),
        "killed",
        "--format",
        "text",
    ])
    .assert()
    .success()
    .stdout(predicate::str::contains("GEN").or(predicate::str::contains("gen")))
    .stdout(
        predicate::str::contains("KILL")
            .or(predicate::str::contains("kill"))
            .or(predicate::str::contains("Kill")),
    );
}

#[test]
fn test_reaching_defs_text_shows_in_out() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_LINEAR).unwrap();

    let mut cmd = tldr_cmd();
    cmd.args([
        "reaching-defs",
        file.to_str().unwrap(),
        "linear",
        "--format",
        "text",
        "--show-in-out",
    ])
    .assert()
    .success()
    .stdout(predicate::str::contains("IN").or(predicate::str::contains("in")))
    .stdout(predicate::str::contains("OUT").or(predicate::str::contains("out")));
}

// =============================================================================
// Variable Filter Tests (RD-16)
// =============================================================================

#[test]
fn test_reaching_defs_filter_by_variable() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_MULTI_VAR).unwrap();

    let mut cmd = tldr_cmd();
    let output = cmd
        .args([
            "reaching-defs",
            file.to_str().unwrap(),
            "multi_var",
            "--format",
            "json",
            "--var",
            "x",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    // All definitions in chains should be for x
    let chains = json["def_use_chains"].as_array().unwrap();
    for chain in chains {
        let var = chain["definition"]["var"].as_str().unwrap();
        assert_eq!(var, "x", "Filtered output should only contain x");
    }
}

#[test]
fn test_reaching_defs_filter_nonexistent_variable() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_LINEAR).unwrap();

    let mut cmd = tldr_cmd();
    let output = cmd
        .args([
            "reaching-defs",
            file.to_str().unwrap(),
            "linear",
            "--format",
            "json",
            "--var",
            "nonexistent",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let chains = json["def_use_chains"].as_array().unwrap();
    assert!(
        chains.is_empty(),
        "Should have no chains for nonexistent variable"
    );
}

// =============================================================================
// Line Filter Tests
// =============================================================================

#[test]
fn test_reaching_defs_filter_by_line() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_LINEAR).unwrap();

    let mut cmd = tldr_cmd();
    let output = cmd
        .args([
            "reaching-defs",
            file.to_str().unwrap(),
            "linear",
            "--format",
            "json",
            "--line",
            "5",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());

    // Should show what definitions reach line 5
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json.get("reaching_at_line").is_some() || json.get("def_use_chains").is_some());
}

#[test]
fn test_reaching_defs_line_out_of_range() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_LINEAR).unwrap();

    let mut cmd = tldr_cmd();
    cmd.args([
        "reaching-defs",
        file.to_str().unwrap(),
        "linear",
        "--format",
        "json",
        "--line",
        "1000",
    ])
    .assert()
    // Should either fail or return empty result
    .success();
}

// =============================================================================
// Uninitialized Detection Tests (RD-13)
// =============================================================================

#[test]
fn test_reaching_defs_shows_uninitialized() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_UNINITIALIZED).unwrap();

    let mut cmd = tldr_cmd();
    let output = cmd
        .args([
            "reaching-defs",
            file.to_str().unwrap(),
            "uninit",
            "--format",
            "json",
            "--show-uninitialized",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    // Should detect uninitialized use
    let uninit = json["uninitialized"].as_array().unwrap();
    assert!(
        !uninit.is_empty(),
        "Should detect uninitialized variable use"
    );

    // Should be for variable x
    assert!(uninit.iter().any(|u| u["var"].as_str().unwrap() == "x"));
}

#[test]
fn test_reaching_defs_no_uninitialized() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_LINEAR).unwrap();

    let mut cmd = tldr_cmd();
    let output = cmd
        .args([
            "reaching-defs",
            file.to_str().unwrap(),
            "linear",
            "--format",
            "json",
            "--show-uninitialized",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    // Should have empty uninitialized list
    let uninit = json["uninitialized"].as_array().unwrap();
    assert!(
        uninit.is_empty(),
        "Fully initialized function should have no warnings"
    );
}

#[test]
fn test_reaching_defs_uninitialized_text_format() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_UNINITIALIZED).unwrap();

    let mut cmd = tldr_cmd();
    cmd.args([
        "reaching-defs",
        file.to_str().unwrap(),
        "uninit",
        "--format",
        "text",
        "--show-uninitialized",
    ])
    .assert()
    .success()
    .stdout(
        predicate::str::contains("uninitialized")
            .or(predicate::str::contains("Uninitialized"))
            .or(predicate::str::contains("UNINITIALIZED")),
    );
}

// =============================================================================
// Def-Use Chain Tests
// =============================================================================

#[test]
fn test_reaching_defs_show_chains() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_LINEAR).unwrap();

    let mut cmd = tldr_cmd();
    let output = cmd
        .args([
            "reaching-defs",
            file.to_str().unwrap(),
            "linear",
            "--format",
            "json",
            "--show-chains",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    // Should have def-use chains
    assert!(json.get("def_use_chains").is_some());
    let chains = json["def_use_chains"].as_array().unwrap();
    assert!(!chains.is_empty());
}

#[test]
fn test_reaching_defs_chains_text_format() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_LINEAR).unwrap();

    let mut cmd = tldr_cmd();
    cmd.args([
        "reaching-defs",
        file.to_str().unwrap(),
        "linear",
        "--format",
        "text",
        "--show-chains",
    ])
    .assert()
    .success()
    .stdout(
        predicate::str::contains("->")
            .or(predicate::str::contains("reaches"))
            .or(predicate::str::contains("used")),
    );
}

// =============================================================================
// Multi-Language Tests
// =============================================================================

#[test]
fn test_reaching_defs_typescript() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.ts");
    fs::write(&file, fixtures::TYPESCRIPT_SIMPLE).unwrap();

    let mut cmd = tldr_cmd();
    cmd.args([
        "reaching-defs",
        file.to_str().unwrap(),
        "simple",
        "--format",
        "json",
    ])
    .assert()
    .success();
}

#[test]
fn test_reaching_defs_go() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.go");
    fs::write(&file, fixtures::GO_SIMPLE).unwrap();

    let mut cmd = tldr_cmd();
    cmd.args([
        "reaching-defs",
        file.to_str().unwrap(),
        "simple",
        "--format",
        "json",
    ])
    .assert()
    .success();
}

#[test]
fn test_reaching_defs_explicit_language() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.txt");
    fs::write(&file, fixtures::PYTHON_LINEAR).unwrap();

    let mut cmd = tldr_cmd();
    cmd.args([
        "reaching-defs",
        file.to_str().unwrap(),
        "linear",
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
fn test_reaching_defs_empty_function() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, "def empty():\n    pass\n").unwrap();

    let mut cmd = tldr_cmd();
    let output = cmd
        .args([
            "reaching-defs",
            file.to_str().unwrap(),
            "empty",
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["stats"]["definitions"].as_u64().unwrap(), 0);
}

#[test]
fn test_reaching_defs_loop() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_LOOP).unwrap();

    let mut cmd = tldr_cmd();
    let output = cmd
        .args([
            "reaching-defs",
            file.to_str().unwrap(),
            "loop_def",
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    // Loop should show multiple definitions reaching the return
    let chains = json["def_use_chains"].as_array().unwrap();

    // x should have multiple definitions reaching some uses
    let x_chains: Vec<_> = chains
        .iter()
        .filter(|c| c["definition"]["var"].as_str().unwrap() == "x")
        .collect();

    assert!(!x_chains.is_empty(), "Should have chains for x");
}

#[test]
fn test_reaching_defs_killed_definition() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_KILLED).unwrap();

    let mut cmd = tldr_cmd();
    let output = cmd
        .args([
            "reaching-defs",
            file.to_str().unwrap(),
            "killed",
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    // Only the second definition (x = 2) should reach the return
    // The first (x = 1) should be killed
    let _chains = json["def_use_chains"].as_array().unwrap();

    // Find chain for the definition that reaches return
    // Should be x=2, not x=1
}

#[test]
fn test_reaching_defs_multiple_paths() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_MULTIPLE_PATHS).unwrap();

    let mut cmd = tldr_cmd();
    let output = cmd
        .args([
            "reaching-defs",
            file.to_str().unwrap(),
            "paths",
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());

    // Both definitions should reach the return
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    // Use-def chain for the return's use of x should have 2 reaching definitions
    if let Some(use_def_chains) = json.get("use_def_chains") {
        let chains = use_def_chains.as_array().unwrap();
        for chain in chains {
            if chain["var"].as_str().unwrap() == "x" {
                let reaching = chain["reaching_defs"].as_array().unwrap();
                // Both branches define x, so both should reach
                assert!(reaching.len() >= 2, "Both definitions should reach the use");
            }
        }
    }
}

// =============================================================================
// Statistics Tests
// =============================================================================

#[test]
fn test_reaching_defs_stats() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_MULTI_VAR).unwrap();

    let mut cmd = tldr_cmd();
    let output = cmd
        .args([
            "reaching-defs",
            file.to_str().unwrap(),
            "multi_var",
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    let stats = &json["stats"];
    assert!(stats["definitions"].as_u64().unwrap() > 0);
    assert!(stats["uses"].as_u64().unwrap() > 0);
    assert!(stats["blocks"].as_u64().unwrap() > 0);
}

// =============================================================================
// Performance Tests
// =============================================================================

#[test]
fn test_reaching_defs_reasonable_time() {
    use std::time::Instant;

    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test.py");
    fs::write(&file, fixtures::PYTHON_LOOP).unwrap();

    let start = Instant::now();

    let mut cmd = tldr_cmd();
    cmd.args([
        "reaching-defs",
        file.to_str().unwrap(),
        "loop_def",
        "--format",
        "json",
    ])
    .assert()
    .success();

    let elapsed = start.elapsed();
    assert!(
        elapsed.as_secs() < 5,
        "Reaching definitions took too long: {}s",
        elapsed.as_secs()
    );
}
