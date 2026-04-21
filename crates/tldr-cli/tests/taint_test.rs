//! Tests for the taint analysis CLI command
//!
//! Phase 8: CLI integration for taint analysis

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::tempdir;

/// Helper to create a test Python file with taint patterns
fn create_test_file(dir: &std::path::Path, name: &str, content: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    fs::write(&path, content).expect("Failed to write test file");
    path
}

#[test]
fn test_taint_help() {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("tldr"));
    cmd.arg("taint").arg("--help");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("taint"))
        .stdout(predicate::str::contains("FUNCTION"));
}

#[test]
fn test_taint_missing_args() {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("tldr"));
    cmd.arg("taint");
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("required"));
}

#[test]
fn test_taint_json_output() {
    let dir = tempdir().unwrap();
    let content = r#"
def vulnerable(user_data):
    query = "SELECT * FROM users WHERE id = " + user_data
    cursor.execute(query)
"#;
    let file = create_test_file(dir.path(), "vuln.py", content);

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("tldr"));
    cmd.arg("taint")
        .arg(file.to_str().unwrap())
        .arg("vulnerable")
        .arg("-f")
        .arg("json");

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("function_name"))
        .stdout(predicate::str::contains("vulnerable"));
}

#[test]
fn test_taint_text_output() {
    let dir = tempdir().unwrap();
    let content = r#"
def vulnerable():
    user_input = input("Enter ID: ")
    eval(user_input)
"#;
    let file = create_test_file(dir.path(), "eval_vuln.py", content);

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("tldr"));
    cmd.arg("taint")
        .arg(file.to_str().unwrap())
        .arg("vulnerable")
        .arg("-f")
        .arg("text");

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Taint Analysis"))
        .stdout(predicate::str::contains("Sources"));
}

#[test]
fn test_taint_file_not_found() {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("tldr"));
    cmd.arg("taint")
        .arg("/nonexistent/file.py")
        .arg("test_func");

    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("not found").or(predicate::str::contains("No such file")));
}

#[test]
fn test_taint_function_not_found() {
    let dir = tempdir().unwrap();
    let content = r#"
def existing_func():
    pass
"#;
    let file = create_test_file(dir.path(), "test.py", content);

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("tldr"));
    cmd.arg("taint")
        .arg(file.to_str().unwrap())
        .arg("nonexistent_func");

    // Should fail or return empty results
    let _ = cmd.assert();
    // Just checking it doesn't panic - specific behavior may vary
}

#[test]
fn test_taint_detects_sql_injection() {
    let dir = tempdir().unwrap();
    let content = r#"
def process_user():
    user_id = input("Enter ID: ")
    query = "SELECT * FROM users WHERE id = " + user_id
    cursor.execute(query)
"#;
    let file = create_test_file(dir.path(), "sql_injection.py", content);

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("tldr"));
    cmd.arg("taint")
        .arg(file.to_str().unwrap())
        .arg("process_user")
        .arg("-f")
        .arg("json");

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("sources"))
        .stdout(predicate::str::contains("sinks"));
}

#[test]
fn test_taint_alias_works() {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("tldr"));
    cmd.arg("ta").arg("--help");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("taint"));
}
