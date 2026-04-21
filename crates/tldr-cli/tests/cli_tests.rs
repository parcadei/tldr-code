//! CLI integration tests
//!
//! Tests verify:
//! - All commands execute without error
//! - Help text is correct
//! - JSON output is valid
//! - Exit codes are correct

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
// Help and Version Tests
// =============================================================================

#[test]
fn test_help_text() {
    let mut cmd = tldr_cmd();
    cmd.arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "TLDR provides code analysis commands",
        ))
        .stdout(predicate::str::contains("tree"))
        .stdout(predicate::str::contains("structure"))
        .stdout(predicate::str::contains("calls"))
        .stdout(predicate::str::contains("impact"))
        .stdout(predicate::str::contains("dead"))
        .stdout(predicate::str::contains("cfg"))
        .stdout(predicate::str::contains("dfg"))
        .stdout(predicate::str::contains("slice"))
        .stdout(predicate::str::contains("search"))
        .stdout(predicate::str::contains("context"))
        .stdout(predicate::str::contains("smells"))
        .stdout(predicate::str::contains("secrets"));
}

#[test]
fn test_version() {
    let mut cmd = tldr_cmd();
    cmd.arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("tldr"));
}

// =============================================================================
// Tree Command Tests
// =============================================================================

#[test]
fn test_tree_json_output() {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join("subdir")).unwrap();
    fs::write(temp.path().join("file1.py"), "# test").unwrap();
    fs::write(temp.path().join("subdir/file2.py"), "# test2").unwrap();

    let mut cmd = tldr_cmd();
    cmd.args(["tree", temp.path().to_str().unwrap(), "-q"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"type\""))
        .stdout(predicate::str::contains("\"children\""));
}

#[test]
fn test_tree_with_extension_filter() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("file.py"), "# python").unwrap();
    fs::write(temp.path().join("file.rs"), "// rust").unwrap();

    let mut cmd = tldr_cmd();
    cmd.args(["tree", temp.path().to_str().unwrap(), "--ext", ".py", "-q"])
        .assert()
        .success()
        .stdout(predicate::str::contains("file.py"))
        .stdout(predicate::str::contains("file.rs").not());
}

#[test]
fn test_tree_nonexistent_path() {
    let mut cmd = tldr_cmd();
    cmd.args(["tree", "/nonexistent/path/that/does/not/exist", "-q"])
        .assert()
        .failure();
}

// =============================================================================
// Structure Command Tests
// =============================================================================

#[test]
fn test_structure_json_output() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("test.py"),
        r#"
def foo():
    pass

class Bar:
    def method(self):
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
    ])
    .assert()
    .success()
    .stdout(predicate::str::contains("\"functions\""))
    .stdout(predicate::str::contains("\"classes\""));
}

// =============================================================================
// Search Command Tests
// =============================================================================

#[test]
fn test_search_finds_pattern() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("test.py"),
        r#"
def find_me():
    pass

def another():
    pass
"#,
    )
    .unwrap();

    let mut cmd = tldr_cmd();
    cmd.args(["search", "find_me", temp.path().to_str().unwrap(), "-q"])
        .assert()
        .success()
        .stdout(predicate::str::contains("find_me"));
}

#[test]
fn test_search_invalid_regex() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("test.py"), "content").unwrap();

    let mut cmd = tldr_cmd();
    cmd.args(["search", "[invalid", temp.path().to_str().unwrap(), "-q"])
        .assert()
        .failure();
}

// =============================================================================
// Output Format Tests
// =============================================================================

#[test]
fn test_json_format() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("test.py"), "# test").unwrap();

    let mut cmd = tldr_cmd();
    let output = cmd
        .args(["tree", temp.path().to_str().unwrap(), "-f", "json", "-q"])
        .output()
        .unwrap();

    // Should be valid JSON
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json.is_object());
}

#[test]
fn test_compact_format() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("test.py"), "# test").unwrap();

    let mut cmd = tldr_cmd();
    let output = cmd
        .args(["tree", temp.path().to_str().unwrap(), "-f", "compact", "-q"])
        .output()
        .unwrap();

    // Compact output should not have newlines within the JSON (only at the end)
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "Compact JSON should be single line");
}

#[test]
fn test_text_format() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("test.py"), "# test").unwrap();

    let mut cmd = tldr_cmd();
    cmd.args(["tree", temp.path().to_str().unwrap(), "-f", "text", "-q"])
        .assert()
        .success();
}

// =============================================================================
// Alias Tests
// =============================================================================

#[test]
fn test_tree_alias() {
    let mut cmd = tldr_cmd();
    cmd.args(["t", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("file tree"));
}

#[test]
fn test_structure_alias() {
    let mut cmd = tldr_cmd();
    cmd.args(["s", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("code structure"));
}

#[test]
fn test_calls_alias() {
    let mut cmd = tldr_cmd();
    cmd.args(["c", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("call graph"));
}

// =============================================================================
// CFG/DFG/Slice Tests (require actual Python files with functions)
// =============================================================================

#[test]
fn test_cfg_command_help() {
    let mut cmd = tldr_cmd();
    cmd.args(["cfg", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("control flow graph"));
}

#[test]
fn test_dfg_command_help() {
    let mut cmd = tldr_cmd();
    cmd.args(["dfg", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("data flow graph"));
}

#[test]
fn test_slice_command_help() {
    let mut cmd = tldr_cmd();
    cmd.args(["slice", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("program slice"));
}

// =============================================================================
// Quality/Security Command Tests
// =============================================================================

#[test]
fn test_smells_command() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

    let mut cmd = tldr_cmd();
    cmd.args(["smells", temp.path().to_str().unwrap(), "-q"])
        .assert()
        .success()
        .stdout(predicate::str::contains("smells"));
}

#[test]
fn test_secrets_command() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("test.py"), "# no secrets here").unwrap();

    let mut cmd = tldr_cmd();
    cmd.args(["secrets", temp.path().to_str().unwrap(), "-q"])
        .assert()
        .success()
        .stdout(predicate::str::contains("findings"));
}

// =============================================================================
// Cold Start Performance Test (M15 Mitigation)
// =============================================================================

#[test]
fn test_cold_start_performance() {
    use std::time::Instant;

    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("test.py"), "# test").unwrap();

    let start = Instant::now();
    let mut cmd = tldr_cmd();
    cmd.args(["tree", temp.path().to_str().unwrap(), "-q"])
        .assert()
        .success();
    let elapsed = start.elapsed();

    // Should complete in under 1 second (generous for CI)
    // Target is <100ms but allow for CI overhead
    assert!(
        elapsed.as_millis() < 1000,
        "Cold start took {}ms, expected <1000ms",
        elapsed.as_millis()
    );
}
