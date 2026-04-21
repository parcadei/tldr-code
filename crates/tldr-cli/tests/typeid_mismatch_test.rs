//! Tests for TypeId mismatch bug fix
//!
//! The global CLI `lang` arg is `Option<Language>` (enum), but health/debt
//! subcommands had their own `lang: Option<String>` causing a Clap TypeId
//! mismatch panic when `-l` was used.
//!
//! These tests verify:
//! 1. `health -l python` does not crash (was panicking)
//! 2. `debt -l python` does not crash (was panicking)
//! 3. `health` without `-l` still works
//! 4. `debt` without `-l` still works

use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::process::Command;

/// Get the path to the test binary
fn tldr_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
}

/// Health with -l flag should NOT crash with TypeId mismatch
#[test]
fn test_health_with_lang_flag_no_crash() {
    let fixture = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/simple.py");
    let mut cmd = tldr_cmd();
    cmd.args(["health", fixture, "-l", "python", "--format", "json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("overall_score").or(predicate::str::contains("health")));
}

/// Debt with -l flag should NOT crash with TypeId mismatch
#[test]
fn test_debt_with_lang_flag_no_crash() {
    let fixture = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/simple.py");
    let mut cmd = tldr_cmd();
    cmd.args(["debt", fixture, "-l", "python", "--format", "json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("total_debt").or(predicate::str::contains("debt")));
}

/// Health without -l flag should still work
#[test]
fn test_health_without_lang_flag() {
    let fixture = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/simple.py");
    let mut cmd = tldr_cmd();
    cmd.args(["health", fixture, "--format", "json"])
        .assert()
        .success();
}

/// Debt without -l flag should still work
#[test]
fn test_debt_without_lang_flag() {
    let fixture = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/simple.py");
    let mut cmd = tldr_cmd();
    cmd.args(["debt", fixture, "--format", "json"])
        .assert()
        .success();
}
