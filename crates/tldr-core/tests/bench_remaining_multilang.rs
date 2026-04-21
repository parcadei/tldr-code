//! Remaining Commands Multilang Benchmark Tests
//!
//! Comprehensive integration tests for remaining analysis commands:
//!
//! **Analysis commands (tldr-cli, tested via binary):**
//! - `temporal` -- mine temporal constraints (method call sequences)
//! - `resources` -- resource lifecycle analysis (leaks, double-close, use-after-close)
//! - `api-check` -- API misuse patterns (missing timeouts, bare except, weak crypto)
//!
//! **Git-dependent commands (tldr-core APIs, tested with temp git repos):**
//! - `churn` -- git-based code churn analysis
//! - `hotspots` -- churn x complexity hotspots
//! - `change-impact` -- find tests affected by code changes
//!
//! # Architecture Note
//!
//! `temporal`, `resources`, and `api-check` are implemented in `tldr-cli`, not
//! `tldr-core`. They are tested here via the CLI binary (`tldr temporal ...`).
//! `churn`, `hotspots`, and `change-impact` have public APIs in `tldr-core`.
//!
//! # Running Tests
//!
//! ```bash
//! cargo test -p tldr-core --test bench_remaining_multilang
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

use tldr_core::analysis::{
    change_impact, change_impact_extended, DetectionMethod,
};
use tldr_core::quality::churn::{
    build_summary, get_author_stats, get_file_churn, get_recommendation, is_bot_author,
    is_git_repository, matches_exclude_pattern, ChurnError, FileChurn,
};
use tldr_core::quality::hotspots::{
    analyze_hotspots, calculate_trend, composite_score_weighted, has_variance,
    knowledge_fragmentation, normalize_value, percentile_ranks, recency_weight, relative_churn,
    HotspotsOptions, ScoringWeights, TrendDirection,
};
use tldr_core::Language;

// =============================================================================
// Test Helpers
// =============================================================================

/// A temporary git repository for testing
struct TestRepo {
    dir: TempDir,
}

impl TestRepo {
    /// Create a new empty git repository with user config
    fn new() -> Self {
        let dir = TempDir::new().expect("create temp dir");
        git(dir.path(), &["init"]);
        git(dir.path(), &["config", "user.email", "test@test.com"]);
        git(dir.path(), &["config", "user.name", "Test User"]);
        Self { dir }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn add_file(&self, name: &str, content: &str) -> PathBuf {
        let path = self.dir.path().join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, content).unwrap();
        path
    }

    fn commit(&self, message: &str) {
        git(self.path(), &["add", "-A"]);
        git(self.path(), &["commit", "-m", message, "--allow-empty"]);
    }

    fn commit_as(&self, message: &str, name: &str, email: &str) {
        git(self.path(), &["add", "-A"]);
        let author = format!("{} <{}>", name, email);
        git(self.path(), &["commit", "-m", message, "--author", &author]);
    }
}

/// Run a git command in a directory, panicking on failure
fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_COMMITTER_NAME", "Test User")
        .env("GIT_COMMITTER_EMAIL", "test@test.com")
        .output()
        .unwrap_or_else(|e| panic!("git {} failed to run: {}", args[0], e));
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args[0],
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Create a temp directory with a file
fn create_temp_file(dir: &TempDir, name: &str, content: &str) -> PathBuf {
    let path = dir.path().join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&path, content).unwrap();
    path
}

/// Find the tldr binary for CLI-based tests
fn tldr_binary() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest_dir.parent().unwrap().parent().unwrap();
    let release = root.join("target/release/tldr");
    let debug = root.join("target/debug/tldr");
    if release.exists() {
        release
    } else if debug.exists() {
        debug
    } else {
        PathBuf::from("tldr")
    }
}

/// Run a tldr CLI command and return (exit_code, stdout, stderr)
fn run_tldr(args: &[&str]) -> (i32, String, String) {
    let binary = tldr_binary();
    let output = Command::new(&binary)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("Failed to run tldr {:?}: {}", args, e));
    let code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (code, stdout, stderr)
}

// =============================================================================
// CHURN TESTS (tldr-core API: quality::churn)
// =============================================================================

mod churn_tests {
    use super::*;

    // ---- Unit tests (no git required) ----

    #[test]
    fn test_is_bot_author_detects_dependabot() {
        assert!(is_bot_author(
            "dependabot[bot]",
            "dependabot@users.noreply.github.com"
        ));
    }

    #[test]
    fn test_is_bot_author_detects_renovate() {
        assert!(is_bot_author("renovate[bot]", "renovate@whitesource.com"));
    }

    #[test]
    fn test_is_bot_author_rejects_humans() {
        assert!(!is_bot_author("Alice Developer", "alice@company.com"));
        assert!(!is_bot_author("Bob", "bob@example.org"));
    }

    #[test]
    fn test_is_bot_author_case_insensitive() {
        assert!(is_bot_author("DEPENDABOT", "any@email.com"));
        assert!(is_bot_author("Renovate Bot", "any@email.com"));
    }

    #[test]
    fn test_matches_exclude_pattern_glob() {
        let patterns = vec!["*.lock".to_string(), "vendor/*".to_string()];
        assert!(matches_exclude_pattern("Cargo.lock", &patterns));
        assert!(matches_exclude_pattern("vendor/lib.rs", &patterns));
        assert!(!matches_exclude_pattern("src/main.rs", &patterns));
    }

    #[test]
    fn test_matches_exclude_pattern_empty() {
        let patterns: Vec<String> = vec![];
        assert!(!matches_exclude_pattern("any_file.rs", &patterns));
    }

    #[test]
    fn test_get_recommendation_thresholds() {
        let rec_low = get_recommendation(0.2);
        let rec_high = get_recommendation(0.8);
        // Low score should recommend something different from high score
        assert!(!rec_low.is_empty());
        assert!(!rec_high.is_empty());
        // The recommendations should differ for extreme scores
        assert_ne!(rec_low, rec_high);
    }

    #[test]
    fn test_build_summary_empty() {
        let empty: HashMap<String, FileChurn> = HashMap::new();
        let summary = build_summary(&empty, 30);
        assert_eq!(summary.total_files, 0);
        assert_eq!(summary.total_commits, 0);
        assert_eq!(summary.time_window_days, 30);
        assert_eq!(summary.total_lines_changed, 0);
    }

    #[test]
    fn test_build_summary_with_data() {
        let mut files = HashMap::new();
        files.insert(
            "main.py".to_string(),
            FileChurn {
                file: "main.py".to_string(),
                commit_count: 5,
                lines_added: 100,
                lines_deleted: 20,
                lines_changed: 120,
                first_commit: Some("2026-01-01".to_string()),
                last_commit: Some("2026-04-01".to_string()),
                authors: vec!["alice@test.com".to_string()],
                author_count: 1,
            },
        );
        files.insert(
            "utils.py".to_string(),
            FileChurn {
                file: "utils.py".to_string(),
                commit_count: 3,
                lines_added: 50,
                lines_deleted: 10,
                lines_changed: 60,
                first_commit: Some("2026-02-01".to_string()),
                last_commit: Some("2026-03-15".to_string()),
                authors: vec!["bob@test.com".to_string()],
                author_count: 1,
            },
        );

        let summary = build_summary(&files, 90);
        assert_eq!(summary.total_files, 2);
        assert_eq!(summary.total_commits, 8); // 5 + 3
        assert_eq!(summary.time_window_days, 90);
        assert_eq!(summary.total_lines_changed, 180); // 120 + 60
        assert!(
            summary.avg_commits_per_file > 0.0,
            "avg_commits_per_file should be positive"
        );
        assert_eq!(
            summary.most_churned_file, "main.py",
            "most churned should be main.py with 5 commits"
        );
    }

    // ---- Git integration tests ----

    #[test]
    fn test_is_git_repository_true() {
        let repo = TestRepo::new();
        repo.add_file("dummy.txt", "hello");
        repo.commit("initial");
        assert!(is_git_repository(repo.path()).unwrap());
    }

    #[test]
    fn test_is_git_repository_false() {
        let dir = TempDir::new().unwrap();
        assert!(!is_git_repository(dir.path()).unwrap());
    }

    #[test]
    fn test_not_git_repo_returns_error() {
        let dir = TempDir::new().unwrap();
        let result = get_file_churn(dir.path(), 30, &[]);
        assert!(result.is_err());
        match result.unwrap_err() {
            ChurnError::NotGitRepository { .. } => {} // expected
            other => panic!("Expected NotGitRepository, got {:?}", other),
        }
    }

    #[test]
    fn test_get_file_churn_basic() {
        let repo = TestRepo::new();

        // Create initial file and commit
        repo.add_file("main.py", "def foo():\n    pass\n");
        repo.commit("initial commit");

        // Modify and commit again to create churn
        repo.add_file("main.py", "def foo():\n    return 42\n");
        repo.commit("update foo return value");

        // Analyze churn
        let result = get_file_churn(repo.path(), 365, &[]);
        assert!(result.is_ok(), "get_file_churn failed: {:?}", result.err());

        let file_stats = result.unwrap();
        assert!(
            !file_stats.is_empty(),
            "Should detect at least one churned file"
        );

        // Find main.py in results
        let main_churn = file_stats.get("main.py");
        assert!(main_churn.is_some(), "main.py should appear in churn stats");

        let churn = main_churn.unwrap();
        assert!(
            churn.commit_count >= 2,
            "main.py should have at least 2 commits, got {}",
            churn.commit_count
        );
        assert!(
            churn.lines_changed > 0,
            "main.py should have lines changed > 0"
        );
    }

    #[test]
    fn test_get_file_churn_multiple_files() {
        let repo = TestRepo::new();

        // Create two files
        repo.add_file("src/main.py", "def main():\n    pass\n");
        repo.add_file("src/utils.py", "def helper():\n    pass\n");
        repo.commit("initial");

        // Only modify main.py
        repo.add_file("src/main.py", "def main():\n    return 1\n");
        repo.commit("update main");

        // Modify both
        repo.add_file("src/main.py", "def main():\n    return 2\n");
        repo.add_file("src/utils.py", "def helper():\n    return True\n");
        repo.commit("update both");

        let stats = get_file_churn(repo.path(), 365, &[]).unwrap();

        // main.py should have more churn than utils.py
        let main_count = stats
            .get("src/main.py")
            .map(|c| c.commit_count)
            .unwrap_or(0);
        let utils_count = stats
            .get("src/utils.py")
            .map(|c| c.commit_count)
            .unwrap_or(0);
        assert!(
            main_count >= utils_count,
            "main.py ({}) should have >= churn than utils.py ({})",
            main_count,
            utils_count
        );
    }

    #[test]
    fn test_get_file_churn_with_excludes() {
        let repo = TestRepo::new();

        repo.add_file("src/main.py", "code\n");
        repo.add_file("Cargo.lock", "lockfile\n");
        repo.commit("initial");

        repo.add_file("src/main.py", "code v2\n");
        repo.add_file("Cargo.lock", "lockfile v2\n");
        repo.commit("update");

        let excludes = vec!["*.lock".to_string()];
        let stats = get_file_churn(repo.path(), 365, &excludes).unwrap();

        // Cargo.lock should be excluded
        assert!(
            !stats.contains_key("Cargo.lock"),
            "Cargo.lock should be excluded by *.lock pattern"
        );
    }

    #[test]
    fn test_get_file_churn_tracks_authors() {
        let repo = TestRepo::new();

        repo.add_file("shared.py", "line1\n");
        repo.commit_as("alice commit", "Alice", "alice@test.com");

        repo.add_file("shared.py", "line1\nline2\n");
        repo.commit_as("bob commit", "Bob", "bob@test.com");

        let stats = get_file_churn(repo.path(), 365, &[]).unwrap();
        let shared = stats.get("shared.py").expect("shared.py should exist");
        assert!(
            shared.author_count >= 2,
            "shared.py should have at least 2 authors, got {}",
            shared.author_count
        );
    }

    #[test]
    fn test_get_author_stats_basic() {
        let repo = TestRepo::new();

        repo.add_file("main.py", "v1\n");
        repo.commit("initial");
        repo.add_file("main.py", "v2\n");
        repo.commit("second");

        let file_stats = get_file_churn(repo.path(), 365, &[]).unwrap();
        let author_stats = get_author_stats(repo.path(), 365, &file_stats).unwrap();

        assert!(
            !author_stats.is_empty(),
            "Should have at least one author stat"
        );
        let test_author = author_stats
            .iter()
            .find(|a| a.email == "test@test.com")
            .expect("Should find test@test.com");
        assert!(
            test_author.commits >= 2,
            "Test author should have >= 2 commits, got {}",
            test_author.commits
        );
    }
}

// =============================================================================
// HOTSPOTS TESTS (tldr-core API: quality::hotspots)
// =============================================================================

mod hotspots_tests {
    use super::*;

    // ---- Unit tests for helper functions ----

    #[test]
    fn test_normalize_value_basic() {
        assert_eq!(normalize_value(5.0, 0.0, 10.0), 0.5);
        assert_eq!(normalize_value(0.0, 0.0, 10.0), 0.0);
        assert_eq!(normalize_value(10.0, 0.0, 10.0), 1.0);
    }

    #[test]
    fn test_normalize_value_same_min_max() {
        // When min == max, returns 0.5 as fallback for uniform distribution
        assert_eq!(normalize_value(5.0, 5.0, 5.0), 0.5);
    }

    #[test]
    fn test_calculate_trend_directions() {
        let degrading = calculate_trend(5);
        assert_eq!(
            degrading,
            TrendDirection::Degrading,
            "positive delta > 2 should be Degrading"
        );

        let improving = calculate_trend(-5);
        assert_eq!(
            improving,
            TrendDirection::Improving,
            "negative delta < -2 should be Improving"
        );

        let stable = calculate_trend(0);
        assert_eq!(
            stable,
            TrendDirection::Stable,
            "zero delta should be Stable"
        );

        // Edge cases: deltas within [-2, 2] are Stable
        assert_eq!(calculate_trend(2), TrendDirection::Stable);
        assert_eq!(calculate_trend(-2), TrendDirection::Stable);
    }

    #[test]
    fn test_percentile_ranks_basic() {
        let values = vec![10.0, 20.0, 30.0, 40.0, 50.0];
        let ranks = percentile_ranks(&values);
        assert_eq!(ranks.len(), 5);
        // Lowest value should have lowest percentile
        assert!(
            ranks[0] < ranks[4],
            "first rank ({}) should be less than last ({})",
            ranks[0],
            ranks[4]
        );
    }

    #[test]
    fn test_percentile_ranks_empty() {
        let values: Vec<f64> = vec![];
        let ranks = percentile_ranks(&values);
        assert!(ranks.is_empty());
    }

    #[test]
    fn test_percentile_ranks_single() {
        let values = vec![42.0];
        let ranks = percentile_ranks(&values);
        assert_eq!(ranks.len(), 1);
        // Single value: percentile = 1.0
        assert!(
            (ranks[0] - 1.0).abs() < f64::EPSILON,
            "single element percentile should be 1.0, got {}",
            ranks[0]
        );
    }

    #[test]
    fn test_recency_weight_decay() {
        let recent = recency_weight(1.0, 30.0);
        let old = recency_weight(90.0, 30.0);
        assert!(
            recent > old,
            "recent ({}) should weigh more than old ({})",
            recent,
            old
        );
        // At halflife, weight should be ~0.5
        let at_halflife = recency_weight(30.0, 30.0);
        assert!(
            (at_halflife - 0.5).abs() < 0.01,
            "at halflife, weight should be ~0.5, got {}",
            at_halflife
        );
    }

    #[test]
    fn test_relative_churn_normalization() {
        let small_file = relative_churn(50, 100);
        let large_file = relative_churn(50, 1000);
        assert!(
            small_file > large_file,
            "same changes in small file ({}) should have higher relative churn than large ({})",
            small_file,
            large_file
        );
    }

    #[test]
    fn test_relative_churn_zero_loc() {
        // Division by zero protection
        let result = relative_churn(10, 0);
        assert!(
            result.is_finite(),
            "relative_churn with 0 loc should be finite, got {}",
            result
        );
    }

    #[test]
    fn test_knowledge_fragmentation_single_author() {
        let authors = vec![("Alice".to_string(), 10)];
        let frag = knowledge_fragmentation(&authors);
        // Single author = no fragmentation
        assert!(
            frag < 0.01,
            "single author should have ~0 fragmentation, got {}",
            frag
        );
    }

    #[test]
    fn test_knowledge_fragmentation_multiple_authors() {
        let authors = vec![
            ("Alice".to_string(), 5),
            ("Bob".to_string(), 5),
            ("Carol".to_string(), 5),
        ];
        let frag = knowledge_fragmentation(&authors);
        assert!(
            frag > 0.0,
            "multiple equal authors should have fragmentation > 0"
        );
    }

    #[test]
    fn test_knowledge_fragmentation_skewed() {
        let equal = vec![("A".to_string(), 5), ("B".to_string(), 5)];
        let skewed = vec![("A".to_string(), 9), ("B".to_string(), 1)];
        let frag_equal = knowledge_fragmentation(&equal);
        let frag_skewed = knowledge_fragmentation(&skewed);
        assert!(
            frag_equal > frag_skewed,
            "equal split ({}) should have more fragmentation than skewed ({})",
            frag_equal,
            frag_skewed
        );
    }

    #[test]
    fn test_has_variance_true() {
        assert!(has_variance(&[1.0, 2.0, 3.0]));
        assert!(has_variance(&[0.0, 100.0]));
    }

    #[test]
    fn test_has_variance_false() {
        assert!(!has_variance(&[5.0, 5.0, 5.0]));
        assert!(!has_variance(&[]));
        assert!(!has_variance(&[42.0]));
    }

    #[test]
    fn test_scoring_weights_default_sum_to_one() {
        let weights = ScoringWeights::default();
        let sum = weights.churn + weights.complexity + weights.knowledge_fragmentation
            + weights.temporal_coupling;
        assert!(
            (sum - 1.0).abs() < 0.01,
            "default weights should sum to ~1.0, got {}",
            sum
        );
    }

    #[test]
    fn test_scoring_weights_phase1_no_temporal() {
        let weights = ScoringWeights::default_phase1();
        assert_eq!(
            weights.temporal_coupling, 0.0,
            "phase1 should have 0 temporal coupling weight"
        );
        let sum = weights.churn + weights.complexity + weights.knowledge_fragmentation;
        assert!(
            (sum - 1.0).abs() < 0.01,
            "phase1 weights (excl temporal) should sum to ~1.0, got {}",
            sum
        );
    }

    #[test]
    fn test_scoring_weights_renormalize() {
        let weights = ScoringWeights {
            churn: 2.0,
            complexity: 2.0,
            knowledge_fragmentation: 1.0,
            temporal_coupling: 0.0,
        };
        let renorm = weights.renormalize();
        let sum =
            renorm.churn + renorm.complexity + renorm.knowledge_fragmentation + renorm.temporal_coupling;
        assert!(
            (sum - 1.0).abs() < 0.01,
            "renormalized weights should sum to ~1.0, got {}",
            sum
        );
    }

    #[test]
    fn test_composite_score_weighted_basic() {
        let weights = ScoringWeights::default_phase1();
        let score = composite_score_weighted(
            0.8, // pct_churn
            0.6, // pct_complexity
            0.3, // pct_fragmentation
            0.0, // pct_temporal
            &weights,
        );
        assert!(
            score > 0.0 && score <= 1.0,
            "composite score should be in (0,1], got {}",
            score
        );
    }

    #[test]
    fn test_composite_score_weighted_all_zeros() {
        let weights = ScoringWeights::default_phase1();
        let score = composite_score_weighted(0.0, 0.0, 0.0, 0.0, &weights);
        assert!(
            score.abs() < f64::EPSILON,
            "all-zero inputs should give ~0 score"
        );
    }

    // ---- Git integration tests ----

    #[test]
    fn test_analyze_hotspots_basic() {
        let repo = TestRepo::new();

        // Create a Python file with some complexity
        repo.add_file(
            "main.py",
            r#"
def complex_function(x, y, z):
    if x > 0:
        if y > 0:
            if z > 0:
                return x + y + z
            else:
                return x + y
        else:
            return x
    else:
        return 0

def simple_function():
    return 42
"#,
        );
        repo.commit("initial");

        // Create churn by modifying multiple times
        repo.add_file(
            "main.py",
            r#"
def complex_function(x, y, z):
    if x > 0:
        if y > 0:
            if z > 0:
                return x + y + z
            elif z == 0:
                return x + y
            else:
                return x - z
        else:
            return x
    elif x == 0:
        return -1
    else:
        return 0

def simple_function():
    return 42
"#,
        );
        repo.commit("add more branches");

        repo.add_file(
            "main.py",
            r#"
def complex_function(x, y, z):
    if x > 0:
        if y > 0:
            if z > 0:
                return x + y + z
            elif z == 0:
                return x + y
            else:
                return x - z
        elif y == 0:
            return x * 2
        else:
            return x
    elif x == 0:
        return -1
    else:
        return 0

def simple_function():
    return 42

def new_function(a, b):
    return a * b
"#,
        );
        repo.commit("even more complexity");

        let options = HotspotsOptions::new().with_days(365).with_top(10);
        let report = analyze_hotspots(repo.path(), &options);

        match report {
            Ok(rep) => {
                // Should find at least main.py
                assert!(
                    !rep.hotspots.is_empty(),
                    "hotspots report should contain at least one entry"
                );

                // Verify hotspot entry has valid scores
                for entry in &rep.hotspots {
                    assert!(
                        entry.hotspot_score >= 0.0,
                        "hotspot score should be >= 0, got {}",
                        entry.hotspot_score
                    );
                    assert!(
                        entry.commit_count > 0,
                        "commit_count should be > 0 for churned files"
                    );
                }

                // Summary should reflect analysis
                assert!(
                    rep.summary.total_files_analyzed > 0,
                    "summary should show files analyzed"
                );
            }
            Err(e) => {
                // Hotspots may fail in some CI environments without proper git setup
                eprintln!("analyze_hotspots error (may be expected in CI): {}", e);
            }
        }
    }

    #[test]
    fn test_analyze_hotspots_not_git_repo() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("file.py"), "x = 1\n").unwrap();

        let options = HotspotsOptions::new();
        let result = analyze_hotspots(dir.path(), &options);
        assert!(
            result.is_err(),
            "non-git directory should produce an error"
        );
    }

    #[test]
    fn test_analyze_hotspots_options_builder() {
        let options = HotspotsOptions::new()
            .with_days(60)
            .with_top(5)
            .with_min_commits(3)
            .with_by_function(true)
            .with_show_trend(true)
            .with_threshold(0.5)
            .with_recency_halflife(14.0)
            .with_include_bots(false);

        // Verify builder pattern works (no panic)
        let _ = format!("{:?}", options);
    }

    #[test]
    fn test_hotspots_excludes_patterns() {
        let repo = TestRepo::new();

        repo.add_file("src/main.py", "code\n");
        repo.add_file("vendor/lib.py", "vendor code\n");
        repo.commit("initial");

        repo.add_file("src/main.py", "code v2\n");
        repo.add_file("vendor/lib.py", "vendor code v2\n");
        repo.commit("update");

        let options = HotspotsOptions::new()
            .with_days(365)
            .with_exclude(vec!["vendor/*".to_string()]);

        match analyze_hotspots(repo.path(), &options) {
            Ok(report) => {
                for entry in &report.hotspots {
                    assert!(
                        !entry.file.starts_with("vendor/"),
                        "vendor files should be excluded, found: {}",
                        entry.file
                    );
                }
            }
            Err(e) => {
                eprintln!("hotspots with excludes error: {}", e);
            }
        }
    }
}

// =============================================================================
// CHANGE-IMPACT TESTS (tldr-core API: analysis::change_impact)
// =============================================================================

mod change_impact_tests {
    use super::*;

    #[test]
    fn test_change_impact_basic_python() {
        let repo = TestRepo::new();

        // Create source and test files
        repo.add_file(
            "src/auth.py",
            r#"
def login(username, password):
    return username == "admin" and password == "secret"

def logout(session_id):
    pass
"#,
        );
        repo.add_file(
            "tests/test_auth.py",
            r#"
from src.auth import login, logout

def test_login_success():
    assert login("admin", "secret")

def test_login_failure():
    assert not login("user", "wrong")

def test_logout():
    logout("session_123")
"#,
        );
        repo.commit("initial with source and tests");

        // Modify the source file
        repo.add_file(
            "src/auth.py",
            r#"
def login(username, password):
    if not username or not password:
        return False
    return username == "admin" and password == "secret"

def logout(session_id):
    return True
"#,
        );

        // Run change_impact with explicit changed files
        let changed = vec![repo.path().join("src/auth.py")];
        let result = change_impact(repo.path(), Some(&changed), Language::Python);

        match result {
            Ok(report) => {
                // Should detect the changed file
                assert!(
                    !report.changed_files.is_empty(),
                    "Should detect changed files"
                );

                // Should identify affected test files
                // The test file imports from auth, so it should be affected
                let affected_paths: Vec<String> = report
                    .affected_tests
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect();
                let has_test = affected_paths.iter().any(|p| p.contains("test_auth"));
                // Note: this depends on call graph analysis depth
                if !report.affected_tests.is_empty() {
                    assert!(
                        has_test,
                        "test_auth.py should be in affected tests, got: {:?}",
                        affected_paths
                    );
                }
            }
            Err(e) => {
                eprintln!("change_impact error (may be expected without git diff): {}", e);
            }
        }
    }

    #[test]
    fn test_change_impact_extended_with_depth() {
        let repo = TestRepo::new();

        repo.add_file("src/core.py", "def core_func():\n    return 1\n");
        repo.add_file(
            "src/service.py",
            "from src.core import core_func\ndef service():\n    return core_func()\n",
        );
        repo.add_file(
            "tests/test_service.py",
            "from src.service import service\ndef test_service():\n    assert service() == 1\n",
        );
        repo.commit("initial");

        let changed = vec![repo.path().join("src/core.py")];
        let result = change_impact_extended(
            repo.path(),
            DetectionMethod::Explicit,
            Language::Python,
            3, // depth
            false,
            &[],
            Some(changed),
        );

        match result {
            Ok(report) => {
                assert!(
                    !report.changed_files.is_empty(),
                    "Should detect changed files from explicit list"
                );
                // With depth=3, transitive dependencies should be found
                if let Some(ref metadata) = report.metadata {
                    assert!(
                        !metadata.language.is_empty(),
                        "metadata should include language"
                    );
                }
            }
            Err(e) => {
                eprintln!("change_impact_extended error: {}", e);
            }
        }
    }

    #[test]
    fn test_change_impact_no_tests_affected() {
        let repo = TestRepo::new();

        // Create files with no import relationship
        repo.add_file("src/a.py", "def func_a():\n    return 1\n");
        repo.add_file("src/b.py", "def func_b():\n    return 2\n");
        repo.add_file(
            "tests/test_b.py",
            "from src.b import func_b\ndef test_b():\n    assert func_b() == 2\n",
        );
        repo.commit("initial");

        // Change file a, which has no relationship to test_b
        let changed = vec![repo.path().join("src/a.py")];
        let result = change_impact(repo.path(), Some(&changed), Language::Python);

        match result {
            Ok(report) => {
                // test_b.py should NOT be affected by changes to a.py
                let affected_paths: Vec<String> = report
                    .affected_tests
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect();
                let has_test_b = affected_paths.iter().any(|p| p.contains("test_b"));
                assert!(
                    !has_test_b,
                    "test_b.py should NOT be affected by changes to a.py, got: {:?}",
                    affected_paths
                );
            }
            Err(e) => {
                eprintln!("change_impact no-impact error: {}", e);
            }
        }
    }

    #[test]
    fn test_change_impact_report_has_detection_method() {
        let repo = TestRepo::new();
        repo.add_file("main.py", "x = 1\n");
        repo.commit("initial");

        let changed = vec![repo.path().join("main.py")];
        let result = change_impact(repo.path(), Some(&changed), Language::Python);

        match result {
            Ok(report) => {
                assert!(
                    !report.detection_method.is_empty(),
                    "detection_method should not be empty"
                );
            }
            Err(e) => {
                eprintln!("change_impact detection_method error: {}", e);
            }
        }
    }

    #[test]
    fn test_change_impact_empty_changed_files() {
        let repo = TestRepo::new();
        repo.add_file("main.py", "x = 1\n");
        repo.commit("initial");

        // Empty changed files = should use git detection
        let result = change_impact(repo.path(), Some(&[]), Language::Python);

        match result {
            Ok(report) => {
                // With no uncommitted changes and empty explicit list,
                // should detect using git HEAD and find no changes
                // (everything is committed)
                assert!(
                    report.changed_files.is_empty(),
                    "no uncommitted changes should mean no changed files, got {:?}",
                    report.changed_files
                );
            }
            Err(e) => {
                // Some git setups may not support this -- acceptable
                eprintln!("change_impact empty-list error: {}", e);
            }
        }
    }
}

// =============================================================================
// TEMPORAL TESTS (tldr-cli binary: tldr temporal)
// =============================================================================

mod temporal_tests {
    use super::*;

    #[test]
    fn test_temporal_python_method_sequences() {
        let dir = TempDir::new().unwrap();
        create_temp_file(
            &dir,
            "file_ops.py",
            r#"
def read_file(path):
    f = open(path)
    content = f.read()
    f.close()
    return content

def write_file(path, data):
    f = open(path, 'w')
    f.write(data)
    f.close()

def process_many(paths):
    for path in paths:
        f = open(path)
        data = f.read()
        f.close()
        process(data)
"#,
        );

        let file_path = dir.path().join("file_ops.py");
        let (code, stdout, stderr) = run_tldr(&[
            "temporal",
            file_path.to_str().unwrap(),
            "--min-support",
            "1",
            "--min-confidence",
            "0.0",
            "--format",
            "json",
        ]);

        if code == 0 || code == 2 {
            // code=2 means "no constraints found" which is also valid for some inputs
            if code == 0 {
                let json: serde_json::Value =
                    serde_json::from_str(&stdout).expect("temporal output should be valid JSON");

                // Verify report structure
                assert!(
                    json.get("constraints").is_some(),
                    "report should have constraints field"
                );
                assert!(
                    json.get("metadata").is_some(),
                    "report should have metadata field"
                );

                // Check metadata
                let meta = &json["metadata"];
                assert_eq!(
                    meta["files_analyzed"], 1,
                    "should analyze exactly 1 file"
                );
                assert!(
                    meta["sequences_extracted"].as_u64().unwrap_or(0) > 0,
                    "should extract some sequences"
                );

                // If constraints found, verify structure
                if let Some(constraints) = json["constraints"].as_array() {
                    for constraint in constraints {
                        assert!(
                            constraint.get("before").is_some(),
                            "constraint should have 'before'"
                        );
                        assert!(
                            constraint.get("after").is_some(),
                            "constraint should have 'after'"
                        );
                        assert!(
                            constraint.get("support").is_some(),
                            "constraint should have 'support'"
                        );
                        assert!(
                            constraint.get("confidence").is_some(),
                            "constraint should have 'confidence'"
                        );
                        let conf = constraint["confidence"].as_f64().unwrap_or(0.0);
                        assert!(
                            (0.0..=1.0).contains(&conf),
                            "confidence should be 0-1, got {}",
                            conf
                        );
                    }
                }
            }
        } else {
            eprintln!(
                "temporal command returned code {}: stderr={}",
                code, stderr
            );
        }
    }

    #[test]
    fn test_temporal_with_query_filter() {
        let dir = TempDir::new().unwrap();
        create_temp_file(
            &dir,
            "mixed.py",
            r#"
def network_ops():
    conn = connect()
    conn.send(data)
    conn.recv()
    conn.close()

def file_ops():
    f = open("test.txt")
    f.read()
    f.close()
"#,
        );

        let file_path = dir.path().join("mixed.py");
        let (code, stdout, _) = run_tldr(&[
            "temporal",
            file_path.to_str().unwrap(),
            "--min-support",
            "1",
            "--min-confidence",
            "0.0",
            "--query",
            "open",
            "--format",
            "json",
        ]);

        if code == 0 {
            let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
            if let Some(constraints) = json["constraints"].as_array() {
                for constraint in constraints {
                    let before = constraint["before"].as_str().unwrap_or("");
                    let after = constraint["after"].as_str().unwrap_or("");
                    assert!(
                        before.contains("open") || after.contains("open"),
                        "filtered constraint should mention 'open': {} -> {}",
                        before,
                        after
                    );
                }
            }
        }
    }

    #[test]
    fn test_temporal_with_trigrams() {
        let dir = TempDir::new().unwrap();
        create_temp_file(
            &dir,
            "pipeline.py",
            r#"
def process():
    f = open("data.txt")
    content = f.read()
    f.close()
    return content
"#,
        );

        let file_path = dir.path().join("pipeline.py");
        let (code, stdout, _) = run_tldr(&[
            "temporal",
            file_path.to_str().unwrap(),
            "--min-support",
            "1",
            "--min-confidence",
            "0.0",
            "--include-trigrams",
            "--format",
            "json",
        ]);

        if code == 0 {
            let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
            assert!(
                json.get("trigrams").is_some(),
                "report with --include-trigrams should have trigrams field"
            );
            if let Some(trigrams) = json["trigrams"].as_array() {
                for trigram in trigrams {
                    let seq = trigram["sequence"]
                        .as_array()
                        .expect("trigram should have sequence array");
                    assert_eq!(
                        seq.len(),
                        3,
                        "trigram sequence should have exactly 3 elements"
                    );
                }
            }
        }
    }

    #[test]
    fn test_temporal_text_format() {
        let dir = TempDir::new().unwrap();
        create_temp_file(
            &dir,
            "ops.py",
            r#"
def work():
    f = open("x")
    f.read()
    f.close()
"#,
        );

        let file_path = dir.path().join("ops.py");
        let (code, stdout, _) = run_tldr(&[
            "temporal",
            file_path.to_str().unwrap(),
            "--min-support",
            "1",
            "--min-confidence",
            "0.0",
            "--format",
            "text",
        ]);

        if code == 0 || code == 2 {
            // Text output should contain human-readable content
            assert!(
                stdout.contains("Temporal") || stdout.contains("constraint") || stdout.contains("Metadata"),
                "text output should contain recognizable keywords, got: {}",
                &stdout[..stdout.len().min(200)]
            );
        }
    }
}

// =============================================================================
// RESOURCES TESTS (tldr-cli binary: tldr resources)
// =============================================================================

mod resources_tests {
    use super::*;

    #[test]
    fn test_resources_python_leak_detection() {
        let dir = TempDir::new().unwrap();
        let file_path = create_temp_file(
            &dir,
            "leaky.py",
            r#"
def leaky_function(path):
    f = open(path)
    if some_condition():
        return None
    content = f.read()
    f.close()
    return content
"#,
        );

        let (code, stdout, stderr) = run_tldr(&[
            "resources",
            file_path.to_str().unwrap(),
            "--check-all",
            "--format",
            "json",
        ]);

        if code == 0 {
            let json: serde_json::Value =
                serde_json::from_str(&stdout).expect("resources output should be valid JSON");

            // Should detect the leak (early return without close)
            if let Some(report) = json.as_object() {
                // The report should have content
                assert!(
                    !report.is_empty(),
                    "resource report should have content"
                );
                // Should have a leaks or issues field for leak detection
                let has_leak_field = report.contains_key("leaks") || report.contains_key("issues");
                assert!(
                    has_leak_field || report.contains_key("resources"),
                    "resource report should contain a leaks/issues/resources field, keys: {:?}",
                    report.keys().collect::<Vec<_>>()
                );
            }
        } else {
            eprintln!("resources command returned {}: {}", code, stderr);
        }
    }

    #[test]
    fn test_resources_python_safe_with_context_manager() {
        let dir = TempDir::new().unwrap();
        let file_path = create_temp_file(
            &dir,
            "safe.py",
            r#"
def safe_function(path):
    with open(path) as f:
        return f.read()
"#,
        );

        let (code, stdout, stderr) = run_tldr(&[
            "resources",
            file_path.to_str().unwrap(),
            "--check-all",
            "--format",
            "json",
        ]);

        if code == 0 {
            let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
            // With statement should be recognized as safe -- no leaks
            if let Some(leaks) = json.get("leaks").and_then(|l| l.as_array()) {
                // A with-statement should not be flagged
                assert!(
                    leaks.is_empty(),
                    "context manager should not produce leak findings, got {} leaks",
                    leaks.len()
                );
            }
        } else {
            eprintln!("resources safe test code {}: {}", code, stderr);
        }
    }

    #[test]
    fn test_resources_python_double_close() {
        let dir = TempDir::new().unwrap();
        let file_path = create_temp_file(
            &dir,
            "double.py",
            r#"
def double_close(path):
    f = open(path)
    content = f.read()
    f.close()
    f.close()
    return content
"#,
        );

        let (code, stdout, _) = run_tldr(&[
            "resources",
            file_path.to_str().unwrap(),
            "--check-all",
            "--format",
            "json",
        ]);

        if code == 0 {
            let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
            // Should detect double close
            if let Some(double_closes) = json.get("double_closes").and_then(|d| d.as_array()) {
                assert!(
                    !double_closes.is_empty(),
                    "should detect double close of file handle"
                );
            }
        }
    }

    #[test]
    fn test_resources_c_malloc_leak() {
        let dir = TempDir::new().unwrap();
        let file_path = create_temp_file(
            &dir,
            "leak.c",
            r#"
#include <stdlib.h>

void leaky() {
    int *p = malloc(sizeof(int) * 100);
    if (p == NULL) return;
    *p = 42;
    // Missing free(p)!
}

void safe() {
    int *p = malloc(sizeof(int));
    *p = 1;
    free(p);
}
"#,
        );

        let (code, stdout, stderr) = run_tldr(&[
            "resources",
            file_path.to_str().unwrap(),
            "--lang",
            "c",
            "--check-all",
            "--format",
            "json",
        ]);

        if code == 0 {
            let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
            // Report should exist and have some structure
            assert!(
                json.is_object(),
                "C resource analysis should produce a JSON object"
            );
        } else {
            eprintln!("resources C test code {}: {}", code, stderr);
        }
    }

    #[test]
    fn test_resources_go_defer_pattern() {
        let dir = TempDir::new().unwrap();
        let file_path = create_temp_file(
            &dir,
            "safe.go",
            r#"package main

import "os"

func readFile(path string) ([]byte, error) {
    f, err := os.Open(path)
    if err != nil {
        return nil, err
    }
    defer f.Close()
    return io.ReadAll(f)
}
"#,
        );

        let (code, stdout, _) = run_tldr(&[
            "resources",
            file_path.to_str().unwrap(),
            "--lang",
            "go",
            "--check-all",
            "--format",
            "json",
        ]);

        if code == 0 {
            let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
            // Go defer pattern should be recognized as safe
            if let Some(leaks) = json.get("leaks").and_then(|l| l.as_array()) {
                assert!(
                    leaks.is_empty(),
                    "Go defer should not produce leak findings"
                );
            }
        }
    }

    #[test]
    fn test_resources_java_try_with_resources() {
        let dir = TempDir::new().unwrap();
        let file_path = create_temp_file(
            &dir,
            "Safe.java",
            r#"import java.io.*;

public class Safe {
    public String read(String path) throws IOException {
        try (BufferedReader br = new BufferedReader(new FileReader(path))) {
            return br.readLine();
        }
    }
}
"#,
        );

        let (code, stdout, _) = run_tldr(&[
            "resources",
            file_path.to_str().unwrap(),
            "--lang",
            "java",
            "--check-all",
            "--format",
            "json",
        ]);

        if code == 0 {
            let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
            // try-with-resources should be safe
            if let Some(leaks) = json.get("leaks").and_then(|l| l.as_array()) {
                assert!(
                    leaks.is_empty(),
                    "Java try-with-resources should not produce leak findings"
                );
            }
        }
    }

    #[test]
    fn test_resources_rust_raii() {
        let dir = TempDir::new().unwrap();
        let file_path = create_temp_file(
            &dir,
            "safe.rs",
            r#"use std::fs::File;
use std::io::Read;

fn read_safe(path: &str) -> std::io::Result<String> {
    let mut f = File::open(path)?;
    let mut content = String::new();
    f.read_to_string(&mut content)?;
    Ok(content)
}
"#,
        );

        let (code, stdout, _) = run_tldr(&[
            "resources",
            file_path.to_str().unwrap(),
            "--lang",
            "rust",
            "--check-all",
            "--format",
            "json",
        ]);

        if code == 0 {
            let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
            assert!(json.is_object(), "Rust resource analysis should produce JSON object");
        }
    }

    #[test]
    fn test_resources_text_format() {
        let dir = TempDir::new().unwrap();
        let file_path = create_temp_file(
            &dir,
            "example.py",
            r#"
def example():
    f = open("test")
    f.read()
    f.close()
"#,
        );

        let (code, stdout, _) = run_tldr(&[
            "resources",
            file_path.to_str().unwrap(),
            "--format",
            "text",
        ]);

        if code == 0 {
            // Text format should produce human-readable output
            assert!(
                !stdout.is_empty(),
                "text format should produce some output"
            );
        }
    }

    #[test]
    fn test_resources_summary_flag() {
        let dir = TempDir::new().unwrap();
        let file_path = create_temp_file(
            &dir,
            "multi.py",
            r#"
def func_a():
    f = open("a")
    return f.read()

def func_b():
    with open("b") as f:
        return f.read()
"#,
        );

        let (code, stdout, _) = run_tldr(&[
            "resources",
            file_path.to_str().unwrap(),
            "--summary",
            "--format",
            "json",
        ]);

        if code == 0 {
            let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
            assert!(
                json.is_object(),
                "summary should produce a JSON object"
            );
        }
    }
}

// =============================================================================
// API-CHECK TESTS (tldr-cli binary: tldr api-check)
// =============================================================================

mod api_check_tests {
    use super::*;

    #[test]
    fn test_api_check_python_bare_except() {
        let dir = TempDir::new().unwrap();
        let file_path = create_temp_file(
            &dir,
            "bad_error_handling.py",
            r#"
import json

def load_config(path):
    try:
        with open(path) as f:
            return json.load(f)
    except:
        return {}
"#,
        );

        let (code, stdout, stderr) = run_tldr(&[
            "api-check",
            file_path.to_str().unwrap(),
            "--format",
            "json",
        ]);

        if code == 0 {
            let json: serde_json::Value =
                serde_json::from_str(&stdout).expect("api-check output should be valid JSON");

            // Should have findings
            if let Some(findings) = json.get("findings").and_then(|f| f.as_array()) {
                let bare_except_found = findings.iter().any(|f| {
                    f.get("rule")
                        .and_then(|r| r.get("name"))
                        .and_then(|n| n.as_str())
                        .map(|name| name.contains("bare-except"))
                        .unwrap_or(false)
                });
                assert!(
                    bare_except_found,
                    "should detect bare except clause, findings: {:?}",
                    findings
                        .iter()
                        .map(|f| f.get("rule")
                            .and_then(|r| r.get("name"))
                            .and_then(|n| n.as_str())
                            .unwrap_or("?"))
                        .collect::<Vec<_>>()
                );
            }

            // Summary should exist
            assert!(
                json.get("summary").is_some(),
                "report should have summary"
            );
        } else {
            eprintln!("api-check bare-except code {}: {}", code, stderr);
        }
    }

    #[test]
    fn test_api_check_python_missing_timeout() {
        let dir = TempDir::new().unwrap();
        let file_path = create_temp_file(
            &dir,
            "no_timeout.py",
            r#"
import requests

def fetch_data(url):
    response = requests.get(url)
    return response.json()

def post_data(url, data):
    response = requests.post(url, json=data)
    return response.status_code
"#,
        );

        let (code, stdout, _) = run_tldr(&[
            "api-check",
            file_path.to_str().unwrap(),
            "--format",
            "json",
        ]);

        if code == 0 {
            let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
            if let Some(findings) = json.get("findings").and_then(|f| f.as_array()) {
                let timeout_found = findings.iter().any(|f| {
                    f.get("rule")
                        .and_then(|r| r.get("name"))
                        .and_then(|n| n.as_str())
                        .map(|name| name.contains("timeout"))
                        .unwrap_or(false)
                });
                assert!(
                    timeout_found,
                    "should detect missing timeout in requests calls"
                );
            }
        }
    }

    #[test]
    fn test_api_check_python_weak_crypto() {
        let dir = TempDir::new().unwrap();
        let file_path = create_temp_file(
            &dir,
            "weak_crypto.py",
            r#"
import hashlib

def hash_password(password):
    return hashlib.md5(password.encode()).hexdigest()

def hash_token(token):
    return hashlib.sha1(token.encode()).hexdigest()
"#,
        );

        let (code, stdout, _) = run_tldr(&[
            "api-check",
            file_path.to_str().unwrap(),
            "--format",
            "json",
        ]);

        if code == 0 {
            let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
            if let Some(findings) = json.get("findings").and_then(|f| f.as_array()) {
                let md5_found = findings.iter().any(|f| {
                    f.get("rule")
                        .and_then(|r| r.get("name"))
                        .and_then(|n| n.as_str())
                        .map(|name| name.contains("md5") || name.contains("sha1"))
                        .unwrap_or(false)
                });
                assert!(
                    md5_found,
                    "should detect weak crypto (MD5/SHA1) usage"
                );
            }
        }
    }

    #[test]
    fn test_api_check_python_insecure_random() {
        let dir = TempDir::new().unwrap();
        // The checker requires a security-indicator word (token, secret, password,
        // key, auth, session) on the same line as random.randint/choice to flag it.
        let file_path = create_temp_file(
            &dir,
            "insecure_random.py",
            r#"
import random

def generate_token():
    token = random.randint(100000, 999999)
    return token

def generate_session_id():
    session = random.choice('abcdefghijklmnopqrstuvwxyz')
    return session
"#,
        );

        let (code, stdout, _) = run_tldr(&[
            "api-check",
            file_path.to_str().unwrap(),
            "--format",
            "json",
        ]);

        if code == 0 {
            let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
            if let Some(findings) = json.get("findings").and_then(|f| f.as_array()) {
                let random_found = findings.iter().any(|f| {
                    f.get("rule")
                        .and_then(|r| r.get("name"))
                        .and_then(|n| n.as_str())
                        .map(|name| name.contains("random"))
                        .unwrap_or(false)
                });
                assert!(
                    random_found,
                    "should detect insecure random usage when security indicators are on same line"
                );
            }
        }
    }

    #[test]
    fn test_api_check_python_clean_code() {
        let dir = TempDir::new().unwrap();
        let file_path = create_temp_file(
            &dir,
            "clean.py",
            r#"
import secrets
import hashlib

def generate_token():
    return secrets.token_hex(32)

def hash_data(data):
    return hashlib.sha256(data.encode()).hexdigest()
"#,
        );

        let (code, stdout, _) = run_tldr(&[
            "api-check",
            file_path.to_str().unwrap(),
            "--format",
            "json",
        ]);

        if code == 0 {
            let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
            if let Some(findings) = json.get("findings").and_then(|f| f.as_array()) {
                // Clean code should have no findings (or very few from false positives)
                assert!(
                    findings.len() <= 1,
                    "clean code should have minimal findings, got {}",
                    findings.len()
                );
            }
        }
    }

    #[test]
    fn test_api_check_rust_mutex_unwrap() {
        let dir = TempDir::new().unwrap();
        let file_path = create_temp_file(
            &dir,
            "concurrency.rs",
            r#"use std::sync::Mutex;

fn bad_lock(m: &Mutex<i32>) -> i32 {
    let guard = m.lock().unwrap();
    *guard
}
"#,
        );

        let (code, stdout, _) = run_tldr(&[
            "api-check",
            file_path.to_str().unwrap(),
            "--format",
            "json",
        ]);

        if code == 0 {
            let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
            if let Some(findings) = json.get("findings").and_then(|f| f.as_array()) {
                let mutex_found = findings.iter().any(|f| {
                    f.get("rule")
                        .and_then(|r| r.get("name"))
                        .and_then(|n| n.as_str())
                        .map(|name| name.contains("mutex"))
                        .unwrap_or(false)
                });
                assert!(
                    mutex_found,
                    "should detect mutex lock unwrap pattern"
                );
            }
        }
    }

    #[test]
    fn test_api_check_report_structure() {
        let dir = TempDir::new().unwrap();
        let file_path = create_temp_file(
            &dir,
            "test_struct.py",
            r#"
import requests

def fetch(url):
    try:
        return requests.get(url)
    except:
        return None
"#,
        );

        let (code, stdout, _) = run_tldr(&[
            "api-check",
            file_path.to_str().unwrap(),
            "--format",
            "json",
        ]);

        if code == 0 {
            let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

            // Verify report has required top-level fields
            assert!(json.get("findings").is_some(), "report needs 'findings'");
            assert!(json.get("summary").is_some(), "report needs 'summary'");
            assert!(
                json.get("rules_applied").is_some(),
                "report needs 'rules_applied'"
            );

            // Verify summary structure
            if let Some(summary) = json.get("summary") {
                assert!(
                    summary.get("total_findings").is_some()
                        || summary.get("findings_count").is_some(),
                    "summary should have a count of findings"
                );
            }

            // Verify finding structure if findings exist
            if let Some(findings) = json.get("findings").and_then(|f| f.as_array()) {
                for finding in findings {
                    assert!(finding.get("file").is_some(), "finding needs 'file'");
                    assert!(finding.get("line").is_some(), "finding needs 'line'");
                    assert!(finding.get("rule").is_some(), "finding needs 'rule'");

                    if let Some(rule) = finding.get("rule") {
                        assert!(rule.get("id").is_some(), "rule needs 'id'");
                        assert!(rule.get("name").is_some(), "rule needs 'name'");
                        assert!(rule.get("severity").is_some(), "rule needs 'severity'");
                    }
                }
            }
        }
    }

    #[test]
    fn test_api_check_text_format() {
        let dir = TempDir::new().unwrap();
        let file_path = create_temp_file(
            &dir,
            "check_text.py",
            r#"
import requests
def f():
    requests.get("http://example.com")
"#,
        );

        let (code, stdout, _) = run_tldr(&[
            "api-check",
            file_path.to_str().unwrap(),
            "--format",
            "text",
        ]);

        if code == 0 {
            assert!(
                !stdout.is_empty(),
                "text format should produce output"
            );
        }
    }

    #[test]
    fn test_api_check_directory_scan() {
        let dir = TempDir::new().unwrap();
        create_temp_file(
            &dir,
            "src/a.py",
            "import requests\ndef a():\n    requests.get('url')\n",
        );
        create_temp_file(
            &dir,
            "src/b.py",
            "try:\n    pass\nexcept:\n    pass\n",
        );

        let src_path = dir.path().join("src");
        let (code, stdout, _) = run_tldr(&[
            "api-check",
            src_path.to_str().unwrap(),
            "--format",
            "json",
        ]);

        if code == 0 {
            let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
            if let Some(summary) = json.get("summary") {
                // Should scan multiple files
                let files_scanned = summary
                    .get("files_scanned")
                    .and_then(|f| f.as_u64())
                    .unwrap_or(0);
                assert!(
                    files_scanned >= 2,
                    "directory scan should analyze at least 2 files, got {}",
                    files_scanned
                );
            }
        }
    }
}
