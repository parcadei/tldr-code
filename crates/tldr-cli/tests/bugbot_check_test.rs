//! End-to-end integration tests for `bugbot check`
//!
//! Each test creates a real git repository in a temp directory, makes specific
//! changes (signature regressions, born-dead functions, etc.), runs the binary,
//! and verifies the JSON/text output and exit codes.

use std::path::{Path, PathBuf};
use std::process::Command;

// ---------------------------------------------------------------------------
// Test harness helpers
// ---------------------------------------------------------------------------

/// Get a `Command` pointing at the built `tldr` binary.
fn tldr_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_tldr"))
}

/// Create a test git repo with an initial commit so HEAD exists.
fn create_test_repo() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();

    Command::new("git")
        .args(["init"])
        .current_dir(&path)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(&path)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(&path)
        .output()
        .unwrap();

    // Initial commit so HEAD exists
    std::fs::write(path.join("init.rs"), "fn _init() {}\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(&path)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "init"])
        .current_dir(&path)
        .output()
        .unwrap();

    (dir, path)
}

/// Write a file into the repo directory.
fn write_file(dir: &Path, name: &str, content: &str) {
    std::fs::write(dir.join(name), content).unwrap();
}

/// Stage all files and commit with the given message.
fn git_add_commit(dir: &Path, message: &str) {
    Command::new("git")
        .args(["add", "."])
        .current_dir(dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(dir)
        .output()
        .unwrap();
}

/// Run `tldr --lang rust --format json bugbot check <path> [extra_args...]`
fn run_bugbot_check(path: &Path, extra_args: &[&str]) -> std::process::Output {
    tldr_bin()
        .args(["--lang", "rust", "--format", "json", "bugbot", "check"])
        .arg(path)
        .args(extra_args)
        .output()
        .expect("bugbot check failed to run")
}

/// Run `tldr --lang rust --format text bugbot check <path> [extra_args...]`
fn run_bugbot_check_text(path: &Path, extra_args: &[&str]) -> std::process::Output {
    tldr_bin()
        .args(["--lang", "rust", "--format", "text", "bugbot", "check"])
        .arg(path)
        .args(extra_args)
        .output()
        .expect("bugbot check (text) failed to run")
}

/// Parse stdout as JSON, panicking with a helpful message on failure.
fn parse_json(output: &std::process::Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "Failed to parse JSON: {}\nstdout: {}\nstderr: {}",
            e,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Commit a 2-param function, then remove one param.
/// Expect a signature-regression finding with severity "high".
#[test]
fn test_e2e_signature_regression() {
    let (_dir, path) = create_test_repo();

    // Commit a function with two parameters
    write_file(
        &path,
        "lib.rs",
        "pub fn compute(x: i32, y: i32) -> i32 {\n    x + y\n}\n",
    );
    git_add_commit(&path, "add compute");

    // Remove parameter y (signature regression)
    write_file(
        &path,
        "lib.rs",
        "pub fn compute(x: i32) -> i32 {\n    x * 2\n}\n",
    );

    let output = run_bugbot_check(&path, &["--no-fail"]);
    assert!(
        output.status.success(),
        "bugbot check should exit 0 with --no-fail, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json = parse_json(&output);
    let findings = json["findings"]
        .as_array()
        .expect("findings should be array");

    let sig_findings: Vec<&serde_json::Value> = findings
        .iter()
        .filter(|f| f["finding_type"] == "signature-regression")
        .collect();

    assert!(
        !sig_findings.is_empty(),
        "expected at least 1 signature-regression finding, got 0.\nfull output: {}",
        serde_json::to_string_pretty(&json).unwrap()
    );

    let first = &sig_findings[0];
    assert_eq!(
        first["severity"], "high",
        "signature-regression should be severity high"
    );
    assert!(
        first["function"].as_str().unwrap_or("").contains("compute"),
        "finding function should contain 'compute', got: {}",
        first["function"]
    );
}

/// Add a new function that is never called anywhere.
/// Expect a born-dead finding.
#[test]
fn test_e2e_born_dead() {
    let (_dir, path) = create_test_repo();

    // Commit a simple program
    write_file(
        &path,
        "lib.rs",
        "fn main() {\n    helper();\n}\n\nfn helper() -> i32 {\n    42\n}\n",
    );
    git_add_commit(&path, "add main and helper");

    // Add a new function that is never called
    write_file(
        &path,
        "lib.rs",
        "fn main() {\n    helper();\n}\n\nfn helper() -> i32 {\n    42\n}\n\nfn unused_func() -> bool {\n    true\n}\n",
    );

    let output = run_bugbot_check(&path, &["--no-fail"]);
    assert!(
        output.status.success(),
        "bugbot check should exit 0 with --no-fail, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json = parse_json(&output);
    let findings = json["findings"]
        .as_array()
        .expect("findings should be array");

    let dead_findings: Vec<&serde_json::Value> = findings
        .iter()
        .filter(|f| f["finding_type"] == "born-dead")
        .collect();

    assert!(
        !dead_findings.is_empty(),
        "expected at least 1 born-dead finding, got 0.\nfull output: {}",
        serde_json::to_string_pretty(&json).unwrap()
    );

    assert!(
        dead_findings[0]["function"]
            .as_str()
            .unwrap_or("")
            .contains("unused_func"),
        "born-dead finding should reference 'unused_func', got: {}",
        dead_findings[0]["function"]
    );
}

/// No uncommitted changes => empty findings, "no_changes_detected" note.
#[test]
fn test_e2e_no_changes() {
    let (_dir, path) = create_test_repo();

    // Commit some real code, then make NO further changes
    write_file(
        &path,
        "lib.rs",
        "pub fn stable(x: i32) -> i32 {\n    x\n}\n",
    );
    git_add_commit(&path, "add stable function");

    let output = run_bugbot_check(&path, &[]);
    assert!(
        output.status.success(),
        "bugbot check with no changes should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json = parse_json(&output);
    let findings = json["findings"]
        .as_array()
        .expect("findings should be array");
    assert!(
        findings.is_empty(),
        "expected 0 findings when no changes, got {}",
        findings.len()
    );

    let notes = json["notes"].as_array().expect("notes should be array");
    assert!(
        notes.iter().any(|n| n == "no_changes_detected"),
        "notes should contain 'no_changes_detected', got: {:?}",
        notes
    );
}

/// Add a brand new file with functions. Should not crash and should report it
/// in changed_files. The file must be staged for git to detect it as a change.
#[test]
fn test_e2e_new_file() {
    let (_dir, path) = create_test_repo();

    // Commit initial state
    write_file(&path, "lib.rs", "fn existing() {}\n");
    git_add_commit(&path, "initial code");

    // Add a brand new file and stage it (git diff --staged will see it)
    write_file(
        &path,
        "extra.rs",
        "pub fn new_helper() -> u32 {\n    99\n}\n\nfn another() -> bool {\n    false\n}\n",
    );
    Command::new("git")
        .args(["add", "extra.rs"])
        .current_dir(&path)
        .output()
        .unwrap();

    let output = run_bugbot_check(&path, &["--no-fail", "--staged"]);
    assert!(
        output.status.success(),
        "bugbot check should not crash on new files, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json = parse_json(&output);

    // Verify changed_files contains the new file
    let changed_files = json["changed_files"]
        .as_array()
        .expect("changed_files should be array");
    let has_extra = changed_files
        .iter()
        .any(|f| f.as_str().unwrap_or("").contains("extra.rs"));
    assert!(
        has_extra,
        "changed_files should contain extra.rs, got: {:?}",
        changed_files
    );
}

/// When findings exist and --no-fail is NOT set, exit code should be non-zero.
#[test]
fn test_e2e_exit_code_with_findings() {
    let (_dir, path) = create_test_repo();

    // Commit a function, then break its signature
    write_file(
        &path,
        "lib.rs",
        "pub fn compute(x: i32, y: i32) -> i32 {\n    x + y\n}\n",
    );
    git_add_commit(&path, "add compute");

    write_file(
        &path,
        "lib.rs",
        "pub fn compute(x: i32) -> i32 {\n    x * 2\n}\n",
    );

    // Run WITHOUT --no-fail
    let output = run_bugbot_check(&path, &[]);
    assert!(
        !output.status.success(),
        "bugbot check should exit non-zero when findings exist (without --no-fail)"
    );
}

/// When findings exist but --no-fail is set, exit code should be 0 and findings
/// should still be present.
#[test]
fn test_e2e_no_fail_flag() {
    let (_dir, path) = create_test_repo();

    write_file(
        &path,
        "lib.rs",
        "pub fn compute(x: i32, y: i32) -> i32 {\n    x + y\n}\n",
    );
    git_add_commit(&path, "add compute");

    write_file(
        &path,
        "lib.rs",
        "pub fn compute(x: i32) -> i32 {\n    x * 2\n}\n",
    );

    let output = run_bugbot_check(&path, &["--no-fail"]);
    assert!(
        output.status.success(),
        "bugbot check with --no-fail should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json = parse_json(&output);
    let findings = json["findings"]
        .as_array()
        .expect("findings should be array");
    assert!(
        !findings.is_empty(),
        "findings should still be present with --no-fail"
    );
}

/// Verify that the JSON output has ALL required top-level fields.
#[test]
fn test_e2e_json_schema() {
    let (_dir, path) = create_test_repo();

    // Make a simple change so there is something to analyze
    write_file(
        &path,
        "lib.rs",
        "pub fn foo(x: i32) -> i32 {\n    x + 1\n}\n",
    );
    git_add_commit(&path, "add foo");

    write_file(
        &path,
        "lib.rs",
        "pub fn foo(x: i32) -> i32 {\n    x + 2\n}\n",
    );

    let output = run_bugbot_check(&path, &["--no-fail"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json = parse_json(&output);

    // All required top-level fields
    let required_fields = [
        "tool",
        "mode",
        "language",
        "base_ref",
        "detection_method",
        "timestamp",
        "changed_files",
        "findings",
        "summary",
        "elapsed_ms",
        "errors",
        "notes",
    ];

    for field in &required_fields {
        assert!(
            !json[field].is_null(),
            "required field '{}' is missing from output JSON.\nFull output: {}",
            field,
            serde_json::to_string_pretty(&json).unwrap()
        );
    }

    // Type checks
    assert_eq!(json["tool"], "bugbot");
    assert_eq!(json["mode"], "check");
    assert!(json["language"].is_string());
    assert!(json["base_ref"].is_string());
    assert!(json["detection_method"].is_string());
    assert!(json["timestamp"].is_string());
    assert!(json["changed_files"].is_array());
    assert!(json["findings"].is_array());
    assert!(json["summary"].is_object());
    assert!(json["elapsed_ms"].is_number());
    assert!(json["errors"].is_array());
    assert!(json["notes"].is_array());

    // Summary sub-fields
    let summary = &json["summary"];
    assert!(summary["total_findings"].is_number());
    assert!(summary["by_severity"].is_object());
    assert!(summary["by_type"].is_object());
    assert!(summary["files_analyzed"].is_number());
    assert!(summary["functions_analyzed"].is_number());
}

/// Create many functions with signature changes, then limit with --max-findings.
#[test]
fn test_e2e_max_findings() {
    let (_dir, path) = create_test_repo();

    // Commit many functions with 2 params each
    let mut original = String::new();
    for i in 0..10 {
        original.push_str(&format!(
            "pub fn func_{i}(a: i32, b: i32) -> i32 {{\n    a + b + {i}\n}}\n\n"
        ));
    }
    write_file(&path, "lib.rs", &original);
    git_add_commit(&path, "add many functions");

    // Change all signatures (remove param b)
    let mut modified = String::new();
    for i in 0..10 {
        modified.push_str(&format!(
            "pub fn func_{i}(a: i32) -> i32 {{\n    a + {i}\n}}\n\n"
        ));
    }
    write_file(&path, "lib.rs", &modified);

    let output = run_bugbot_check(&path, &["--no-fail", "--max-findings", "2"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json = parse_json(&output);
    let findings = json["findings"]
        .as_array()
        .expect("findings should be array");

    assert!(
        findings.len() <= 2,
        "expected at most 2 findings with --max-findings 2, got {}",
        findings.len()
    );
}

/// Changing only the function body (not the signature) should NOT produce a
/// signature-regression finding.
#[test]
fn test_e2e_body_only_change_no_regression() {
    let (_dir, path) = create_test_repo();

    write_file(&path, "lib.rs", "fn foo(x: i32) -> bool {\n    x > 0\n}\n");
    git_add_commit(&path, "add foo");

    // Only change the body, keep signature identical
    write_file(&path, "lib.rs", "fn foo(x: i32) -> bool {\n    x > 1\n}\n");

    let output = run_bugbot_check(&path, &["--no-fail"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json = parse_json(&output);
    let findings = json["findings"]
        .as_array()
        .expect("findings should be array");

    let sig_regression_findings: Vec<&serde_json::Value> = findings
        .iter()
        .filter(|f| f["finding_type"] == "signature-regression")
        .collect();

    assert!(
        sig_regression_findings.is_empty(),
        "body-only change should produce 0 signature-regression findings, got {}.\nfindings: {}",
        sig_regression_findings.len(),
        serde_json::to_string_pretty(findings).unwrap()
    );
}

/// With --format text, output should be human-readable text, not JSON.
#[test]
fn test_e2e_text_format() {
    let (_dir, path) = create_test_repo();

    write_file(&path, "lib.rs", "fn bar() -> i32 {\n    1\n}\n");
    git_add_commit(&path, "add bar");

    // Make a body-only change (just to have something to analyze)
    write_file(&path, "lib.rs", "fn bar() -> i32 {\n    2\n}\n");

    let output = run_bugbot_check_text(&path, &["--no-fail"]);
    assert!(
        output.status.success(),
        "text format should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should contain the summary line
    assert!(
        stdout.contains("bugbot check --"),
        "text output should contain 'bugbot check --' summary line, got:\n{}",
        stdout
    );

    // Should NOT be valid JSON
    let json_result: Result<serde_json::Value, _> = serde_json::from_slice(&output.stdout);
    assert!(
        json_result.is_err(),
        "text format output should NOT be valid JSON"
    );
}

/// Run bugbot check against the actual tldr codebase.
/// This is a smoke test: it should not panic and should produce valid JSON.
#[test]
#[ignore] // slow and environment-dependent
fn test_e2e_dogfood_no_crash() {
    let codebase =
        PathBuf::from("/Users/cosimo/.opc-dev/opc/packages/tldr-code/tldr-rs-v2-canonical");

    let output = tldr_bin()
        .args([
            "--lang",
            "rust",
            "--format",
            "json",
            "bugbot",
            "check",
            "--no-fail",
        ])
        .arg(&codebase)
        .output()
        .expect("bugbot check on codebase failed to run");

    assert!(
        output.status.success(),
        "dogfood run should exit 0 with --no-fail, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json = parse_json(&output);
    assert!(
        json["elapsed_ms"].is_number(),
        "elapsed_ms should be present and numeric"
    );
    assert!(
        json["findings"].is_array(),
        "findings should be an array in dogfood output"
    );
}

/// Exit code should be 1 (not 0, not other) when findings exist without --no-fail.
/// This verifies the process::exit(1) replacement with proper error propagation.
#[test]
fn test_e2e_exit_code_is_1_for_findings() {
    let (_dir, path) = create_test_repo();

    write_file(
        &path,
        "lib.rs",
        "pub fn compute(x: i32, y: i32) -> i32 {\n    x + y\n}\n",
    );
    git_add_commit(&path, "add compute");

    write_file(
        &path,
        "lib.rs",
        "pub fn compute(x: i32) -> i32 {\n    x * 2\n}\n",
    );

    let output = run_bugbot_check(&path, &[]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "exit code should be exactly 1 when findings exist, got: {:?}",
        output.status.code()
    );
}

/// --max-findings 0 should report all findings (unlimited).
#[test]
fn test_e2e_max_findings_zero_unlimited() {
    let (_dir, path) = create_test_repo();

    // Commit 5 functions with 2 params each
    let mut original = String::new();
    for i in 0..5 {
        original.push_str(&format!(
            "pub fn func_{i}(a: i32, b: i32) -> i32 {{\n    a + b + {i}\n}}\n\n"
        ));
    }
    write_file(&path, "lib.rs", &original);
    git_add_commit(&path, "add functions");

    // Remove a parameter from all 5 functions
    let mut modified = String::new();
    for i in 0..5 {
        modified.push_str(&format!(
            "pub fn func_{i}(a: i32) -> i32 {{\n    a + {i}\n}}\n\n"
        ));
    }
    write_file(&path, "lib.rs", &modified);

    let output = run_bugbot_check(&path, &["--no-fail", "--max-findings", "0"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json = parse_json(&output);
    let findings = json["findings"]
        .as_array()
        .expect("findings should be array");

    // All 5 signature regressions should be reported (no truncation)
    assert!(
        findings.len() >= 5,
        "expected at least 5 findings with --max-findings 0 (unlimited), got {}",
        findings.len()
    );

    // Should not have a truncation note
    let notes = json["notes"].as_array().expect("notes should be array");
    let has_truncation = notes
        .iter()
        .any(|n| n.as_str().unwrap_or("").starts_with("truncated_to_"));
    assert!(
        !has_truncation,
        "should not have truncation note with --max-findings 0, got: {:?}",
        notes
    );
}
