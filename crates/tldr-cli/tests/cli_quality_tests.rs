//! CLI Quality Commands Test Suite
//!
//! Comprehensive test coverage for quality/health/debt/churn CLI commands:
//! - smells: Code smell detection (God Class, Long Method, etc.)
//! - health: Comprehensive code health dashboard
//! - debt: Technical debt analysis using SQALE method
//! - churn: Git-based file churn analysis
//! - maintainability: Maintainability Index calculation
//! - coverage: Parse and report code coverage from existing reports
//! - hotspots: Identify high-risk code regions (churn x complexity)
//!
//! Tests cover: CLI args, options, output formats, error handling

use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

/// Get the path to the tldr binary
fn tldr_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
}

/// Setup a temp directory with a git repository for churn/hotspots tests
fn setup_git_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    let path = temp.path();

    // Initialize git repo
    let _ = Command::new("git")
        .args(["init"])
        .current_dir(path)
        .output();

    // Configure git user
    let _ = Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(path)
        .output();
    let _ = Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(path)
        .output();

    temp
}

/// Create initial commit in git repo
fn create_initial_commit(temp: &TempDir) {
    let path = temp.path();
    let _ = Command::new("git")
        .args(["add", "."])
        .current_dir(path)
        .output();
    let _ = Command::new("git")
        .args(["commit", "-m", "initial"])
        .current_dir(path)
        .output();
}

// =============================================================================
// Smells Command Tests
// =============================================================================

#[cfg(test)]
mod smells_tests {
    use super::*;

    /// Test smells command help output
    #[test]
    fn test_smells_help() {
        let mut cmd = tldr_cmd();
        cmd.args(["smells", "--help"]);
        cmd.assert()
            .success()
            .stdout(predicate::str::contains("code smells"))
            .stdout(predicate::str::contains("--threshold"))
            .stdout(predicate::str::contains("--smell-type"))
            .stdout(predicate::str::contains("--suggest"));
    }

    /// Test smells command with default path
    #[test]
    fn test_smells_default_path() {
        let temp = TempDir::new().unwrap();
        fs::write(
            temp.path().join("test.py"),
            r#"
def very_long_function_name_that_does_too_many_things(arg1, arg2, arg3, arg4, arg5):
    x = 1
    y = 2
    z = 3
    a = 4
    b = 5
    c = 6
    d = 7
    e = 8
    f = 9
    g = 10
    return x + y + z + a + b + c + d + e + f + g

class GodClass:
    def method1(self): pass
    def method2(self): pass
    def method3(self): pass
    def method4(self): pass
    def method5(self): pass
"#,
        )
        .unwrap();

        let mut cmd = tldr_cmd();
        cmd.current_dir(temp.path());
        cmd.args(["smells", "-q"]);
        cmd.assert().success();
    }

    /// Test smells command with explicit path
    #[test]
    fn test_smells_explicit_path() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["smells", temp.path().to_str().unwrap(), "-q"]);
        cmd.assert().success();
    }

    /// Test smells command with threshold presets
    #[test]
    fn test_smells_threshold_strict() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "smells",
            temp.path().to_str().unwrap(),
            "--threshold",
            "strict",
            "-q",
        ]);
        cmd.assert().success();
    }

    #[test]
    fn test_smells_threshold_relaxed() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "smells",
            temp.path().to_str().unwrap(),
            "--threshold",
            "relaxed",
            "-q",
        ]);
        cmd.assert().success();
    }

    /// Test smells command with smell type filter
    #[test]
    fn test_smells_filter_god_class() {
        let temp = TempDir::new().unwrap();
        fs::write(
            temp.path().join("test.py"),
            r#"
class BigClass:
    def m1(self): pass
    def m2(self): pass
    def m3(self): pass
"#,
        )
        .unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "smells",
            temp.path().to_str().unwrap(),
            "--smell-type",
            "god-class",
            "-q",
        ]);
        cmd.assert().success();
    }

    #[test]
    fn test_smells_filter_long_method() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "smells",
            temp.path().to_str().unwrap(),
            "--smell-type",
            "long-method",
            "-q",
        ]);
        cmd.assert().success();
    }

    #[test]
    fn test_smells_filter_long_parameter_list() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "smells",
            temp.path().to_str().unwrap(),
            "--smell-type",
            "long-parameter-list",
            "-q",
        ]);
        cmd.assert().success();
    }

    #[test]
    fn test_smells_filter_feature_envy() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "smells",
            temp.path().to_str().unwrap(),
            "--smell-type",
            "feature-envy",
            "-q",
        ]);
        cmd.assert().success();
    }

    #[test]
    fn test_smells_filter_data_clumps() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "smells",
            temp.path().to_str().unwrap(),
            "--smell-type",
            "data-clumps",
            "-q",
        ]);
        cmd.assert().success();
    }

    /// Test smells CLI accepts --smell-type middle-man
    #[test]
    fn test_smells_filter_middle_man() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "smells",
            temp.path().to_str().unwrap(),
            "--smell-type",
            "middle-man",
            "-q",
        ]);
        cmd.assert().success();
    }

    /// Test smells CLI accepts --smell-type refused-bequest
    #[test]
    fn test_smells_filter_refused_bequest() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "smells",
            temp.path().to_str().unwrap(),
            "--smell-type",
            "refused-bequest",
            "-q",
        ]);
        cmd.assert().success();
    }

    /// Test smells CLI accepts --smell-type inappropriate-intimacy
    #[test]
    fn test_smells_filter_inappropriate_intimacy() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "smells",
            temp.path().to_str().unwrap(),
            "--smell-type",
            "inappropriate-intimacy",
            "-q",
        ]);
        cmd.assert().success();
    }

    /// Test smells command with suggestions flag
    #[test]
    fn test_smells_suggest_flag() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["smells", temp.path().to_str().unwrap(), "--suggest", "-q"]);
        cmd.assert().success();
    }

    /// Test smells command JSON output
    #[test]
    fn test_smells_json_output() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["smells", temp.path().to_str().unwrap(), "-f", "json", "-q"]);
        let output = cmd.output().unwrap();

        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(json.is_object());
    }

    /// Test smells command compact output
    #[test]
    fn test_smells_compact_output() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "smells",
            temp.path().to_str().unwrap(),
            "-f",
            "compact",
            "-q",
        ]);
        let output = cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);

        let lines: Vec<&str> = stdout.lines().collect();
        assert_eq!(lines.len(), 1, "Compact output should be single line");
    }

    /// Test smells command with nonexistent path
    #[test]
    #[ignore = "BUG: See bugs_cli_quality.md - Issue 9"]
    fn test_smells_nonexistent_path() {
        let mut cmd = tldr_cmd();
        cmd.args(["smells", "/nonexistent/path/xyz123", "-q"]);
        cmd.assert().failure();
    }

    /// Test smells command with empty directory
    #[test]
    fn test_smells_empty_directory() {
        let temp = TempDir::new().unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["smells", temp.path().to_str().unwrap(), "-q"]);
        cmd.assert().success();
    }
}

// =============================================================================
// Health Command Tests
// =============================================================================

#[cfg(test)]
mod health_tests {
    use super::*;

    /// Test health command help output
    #[test]
    fn test_health_help() {
        let mut cmd = tldr_cmd();
        cmd.args(["health", "--help"]);
        cmd.assert()
            .success()
            .stdout(predicate::str::contains("health"))
            .stdout(predicate::str::contains("--detail"))
            .stdout(predicate::str::contains("--quick"))
            .stdout(predicate::str::contains("--preset"));
    }

    /// Test health command with default path
    #[test]
    fn test_health_default_path() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.current_dir(temp.path());
        cmd.args(["health", "-q"]);
        cmd.assert().success();
    }

    /// Test health command with explicit path
    #[test]
    fn test_health_explicit_path() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["health", temp.path().to_str().unwrap(), "-q"]);
        cmd.assert().success();
    }

    /// Test health command with quick mode
    #[test]
    fn test_health_quick_mode() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["health", temp.path().to_str().unwrap(), "--quick", "-q"]);
        cmd.assert().success();
    }

    /// Test health command with all detail options
    #[test]
    fn test_health_detail_complexity() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "health",
            temp.path().to_str().unwrap(),
            "--detail",
            "complexity",
            "-q",
        ]);
        cmd.assert().success();
    }

    #[test]
    fn test_health_detail_cohesion() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "health",
            temp.path().to_str().unwrap(),
            "--detail",
            "cohesion",
            "-q",
        ]);
        cmd.assert().success();
    }

    #[test]
    fn test_health_detail_all() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "health",
            temp.path().to_str().unwrap(),
            "--detail",
            "all",
            "-q",
        ]);
        cmd.assert().success();
    }

    /// Test health command quick + coupling conflict (should fail)
    #[test]
    fn test_health_quick_coupling_conflict() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "health",
            temp.path().to_str().unwrap(),
            "--quick",
            "--detail",
            "coupling",
            "-q",
        ]);
        cmd.assert()
            .failure()
            .stderr(predicate::str::contains("requires full mode"));
    }

    /// Test health command quick + similarity conflict (should fail)
    #[test]
    fn test_health_quick_similarity_conflict() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "health",
            temp.path().to_str().unwrap(),
            "--quick",
            "--detail",
            "similarity",
            "-q",
        ]);
        cmd.assert()
            .failure()
            .stderr(predicate::str::contains("requires full mode"));
    }

    /// Test health command with preset options
    #[test]
    fn test_health_preset_strict() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "health",
            temp.path().to_str().unwrap(),
            "--preset",
            "strict",
            "-q",
        ]);
        cmd.assert().success();
    }

    #[test]
    fn test_health_preset_default() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "health",
            temp.path().to_str().unwrap(),
            "--preset",
            "default",
            "-q",
        ]);
        cmd.assert().success();
    }

    #[test]
    fn test_health_preset_relaxed() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "health",
            temp.path().to_str().unwrap(),
            "--preset",
            "relaxed",
            "-q",
        ]);
        cmd.assert().success();
    }

    /// Test health command with invalid detail option
    #[test]
    fn test_health_invalid_detail() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "health",
            temp.path().to_str().unwrap(),
            "--detail",
            "invalid_option",
            "-q",
        ]);
        cmd.assert().failure();
    }

    /// Test health command JSON output
    #[test]
    fn test_health_json_output() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["health", temp.path().to_str().unwrap(), "-f", "json", "-q"]);
        let output = cmd.output().unwrap();

        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(json.is_object());
    }

    /// Test health command with nonexistent path
    #[test]
    fn test_health_nonexistent_path() {
        let mut cmd = tldr_cmd();
        cmd.args(["health", "/nonexistent/path/xyz123", "-q"]);
        cmd.assert()
            .failure()
            .stderr(predicate::str::contains("Path not found"));
    }

    /// Test health command with language flag
    #[test]
    fn test_health_with_language() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "health",
            temp.path().to_str().unwrap(),
            "--lang",
            "python",
            "-q",
        ]);
        cmd.assert().success();
    }
}

// =============================================================================
// Debt Command Tests
// =============================================================================

#[cfg(test)]
mod debt_tests {
    use super::*;

    /// Test debt command help output
    #[test]
    fn test_debt_help() {
        let mut cmd = tldr_cmd();
        cmd.args(["debt", "--help"]);
        cmd.assert()
            .success()
            .stdout(predicate::str::contains("debt"))
            .stdout(predicate::str::contains("--category"))
            .stdout(predicate::str::contains("--top"))
            .stdout(predicate::str::contains("--min-debt"))
            .stdout(predicate::str::contains("--hourly-rate"));
    }

    /// Test debt command with default path
    #[test]
    fn test_debt_default_path() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.current_dir(temp.path());
        cmd.args(["debt", "-q"]);
        cmd.assert().success();
    }

    /// Test debt command with explicit path
    #[test]
    fn test_debt_explicit_path() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["debt", temp.path().to_str().unwrap(), "-q"]);
        cmd.assert().success();
    }

    /// Test debt command with category filters
    #[test]
    fn test_debt_category_reliability() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "debt",
            temp.path().to_str().unwrap(),
            "--category",
            "reliability",
            "-q",
        ]);
        cmd.assert().success();
    }

    #[test]
    fn test_debt_category_security() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "debt",
            temp.path().to_str().unwrap(),
            "--category",
            "security",
            "-q",
        ]);
        cmd.assert().success();
    }

    #[test]
    fn test_debt_category_maintainability() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "debt",
            temp.path().to_str().unwrap(),
            "--category",
            "maintainability",
            "-q",
        ]);
        cmd.assert().success();
    }

    #[test]
    fn test_debt_category_efficiency() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "debt",
            temp.path().to_str().unwrap(),
            "--category",
            "efficiency",
            "-q",
        ]);
        cmd.assert().success();
    }

    #[test]
    fn test_debt_category_changeability() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "debt",
            temp.path().to_str().unwrap(),
            "--category",
            "changeability",
            "-q",
        ]);
        cmd.assert().success();
    }

    #[test]
    fn test_debt_category_testability() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "debt",
            temp.path().to_str().unwrap(),
            "--category",
            "testability",
            "-q",
        ]);
        cmd.assert().success();
    }

    /// Test debt command with top limit
    #[test]
    fn test_debt_top_limit() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["debt", temp.path().to_str().unwrap(), "--top", "5", "-q"]);
        cmd.assert().success();
    }

    /// Test debt command with min-debt filter
    #[test]
    fn test_debt_min_debt() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "debt",
            temp.path().to_str().unwrap(),
            "--min-debt",
            "10",
            "-q",
        ]);
        cmd.assert().success();
    }

    /// Test debt command with hourly rate
    #[test]
    fn test_debt_hourly_rate() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "debt",
            temp.path().to_str().unwrap(),
            "--hourly-rate",
            "100.0",
            "-q",
        ]);
        cmd.assert().success();
    }

    /// Test debt command JSON output
    #[test]
    fn test_debt_json_output() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["debt", temp.path().to_str().unwrap(), "-f", "json", "-q"]);
        let output = cmd.output().unwrap();

        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(json.is_object());
    }

    /// Test debt command with nonexistent path
    #[test]
    fn test_debt_nonexistent_path() {
        let mut cmd = tldr_cmd();
        cmd.args(["debt", "/nonexistent/path/xyz123", "-q"]);
        cmd.assert()
            .failure()
            .stderr(predicate::str::contains("Path not found"));
    }

    /// Test debt command with invalid category (clap validates this)
    #[test]
    #[ignore = "BUG: See bugs_cli_quality.md - Issue 3"]
    fn test_debt_invalid_category() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "debt",
            temp.path().to_str().unwrap(),
            "--category",
            "invalid",
            "-q",
        ]);
        cmd.assert().failure();
    }
}

// =============================================================================
// Churn Command Tests
// =============================================================================

#[cfg(test)]
mod churn_tests {
    use super::*;

    /// Test churn command help output
    #[test]
    fn test_churn_help() {
        let mut cmd = tldr_cmd();
        cmd.args(["churn", "--help"]);
        cmd.assert()
            .success()
            .stdout(predicate::str::contains("churn"))
            .stdout(predicate::str::contains("--days"))
            .stdout(predicate::str::contains("--top"))
            .stdout(predicate::str::contains("--exclude"))
            .stdout(predicate::str::contains("--authors"))
            .stdout(predicate::str::contains("--hotspots"));
    }

    /// Test churn command with git repository
    #[test]
    fn test_churn_git_repo() {
        let temp = setup_git_repo();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();
        create_initial_commit(&temp);

        let mut cmd = tldr_cmd();
        cmd.args(["churn", temp.path().to_str().unwrap(), "-q"]);
        cmd.assert().success();
    }

    /// Test churn command with days option
    #[test]
    fn test_churn_days() {
        let temp = setup_git_repo();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();
        create_initial_commit(&temp);

        let mut cmd = tldr_cmd();
        cmd.args(["churn", temp.path().to_str().unwrap(), "--days", "30", "-q"]);
        cmd.assert().success();
    }

    /// Test churn command with top limit
    #[test]
    fn test_churn_top() {
        let temp = setup_git_repo();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();
        create_initial_commit(&temp);

        let mut cmd = tldr_cmd();
        cmd.args(["churn", temp.path().to_str().unwrap(), "--top", "10", "-q"]);
        cmd.assert().success();
    }

    /// Test churn command with exclude pattern
    #[test]
    fn test_churn_exclude() {
        let temp = setup_git_repo();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();
        fs::write(temp.path().join("test.txt"), "text").unwrap();
        create_initial_commit(&temp);

        let mut cmd = tldr_cmd();
        cmd.args([
            "churn",
            temp.path().to_str().unwrap(),
            "--exclude",
            "*.txt",
            "-q",
        ]);
        cmd.assert().success();
    }

    /// Test churn command with authors flag
    #[test]
    fn test_churn_authors() {
        let temp = setup_git_repo();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();
        create_initial_commit(&temp);

        let mut cmd = tldr_cmd();
        cmd.args(["churn", temp.path().to_str().unwrap(), "--authors", "-q"]);
        cmd.assert().success();
    }

    /// Test churn command with hotspots flag
    #[test]
    fn test_churn_hotspots() {
        let temp = setup_git_repo();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();
        create_initial_commit(&temp);

        let mut cmd = tldr_cmd();
        cmd.args(["churn", temp.path().to_str().unwrap(), "--hotspots", "-q"]);
        cmd.assert().success();
    }

    /// Test churn command JSON output
    #[test]
    fn test_churn_json_output() {
        let temp = setup_git_repo();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();
        create_initial_commit(&temp);

        let mut cmd = tldr_cmd();
        cmd.args(["churn", temp.path().to_str().unwrap(), "-f", "json", "-q"]);
        let output = cmd.output().unwrap();

        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(json.is_object());
    }

    /// Test churn command with non-git directory
    #[test]
    fn test_churn_not_git_repo() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["churn", temp.path().to_str().unwrap(), "-q"]);
        cmd.assert()
            .failure()
            .stderr(predicate::str::contains("Not a git repository"));
    }
}

// =============================================================================
// Coverage Command Tests
// =============================================================================

#[cfg(test)]
mod coverage_tests {
    use super::*;

    /// Test coverage command help output
    #[test]
    fn test_coverage_help() {
        let mut cmd = tldr_cmd();
        cmd.args(["coverage", "--help"]);
        cmd.assert()
            .success()
            .stdout(predicate::str::contains("coverage"))
            .stdout(predicate::str::contains("--report-format"))
            .stdout(predicate::str::contains("--threshold"))
            .stdout(predicate::str::contains("--by-file"))
            .stdout(predicate::str::contains("--uncovered"))
            .stdout(predicate::str::contains("--uncovered-only"));
    }

    /// Create a sample Cobertura XML coverage report
    fn create_cobertura_report(temp: &TempDir) -> PathBuf {
        let report_path = temp.path().join("coverage.xml");
        let xml_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<coverage line-rate="0.85" branch-rate="0.75" version="1.9">
    <sources>
        <source>.</source>
    </sources>
    <packages>
        <package name="pkg" line-rate="0.85">
            <classes>
                <class name="module" filename="module.py" line-rate="0.85">
                    <methods/>
                    <lines>
                        <line number="1" hits="1"/>
                        <line number="2" hits="1"/>
                        <line number="3" hits="0"/>
                        <line number="4" hits="1"/>
                    </lines>
                </class>
            </classes>
        </package>
    </packages>
</coverage>"#;
        fs::write(&report_path, xml_content).unwrap();
        report_path
    }

    /// Create a sample LCOV coverage report
    fn create_lcov_report(temp: &TempDir) -> PathBuf {
        let report_path = temp.path().join("coverage.lcov");
        let lcov_content = r#"SF:module.py
DA:1,1
DA:2,1
DA:3,0
DA:4,1
LF:4
LH:3
end_of_record"#;
        fs::write(&report_path, lcov_content).unwrap();
        report_path
    }

    /// Test coverage command with Cobertura XML
    #[test]
    fn test_coverage_cobertura() {
        let temp = TempDir::new().unwrap();
        let report = create_cobertura_report(&temp);

        let mut cmd = tldr_cmd();
        cmd.args(["coverage", report.to_str().unwrap(), "-q"]);
        cmd.assert().success();
    }

    /// Test coverage command with explicit Cobertura format
    #[test]
    fn test_coverage_cobertura_explicit() {
        let temp = TempDir::new().unwrap();
        let report = create_cobertura_report(&temp);

        let mut cmd = tldr_cmd();
        cmd.args([
            "coverage",
            report.to_str().unwrap(),
            "--report-format",
            "cobertura",
            "-q",
        ]);
        cmd.assert().success();
    }

    /// Test coverage command with LCOV format
    #[test]
    fn test_coverage_lcov() {
        let temp = TempDir::new().unwrap();
        let report = create_lcov_report(&temp);

        let mut cmd = tldr_cmd();
        cmd.args([
            "coverage",
            report.to_str().unwrap(),
            "--report-format",
            "lcov",
            "-q",
        ]);
        cmd.assert().success();
    }

    /// Test coverage command with threshold option
    #[test]
    fn test_coverage_threshold() {
        let temp = TempDir::new().unwrap();
        let report = create_cobertura_report(&temp);

        let mut cmd = tldr_cmd();
        cmd.args([
            "coverage",
            report.to_str().unwrap(),
            "--threshold",
            "80.0",
            "-q",
        ]);
        cmd.assert().success();
    }

    /// Test coverage command with by-file option
    #[test]
    fn test_coverage_by_file() {
        let temp = TempDir::new().unwrap();
        let report = create_cobertura_report(&temp);

        let mut cmd = tldr_cmd();
        cmd.args(["coverage", report.to_str().unwrap(), "--by-file", "-q"]);
        cmd.assert().success();
    }

    /// Test coverage command with uncovered option
    #[test]
    fn test_coverage_uncovered() {
        let temp = TempDir::new().unwrap();
        let report = create_cobertura_report(&temp);

        let mut cmd = tldr_cmd();
        cmd.args(["coverage", report.to_str().unwrap(), "--uncovered", "-q"]);
        cmd.assert().success();
    }

    /// Test coverage command with uncovered-only option
    #[test]
    fn test_coverage_uncovered_only() {
        let temp = TempDir::new().unwrap();
        let report = create_cobertura_report(&temp);

        let mut cmd = tldr_cmd();
        cmd.args([
            "coverage",
            report.to_str().unwrap(),
            "--uncovered-only",
            "-q",
        ]);
        cmd.assert().success();
    }

    /// Test coverage command with filter option
    #[test]
    fn test_coverage_filter() {
        let temp = TempDir::new().unwrap();
        let report = create_cobertura_report(&temp);

        let mut cmd = tldr_cmd();
        cmd.args([
            "coverage",
            report.to_str().unwrap(),
            "--filter",
            "module",
            "-q",
        ]);
        cmd.assert().success();
    }

    /// Test coverage command with sort asc
    #[test]
    fn test_coverage_sort_asc() {
        let temp = TempDir::new().unwrap();
        let report = create_cobertura_report(&temp);

        let mut cmd = tldr_cmd();
        cmd.args(["coverage", report.to_str().unwrap(), "--sort", "asc", "-q"]);
        cmd.assert().success();
    }

    /// Test coverage command with sort desc
    #[test]
    fn test_coverage_sort_desc() {
        let temp = TempDir::new().unwrap();
        let report = create_cobertura_report(&temp);

        let mut cmd = tldr_cmd();
        cmd.args(["coverage", report.to_str().unwrap(), "--sort", "desc", "-q"]);
        cmd.assert().success();
    }

    /// Test coverage command JSON output
    #[test]
    fn test_coverage_json_output() {
        let temp = TempDir::new().unwrap();
        let report = create_cobertura_report(&temp);

        let mut cmd = tldr_cmd();
        cmd.args(["coverage", report.to_str().unwrap(), "-f", "json", "-q"]);
        let output = cmd.output().unwrap();

        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(json.is_object());
    }

    /// Test coverage command with nonexistent report file
    #[test]
    fn test_coverage_nonexistent_file() {
        let mut cmd = tldr_cmd();
        cmd.args(["coverage", "/nonexistent/coverage.xml", "-q"]);
        cmd.assert().failure();
    }
}

// =============================================================================
// Hotspots Command Tests
// =============================================================================

#[cfg(test)]
mod hotspots_tests {
    use super::*;

    /// Test hotspots command help output
    #[test]
    fn test_hotspots_help() {
        let mut cmd = tldr_cmd();
        cmd.args(["hotspots", "--help"]);
        cmd.assert()
            .success()
            .stdout(predicate::str::contains("hotspot"))
            .stdout(predicate::str::contains("--days"))
            .stdout(predicate::str::contains("--top"))
            .stdout(predicate::str::contains("--by-function"))
            .stdout(predicate::str::contains("--show-trend"))
            .stdout(predicate::str::contains("--min-commits"))
            .stdout(predicate::str::contains("--exclude"))
            .stdout(predicate::str::contains("--threshold"))
            .stdout(predicate::str::contains("--since"));
    }

    /// Test hotspots command with git repository
    #[test]
    fn test_hotspots_git_repo() {
        let temp = setup_git_repo();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();
        create_initial_commit(&temp);

        let mut cmd = tldr_cmd();
        cmd.args(["hotspots", temp.path().to_str().unwrap(), "-q"]);
        cmd.assert().success();
    }

    /// Test hotspots command with days option
    #[test]
    fn test_hotspots_days() {
        let temp = setup_git_repo();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();
        create_initial_commit(&temp);

        let mut cmd = tldr_cmd();
        cmd.args([
            "hotspots",
            temp.path().to_str().unwrap(),
            "--days",
            "90",
            "-q",
        ]);
        cmd.assert().success();
    }

    /// Test hotspots command with top limit
    #[test]
    fn test_hotspots_top() {
        let temp = setup_git_repo();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();
        create_initial_commit(&temp);

        let mut cmd = tldr_cmd();
        cmd.args([
            "hotspots",
            temp.path().to_str().unwrap(),
            "--top",
            "10",
            "-q",
        ]);
        cmd.assert().success();
    }

    /// Test hotspots command with by-function flag
    #[test]
    fn test_hotspots_by_function() {
        let temp = setup_git_repo();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();
        create_initial_commit(&temp);

        let mut cmd = tldr_cmd();
        cmd.args([
            "hotspots",
            temp.path().to_str().unwrap(),
            "--by-function",
            "-q",
        ]);
        cmd.assert().success();
    }

    /// Test hotspots command with show-trend flag
    #[test]
    fn test_hotspots_show_trend() {
        let temp = setup_git_repo();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();
        create_initial_commit(&temp);

        let mut cmd = tldr_cmd();
        cmd.args([
            "hotspots",
            temp.path().to_str().unwrap(),
            "--show-trend",
            "-q",
        ]);
        cmd.assert().success();
    }

    /// Test hotspots command with min-commits
    #[test]
    fn test_hotspots_min_commits() {
        let temp = setup_git_repo();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();
        create_initial_commit(&temp);

        let mut cmd = tldr_cmd();
        cmd.args([
            "hotspots",
            temp.path().to_str().unwrap(),
            "--min-commits",
            "1",
            "-q",
        ]);
        cmd.assert().success();
    }

    /// Test hotspots command with exclude pattern
    #[test]
    fn test_hotspots_exclude() {
        let temp = setup_git_repo();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();
        fs::write(temp.path().join("test.txt"), "text").unwrap();
        create_initial_commit(&temp);

        let mut cmd = tldr_cmd();
        cmd.args([
            "hotspots",
            temp.path().to_str().unwrap(),
            "--exclude",
            "*.txt",
            "-q",
        ]);
        cmd.assert().success();
    }

    /// Test hotspots command with threshold
    #[test]
    fn test_hotspots_threshold() {
        let temp = setup_git_repo();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();
        create_initial_commit(&temp);

        let mut cmd = tldr_cmd();
        cmd.args([
            "hotspots",
            temp.path().to_str().unwrap(),
            "--threshold",
            "0.5",
            "-q",
        ]);
        cmd.assert().success();
    }

    /// Test hotspots command with since date
    #[test]
    fn test_hotspots_since() {
        let temp = setup_git_repo();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();
        create_initial_commit(&temp);

        let mut cmd = tldr_cmd();
        cmd.args([
            "hotspots",
            temp.path().to_str().unwrap(),
            "--since",
            "2024-01-01",
            "-q",
        ]);
        cmd.assert().success();
    }

    /// Test hotspots command JSON output
    #[test]
    fn test_hotspots_json_output() {
        let temp = setup_git_repo();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();
        create_initial_commit(&temp);

        let mut cmd = tldr_cmd();
        cmd.args([
            "hotspots",
            temp.path().to_str().unwrap(),
            "-f",
            "json",
            "-q",
        ]);
        let output = cmd.output().unwrap();

        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(json.is_object());
    }

    /// Test hotspots command with non-git directory
    #[test]
    fn test_hotspots_not_git_repo() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["hotspots", temp.path().to_str().unwrap(), "-q"]);
        cmd.assert()
            .failure()
            .stderr(predicate::str::contains("Not a git repository"));
    }
}
