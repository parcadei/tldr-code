//! Doctor command tests
//!
//! Tests verify:
//! - DoctorArgs struct defaults
//! - Tool detection (finds cargo on dev machine)
//! - JSON output structure
//! - Install mode error handling
//! - Text output formatting

use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::process::Command;

/// Get the path to the test binary
fn tldr_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
}

// =============================================================================
// DoctorArgs Struct Tests
// =============================================================================

#[test]
fn test_doctor_args_default() {
    // Running `tldr doctor` without --install should run check mode (no install arg)
    let mut cmd = tldr_cmd();
    cmd.args(["doctor", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--install"))
        .stdout(predicate::str::contains("diagnostic tools"));
}

// =============================================================================
// Check Mode Tests
// =============================================================================

#[test]
fn test_doctor_check_finds_cargo() {
    // cargo should be found on any dev machine running these tests
    let mut cmd = tldr_cmd();
    let output = cmd.args(["doctor", "-f", "json", "-q"]).output().unwrap();

    assert!(output.status.success(), "doctor command should succeed");

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("doctor output should be valid JSON");

    // Rust entry should exist and have type_checker info
    let rust = json.get("rust").expect("should have rust entry");
    let type_checker = rust.get("type_checker").expect("should have type_checker");

    assert_eq!(
        type_checker.get("name").and_then(|v| v.as_str()),
        Some("cargo"),
        "rust type_checker should be cargo"
    );
    assert_eq!(
        type_checker.get("installed").and_then(|v| v.as_bool()),
        Some(true),
        "cargo should be installed on dev machine"
    );
    assert!(
        type_checker.get("path").and_then(|v| v.as_str()).is_some(),
        "cargo path should be present"
    );
}

#[test]
fn test_doctor_json_structure() {
    // Verify JSON output has correct shape for all supported languages
    let mut cmd = tldr_cmd();
    let output = cmd.args(["doctor", "-f", "json", "-q"]).output().unwrap();

    assert!(output.status.success());

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("doctor output should be valid JSON");

    // Check that it's an object
    assert!(json.is_object(), "output should be a JSON object");

    // Verify expected languages are present
    let expected_langs = [
        "python",
        "typescript",
        "javascript",
        "go",
        "rust",
        "java",
        "c",
        "cpp",
        "ruby",
        "php",
        "kotlin",
        "swift",
        "csharp",
        "scala",
        "elixir",
        "lua",
    ];

    for lang in expected_langs {
        let lang_entry = json
            .get(lang)
            .unwrap_or_else(|| panic!("should have {} entry", lang));

        // Each language should have type_checker and linter keys
        assert!(
            lang_entry.get("type_checker").is_some(),
            "{} should have type_checker key",
            lang
        );
        assert!(
            lang_entry.get("linter").is_some(),
            "{} should have linter key",
            lang
        );

        // If type_checker is not null, it should have required fields
        if let Some(tc) = lang_entry.get("type_checker") {
            if !tc.is_null() {
                assert!(
                    tc.get("name").is_some(),
                    "{} type_checker should have name",
                    lang
                );
                assert!(
                    tc.get("installed").is_some(),
                    "{} type_checker should have installed",
                    lang
                );
                assert!(
                    tc.get("path").is_some(),
                    "{} type_checker should have path",
                    lang
                );
                assert!(
                    tc.get("install").is_some(),
                    "{} type_checker should have install",
                    lang
                );
            }
        }

        // If linter is not null, it should have required fields
        if let Some(linter) = lang_entry.get("linter") {
            if !linter.is_null() {
                assert!(
                    linter.get("name").is_some(),
                    "{} linter should have name",
                    lang
                );
                assert!(
                    linter.get("installed").is_some(),
                    "{} linter should have installed",
                    lang
                );
                assert!(
                    linter.get("path").is_some(),
                    "{} linter should have path",
                    lang
                );
                assert!(
                    linter.get("install").is_some(),
                    "{} linter should have install",
                    lang
                );
            }
        }
    }
}

// =============================================================================
// Install Mode Tests
// =============================================================================

#[test]
fn test_doctor_install_invalid_lang() {
    // Trying to install for unknown language should error
    let mut cmd = tldr_cmd();
    cmd.args(["doctor", "--install", "cobol", "-q"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cobol").or(predicate::str::contains("unknown")));
}

#[test]
fn test_doctor_install_help_shows_option() {
    let mut cmd = tldr_cmd();
    cmd.args(["doctor", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("install"));
}

// =============================================================================
// Text Output Format Tests
// =============================================================================

#[test]
fn test_doctor_text_output_format() {
    // Text output should have headers and status markers
    let mut cmd = tldr_cmd();
    cmd.args(["doctor", "-f", "text"])
        .assert()
        .success()
        // Should have title/header
        .stdout(predicate::str::contains("Diagnostics").or(predicate::str::contains("Doctor")))
        // Should show some language sections
        .stdout(predicate::str::contains("Python").or(predicate::str::contains("python")))
        .stdout(predicate::str::contains("Rust").or(predicate::str::contains("rust")))
        // Should have status indicators
        .stdout(
            predicate::str::contains("[OK]")
                .or(predicate::str::contains("[X]"))
                .or(predicate::str::contains("installed")),
        );
}

#[test]
fn test_doctor_default_format_produces_output() {
    // Running without -f flag should produce output (JSON is the default for all commands)
    let mut cmd = tldr_cmd();
    let output = cmd.args(["doctor"]).output().unwrap();

    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.is_empty(), "doctor should produce output");
}
