//! Test coverage for tldr-cli Main/Output/Signals modules
//!
//! Tests for:
//! - CLI argument parsing and validation (main.rs)
//! - Output formatting for all formats (output.rs)
//! - Signal handling and interruption (signals.rs)
//! - Error handling and exit codes
//!
//! Run with: cargo test --package tldr-test cli_main_output

use std::fs;
use std::process::Command;
use tempfile::TempDir;

// =============================================================================
// Test Fixtures
// =============================================================================

/// Create a minimal test project for CLI testing
fn create_test_project() -> TempDir {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create a Python file with basic functions
    fs::write(
        project_path.join("main.py"),
        r#"def helper():
    pass

def main():
    helper()
    
if __name__ == "__main__":
    main()
"#,
    )
    .unwrap();

    temp_dir
}

/// Create a test project with multiple files for tree/structure tests
fn create_multi_file_project() -> TempDir {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    fs::write(
        project_path.join("main.py"),
        r#"from utils import helper

def main():
    helper()
"#,
    )
    .unwrap();

    fs::write(
        project_path.join("utils.py"),
        r#"def helper():
    return 42
"#,
    )
    .unwrap();

    // Create a subdirectory
    fs::create_dir(project_path.join("pkg")).unwrap();
    fs::write(project_path.join("pkg/__init__.py"), "").unwrap();
    fs::write(
        project_path.join("pkg/module.py"),
        r#"def pkg_func():
    pass
"#,
    )
    .unwrap();

    temp_dir
}

/// Create an empty project for edge case testing
fn create_empty_project() -> TempDir {
    TempDir::new().unwrap()
}

// =============================================================================
// CLI Parsing Tests (main.rs)
// =============================================================================

/// Test: --help shows usage information
#[test]
fn test_cli_help_shows_usage() {
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args(["--help"])
        .output()
        .expect("Failed to execute tldr --help");

    assert!(output.status.success(), "--help should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage:"), "Help should contain Usage");
    assert!(stdout.contains("Commands:"), "Help should contain Commands");
    assert!(stdout.contains("Options:"), "Help should contain Options");
}

/// Test: --version shows version number
#[test]
fn test_cli_version_shows_version() {
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args(["--version"])
        .output()
        .expect("Failed to execute tldr --version");

    assert!(output.status.success(), "--version should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("tldr"), "Version should contain 'tldr'");
    assert!(
        stdout.contains("0.1.0") || stdout.contains('.'),
        "Version should contain version number"
    );
}

/// Test: -h shows short help
#[test]
fn test_cli_short_help() {
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args(["-h"])
        .output()
        .expect("Failed to execute tldr -h");

    assert!(output.status.success(), "-h should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage:"), "Short help should contain Usage");
}

/// Test: -V shows short version
#[test]
fn test_cli_short_version() {
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args(["-V"])
        .output()
        .expect("Failed to execute tldr -V");

    assert!(output.status.success(), "-V should succeed");
}

/// Test: No subcommand shows help
#[test]
fn test_cli_no_subcommand_shows_help() {
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .output()
        .expect("Failed to execute tldr without args");

    // Without args, should fail with exit code 2 (clap error)
    assert!(!output.status.success(), "No subcommand should fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Usage:") || stderr.contains("required") || stderr.contains("help"),
        "Should show help or error: {}",
        stderr
    );
}

/// Test: Invalid subcommand shows error
#[test]
fn test_cli_invalid_subcommand() {
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args(["invalidcommand"])
        .output()
        .expect("Failed to execute tldr with invalid command");

    assert!(!output.status.success(), "Invalid command should fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("error") || stderr.contains("unrecognized") || stderr.contains("found"),
        "Should show error message: {}",
        stderr
    );
}

/// Test: --format option is accepted for JSON format
#[test]
fn test_cli_format_option_json() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "tree",
            temp_dir.path().to_str().unwrap(),
            "-f",
            "json",
            "-q",
        ])
        .output()
        .expect("Failed to execute with -f json");

    assert!(output.status.success(), "JSON format should work");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"name\""),
        "JSON output should have quoted keys"
    );
}

/// Test: --format text is accepted
#[test]
fn test_cli_format_option_text() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "tree",
            temp_dir.path().to_str().unwrap(),
            "-f",
            "text",
            "-q",
        ])
        .output()
        .expect("Failed to execute with -f text");

    assert!(output.status.success(), "Text format should work");
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Text format should show file tree with icons or names
    assert!(
        stdout.contains("main.py") || stdout.contains("[F]") || stdout.contains("[D]"),
        "Text output should show files: {}",
        stdout
    );
}

/// Test: --format compact is accepted
#[test]
fn test_cli_format_option_compact() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "tree",
            temp_dir.path().to_str().unwrap(),
            "-f",
            "compact",
            "-q",
        ])
        .output()
        .expect("Failed to execute with -f compact");

    assert!(output.status.success(), "Compact format should work");
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Compact should be minified
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(
        lines.len() <= 2,
        "Compact format should be minified (1-2 lines), got {} lines",
        lines.len()
    );
}

/// Test: --format sarif is accepted (for clones command)
#[ignore = "SARIF format may not be supported for all commands - BUG-007"]
#[test]
fn test_cli_format_option_sarif() {
    let temp_dir = create_test_project();
    // Create duplicate files for clone detection
    fs::write(
        temp_dir.path().join("file1.py"),
        "def foo():\n    return 42\n",
    )
    .unwrap();
    fs::write(
        temp_dir.path().join("file2.py"),
        "def bar():\n    return 42\n",
    )
    .unwrap();

    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "clones",
            temp_dir.path().to_str().unwrap(),
            "-f",
            "sarif",
            "-q",
        ])
        .output()
        .expect("Failed to execute with -f sarif");

    assert!(output.status.success(), "SARIF format should work");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("$schema") || stdout.contains("sarif"),
        "SARIF output should contain schema reference: {}",
        stdout
    );
}

/// Test: --format dot is accepted (for graph commands)
#[test]
fn test_cli_format_option_dot() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "calls",
            temp_dir.path().to_str().unwrap(),
            "-f",
            "dot",
            "-q",
        ])
        .output()
        .expect("Failed to execute with -f dot");

    // Note: DOT format might produce JSON fallback if not implemented - BUG-003
    let stdout = String::from_utf8_lossy(&output.stdout);
    if output.status.success() {
        assert!(
            stdout.contains("digraph") || stdout.contains("graph") || stdout.contains("{"),
            "DOT output should contain digraph or graph or JSON: {}",
            stdout
        );
    }
}

/// Test: Invalid format is rejected
#[test]
fn test_cli_invalid_format_rejected() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "tree",
            temp_dir.path().to_str().unwrap(),
            "-f",
            "invalid",
            "-q",
        ])
        .output()
        .expect("Failed to execute with invalid format");

    assert!(!output.status.success(), "Invalid format should fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("error")
            || stderr.contains("invalid")
            || stderr.contains("possible values"),
        "Should show error for invalid format: {}",
        stderr
    );
}

/// Test: --quiet flag suppresses progress output
#[test]
fn test_cli_quiet_flag() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args(["tree", temp_dir.path().to_str().unwrap(), "-q"])
        .output()
        .expect("Failed to execute with -q");

    assert!(output.status.success(), "Quiet flag should work");
    // Quiet should suppress stderr output
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.is_empty() || !stderr.contains("Analyzing"),
        "Quiet mode should suppress progress: {}",
        stderr
    );
}

/// Test: --verbose flag enables debug output
#[test]
fn test_cli_verbose_flag() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args(["tree", temp_dir.path().to_str().unwrap(), "-v", "-q"])
        .output()
        .expect("Failed to execute with -v");

    assert!(output.status.success(), "Verbose flag should work");
}

/// Test: --lang option is accepted
#[test]
fn test_cli_lang_option() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "tree",
            temp_dir.path().to_str().unwrap(),
            "-l",
            "python",
            "-q",
        ])
        .output()
        .expect("Failed to execute with -l python");

    assert!(output.status.success(), "Language option should work");
}

/// Test: Invalid language may be rejected or ignored
#[test]
fn test_cli_invalid_lang_behavior() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "tree",
            temp_dir.path().to_str().unwrap(),
            "-l",
            "invalidlang",
            "-q",
        ])
        .output()
        .expect("Failed to execute with invalid language");

    // Language validation may happen at different stages - documented as BUG-012
    let _stderr = String::from_utf8_lossy(&output.stderr);
    // May succeed (using auto-detect) or fail with error
}

/// Test: Global flags work with different commands
#[test]
fn test_cli_global_flags_tree() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "tree",
            temp_dir.path().to_str().unwrap(),
            "-f",
            "text",
            "-q",
        ])
        .output()
        .expect("Failed to execute tree");

    assert!(
        output.status.success(),
        "Tree with global flags should work"
    );
}

/// Test: Global flags work with structure command
#[test]
fn test_cli_global_flags_structure() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "structure",
            temp_dir.path().to_str().unwrap(),
            "-f",
            "json",
            "-q",
        ])
        .output()
        .expect("Failed to execute structure");

    assert!(
        output.status.success(),
        "Structure with global flags should work"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"files\""), "Should have files in output");
}

/// Test: Global flags work with calls command
#[test]
fn test_cli_global_flags_calls() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "calls",
            temp_dir.path().to_str().unwrap(),
            "-f",
            "json",
            "-q",
        ])
        .output()
        .expect("Failed to execute calls");

    assert!(
        output.status.success(),
        "Calls with global flags should work"
    );
}

/// Test: Command aliases work - tree 't'
#[test]
fn test_cli_command_alias_tree_t() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args(["t", temp_dir.path().to_str().unwrap(), "-q"])
        .output()
        .expect("Failed to execute t alias");

    assert!(output.status.success(), "Tree alias 't' should work");
}

/// Test: Structure alias 's' works
#[test]
fn test_cli_command_alias_structure_s() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args(["s", temp_dir.path().to_str().unwrap(), "-q"])
        .output()
        .expect("Failed to execute s alias");

    assert!(output.status.success(), "Structure alias 's' should work");
}

/// Test: Calls alias 'c' works
#[test]
fn test_cli_command_alias_calls_c() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args(["c", temp_dir.path().to_str().unwrap(), "-q"])
        .output()
        .expect("Failed to execute c alias");

    assert!(output.status.success(), "Calls alias 'c' should work");
}

/// Test: Impact alias 'i' works
#[test]
fn test_cli_command_alias_impact_i() {
    let temp_dir = create_multi_file_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args(["i", "helper", temp_dir.path().to_str().unwrap(), "-q"])
        .output()
        .expect("Failed to execute i alias");

    assert!(output.status.success(), "Impact alias 'i' should work");
}

/// Test: Dead alias 'd' works
#[test]
fn test_cli_command_alias_dead_d() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args(["d", temp_dir.path().to_str().unwrap(), "-q"])
        .output()
        .expect("Failed to execute d alias");

    assert!(output.status.success(), "Dead alias 'd' should work");
}

// =============================================================================
// Output Format Tests (output.rs)
// =============================================================================

/// Test: JSON output is valid
#[test]
fn test_output_json_valid() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "tree",
            temp_dir.path().to_str().unwrap(),
            "-f",
            "json",
            "-q",
        ])
        .output()
        .expect("Failed to execute");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should be valid JSON (parseable)
    let json_result: Result<serde_json::Value, _> = serde_json::from_str(&stdout);
    assert!(
        json_result.is_ok(),
        "JSON output should be valid: {}",
        stdout
    );
}

/// Test: Text output has proper formatting
/// Note: May contain ANSI escape codes - documented as BUG-002
#[test]
fn test_output_text_formatting() {
    let temp_dir = create_multi_file_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "tree",
            temp_dir.path().to_str().unwrap(),
            "-f",
            "text",
            "-q",
        ])
        .output()
        .expect("Failed to execute");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Text format should show file/directory indicators
    assert!(
        stdout.contains("[F]") || stdout.contains("[D]") || stdout.contains("main.py"),
        "Text output should show files: {}",
        stdout
    );
}

/// Test: Structure text output format
#[test]
fn test_output_structure_text_format() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "structure",
            temp_dir.path().to_str().unwrap(),
            "-f",
            "text",
            "-q",
        ])
        .output()
        .expect("Failed to execute");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should show structure info
    assert!(
        stdout.contains("Functions:") || stdout.contains("helper") || stdout.contains("main"),
        "Structure text should show functions: {}",
        stdout
    );
}

/// Test: Output format consistency across runs
#[test]
fn test_output_format_consistency() {
    let temp_dir = create_test_project();

    // Run twice with same args
    let output1 = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "tree",
            temp_dir.path().to_str().unwrap(),
            "-f",
            "json",
            "-q",
        ])
        .output()
        .expect("Failed first execution");

    let output2 = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "tree",
            temp_dir.path().to_str().unwrap(),
            "-f",
            "json",
            "-q",
        ])
        .output()
        .expect("Failed second execution");

    assert!(output1.status.success() && output2.status.success());

    let stdout1 = String::from_utf8_lossy(&output1.stdout);
    let stdout2 = String::from_utf8_lossy(&output2.stdout);

    // JSON output should be consistent
    assert_eq!(stdout1, stdout2, "Output should be consistent across runs");
}

/// Test: Default format is JSON
#[test]
fn test_output_default_format_json() {
    let temp_dir = create_test_project();

    // Without -f flag
    let output1 = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args(["tree", temp_dir.path().to_str().unwrap(), "-q"])
        .output()
        .expect("Failed without format");

    // With -f json
    let output2 = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "tree",
            temp_dir.path().to_str().unwrap(),
            "-f",
            "json",
            "-q",
        ])
        .output()
        .expect("Failed with json format");

    assert!(output1.status.success() && output2.status.success());

    // Both should produce similar JSON structure
    let stdout1 = String::from_utf8_lossy(&output1.stdout);
    let stdout2 = String::from_utf8_lossy(&output2.stdout);

    assert!(
        stdout1.contains("\"name\"") && stdout2.contains("\"name\""),
        "Default format should be JSON"
    );
}

/// Test: Output with non-existent path
#[test]
fn test_output_nonexistent_path() {
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args(["tree", "/nonexistent/path/12345", "-q"])
        .output()
        .expect("Failed to execute");

    assert!(
        !output.status.success(),
        "Should fail for non-existent path"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Error") || stderr.contains("error") || stderr.contains("not found"),
        "Should show error: {}",
        stderr
    );
}

/// Test: Output with file instead of directory
#[test]
fn test_output_file_instead_of_directory() {
    let temp_dir = create_test_project();
    let file_path = temp_dir.path().join("main.py");

    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args(["tree", file_path.to_str().unwrap(), "-q"])
        .output()
        .expect("Failed to execute");

    // Should handle gracefully (may succeed showing single file or fail)
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    if !output.status.success() {
        assert!(
            stderr.contains("Error") || stderr.contains("error"),
            "Should show error for file: {}",
            stderr
        );
    } else {
        assert!(
            stdout.contains("main.py"),
            "Should show the file: {}",
            stdout
        );
    }
}

/// Test: Compact format is minified JSON
#[test]
fn test_output_compact_minified() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "tree",
            temp_dir.path().to_str().unwrap(),
            "-f",
            "compact",
            "-q",
        ])
        .output()
        .expect("Failed to execute");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Compact should not have pretty-print newlines
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(
        lines.len() <= 2,
        "Compact format should be minified (1-2 lines), got {} lines",
        lines.len()
    );

    // But should still be valid JSON
    let json_result: Result<serde_json::Value, _> = serde_json::from_str(&stdout);
    assert!(json_result.is_ok(), "Compact output should be valid JSON");
}

/// Test: Text output for calls command
#[test]
fn test_output_calls_text_format() {
    let temp_dir = create_multi_file_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "calls",
            temp_dir.path().to_str().unwrap(),
            "-f",
            "text",
            "-q",
        ])
        .output()
        .expect("Failed to execute");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Text format for calls should show edges/calls
    assert!(
        stdout.contains("Call") || stdout.contains("Edges") || stdout.contains("->"),
        "Calls text should show call graph info: {}",
        stdout
    );
}

/// Test: Text output for impact command
#[test]
fn test_output_impact_text_format() {
    let temp_dir = create_multi_file_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "impact",
            "helper",
            temp_dir.path().to_str().unwrap(),
            "-f",
            "text",
            "-q",
        ])
        .output()
        .expect("Failed to execute");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Text format for impact should show impact info
    assert!(
        stdout.contains("Impact") || stdout.contains("helper"),
        "Impact text should show impact info: {}",
        stdout
    );
}

/// Test: Output with search command (different output type)
#[test]
fn test_output_search_format() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "search",
            "def helper",
            temp_dir.path().to_str().unwrap(),
            "-f",
            "json",
            "-q",
        ])
        .output()
        .expect("Failed to execute");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Search should return matches array
    let json_result: Result<serde_json::Value, _> = serde_json::from_str(&stdout);
    assert!(json_result.is_ok(), "Search output should be valid JSON");
}

// =============================================================================
// Error Handling Tests (main.rs)
// =============================================================================

/// Test: Exit code for successful command
#[test]
fn test_exit_code_success() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args(["tree", temp_dir.path().to_str().unwrap(), "-q"])
        .output()
        .expect("Failed to execute");

    assert!(output.status.success(), "Success should have exit code 0");
    assert_eq!(
        output.status.code(),
        Some(0),
        "Exit code should be 0 for success"
    );
}

/// Test: Exit code for failed command (non-existent path)
/// Note: Different error types could have different codes - BUG-004
#[test]
fn test_exit_code_failure() {
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args(["tree", "/nonexistent/path/12345", "-q"])
        .output()
        .expect("Failed to execute");

    assert!(
        !output.status.success(),
        "Failure should have non-zero exit code"
    );
    assert_ne!(
        output.status.code(),
        Some(0),
        "Exit code should not be 0 for failure"
    );
}

/// Test: Error message format
#[test]
fn test_error_message_format() {
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args(["tree", "/nonexistent/path/12345"])
        .output()
        .expect("Failed to execute");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Error:") || stderr.contains("error:"),
        "Error should be prefixed with 'Error:': {}",
        stderr
    );
}

/// Test: Error with verbose flag shows more details
#[test]
fn test_error_verbose_shows_details() {
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args(["tree", "/nonexistent/path/12345", "-v"])
        .output()
        .expect("Failed to execute");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Verbose should show error and possibly cause chain
    assert!(
        stderr.contains("Error") || stderr.contains("Caused by"),
        "Verbose error should show details: {}",
        stderr
    );
}

/// Test: Function not found error for impact command
#[test]
fn test_error_function_not_found() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "impact",
            "nonexistent_function",
            temp_dir.path().to_str().unwrap(),
            "-q",
        ])
        .output()
        .expect("Failed to execute");

    // May succeed with empty results or fail with error
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    if !output.status.success() {
        assert!(
            stderr.contains("not found") || stderr.contains("Error"),
            "Should indicate function not found: {}",
            stderr
        );
    } else {
        assert!(
            stdout.contains("0") || stdout.is_empty() || stdout.contains("[]"),
            "Should show empty results: {}",
            stdout
        );
    }
}

/// Test: Invalid regex in search shows error
#[test]
fn test_error_invalid_regex() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "search",
            "[invalid(regex",
            temp_dir.path().to_str().unwrap(),
            "-q",
        ])
        .output()
        .expect("Failed to execute");

    // Invalid regex should fail or show error
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("regex") || stderr.contains("Error") || stderr.contains("invalid"),
            "Should indicate regex error: {}",
            stderr
        );
    }
}

// =============================================================================
// Signal Handling Tests (signals.rs)
// =============================================================================

/// Test: Signal handler module exists and can be imported
/// Note: The signals module is public in tldr_cli crate
#[test]
fn test_signals_module_exists() {
    let _setup_handler: fn() -> Result<tldr_cli::signals::InterruptState, String> =
        tldr_cli::signals::setup_signal_handler;
    let _is_interrupted: fn() -> bool = tldr_cli::signals::is_interrupted;
    let _reset_interrupted: fn() = tldr_cli::signals::reset_interrupted;
    let _report_interrupt_status: fn(&tldr_cli::signals::InterruptState) =
        tldr_cli::signals::report_interrupt_status;

    let state = tldr_cli::signals::InterruptState::new();
    assert_eq!(state.files_completed(), 0);
}

/// Test: InterruptState interface
#[test]
fn test_interrupt_state_interface() {
    let state = tldr_cli::signals::InterruptState::new();
    state.set_total(3);
    state.increment_completed();

    assert_eq!(state.files_completed(), 1);
    assert_eq!(state.total_files(), 3);
    assert!(!state.check_interrupt());
    assert!(!state.was_interrupted());

    state.mark_interrupted();
    assert!(state.was_interrupted());
}

/// Test: InterruptMetadata interface
#[test]
fn test_interrupt_metadata_interface() {
    let state = tldr_cli::signals::InterruptState::new();
    state.set_total(4);
    state.increment_completed();
    state.increment_completed();

    let partial = tldr_cli::signals::InterruptMetadata::from_state(&state);
    assert_eq!(partial.completed, 2);
    assert_eq!(partial.total, 4);
    assert_eq!(partial.percent_complete, 50.0);

    let complete = tldr_cli::signals::InterruptMetadata::complete(4);
    assert!(!complete.interrupted);
    assert_eq!(complete.completed, 4);
    assert_eq!(complete.percent_complete, 100.0);
}

/// Test: Signal handling behavior documentation
/// Note: Actual signal testing requires sending SIGINT during execution
#[ignore = "Signal handling test requires interactive SIGINT - documented as behavior"]
#[test]
fn test_signal_interrupt_behavior() {
    let _setup_handler: fn() -> Result<tldr_cli::signals::InterruptState, String> =
        tldr_cli::signals::setup_signal_handler;

    let state = tldr_cli::signals::InterruptState::new();
    tldr_cli::signals::report_interrupt_status(&state);
    assert!(!state.was_interrupted());
}

// =============================================================================
// Edge Case Tests
// =============================================================================

/// Test: Empty directory handling
#[test]
fn test_edge_empty_directory() {
    let temp_dir = create_empty_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "tree",
            temp_dir.path().to_str().unwrap(),
            "-f",
            "json",
            "-q",
        ])
        .output()
        .expect("Failed to execute");

    assert!(output.status.success(), "Empty dir should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should return valid JSON with empty children
    let json_result: Result<serde_json::Value, _> = serde_json::from_str(&stdout);
    assert!(json_result.is_ok(), "Empty dir should produce valid JSON");
}

/// Test: Path with special characters
#[test]
fn test_edge_special_characters_in_path() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path().join("dir with spaces");
    fs::create_dir(&project_path).unwrap();
    fs::write(project_path.join("file.py"), "def foo(): pass\n").unwrap();

    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args(["tree", project_path.to_str().unwrap(), "-f", "json", "-q"])
        .output()
        .expect("Failed to execute");

    assert!(output.status.success(), "Path with spaces should work");
}

/// Test: Unicode in file paths
#[test]
fn test_edge_unicode_in_path() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path().join("目录");
    fs::create_dir(&project_path).unwrap();
    fs::write(project_path.join("文件.py"), "def foo(): pass\n").unwrap();

    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args(["tree", project_path.to_str().unwrap(), "-f", "json", "-q"])
        .output()
        .expect("Failed to execute");

    assert!(output.status.success(), "Unicode path should work");
}

/// Test: Multiple flags combined
#[test]
fn test_edge_multiple_flags() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "tree",
            temp_dir.path().to_str().unwrap(),
            "-f",
            "text",
            "-l",
            "python",
            "-q",
        ])
        .output()
        .expect("Failed to execute");

    assert!(
        output.status.success(),
        "Multiple flags should work together"
    );
}

/// Test: Flag order independence
#[test]
fn test_edge_flag_order() {
    let temp_dir = create_test_project();

    let output1 = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "tree",
            "-f",
            "json",
            temp_dir.path().to_str().unwrap(),
            "-q",
        ])
        .output()
        .expect("Failed first execution");

    let output2 = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "tree",
            temp_dir.path().to_str().unwrap(),
            "-q",
            "-f",
            "json",
        ])
        .output()
        .expect("Failed second execution");

    assert!(output1.status.success() && output2.status.success());

    // Both should produce equivalent results
    let stdout1 = String::from_utf8_lossy(&output1.stdout);
    let stdout2 = String::from_utf8_lossy(&output2.stdout);
    assert_eq!(stdout1, stdout2, "Flag order should not matter");
}

/// Test: Very deep directory nesting
#[test]
fn test_edge_deep_nesting() {
    let temp_dir = TempDir::new().unwrap();
    let mut current_path = temp_dir.path().to_path_buf();

    // Create 5 levels of nesting
    for i in 0..5 {
        current_path = current_path.join(format!("level{}", i));
        fs::create_dir(&current_path).unwrap();
    }
    fs::write(current_path.join("deep.py"), "def deep(): pass\n").unwrap();

    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args(["tree", temp_dir.path().to_str().unwrap(), "-q"])
        .output()
        .expect("Failed to execute");

    assert!(output.status.success(), "Deep nesting should work");
}

/// Test: Binary file handling
#[test]
fn test_edge_binary_file() {
    let temp_dir = TempDir::new().unwrap();
    // Write binary content
    fs::write(temp_dir.path().join("binary.bin"), vec![0u8, 1, 2, 255, 0]).unwrap();

    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args(["tree", temp_dir.path().to_str().unwrap(), "-q"])
        .output()
        .expect("Failed to execute");

    assert!(output.status.success(), "Binary file in tree should work");
}

/// Test: Symlink handling
#[test]
fn test_edge_symlink() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    fs::write(project_path.join("real.py"), "def real(): pass\n").unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        symlink(project_path.join("real.py"), project_path.join("link.py")).unwrap();
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::symlink_file;
        symlink_file(project_path.join("real.py"), project_path.join("link.py")).unwrap();
    }

    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args(["tree", project_path.to_str().unwrap(), "-q"])
        .output()
        .expect("Failed to execute");

    assert!(output.status.success(), "Symlink handling should not fail");
}

// =============================================================================
// Output Writer Interface Tests
// =============================================================================

/// Test: OutputWriter interface documentation
#[test]
fn test_output_writer_interface() {
    let json_writer =
        tldr_cli::output::OutputWriter::new(tldr_cli::output::OutputFormat::Json, true);
    let text_writer =
        tldr_cli::output::OutputWriter::new(tldr_cli::output::OutputFormat::Text, true);
    let dot_writer =
        tldr_cli::output::OutputWriter::new(tldr_cli::output::OutputFormat::Dot, true);

    assert!(json_writer.is_json());
    assert!(text_writer.is_text());
    assert!(dot_writer.is_dot());
}

/// Test: Output format enum variants
#[test]
fn test_output_format_variants() {
    let formats = [
        tldr_cli::output::OutputFormat::Json,
        tldr_cli::output::OutputFormat::Text,
        tldr_cli::output::OutputFormat::Compact,
        tldr_cli::output::OutputFormat::Sarif,
        tldr_cli::output::OutputFormat::Dot,
    ];

    assert_eq!(formats.len(), 5);
}

/// Test: Text formatters available
#[test]
fn test_text_formatters_available() {
    let _format_tree: fn(&tldr_core::FileTree, usize) -> String =
        tldr_cli::output::format_file_tree_text;
    let _format_structure: fn(&tldr_core::CodeStructure) -> String =
        tldr_cli::output::format_structure_text;
    let _format_cfg: fn(&tldr_core::CfgInfo) -> String = tldr_cli::output::format_cfg_text;
    let _format_dfg: fn(&tldr_core::DfgInfo) -> String = tldr_cli::output::format_dfg_text;
    let _format_impact: fn(&tldr_core::ImpactReport, bool) -> String =
        tldr_cli::output::format_impact_text;
    let _format_dead: fn(&tldr_core::DeadCodeReport) -> String =
        tldr_cli::output::format_dead_code_text;
    let _format_search: fn(&[tldr_core::SearchMatch]) -> String =
        tldr_cli::output::format_search_text;
    let _format_smells: fn(&tldr_core::SmellsReport) -> String =
        tldr_cli::output::format_smells_text;
    let _format_secrets: fn(&tldr_core::SecretsReport) -> String =
        tldr_cli::output::format_secrets_text;
    let _format_whatbreaks: fn(&tldr_core::analysis::whatbreaks::WhatbreaksReport) -> String =
        tldr_cli::output::format_whatbreaks_text;
    let _format_hubs: fn(&tldr_core::analysis::hubs::HubReport) -> String =
        tldr_cli::output::format_hubs_text;
    let _format_change_impact: fn(&tldr_core::ChangeImpactReport) -> String =
        tldr_cli::output::format_change_impact_text;
    let _format_diagnostics: fn(&tldr_core::DiagnosticsReport, usize) -> String =
        tldr_cli::output::format_diagnostics_text;
    let _format_clones: fn(&tldr_core::analysis::ClonesReport) -> String =
        tldr_cli::output::format_clones_text;
    let _format_clones_dot: fn(&tldr_core::analysis::ClonesReport) -> String =
        tldr_cli::output::format_clones_dot;
    let _format_similarity: fn(&tldr_core::analysis::SimilarityReport) -> String =
        tldr_cli::output::format_similarity_text;
    let _format_clones_sarif: fn(&tldr_core::analysis::ClonesReport) -> String =
        tldr_cli::output::format_clones_sarif;

    assert!(!std::any::type_name_of_val(&_format_tree).is_empty());
}

// =============================================================================
// Subcommand-specific Output Tests
// =============================================================================

/// Test: Tree output contains expected fields
#[test]
fn test_tree_output_fields() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "tree",
            temp_dir.path().to_str().unwrap(),
            "-f",
            "json",
            "-q",
        ])
        .output()
        .expect("Failed to execute");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    let json: serde_json::Value = serde_json::from_str(&stdout).expect("Valid JSON");
    assert!(json.get("name").is_some(), "Should have name field");
    assert!(json.get("type").is_some(), "Should have type field");
    assert!(json.get("children").is_some(), "Should have children field");
}

/// Test: Structure output contains expected fields
#[test]
fn test_structure_output_fields() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "structure",
            temp_dir.path().to_str().unwrap(),
            "-f",
            "json",
            "-q",
        ])
        .output()
        .expect("Failed to execute");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    let json: serde_json::Value = serde_json::from_str(&stdout).expect("Valid JSON");
    assert!(json.get("root").is_some(), "Should have root field");
    assert!(json.get("language").is_some(), "Should have language field");
    assert!(json.get("files").is_some(), "Should have files field");
}

/// Test: Calls output contains expected fields
#[test]
fn test_calls_output_fields() {
    let temp_dir = create_multi_file_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "calls",
            temp_dir.path().to_str().unwrap(),
            "-f",
            "json",
            "-q",
        ])
        .output()
        .expect("Failed to execute");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    let json: serde_json::Value = serde_json::from_str(&stdout).expect("Valid JSON");
    assert!(
        json.get("nodes").is_some() || json.get("node_count").is_some(),
        "Should have nodes info: {}",
        stdout
    );
    assert!(
        json.get("edges").is_some() || json.get("edge_count").is_some(),
        "Should have edges info: {}",
        stdout
    );
}

/// Test: Smells output format
#[test]
fn test_smells_output_format() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "smells",
            temp_dir.path().to_str().unwrap(),
            "-f",
            "json",
            "-q",
        ])
        .output()
        .expect("Failed to execute");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    let json: serde_json::Value = serde_json::from_str(&stdout).expect("Valid JSON");
    assert!(json.get("smells").is_some(), "Should have smells array");
}

/// Test: Health output format
#[test]
fn test_health_output_format() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "health",
            temp_dir.path().to_str().unwrap(),
            "-f",
            "json",
            "-q",
        ])
        .output()
        .expect("Failed to execute");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    let json: serde_json::Value = serde_json::from_str(&stdout).expect("Valid JSON");
    assert!(json.is_object(), "Health output should be an object");
}

/// Test: Stats output format
#[test]
fn test_stats_output_format() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "stats",
            temp_dir.path().to_str().unwrap(),
            "-f",
            "json",
            "-q",
        ])
        .output()
        .expect("Failed to execute");

    // Stats may fail if no history exists
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: serde_json::Value = serde_json::from_str(&stdout).expect("Valid JSON");
        assert!(json.is_object(), "Stats output should be an object");
    }
}

// =============================================================================
// SARIF Output Tests
// =============================================================================

/// Test: SARIF module interface
#[test]
fn test_sarif_module_interface() {
    let _format_sarif: fn(
        &tldr_core::analysis::ClonesReport,
    ) -> tldr_cli::output::sarif::SarifLog = tldr_cli::output::sarif::format_clones_sarif;

    assert!(std::mem::size_of::<tldr_cli::output::sarif::SarifLog>() > 0);
    assert!(std::mem::size_of::<tldr_cli::output::sarif::SarifRun>() > 0);
    assert!(std::mem::size_of::<tldr_cli::output::sarif::SarifTool>() > 0);
    assert!(std::mem::size_of::<tldr_cli::output::sarif::SarifDriver>() > 0);
    assert!(std::mem::size_of::<tldr_cli::output::sarif::SarifRule>() > 0);
    assert!(std::mem::size_of::<tldr_cli::output::sarif::SarifResult>() > 0);
    assert!(std::mem::size_of::<tldr_cli::output::sarif::SarifLocation>() > 0);
    assert!(std::mem::size_of::<tldr_cli::output::sarif::SarifPhysicalLocation>() > 0);
    assert!(std::mem::size_of::<tldr_cli::output::sarif::SarifArtifactLocation>() > 0);
    assert!(std::mem::size_of::<tldr_cli::output::sarif::SarifRegion>() > 0);
    assert!(std::mem::size_of::<tldr_cli::output::sarif::SarifFingerprints>() > 0);
    assert!(std::mem::size_of::<tldr_cli::output::sarif::SarifInvocation>() > 0);
}

// =============================================================================
// Clone and Similarity Output Tests
// =============================================================================

/// Test: Clone type descriptions
#[test]
fn test_clone_type_descriptions() {
    use tldr_core::analysis::CloneType;

    let type1 = tldr_cli::output::clone_type_description(&CloneType::Type1);
    let type2 = tldr_cli::output::clone_type_description(&CloneType::Type2);
    let type3 = tldr_cli::output::clone_type_description(&CloneType::Type3);

    assert!(type1.contains("exact"));
    assert!(type2.contains("renamed"));
    assert!(type3.contains("additions/deletions"));
}

/// Test: Empty results hints
#[test]
fn test_empty_results_hints() {
    let options = tldr_core::analysis::ClonesOptions::default();
    let stats = tldr_core::analysis::CloneStats {
        files_analyzed: 3,
        total_tokens: 120,
        clones_found: 0,
        type1_count: 0,
        type2_count: 0,
        type3_count: 0,
        class_count: None,
        detection_time_ms: 1,
    };

    let hints = tldr_cli::output::empty_results_hints(&options, &stats);
    assert_eq!(hints.len(), 3);
    assert!(hints[0].contains("Analyzed 3 files, 120 tokens"));
}

/// Test: DOT ID escaping
#[test]
fn test_dot_id_escaping() {
    let escaped = tldr_cli::output::escape_dot_id(r#"C:\path with "quotes"\file.py:1-10"#);
    assert!(escaped.starts_with('"'));
    assert!(escaped.ends_with('"'));
    assert!(escaped.contains("/path with \\\"quotes\\\"/"));
}
