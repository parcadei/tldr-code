#![cfg(feature = "semantic")]
//! Regression test for the `semantic`/`embed` clap TypeId mismatch panic.
//!
//! Root cause (pre-VAL-009): the global CLI flag `--lang` is defined as
//! `Option<Language>` (single-valued enum), while `semantic` and `embed`
//! subcommands defined their own `pub lang: Option<Vec<String>>`. Clap
//! collapsed both definitions under the same `lang` key and panicked at
//! runtime on `TypeId` downcast whenever either subcommand saw `--lang`.
//!
//! Fix: renamed the subcommand-local field to `langs` with
//! `#[arg(long = "langs", value_delimiter = ',')]` so the two args have
//! distinct keys. The global `--lang` (single language, by name) continues
//! to work, and the multi-extension filter is now invoked as
//! `--langs rs,py` (extension values, not language names).
//!
//! This test asserts the process exits without a clap panic on both the
//! new `--langs` flag and the inherited global `--lang` flag. We care
//! only about "no panic" here; search correctness is covered by
//! `crates/tldr-core/tests/semantic_tests.rs`.

use std::fs;
use std::process::Command;
use tempfile::tempdir;

fn tldr_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
}

/// `tldr semantic --langs rust` must not trigger a clap TypeId panic.
#[test]
fn test_semantic_langs_flag_does_not_panic() {
    let tmp = tempdir().expect("create tempdir");
    fs::write(tmp.path().join("a.rs"), "pub fn x() {}").expect("write fixture");

    let output = tldr_cmd()
        .arg("semantic")
        .arg("any query")
        .arg(tmp.path())
        .arg("--langs")
        .arg("rs")
        .arg("--format")
        .arg("json")
        .arg("--quiet")
        .output()
        .expect("tldr semantic did not execute");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("panicked at"),
        "tldr semantic --langs rust panicked; stderr was:\n{stderr}"
    );
    assert!(
        !stderr.contains("TypeId"),
        "tldr semantic --langs rust hit a clap TypeId mismatch; stderr was:\n{stderr}"
    );
}

/// `tldr semantic --lang rust` (global flag) must not trigger a clap TypeId panic.
#[test]
fn test_semantic_global_lang_flag_does_not_panic() {
    let tmp = tempdir().expect("create tempdir");
    fs::write(tmp.path().join("a.rs"), "pub fn x() {}").expect("write fixture");

    let output = tldr_cmd()
        .arg("semantic")
        .arg("any query")
        .arg(tmp.path())
        .arg("--lang")
        .arg("rust")
        .arg("--format")
        .arg("json")
        .arg("--quiet")
        .output()
        .expect("tldr semantic did not execute");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("panicked at"),
        "tldr semantic --lang rust panicked; stderr was:\n{stderr}"
    );
    assert!(
        !stderr.contains("TypeId"),
        "tldr semantic --lang rust hit a clap TypeId mismatch; stderr was:\n{stderr}"
    );
}

/// `tldr embed --langs rust` must not trigger a clap TypeId panic.
///
/// `embed` had the exact same `Option<Vec<String>>` shape as `semantic`
/// and was fixed in the same commit to preempt the identical bug.
#[test]
fn test_embed_langs_flag_does_not_panic() {
    let tmp = tempdir().expect("create tempdir");
    fs::write(tmp.path().join("a.rs"), "pub fn x() {}").expect("write fixture");

    let output = tldr_cmd()
        .arg("embed")
        .arg(tmp.path())
        .arg("--langs")
        .arg("rs")
        .arg("--format")
        .arg("json")
        .arg("--quiet")
        .arg("--no-cache")
        .output()
        .expect("tldr embed did not execute");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("panicked at"),
        "tldr embed --langs rust panicked; stderr was:\n{stderr}"
    );
    assert!(
        !stderr.contains("TypeId"),
        "tldr embed --langs rust hit a clap TypeId mismatch; stderr was:\n{stderr}"
    );
}
