//! Tests for the bugbot command group
//!
//! Verifies:
//! - `bugbot check` subcommand exists and runs
//! - JSON output contains expected schema fields
//! - CLI arguments are parsed correctly (--staged, --base-ref, --lang, --max-findings, --no-fail)
//! - Default values are correct
//! - Help text is displayed

use std::process::Command;

/// Helper to get the path to the built binary
fn tldr_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_tldr"))
}

#[test]
fn bugbot_check_runs_and_produces_json() {
    let output = tldr_bin()
        .args(["--lang", "rust", "bugbot", "check", "--no-fail", "."])
        .output()
        .expect("failed to execute bugbot check");

    assert!(
        output.status.success(),
        "bugbot check should exit 0 with --no-fail, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("output should be valid JSON");

    // Verify top-level schema fields
    assert_eq!(json["tool"], "bugbot", "tool field should be 'bugbot'");
    assert_eq!(json["mode"], "check", "mode field should be 'check'");
    assert!(
        json["language"].is_string(),
        "language field should be a string"
    );
    assert!(
        json["base_ref"].is_string(),
        "base_ref field should be a string"
    );
    assert!(
        json["detection_method"].is_string(),
        "detection_method field should be a string"
    );
    assert!(
        json["timestamp"].is_string(),
        "timestamp field should be a string"
    );
    assert!(
        json["changed_files"].is_array(),
        "changed_files should be an array"
    );
    assert!(json["findings"].is_array(), "findings should be an array");
    assert!(json["summary"].is_object(), "summary should be an object");
    assert!(
        json["elapsed_ms"].is_number(),
        "elapsed_ms should be a number"
    );
    assert!(json["errors"].is_array(), "errors should be an array");
    assert!(json["notes"].is_array(), "notes should be an array");
}

#[test]
fn bugbot_check_default_base_ref_is_head() {
    let output = tldr_bin()
        .args(["--lang", "rust", "bugbot", "check", "."])
        .output()
        .expect("failed to execute bugbot check");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");

    assert_eq!(json["base_ref"], "HEAD", "default base_ref should be HEAD");
}

#[test]
fn bugbot_check_default_detection_method_is_uncommitted() {
    let output = tldr_bin()
        .args(["--lang", "rust", "bugbot", "check", "."])
        .output()
        .expect("failed to execute bugbot check");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");

    assert_eq!(
        json["detection_method"], "git:uncommitted",
        "default detection_method should be git:uncommitted"
    );
}

#[test]
fn bugbot_check_staged_flag_changes_detection_method() {
    let output = tldr_bin()
        .args(["--lang", "rust", "bugbot", "check", "--staged", "."])
        .output()
        .expect("failed to execute bugbot check");

    assert!(output.status.success(), "should exit 0");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");

    assert_eq!(
        json["detection_method"], "git:staged",
        "--staged should set detection_method to git:staged"
    );
}

#[test]
fn bugbot_check_custom_base_ref() {
    let output = tldr_bin()
        .args([
            "--lang",
            "rust",
            "bugbot",
            "check",
            "--no-fail",
            "--base-ref",
            "main",
            ".",
        ])
        .output()
        .expect("failed to execute bugbot check");

    assert!(output.status.success(), "should exit 0");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");

    assert_eq!(
        json["base_ref"], "main",
        "--base-ref main should set base_ref to main"
    );
}

#[test]
fn bugbot_check_lang_override() {
    // --lang is a global flag, so it goes before the subcommand
    let output = tldr_bin()
        .args(["--lang", "rust", "bugbot", "check", "--no-fail", "."])
        .output()
        .expect("failed to execute bugbot check");

    assert!(
        output.status.success(),
        "should exit 0 with --no-fail, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");

    assert_eq!(
        json["language"], "rust",
        "--lang rust should set language to rust"
    );
}

#[test]
fn bugbot_check_summary_fields_present() {
    let output = tldr_bin()
        .args(["--lang", "rust", "bugbot", "check", "."])
        .output()
        .expect("failed to execute bugbot check");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");

    let summary = &json["summary"];
    assert!(
        summary["total_findings"].is_number(),
        "summary.total_findings should be a number"
    );
    assert!(
        summary["by_severity"].is_object(),
        "summary.by_severity should be an object"
    );
    assert!(
        summary["by_type"].is_object(),
        "summary.by_type should be an object"
    );
    assert!(
        summary["files_analyzed"].is_number(),
        "summary.files_analyzed should be a number"
    );
    assert!(
        summary["functions_analyzed"].is_number(),
        "summary.functions_analyzed should be a number"
    );
}

#[test]
fn bugbot_check_help_works() {
    let output = tldr_bin()
        .args(["bugbot", "check", "--help"])
        .output()
        .expect("failed to execute bugbot check --help");

    assert!(output.status.success(), "help should exit 0");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("bugbot"),
        "help text should mention 'bugbot'"
    );
    assert!(
        stdout.contains("--staged"),
        "help text should mention --staged flag"
    );
    assert!(
        stdout.contains("--base-ref"),
        "help text should mention --base-ref flag"
    );
    assert!(
        stdout.contains("--max-findings"),
        "help text should mention --max-findings flag"
    );
    assert!(
        stdout.contains("--no-fail"),
        "help text should mention --no-fail flag"
    );
}

#[test]
fn bugbot_help_lists_check_subcommand() {
    let output = tldr_bin()
        .args(["bugbot", "--help"])
        .output()
        .expect("failed to execute bugbot --help");

    assert!(output.status.success(), "bugbot --help should exit 0");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("check"),
        "bugbot help should list 'check' subcommand"
    );
}
