//! Vuln Language Autodetection + No-Cap Integration Tests (VAL-006)
//!
//! Verifies two sibling fixes in `tldr vuln`:
//!
//! 1. Language autodetection when `--lang` is omitted. Previously, the
//!    implementation silently fell back to only `.py` + `.rs` extensions,
//!    so `tldr vuln .` on a TypeScript-only tree reported "0 files
//!    scanned" with no signal to the user. Now, when `--lang` is missing
//!    the command uses `Language::from_directory` (the VAL-002 detector)
//!    to decide what the user meant and:
//!      - runs the scan when the detected language is in the taint
//!        engine's native-analysis set ({Python, Rust});
//!      - emits a stderr error and exits 2 when the detected language is
//!        outside that set, pointing the user at `--lang`;
//!      - preserves the historical empty-report-exit-0 on an empty tree
//!        (no detectable language).
//!
//! 2. Removal of the silent `MAX_DIRECTORY_FILES = 1000` cap in
//!    `collect_files`. After VAL-001 the walker is structurally bounded
//!    (honors .gitignore and default excludes), so the legacy cap has
//!    become a vestigial truncation that silently dropped input on
//!    medium-to-large repos (e.g., ~3900 TS files in `dub`).
//!
//! Explicit `--lang <L>` always bypasses the detect-and-error path —
//! VAL-001's behavior of honoring the user's explicit language choice
//! is preserved.
//!
//! Reference: VAL-006.

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

// =============================================================================
// VAL-006: autodetect runs successfully when detected lang is supported
// =============================================================================

/// When the user omits `--lang` in a Rust project, autodetect picks
/// Rust (via `Cargo.toml`) and the scan runs against `.rs` files
/// only. We seed two Rust sources and one Python decoy; the scan
/// must see exactly 2 files (proving autodetect picked Rust from the
/// manifest — the pre-VAL-006 `None => py|rs` fallback would have
/// picked up all 3).
#[test]
fn test_vuln_autodetects_rust() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    write_file(
        &root.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write_file(
        &root.join("src/main.rs"),
        "fn main() {\n    let _ = std::env::var(\"X\");\n}\n",
    );
    write_file(&root.join("src/lib.rs"), "pub fn lib_fn() {}\n");
    // Decoy: a .py tooling file. Autodetect should pick Rust from
    // Cargo.toml and ignore this file. The pre-VAL-006 None fallback
    // would have scanned it.
    write_file(&root.join("tools/helper.py"), "print('tooling')\n");

    let mut cmd = tldr_cmd();
    cmd.arg("vuln")
        .arg(root)
        .arg("--format")
        .arg("json")
        .arg("--quiet");

    let output = cmd.assert().get_output().clone();
    let stdout = String::from_utf8_lossy(&output.stdout);

    let files_scanned = extract_files_scanned(&stdout)
        .unwrap_or_else(|| panic!("files_scanned missing:\n{}", stdout));
    assert_eq!(
        files_scanned, 2,
        "vuln should autodetect Rust (Cargo.toml present) and scan only the 2 .rs files, skipping the .py decoy; got {} files\nstdout:\n{}",
        files_scanned, stdout
    );
}

/// When the user omits `--lang` in a Python project, autodetect picks
/// Python (via `pyproject.toml`) and the scan runs against `.py`
/// files only. We seed one Python source and one Rust decoy;
/// autodetect must pick Python and ignore the `.rs` file.
#[test]
fn test_vuln_autodetects_python() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    write_file(
        &root.join("pyproject.toml"),
        "[project]\nname = \"demo\"\nversion = \"0.1.0\"\n",
    );
    write_file(
        &root.join("app.py"),
        "import os\ndef bad():\n    os.system(os.environ[\"X\"])\n",
    );
    // Decoy: a .rs build script (some Python projects ship these).
    // Autodetect must pick Python from pyproject.toml and ignore
    // this file. The pre-VAL-006 None fallback would have scanned
    // both extensions.
    write_file(&root.join("build.rs"), "fn main() {}\n");

    let mut cmd = tldr_cmd();
    cmd.arg("vuln")
        .arg(root)
        .arg("--format")
        .arg("json")
        .arg("--quiet");

    let output = cmd.assert().get_output().clone();
    let stdout = String::from_utf8_lossy(&output.stdout);

    let files_scanned = extract_files_scanned(&stdout)
        .unwrap_or_else(|| panic!("files_scanned missing:\n{}", stdout));
    assert_eq!(
        files_scanned, 1,
        "vuln should autodetect Python (pyproject.toml present) and scan only the 1 .py file, skipping the .rs decoy; got {} files\nstdout:\n{}",
        files_scanned, stdout
    );
}

// =============================================================================
// VAL-006: autodetect errors helpfully on unsupported language
// =============================================================================

/// When the user omits `--lang` in a TypeScript project, autodetect
/// picks TypeScript. TS is not in the taint engine's native-analysis
/// set (the Python and Rust paths have dedicated tree-sitter / line
/// analyzers; other languages fall back to the pattern scanner in
/// tldr-core and have weaker guarantees). To avoid silent "scanned
/// 0 files" behavior, we error with exit code 2 and a message that
/// mentions the detected language and points the user at `--lang`.
#[test]
fn test_vuln_errors_on_unsupported_autodetected_lang() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    write_file(
        &root.join("tsconfig.json"),
        "{\"compilerOptions\":{\"strict\":true}}\n",
    );
    write_file(&root.join("src/app.ts"), "export const x: number = 1;\n");

    let mut cmd = tldr_cmd();
    cmd.arg("vuln").arg(root).arg("--format").arg("json");

    let output = cmd.assert().failure().get_output().clone();
    let exit_code = output.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(
        exit_code, 2,
        "unsupported autodetected language should exit 2; got {}\nstderr:\n{}",
        exit_code, stderr
    );
    // The error message must identify the problem and point at a fix.
    assert!(
        stderr.contains("not yet supported") || stderr.contains("typescript"),
        "stderr should explain TS is not yet supported by autodetect; got:\n{}",
        stderr
    );
    assert!(
        stderr.contains("--lang python") || stderr.contains("--lang rust"),
        "stderr should suggest an explicit --lang; got:\n{}",
        stderr
    );
}

/// Explicit `--lang typescript` bypasses the autodetect-and-error
/// path. The user has signalled they know TS is outside the native
/// set; the scan must run (falling through to the line-pattern
/// multi-language backend) and return a valid report without
/// erroring on the language.
#[test]
fn test_vuln_honors_explicit_lang_typescript() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    write_file(
        &root.join("tsconfig.json"),
        "{\"compilerOptions\":{\"strict\":true}}\n",
    );
    // Clean TS file — no vulns expected.
    write_file(&root.join("src/app.ts"), "export const x: number = 1;\n");

    let mut cmd = tldr_cmd();
    cmd.arg("vuln")
        .arg(root)
        .arg("--lang")
        .arg("typescript")
        .arg("--format")
        .arg("json")
        .arg("--quiet");

    let output = cmd.assert().success().get_output().clone();
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Scan produced a report with the TS file visible to it. Do not
    // require findings — a clean file may yield zero. Only require
    // that the run succeeded and looked at something.
    assert!(
        stdout.contains("\"files_scanned\":"),
        "explicit --lang typescript should produce a report with files_scanned field; got:\n{}",
        stdout
    );
}

// =============================================================================
// VAL-006: empty / undetectable tree preserves exit-0 empty report
// =============================================================================

/// On a tree with no detectable language (empty), autodetect returns
/// None and the command must exit 0 with an empty report. The user
/// ran the command; if there's no input, there's no error — just
/// nothing to scan.
#[test]
fn test_vuln_no_detectable_lang_empty_dir() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    let mut cmd = tldr_cmd();
    cmd.arg("vuln")
        .arg(root)
        .arg("--format")
        .arg("json")
        .arg("--quiet");

    let output = cmd.assert().success().get_output().clone();
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("\"files_scanned\": 0") || stdout.contains("\"files_scanned\":0"),
        "empty directory should report 0 files scanned; got:\n{}",
        stdout
    );
}

// =============================================================================
// VAL-006: no silent 1000-file cap on large repos
// =============================================================================

/// Regression for the `MAX_DIRECTORY_FILES = 1000` cap that silently
/// truncated collection on medium-to-large repos. After VAL-001 the
/// walker honors .gitignore and default excludes, so the cap is no
/// longer needed as armor — only harmful.
///
/// We generate 1500 trivial `.rs` files in a flat directory and run
/// with explicit `--lang rust` to avoid autodetection ambiguity (the
/// extension-majority fallback would also resolve Rust, but pinning
/// isolates the cap behavior from the detection behavior). The scan
/// must see >= 1500 files.
#[test]
fn test_vuln_no_cap_on_large_repos() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    let n_files = 1500;
    for i in 0..n_files {
        write_file(
            &root.join(format!("src/f{}.rs", i)),
            &format!("pub fn f_{}() {{}}\n", i),
        );
    }

    let mut cmd = tldr_cmd();
    cmd.arg("vuln")
        .arg(root)
        .arg("--lang")
        .arg("rust")
        .arg("--format")
        .arg("json")
        .arg("--quiet");

    let output = cmd.assert().success().get_output().clone();
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Pull out "files_scanned": N and assert N >= 1500. This proves
    // no silent truncation at 1000. Simple parse — no regex dep.
    let files_scanned = extract_files_scanned(&stdout).unwrap_or_else(|| {
        panic!("files_scanned not present in output:\n{}", stdout);
    });
    assert!(
        files_scanned >= n_files,
        "expected files_scanned >= {} (proving the 1000 cap is gone); got {}\nstdout:\n{}",
        n_files,
        files_scanned,
        stdout
    );
}

/// Extract the integer value of the `"files_scanned"` JSON field from
/// a stdout blob. Returns None if the field is absent or malformed.
fn extract_files_scanned(stdout: &str) -> Option<u32> {
    let idx = stdout.find("\"files_scanned\"")?;
    let after_key = &stdout[idx + "\"files_scanned\"".len()..];
    let colon_idx = after_key.find(':')?;
    let after_colon = after_key[colon_idx + 1..].trim_start();
    let digits: String = after_colon
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}
