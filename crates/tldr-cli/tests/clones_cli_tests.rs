//! CLI tests for clones and dice commands
//!
//! Tests the CLI interface for clone detection and similarity comparison.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

/// Helper to get the tldr binary
fn tldr_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
}

/// Create a temp directory with test Python files for clone detection
fn create_clone_fixtures() -> TempDir {
    let temp = TempDir::new().unwrap();

    // Type-1 clone: identical code in two files
    let file1_content = r#"
def calculate_sum(numbers):
    total = 0
    for num in numbers:
        total += num
    return total

def main():
    data = [1, 2, 3, 4, 5]
    result = calculate_sum(data)
    print(result)
"#;

    let file2_content = r#"
def calculate_sum(numbers):
    total = 0
    for num in numbers:
        total += num
    return total

def process():
    values = [10, 20, 30]
    output = calculate_sum(values)
    print(output)
"#;

    fs::write(temp.path().join("file1.py"), file1_content).unwrap();
    fs::write(temp.path().join("file2.py"), file2_content).unwrap();

    temp
}

// =============================================================================
// clones command tests
// =============================================================================

#[test]
fn test_clones_cli_basic() {
    let temp = create_clone_fixtures();

    tldr_cmd()
        .args(["clones", temp.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("clone_pairs"))
        .stdout(predicate::str::contains("stats"));
}

#[test]
fn test_clones_cli_json_output() {
    let temp = create_clone_fixtures();

    let output = tldr_cmd()
        .args(["clones", temp.path().to_str().unwrap(), "-o", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    // Verify it's valid JSON
    let result: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert!(result.get("clone_pairs").is_some());
    assert!(result.get("stats").is_some());
}

#[test]
fn test_clones_cli_text_output() {
    let temp = create_clone_fixtures();

    tldr_cmd()
        .args(["clones", temp.path().to_str().unwrap(), "-o", "text"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Clone Detection:"));
}

#[test]
fn test_clones_cli_with_threshold() {
    let temp = create_clone_fixtures();

    tldr_cmd()
        .args([
            "clones",
            temp.path().to_str().unwrap(),
            "--threshold",
            "0.9",
        ])
        .assert()
        .success();
}

#[test]
fn test_clones_cli_with_min_tokens() {
    let temp = create_clone_fixtures();

    tldr_cmd()
        .args([
            "clones",
            temp.path().to_str().unwrap(),
            "--min-tokens",
            "10",
        ])
        .assert()
        .success();
}

#[test]
fn test_clones_cli_with_min_lines() {
    let temp = create_clone_fixtures();

    tldr_cmd()
        .args(["clones", temp.path().to_str().unwrap(), "--min-lines", "3"])
        .assert()
        .success();
}

#[test]
fn test_clones_cli_type_filter() {
    let temp = create_clone_fixtures();

    // Filter for Type-1 clones only
    tldr_cmd()
        .args([
            "clones",
            temp.path().to_str().unwrap(),
            "--type-filter",
            "1",
        ])
        .assert()
        .success();
}

#[test]
fn test_clones_cli_normalize_option() {
    let temp = create_clone_fixtures();

    // Test different normalization modes
    for mode in &["none", "identifiers", "literals", "all"] {
        tldr_cmd()
            .args(["clones", temp.path().to_str().unwrap(), "--normalize", mode])
            .assert()
            .success();
    }
}

#[test]
fn test_clones_cli_show_classes() {
    let temp = create_clone_fixtures();

    let output = tldr_cmd()
        .args([
            "clones",
            temp.path().to_str().unwrap(),
            "--show-classes",
            "-o",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let result: serde_json::Value = serde_json::from_slice(&output).unwrap();
    // clone_classes field should be present (may be empty if no transitive clones)
    assert!(result.get("clone_classes").is_some() || result.get("stats").is_some());
}

#[test]
fn test_clones_cli_lang_filter() {
    let temp = create_clone_fixtures();

    tldr_cmd()
        .args([
            "clones",
            temp.path().to_str().unwrap(),
            "--language",
            "python",
        ])
        .assert()
        .success();
}

#[test]
fn test_clones_cli_max_clones() {
    let temp = create_clone_fixtures();

    tldr_cmd()
        .args(["clones", temp.path().to_str().unwrap(), "--max-clones", "5"])
        .assert()
        .success();
}

#[test]
fn test_clones_cli_include_within_file() {
    let temp = create_clone_fixtures();

    tldr_cmd()
        .args([
            "clones",
            temp.path().to_str().unwrap(),
            "--include-within-file",
        ])
        .assert()
        .success();
}

// =============================================================================
// dice command tests
// =============================================================================

#[test]
fn test_dice_cli_basic_files() {
    let temp = create_clone_fixtures();

    let file1 = temp.path().join("file1.py");
    let file2 = temp.path().join("file2.py");

    tldr_cmd()
        .args(["dice", file1.to_str().unwrap(), file2.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("dice_coefficient"));
}

#[test]
fn test_dice_cli_json_output() {
    let temp = create_clone_fixtures();

    let file1 = temp.path().join("file1.py");
    let file2 = temp.path().join("file2.py");

    let output = tldr_cmd()
        .args([
            "dice",
            file1.to_str().unwrap(),
            file2.to_str().unwrap(),
            "-o",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let result: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert!(result.get("dice_coefficient").is_some());
    assert!(result.get("interpretation").is_some());
}

#[test]
fn test_dice_cli_text_output() {
    let temp = create_clone_fixtures();

    let file1 = temp.path().join("file1.py");
    let file2 = temp.path().join("file2.py");

    tldr_cmd()
        .args([
            "dice",
            file1.to_str().unwrap(),
            file2.to_str().unwrap(),
            "-o",
            "text",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Similarity Comparison"));
}

#[test]
fn test_dice_cli_with_line_range() {
    let temp = create_clone_fixtures();

    let file1 = format!("{}:1:5", temp.path().join("file1.py").to_str().unwrap());
    let file2 = format!("{}:1:5", temp.path().join("file2.py").to_str().unwrap());

    tldr_cmd()
        .args(["dice", &file1, &file2])
        .assert()
        .success()
        .stdout(predicate::str::contains("dice_coefficient"));
}

#[test]
fn test_dice_cli_with_normalize() {
    let temp = create_clone_fixtures();

    let file1 = temp.path().join("file1.py");
    let file2 = temp.path().join("file2.py");

    for mode in &["none", "identifiers", "literals", "all"] {
        tldr_cmd()
            .args([
                "dice",
                file1.to_str().unwrap(),
                file2.to_str().unwrap(),
                "--normalize",
                mode,
            ])
            .assert()
            .success();
    }
}

#[test]
fn test_dice_cli_with_lang() {
    let temp = create_clone_fixtures();

    let file1 = temp.path().join("file1.py");
    let file2 = temp.path().join("file2.py");

    tldr_cmd()
        .args([
            "dice",
            file1.to_str().unwrap(),
            file2.to_str().unwrap(),
            "--language",
            "python",
        ])
        .assert()
        .success();
}

// =============================================================================
// Target parsing tests
// =============================================================================

#[test]
fn test_target_parsing_file_only() {
    let temp = create_clone_fixtures();

    let file1 = temp.path().join("file1.py");
    let file2 = temp.path().join("file2.py");

    // Simple file paths should work
    tldr_cmd()
        .args(["dice", file1.to_str().unwrap(), file2.to_str().unwrap()])
        .assert()
        .success();
}

#[test]
fn test_target_parsing_line_range() {
    let temp = create_clone_fixtures();

    // file:start:end format
    let file1 = format!("{}:2:6", temp.path().join("file1.py").to_str().unwrap());
    let file2 = format!("{}:2:6", temp.path().join("file2.py").to_str().unwrap());

    tldr_cmd().args(["dice", &file1, &file2]).assert().success();
}

#[test]
fn test_target_parsing_function_specifier() {
    let temp = create_clone_fixtures();

    // file::function format (may not extract function yet, but should parse)
    let file1 = format!(
        "{}::calculate_sum",
        temp.path().join("file1.py").to_str().unwrap()
    );
    let file2 = format!(
        "{}::calculate_sum",
        temp.path().join("file2.py").to_str().unwrap()
    );

    // This should succeed parsing-wise, though function extraction may fall back to full file
    tldr_cmd().args(["dice", &file1, &file2]).assert().success();
}

#[test]
fn test_target_parsing_invalid_line_numbers() {
    let temp = create_clone_fixtures();

    // Invalid line range (not numbers)
    let file1 = format!("{}:abc:def", temp.path().join("file1.py").to_str().unwrap());
    let file2 = temp.path().join("file2.py");

    tldr_cmd()
        .args(["dice", &file1, file2.to_str().unwrap()])
        .assert()
        .failure();
}

// =============================================================================
// Edge cases
// =============================================================================

#[test]
fn test_clones_cli_empty_directory() {
    let temp = TempDir::new().unwrap();

    // Empty directory should succeed but find no clones
    let output = tldr_cmd()
        .args(["clones", temp.path().to_str().unwrap(), "-o", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let result: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let pairs = result.get("clone_pairs").and_then(|v| v.as_array());
    assert!(pairs.is_none() || pairs.unwrap().is_empty());
}

#[test]
fn test_clones_cli_single_file() {
    let temp = TempDir::new().unwrap();

    let content = r#"
def foo():
    return 1

def bar():
    return 2
"#;
    fs::write(temp.path().join("single.py"), content).unwrap();

    // Single file with include-within-file should still work
    tldr_cmd()
        .args([
            "clones",
            temp.path().to_str().unwrap(),
            "--include-within-file",
        ])
        .assert()
        .success();
}

#[test]
fn test_dice_cli_same_file() {
    let temp = create_clone_fixtures();

    let file1 = temp.path().join("file1.py");

    // Comparing a file to itself should give 100% similarity
    let output = tldr_cmd()
        .args([
            "dice",
            file1.to_str().unwrap(),
            file1.to_str().unwrap(),
            "-o",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let result: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let dice = result
        .get("dice_coefficient")
        .and_then(|v| v.as_f64())
        .unwrap();
    assert!(
        (dice - 1.0).abs() < 0.001,
        "Same file should have 100% similarity"
    );
}

#[test]
fn test_dice_cli_nonexistent_file() {
    tldr_cmd()
        .args(["dice", "/nonexistent/path.py", "/another/missing.py"])
        .assert()
        .failure();
}
