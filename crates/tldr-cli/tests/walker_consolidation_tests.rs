//! Walker Consolidation Integration Tests (VAL-001)
//!
//! Verify that the shared `tldr_core::walker::ProjectWalker` is actually
//! used by the CLI commands that claim to skip vendor/build directories
//! by default, and that `--no-default-ignore` disables that skipping.
//!
//! Reference: VAL-001 walker consolidation migration.

use assert_cmd::Command;
use std::fs;
use std::path::Path;
use tempfile::tempdir;

/// Get the tldr binary under test.
fn tldr_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
}

/// Write a file, creating parent directories if needed.
fn write_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create_dir_all");
    }
    fs::write(path, contents).expect("write");
}

/// Rust source that deliberately triggers multiple smells:
/// - LongParameterList (>5 params)
/// - LongMethod (cyclomatic >= 10 + 50+ LOC)
fn smelly_rust() -> &'static str {
    r#"
pub fn tangled(
    a: i32,
    b: i32,
    c: i32,
    d: i32,
    e: i32,
    f: i32,
    g: i32,
    h: i32,
) -> i32 {
    let mut total = 0;
    if a > 0 { total += 1; } else { total -= 1; }
    if b > 0 { total += 1; } else { total -= 1; }
    if c > 0 { total += 1; } else { total -= 1; }
    if d > 0 { total += 1; } else { total -= 1; }
    if e > 0 { total += 1; } else { total -= 1; }
    if f > 0 { total += 1; } else { total -= 1; }
    if g > 0 { total += 1; } else { total -= 1; }
    if h > 0 { total += 1; } else { total -= 1; }
    if a > b { total *= 2; }
    if b > c { total *= 2; }
    if c > d { total *= 2; }
    if d > e { total *= 2; }
    if e > f { total *= 2; }
    if f > g { total *= 2; }
    if g > h { total *= 2; }
    if a > h { total *= 2; }
    if total > 100 { total = 100; }
    if total < -100 { total = -100; }
    if total == 0 { total = 1; }
    if total == 1 { total = 2; }
    if total == 2 { total = 3; }
    total
}
"#
}

/// Python that triggers a taint sink (os.system on unsanitized input).
fn taint_bait_py() -> &'static str {
    r#"
import os
def bad(request):
    cmd = request.args.get("cmd")
    os.system(cmd)
"#
}

// =============================================================================
// VAL-001: smells skips node_modules by default
// =============================================================================

#[test]
fn test_smells_skips_node_modules() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    // The file in src/ should be scanned and may produce findings.
    write_file(&root.join("src/good.rs"), smelly_rust());
    // The file in node_modules/ must NOT be scanned — any findings with
    // a "node_modules" path would be a regression.
    write_file(&root.join("node_modules/bad.rs"), smelly_rust());

    let mut cmd = tldr_cmd();
    cmd.arg("smells")
        .arg(root)
        .arg("--lang")
        .arg("rust")
        .arg("--format")
        .arg("json");

    let output = cmd.assert().success().get_output().stdout.clone();
    let stdout = String::from_utf8_lossy(&output);

    assert!(
        !stdout.contains("node_modules"),
        "smells output must not reference node_modules paths; got:\n{}",
        stdout
    );
}

// =============================================================================
// VAL-001: secure skips node_modules by default
// =============================================================================

#[test]
fn test_secure_skips_node_modules() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    // Clean Python file in src/.
    write_file(&root.join("src/clean.py"), "def ok():\n    return 1\n");
    // Taint-triggering file in node_modules/ that MUST NOT be reported.
    write_file(&root.join("node_modules/bad.py"), taint_bait_py());

    let mut cmd = tldr_cmd();
    cmd.arg("secure")
        .arg(root)
        .arg("--lang")
        .arg("python")
        .arg("-f")
        .arg("json");

    let output = cmd.assert().success().get_output().stdout.clone();
    let stdout = String::from_utf8_lossy(&output);

    // No finding should have a file under node_modules.
    assert!(
        !stdout.contains("node_modules"),
        "secure output must not reference node_modules paths; got:\n{}",
        stdout
    );
}

// =============================================================================
// VAL-001: vuln --lang filters to only the requested language's files
// =============================================================================

#[test]
fn test_vuln_respects_lang_filter() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    // Clean TypeScript file — the only file --lang typescript should scan.
    write_file(&root.join("src/clean.ts"), "export const x = 1;\n");
    // Python taint bait — MUST NOT be scanned when --lang typescript is
    // requested (it would otherwise show up in files_scanned).
    write_file(&root.join("src/bad.py"), taint_bait_py());

    let mut cmd = tldr_cmd();
    cmd.arg("vuln")
        .arg(root)
        .arg("--lang")
        .arg("typescript")
        .arg("--format")
        .arg("json")
        .arg("--quiet");

    let output = cmd.assert().get_output().stdout.clone();
    let stdout = String::from_utf8_lossy(&output);

    // Expect exactly 1 file scanned: the .ts file. The .py file should
    // be filtered out by is_supported_source_file(path, Some(TypeScript)).
    assert!(
        stdout.contains("\"files_scanned\": 1") || stdout.contains("\"files_scanned\":1"),
        "vuln --lang typescript should scan exactly 1 file (the .ts file), got stdout:\n{}",
        stdout
    );
}

// =============================================================================
// VAL-001: smells --no-default-ignore opts back into vendor dirs
// =============================================================================

#[test]
fn test_smells_no_default_ignore_opt_out() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    // src/ file — scanned under both defaults and --no-default-ignore.
    write_file(&root.join("src/good.rs"), smelly_rust());
    // node_modules/ smell — normally skipped, but --no-default-ignore
    // must force the walker to descend into it.
    write_file(&root.join("node_modules/bad.rs"), smelly_rust());

    let mut cmd = tldr_cmd();
    cmd.arg("smells")
        .arg(root)
        .arg("--lang")
        .arg("rust")
        .arg("--no-default-ignore")
        .arg("--format")
        .arg("json");

    let output = cmd.assert().success().get_output().stdout.clone();
    let stdout = String::from_utf8_lossy(&output);

    // With --no-default-ignore, we should see findings (or at least file
    // references) under node_modules.
    assert!(
        stdout.contains("node_modules"),
        "smells --no-default-ignore must scan node_modules; got:\n{}",
        stdout
    );
}
