//! Test module for churn analysis functionality
//!
//! These tests define expected behavior BEFORE implementation.
//! Tests are designed to FAIL until the churn module is implemented.
//!
//! # Test Categories
//! - Unit tests: Data type serialization and helper functions
//! - Integration tests: Git repository operations (marked #[ignore])
//! - CLI integration tests: Command-line interface behavior (marked #[ignore])

use std::collections::HashMap;
use std::path::PathBuf;

// Import the types that will be implemented
use super::churn::{
    build_summary, check_shallow_clone, format_text_output, get_author_stats, get_file_churn,
    get_file_churn_detailed, get_recommendation, is_bot_author, is_git_repository,
    matches_exclude_pattern, AuthorStats, ChurnError, ChurnReport, ChurnSummary, FileChurn,
    Hotspot,
};

// =============================================================================
// Test Fixture Setup Module
// =============================================================================

/// Test fixture utilities for creating temporary git repositories
pub mod fixtures {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use tempfile::TempDir;

    /// A temporary git repository for testing
    pub struct TestRepo {
        pub dir: TempDir,
    }

    impl TestRepo {
        /// Create a new empty git repository
        pub fn new() -> std::io::Result<Self> {
            let dir = TempDir::new()?;

            // Initialize git repo
            let status = Command::new("git")
                .args(["init"])
                .current_dir(dir.path())
                .output()?
                .status;

            if !status.success() {
                return Err(std::io::Error::other("Failed to initialize git repo"));
            }

            // Configure git user for commits
            Command::new("git")
                .args(["config", "user.email", "test@example.com"])
                .current_dir(dir.path())
                .output()?;

            Command::new("git")
                .args(["config", "user.name", "Test User"])
                .current_dir(dir.path())
                .output()?;

            Ok(Self { dir })
        }

        /// Create a shallow clone of the test repo
        pub fn new_shallow(depth: u32) -> std::io::Result<Self> {
            // First create a normal repo with some commits
            let source = Self::new()?;
            source.add_file("file.txt", "content")?;
            source.commit("Initial commit")?;

            for i in 1..=5 {
                source.add_file("file.txt", &format!("content {}", i))?;
                source.commit(&format!("Commit {}", i))?;
            }

            // Create a shallow clone
            let dir = TempDir::new()?;
            let status = Command::new("git")
                .args([
                    "clone",
                    "--depth",
                    &depth.to_string(),
                    source.path().to_str().unwrap(),
                    dir.path().to_str().unwrap(),
                ])
                .output()?
                .status;

            if !status.success() {
                return Err(std::io::Error::other("Failed to create shallow clone"));
            }

            Ok(Self { dir })
        }

        /// Create a non-git directory
        pub fn new_non_git() -> std::io::Result<Self> {
            let dir = TempDir::new()?;
            // Don't initialize git
            Ok(Self { dir })
        }

        /// Get the path to the repository
        pub fn path(&self) -> &Path {
            self.dir.path()
        }

        /// Add a file to the repository
        pub fn add_file(&self, name: &str, content: &str) -> std::io::Result<PathBuf> {
            let path = self.dir.path().join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, content)?;

            Command::new("git")
                .args(["add", name])
                .current_dir(self.dir.path())
                .output()?;

            Ok(path)
        }

        /// Create a commit with the given message
        pub fn commit(&self, message: &str) -> std::io::Result<()> {
            let status = Command::new("git")
                .args(["commit", "-m", message, "--allow-empty"])
                .current_dir(self.dir.path())
                .output()?
                .status;

            if !status.success() {
                return Err(std::io::Error::other("Failed to commit"));
            }
            Ok(())
        }

        /// Create a commit with a specific author
        pub fn commit_as(&self, message: &str, name: &str, email: &str) -> std::io::Result<()> {
            let author = format!("{} <{}>", name, email);
            let status = Command::new("git")
                .args([
                    "commit",
                    "-m",
                    message,
                    "--author",
                    &author,
                    "--allow-empty",
                ])
                .current_dir(self.dir.path())
                .output()?
                .status;

            if !status.success() {
                return Err(std::io::Error::other("Failed to commit as author"));
            }
            Ok(())
        }

        /// Add a binary file to the repository
        pub fn add_binary_file(&self, name: &str, content: &[u8]) -> std::io::Result<PathBuf> {
            let path = self.dir.path().join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, content)?;

            Command::new("git")
                .args(["add", name])
                .current_dir(self.dir.path())
                .output()?;

            Ok(path)
        }
    }
}

// =============================================================================
// Unit Tests - Data Type Serialization
// =============================================================================

#[cfg(test)]
mod unit_tests {
    use super::*;
    use serde_json;

    /// Test 1: FileChurn serialization matches expected JSON format
    #[test]
    fn test_file_churn_struct() {
        let churn = FileChurn {
            file: "src/core/engine.py".to_string(),
            commit_count: 47,
            lines_added: 1250,
            lines_deleted: 890,
            lines_changed: 2140,
            first_commit: Some("2025-03-15".to_string()),
            last_commit: Some("2026-01-28".to_string()),
            authors: vec![
                "alice@example.com".to_string(),
                "bob@example.com".to_string(),
            ],
            author_count: 2,
        };

        // Verify invariants
        assert_eq!(churn.lines_changed, churn.lines_added + churn.lines_deleted);
        assert_eq!(churn.author_count as usize, churn.authors.len());

        // Verify serialization
        let json = serde_json::to_value(&churn).expect("Should serialize");

        assert_eq!(json["file"], "src/core/engine.py");
        assert_eq!(json["commit_count"], 47);
        assert_eq!(json["lines_added"], 1250);
        assert_eq!(json["lines_deleted"], 890);
        assert_eq!(json["lines_changed"], 2140);
        assert_eq!(json["first_commit"], "2025-03-15");
        assert_eq!(json["last_commit"], "2026-01-28");
        assert_eq!(
            json["authors"],
            serde_json::json!(["alice@example.com", "bob@example.com"])
        );
        assert_eq!(json["author_count"], 2);
    }

    /// Test 2: AuthorStats serialization
    #[test]
    fn test_author_stats_struct() {
        let stats = AuthorStats {
            name: "Alice Smith".to_string(),
            email: "alice@example.com".to_string(),
            commits: 47,
            lines_added: 2500,
            lines_deleted: 1200,
            files_touched: 23,
        };

        let json = serde_json::to_value(&stats).expect("Should serialize");

        assert_eq!(json["name"], "Alice Smith");
        assert_eq!(json["email"], "alice@example.com");
        assert_eq!(json["commits"], 47);
        assert_eq!(json["lines_added"], 2500);
        assert_eq!(json["lines_deleted"], 1200);
        assert_eq!(json["files_touched"], 23);

        // Deserialize and verify roundtrip
        let deserialized: AuthorStats = serde_json::from_value(json).expect("Should deserialize");
        assert_eq!(deserialized, stats);
    }

    /// Test 3: Hotspot serialization
    #[test]
    fn test_hotspot_struct() {
        let hotspot = Hotspot {
            file: "src/core/engine.py".to_string(),
            churn_rank: 1,
            complexity_rank: 2,
            combined_score: 0.823,
            commit_count: 47,
            cyclomatic_complexity: 25,
            recommendation: "Critical: High churn + high complexity. Prioritize refactoring."
                .to_string(),
        };

        // Verify invariants
        assert!(hotspot.combined_score >= 0.0 && hotspot.combined_score <= 1.0);
        assert!(hotspot.churn_rank >= 1);
        assert!(hotspot.complexity_rank >= 1);

        let json = serde_json::to_value(&hotspot).expect("Should serialize");

        assert_eq!(json["file"], "src/core/engine.py");
        assert_eq!(json["churn_rank"], 1);
        assert_eq!(json["complexity_rank"], 2);
        assert!((json["combined_score"].as_f64().unwrap() - 0.823).abs() < 0.001);
        assert_eq!(json["commit_count"], 47);
        assert_eq!(json["cyclomatic_complexity"], 25);
        assert!(json["recommendation"]
            .as_str()
            .unwrap()
            .contains("Critical"));
    }

    /// Test 4: ChurnSummary serialization
    #[test]
    fn test_churn_summary_struct() {
        let summary = ChurnSummary {
            total_files: 156,
            total_commits: 892,
            time_window_days: 365,
            total_lines_changed: 45230,
            avg_commits_per_file: 5.72,
            most_churned_file: "src/core/engine.py".to_string(),
        };

        let json = serde_json::to_value(&summary).expect("Should serialize");

        assert_eq!(json["total_files"], 156);
        assert_eq!(json["total_commits"], 892);
        assert_eq!(json["time_window_days"], 365);
        assert_eq!(json["total_lines_changed"], 45230);
        assert!((json["avg_commits_per_file"].as_f64().unwrap() - 5.72).abs() < 0.01);
        assert_eq!(json["most_churned_file"], "src/core/engine.py");
    }

    /// Test 5: ChurnReport serialization
    #[test]
    fn test_churn_report_struct() {
        let report = ChurnReport {
            files: vec![FileChurn {
                file: "src/main.rs".to_string(),
                commit_count: 10,
                lines_added: 100,
                lines_deleted: 50,
                lines_changed: 150,
                first_commit: Some("2026-01-01".to_string()),
                last_commit: Some("2026-01-31".to_string()),
                authors: vec!["dev@example.com".to_string()],
                author_count: 1,
            }],
            hotspots: vec![],
            authors: vec![],
            summary: ChurnSummary {
                total_files: 1,
                total_commits: 10,
                time_window_days: 30,
                total_lines_changed: 150,
                avg_commits_per_file: 10.0,
                most_churned_file: "src/main.rs".to_string(),
            },
            is_shallow: false,
            shallow_depth: None,
            warnings: vec![],
        };

        let json = serde_json::to_value(&report).expect("Should serialize");

        // Verify top-level structure
        assert!(json["files"].is_array());
        assert!(json["hotspots"].is_array());
        assert!(json["authors"].is_array());
        assert!(json["summary"].is_object());
        assert_eq!(json["is_shallow"], false);

        // shallow_depth and warnings should be omitted when empty/None
        assert!(json.get("shallow_depth").is_none() || json["shallow_depth"].is_null());
        assert!(
            json.get("warnings").is_none()
                || json["warnings"]
                    .as_array()
                    .map(|a| a.is_empty())
                    .unwrap_or(true)
        );
    }

    /// Test 6: Recommendation thresholds
    #[test]
    fn test_recommendation_thresholds() {
        // > 0.7 = Critical
        assert_eq!(
            get_recommendation(0.8),
            "Critical: High churn + high complexity. Prioritize refactoring."
        );
        assert_eq!(
            get_recommendation(0.71),
            "Critical: High churn + high complexity. Prioritize refactoring."
        );
        assert_eq!(
            get_recommendation(1.0),
            "Critical: High churn + high complexity. Prioritize refactoring."
        );

        // > 0.4 but <= 0.7 = Warning
        assert_eq!(
            get_recommendation(0.5),
            "Warning: Moderate risk. Consider simplification."
        );
        assert_eq!(
            get_recommendation(0.41),
            "Warning: Moderate risk. Consider simplification."
        );
        assert_eq!(
            get_recommendation(0.7),
            "Warning: Moderate risk. Consider simplification."
        );

        // <= 0.4 = Low risk
        assert_eq!(get_recommendation(0.3), "Low risk.");
        assert_eq!(get_recommendation(0.4), "Low risk.");
        assert_eq!(get_recommendation(0.0), "Low risk.");
    }

    /// Test 7: Exclude pattern matching
    #[test]
    fn test_exclude_pattern_matching() {
        // This tests the pattern matching logic used in get_file_churn
        // The implementation should use glob/fnmatch semantics

        let patterns = vec![
            "node_modules/*".to_string(),
            "*.lock".to_string(),
            "*-lock.json".to_string(),
        ];

        // Test node_modules patterns
        assert!(matches_exclude_pattern(
            "node_modules/package/index.js",
            &patterns
        ));
        assert!(matches_exclude_pattern("node_modules/foo", &patterns));

        // Test lock file patterns
        // Note: *.lock matches files ending in .lock
        // *-lock.json matches files like package-lock.json
        assert!(matches_exclude_pattern("package-lock.json", &patterns));
        assert!(matches_exclude_pattern("Cargo.lock", &patterns));
        assert!(matches_exclude_pattern("yarn.lock", &patterns));

        // Test non-matching paths
        assert!(!matches_exclude_pattern("src/main.rs", &patterns));
        assert!(!matches_exclude_pattern("lib/index.js", &patterns));
    }

    /// Test: build_summary with empty input
    #[test]
    fn test_build_summary_empty() {
        let file_stats: HashMap<String, FileChurn> = HashMap::new();
        let summary = build_summary(&file_stats, 365);

        assert_eq!(summary.total_files, 0);
        assert_eq!(summary.total_commits, 0);
        assert_eq!(summary.time_window_days, 365);
        assert_eq!(summary.total_lines_changed, 0);
        assert_eq!(summary.avg_commits_per_file, 0.0);
        assert!(summary.most_churned_file.is_empty());
    }

    /// Test: build_summary with multiple files
    #[test]
    fn test_build_summary_with_files() {
        let mut file_stats: HashMap<String, FileChurn> = HashMap::new();

        file_stats.insert(
            "file1.rs".to_string(),
            FileChurn {
                file: "file1.rs".to_string(),
                commit_count: 10,
                lines_added: 100,
                lines_deleted: 50,
                lines_changed: 150,
                first_commit: Some("2026-01-01".to_string()),
                last_commit: Some("2026-01-15".to_string()),
                authors: vec!["a@b.com".to_string()],
                author_count: 1,
            },
        );

        file_stats.insert(
            "file2.rs".to_string(),
            FileChurn {
                file: "file2.rs".to_string(),
                commit_count: 20,
                lines_added: 200,
                lines_deleted: 100,
                lines_changed: 300,
                first_commit: Some("2026-01-05".to_string()),
                last_commit: Some("2026-01-20".to_string()),
                authors: vec!["a@b.com".to_string(), "c@d.com".to_string()],
                author_count: 2,
            },
        );

        let summary = build_summary(&file_stats, 30);

        assert_eq!(summary.total_files, 2);
        assert_eq!(summary.total_commits, 30); // 10 + 20
        assert_eq!(summary.time_window_days, 30);
        assert_eq!(summary.total_lines_changed, 450); // 150 + 300
        assert!((summary.avg_commits_per_file - 15.0).abs() < 0.01); // 30 / 2
        assert_eq!(summary.most_churned_file, "file2.rs"); // Has 20 commits
    }
}

// =============================================================================
// Integration Tests - Git Repository Operations
// =============================================================================

#[cfg(test)]
mod integration_tests {
    use super::fixtures::TestRepo;
    use super::*;

    /// Test 8: is_git_repository returns true for valid git repo
    #[test]
    #[ignore = "Requires git setup - run with --ignored"]
    fn test_is_git_repository_valid() {
        let repo = TestRepo::new().expect("Failed to create test repo");

        let result = is_git_repository(repo.path());

        // The spec says: Ok(true) if path is inside a git repo
        assert!(result.is_ok(), "Should not error for valid git repo");
        assert!(result.unwrap(), "Should return true for git repository");
    }

    /// Test 9: is_git_repository returns false for non-git directory
    #[test]
    #[ignore = "Requires git setup - run with --ignored"]
    fn test_is_git_repository_invalid() {
        let non_repo = TestRepo::new_non_git().expect("Failed to create temp dir");

        let result = is_git_repository(non_repo.path());

        // The spec says: Ok(false) if path is not in a git repo
        assert!(result.is_ok(), "Should not error for non-git directory");
        assert!(
            !result.unwrap(),
            "Should return false for non-git directory"
        );
    }

    /// Test 10: Shallow clone detection
    #[test]
    #[ignore = "Requires git setup - run with --ignored"]
    fn test_shallow_clone_detection() {
        // Test full clone
        let full_repo = TestRepo::new().expect("Failed to create test repo");
        full_repo.add_file("test.txt", "content").unwrap();
        full_repo.commit("Initial").unwrap();

        let (is_shallow, depth) = check_shallow_clone(full_repo.path()).expect("Should not error");
        assert!(!is_shallow, "Full clone should not be shallow");
        assert!(depth.is_none(), "Full clone should have no depth");

        // Test shallow clone
        let shallow_repo = TestRepo::new_shallow(2).expect("Failed to create shallow clone");

        let (is_shallow, depth) =
            check_shallow_clone(shallow_repo.path()).expect("Should not error");
        assert!(is_shallow, "Shallow clone should be detected");
        assert!(depth.is_some(), "Should report depth for shallow clone");
    }

    /// Test 11: get_file_churn with empty repo (no commits in window)
    #[test]
    #[ignore = "Requires git setup - run with --ignored"]
    fn test_get_file_churn_empty_repo() {
        let repo = TestRepo::new().expect("Failed to create test repo");
        // Don't add any commits

        let result = get_file_churn(repo.path(), 30, &[]);

        assert!(result.is_ok(), "Should not error for empty repo");
        let churn = result.unwrap();
        assert!(
            churn.is_empty(),
            "Should return empty HashMap for no commits"
        );
    }

    /// Test 12: get_file_churn with commits
    #[test]
    #[ignore = "Requires git setup - run with --ignored"]
    fn test_get_file_churn_with_commits() {
        let repo = TestRepo::new().expect("Failed to create test repo");

        // Create files and commits
        repo.add_file("src/main.rs", "fn main() {}\n").unwrap();
        repo.commit("Add main").unwrap();

        repo.add_file("src/main.rs", "fn main() {\n    println!(\"hello\");\n}\n")
            .unwrap();
        repo.commit("Update main").unwrap();

        repo.add_file("src/lib.rs", "pub fn helper() {}\n").unwrap();
        repo.commit("Add lib").unwrap();

        let result = get_file_churn(repo.path(), 365, &[]);

        assert!(
            result.is_ok(),
            "Should succeed with commits: {:?}",
            result.err()
        );
        let churn = result.unwrap();

        // main.rs should have 2 commits
        let main_churn = churn
            .get("src/main.rs")
            .expect("main.rs should be in results");
        assert_eq!(main_churn.commit_count, 2);
        assert!(main_churn.lines_added > 0);
        assert_eq!(main_churn.author_count, 1);

        // lib.rs should have 1 commit
        let lib_churn = churn
            .get("src/lib.rs")
            .expect("lib.rs should be in results");
        assert_eq!(lib_churn.commit_count, 1);
    }

    /// Test 13: Numstat parsing - lines added/deleted
    #[test]
    #[ignore = "Requires git setup - run with --ignored"]
    fn test_numstat_parsing() {
        let repo = TestRepo::new().expect("Failed to create test repo");

        // Add file with known content
        repo.add_file("file.txt", "line1\nline2\nline3\n").unwrap();
        repo.commit("Add 3 lines").unwrap();

        // Modify file
        repo.add_file("file.txt", "line1\nmodified\nline3\nnew line\n")
            .unwrap();
        repo.commit("Modify file").unwrap();

        let result = get_file_churn(repo.path(), 365, &[]);
        let churn = result.expect("Should succeed");

        let file_churn = churn
            .get("file.txt")
            .expect("file.txt should be in results");

        // First commit: +3 lines
        // Second commit: +2 lines (modified + new), -1 line (removed line2)
        // Total: lines_added >= 4, lines_deleted >= 1
        assert!(
            file_churn.lines_added >= 4,
            "Should have at least 4 lines added"
        );
        assert!(
            file_churn.lines_deleted >= 1,
            "Should have at least 1 line deleted"
        );
        assert_eq!(
            file_churn.lines_changed,
            file_churn.lines_added + file_churn.lines_deleted,
            "lines_changed invariant"
        );
    }

    /// Test 14: Binary file handling in numstat
    #[test]
    #[ignore = "Requires git setup - run with --ignored"]
    fn test_binary_file_numstat() {
        let repo = TestRepo::new().expect("Failed to create test repo");

        // Add a binary file with NUL bytes (git detects binary by looking for NUL in first 8KB)
        // PNG header alone is not enough - we need actual NUL bytes
        let binary_content = [0x89, 0x50, 0x4E, 0x47, 0x00, 0x00, 0x00, 0x0D]; // NUL bytes included
        repo.add_binary_file("image.png", &binary_content).unwrap();
        repo.commit("Add binary").unwrap();

        let result = get_file_churn(repo.path(), 365, &[]);
        let churn = result.expect("Should succeed");

        let binary_churn = churn
            .get("image.png")
            .expect("image.png should be in results");

        // Binary files show "-" in numstat, so lines should be 0
        assert_eq!(
            binary_churn.lines_added, 0,
            "Binary files should have 0 lines_added"
        );
        assert_eq!(
            binary_churn.lines_deleted, 0,
            "Binary files should have 0 lines_deleted"
        );
        // But commit_count should still be tracked
        assert_eq!(
            binary_churn.commit_count, 1,
            "Commit should still be counted"
        );
    }

    /// Test: get_author_stats
    #[test]
    #[ignore = "Requires git setup - run with --ignored"]
    fn test_get_author_stats() {
        let repo = TestRepo::new().expect("Failed to create test repo");

        // Create commits from different authors
        repo.add_file("file1.txt", "content1\n").unwrap();
        repo.commit_as("Commit by Alice", "Alice", "alice@example.com")
            .unwrap();

        repo.add_file("file2.txt", "content2\n").unwrap();
        repo.commit_as("Commit by Bob", "Bob", "bob@example.com")
            .unwrap();

        repo.add_file("file1.txt", "updated content1\n").unwrap();
        repo.commit_as("Another by Alice", "Alice", "alice@example.com")
            .unwrap();

        let file_stats = get_file_churn(repo.path(), 365, &[]).expect("Should get churn");
        let author_stats =
            get_author_stats(repo.path(), 365, &file_stats).expect("Should get authors");

        // Alice should have 2 commits
        let alice = author_stats.iter().find(|a| a.email == "alice@example.com");
        assert!(alice.is_some(), "Alice should be in stats");
        assert_eq!(alice.unwrap().commits, 2);

        // Bob should have 1 commit
        let bob = author_stats.iter().find(|a| a.email == "bob@example.com");
        assert!(bob.is_some(), "Bob should be in stats");
        assert_eq!(bob.unwrap().commits, 1);
    }
}

// =============================================================================
// CLI Integration Tests
// =============================================================================

#[cfg(test)]
mod cli_tests {
    use super::fixtures::TestRepo;

    use serde_json::Value;

    /// Test 15: JSON output format
    #[test]
    #[ignore = "Requires CLI binary - run with --ignored"]
    fn test_churn_cli_json_output() {
        use std::process::Command;

        let repo = TestRepo::new().expect("Failed to create test repo");
        repo.add_file("test.rs", "fn main() {}").unwrap();
        repo.commit("Initial").unwrap();

        let output = Command::new("cargo")
            .args([
                "run",
                "--",
                "churn",
                repo.path().to_str().unwrap(),
                "--format",
                "json",
            ])
            .output()
            .expect("Failed to run command");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("Output should be valid JSON");

        // Verify schema
        assert!(json["files"].is_array(), "Should have files array");
        assert!(json["summary"].is_object(), "Should have summary object");
        assert!(
            json["summary"]["total_files"].is_number(),
            "Should have total_files"
        );
        assert!(
            json["summary"]["total_commits"].is_number(),
            "Should have total_commits"
        );
        assert!(
            json["summary"]["time_window_days"].is_number(),
            "Should have time_window_days"
        );
    }

    /// Test 16: Text output format
    #[test]
    #[ignore = "Requires CLI binary - run with --ignored"]
    fn test_churn_cli_text_output() {
        use std::process::Command;

        let repo = TestRepo::new().expect("Failed to create test repo");
        repo.add_file("test.rs", "fn main() {}").unwrap();
        repo.commit("Initial").unwrap();

        let output = Command::new("cargo")
            .args([
                "run",
                "--",
                "churn",
                repo.path().to_str().unwrap(),
                "--format",
                "text",
            ])
            .output()
            .expect("Failed to run command");

        let stdout = String::from_utf8_lossy(&output.stdout);

        // Verify text format structure
        assert!(stdout.contains("Code Churn Analysis"), "Should have header");
        assert!(stdout.contains("Time window:"), "Should show time window");
        assert!(
            stdout.contains("Total files changed:"),
            "Should show total files"
        );
    }

    /// Test 17: --days filter
    #[test]
    #[ignore = "Requires CLI binary - run with --ignored"]
    fn test_churn_cli_days_filter() {
        use std::process::Command;

        let repo = TestRepo::new().expect("Failed to create test repo");
        repo.add_file("test.rs", "fn main() {}").unwrap();
        repo.commit("Initial").unwrap();

        // Run with 1 day window
        let output = Command::new("cargo")
            .args([
                "run",
                "--",
                "churn",
                repo.path().to_str().unwrap(),
                "--days",
                "1",
                "--format",
                "json",
            ])
            .output()
            .expect("Failed to run command");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("Output should be valid JSON");

        assert_eq!(json["summary"]["time_window_days"], 1);
    }

    /// Test 18: --top filter
    #[test]
    #[ignore = "Requires CLI binary - run with --ignored"]
    fn test_churn_cli_top_filter() {
        use std::process::Command;

        let repo = TestRepo::new().expect("Failed to create test repo");

        // Create multiple files
        for i in 0..10 {
            repo.add_file(&format!("file{}.rs", i), "content").unwrap();
            repo.commit(&format!("Add file {}", i)).unwrap();
        }

        let output = Command::new("cargo")
            .args([
                "run",
                "--",
                "churn",
                repo.path().to_str().unwrap(),
                "--top",
                "3",
                "--format",
                "json",
            ])
            .output()
            .expect("Failed to run command");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("Output should be valid JSON");

        let files = json["files"].as_array().expect("files should be array");
        assert!(
            files.len() <= 3,
            "Should return at most 3 files with --top 3"
        );
    }

    /// Test 19: --exclude filter
    #[test]
    #[ignore = "Requires CLI binary - run with --ignored"]
    fn test_churn_cli_exclude_filter() {
        use std::process::Command;

        let repo = TestRepo::new().expect("Failed to create test repo");

        // Create files including one that should be excluded
        repo.add_file("src/main.rs", "fn main() {}").unwrap();
        repo.commit("Add main").unwrap();

        repo.add_file("Cargo.lock", "lock content").unwrap();
        repo.commit("Add lock").unwrap();

        let output = Command::new("cargo")
            .args([
                "run",
                "--",
                "churn",
                repo.path().to_str().unwrap(),
                "--exclude",
                "*.lock",
                "--format",
                "json",
            ])
            .output()
            .expect("Failed to run command");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("Output should be valid JSON");

        let files = json["files"].as_array().expect("files should be array");
        let has_lock = files.iter().any(|f| {
            f["file"]
                .as_str()
                .map(|s| s.ends_with(".lock"))
                .unwrap_or(false)
        });

        assert!(!has_lock, "Lock files should be excluded");
    }
}

// =============================================================================
// Error Handling Tests
// =============================================================================

#[cfg(test)]
mod error_tests {
    use super::*;

    /// Test: ChurnError variants
    #[test]
    fn test_churn_error_path_not_found() {
        let error = ChurnError::PathNotFound(PathBuf::from("/nonexistent"));
        let msg = error.to_string();
        assert!(msg.contains("Path not found"), "Error message: {}", msg);
        assert!(msg.contains("/nonexistent"), "Error message: {}", msg);
    }

    #[test]
    fn test_churn_error_not_git_repo() {
        let error = ChurnError::NotGitRepository {
            path: PathBuf::from("/some/path"),
        };
        let msg = error.to_string();
        assert!(
            msg.contains("Not a git repository"),
            "Error message: {}",
            msg
        );
    }

    #[test]
    fn test_churn_error_git_error() {
        let error = ChurnError::GitError {
            command: "git log".to_string(),
            stderr: "fatal: bad revision".to_string(),
            exit_code: Some(128),
        };
        let msg = error.to_string();
        assert!(msg.contains("Git command failed"), "Error message: {}", msg);
        assert!(msg.contains("git log"), "Error message: {}", msg);
    }

    #[test]
    fn test_churn_error_parse_error() {
        let error = ChurnError::ParseError {
            context: "numstat output".to_string(),
            line: "invalid\ttab\tseparated".to_string(),
        };
        let msg = error.to_string();
        assert!(msg.contains("Failed to parse"), "Error message: {}", msg);
    }
}

// =============================================================================
// Additional Tests from Critic Review (Critical/High Priority)
// =============================================================================

#[cfg(test)]
mod critic_review_tests {
    use super::fixtures::TestRepo;
    use super::*;

    /// Test: get_file_churn applies exclude patterns to filter results (Critical)
    ///
    /// Verifies that when exclude patterns are passed to get_file_churn,
    /// matching files are filtered out of the returned HashMap.
    #[test]
    #[ignore = "Requires git setup - run with --ignored"]
    fn test_get_file_churn_with_exclude() {
        let repo = TestRepo::new().expect("Failed to create test repo");

        // Create files including ones that should be excluded
        repo.add_file("src/main.rs", "fn main() {}\n").unwrap();
        repo.commit("Add main").unwrap();

        repo.add_file("node_modules/pkg/index.js", "module.exports = {};\n")
            .unwrap();
        repo.commit("Add node module").unwrap();

        repo.add_file("Cargo.lock", "# lock file\n").unwrap();
        repo.commit("Add lock file").unwrap();

        repo.add_file("src/lib.rs", "pub fn lib() {}\n").unwrap();
        repo.commit("Add lib").unwrap();

        // Get churn with exclude patterns
        let exclude_patterns = vec!["node_modules/*".to_string(), "*.lock".to_string()];
        let result = get_file_churn(repo.path(), 365, &exclude_patterns);

        assert!(result.is_ok(), "Should succeed: {:?}", result.err());
        let churn = result.unwrap();

        // Should have src/main.rs and src/lib.rs
        assert!(
            churn.contains_key("src/main.rs"),
            "main.rs should be included"
        );
        assert!(
            churn.contains_key("src/lib.rs"),
            "lib.rs should be included"
        );

        // Should NOT have node_modules or lock files
        assert!(
            !churn.contains_key("node_modules/pkg/index.js"),
            "node_modules should be excluded"
        );
        assert!(
            !churn.contains_key("Cargo.lock"),
            "Cargo.lock should be excluded"
        );

        // Verify count
        assert_eq!(churn.len(), 2, "Should only have 2 files after exclusions");
    }

    /// Test: --authors flag populates authors array in ChurnReport (High)
    ///
    /// Verifies that when the CLI is called with --authors flag,
    /// the resulting JSON contains a populated authors array.
    #[test]
    #[ignore = "Requires CLI binary - run with --ignored"]
    fn test_churn_cli_authors_flag() {
        use serde_json::Value;
        use std::process::Command;

        let repo = TestRepo::new().expect("Failed to create test repo");

        // Create commits from different authors
        repo.add_file("file1.txt", "content1\n").unwrap();
        repo.commit_as("Commit by Alice", "Alice", "alice@example.com")
            .unwrap();

        repo.add_file("file2.txt", "content2\n").unwrap();
        repo.commit_as("Commit by Bob", "Bob", "bob@example.com")
            .unwrap();

        let output = Command::new("cargo")
            .args([
                "run",
                "--",
                "churn",
                repo.path().to_str().unwrap(),
                "--authors",
                "--format",
                "json",
            ])
            .output()
            .expect("Failed to run command");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("Output should be valid JSON");

        // Verify authors array is populated
        let authors = json["authors"].as_array().expect("authors should be array");
        assert!(
            !authors.is_empty(),
            "authors array should not be empty when --authors flag is used"
        );

        // Verify author structure
        let alice = authors.iter().find(|a| a["email"] == "alice@example.com");
        let bob = authors.iter().find(|a| a["email"] == "bob@example.com");

        assert!(alice.is_some(), "Alice should be in authors");
        assert!(bob.is_some(), "Bob should be in authors");

        // Verify author fields are present
        let alice = alice.unwrap();
        assert!(alice["name"].is_string(), "author should have name");
        assert!(alice["commits"].is_number(), "author should have commits");
        assert!(
            alice["lines_added"].is_number(),
            "author should have lines_added"
        );
        assert!(
            alice["lines_deleted"].is_number(),
            "author should have lines_deleted"
        );
        assert!(
            alice["files_touched"].is_number(),
            "author should have files_touched"
        );
    }

    /// Test: --hotspots flag populates hotspots array in ChurnReport (High)
    ///
    /// Verifies that when the CLI is called with --hotspots flag,
    /// the resulting JSON contains a populated hotspots array.
    #[test]
    #[ignore = "Requires CLI binary - run with --ignored"]
    fn test_churn_cli_hotspots_flag() {
        use serde_json::Value;
        use std::process::Command;

        let repo = TestRepo::new().expect("Failed to create test repo");

        // Create a file with some commits
        repo.add_file("src/main.rs", "fn main() { if true { 1 } else { 2 } }\n")
            .unwrap();
        repo.commit("Add main").unwrap();

        repo.add_file(
            "src/main.rs",
            "fn main() { if true { 1 } else { 2 } }\n// update\n",
        )
        .unwrap();
        repo.commit("Update main").unwrap();

        let output = Command::new("cargo")
            .args([
                "run",
                "--",
                "churn",
                repo.path().to_str().unwrap(),
                "--hotspots",
                "--format",
                "json",
            ])
            .output()
            .expect("Failed to run command");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("Output should be valid JSON");

        // Verify hotspots array is populated
        let hotspots = json["hotspots"]
            .as_array()
            .expect("hotspots should be array");
        assert!(
            !hotspots.is_empty(),
            "hotspots array should not be empty when --hotspots flag is used"
        );

        // Verify hotspot structure
        let hotspot = &hotspots[0];
        assert!(hotspot["file"].is_string(), "hotspot should have file");
        assert!(
            hotspot["churn_rank"].is_number(),
            "hotspot should have churn_rank"
        );
        assert!(
            hotspot["complexity_rank"].is_number(),
            "hotspot should have complexity_rank"
        );
        assert!(
            hotspot["combined_score"].is_number(),
            "hotspot should have combined_score"
        );
        assert!(
            hotspot["commit_count"].is_number(),
            "hotspot should have commit_count"
        );
        assert!(
            hotspot["cyclomatic_complexity"].is_number(),
            "hotspot should have cyclomatic_complexity"
        );
        assert!(
            hotspot["recommendation"].is_string(),
            "hotspot should have recommendation"
        );

        // Verify score is in valid range
        let score = hotspot["combined_score"].as_f64().unwrap();
        assert!(
            (0.0..=1.0).contains(&score),
            "combined_score {} should be in [0, 1]",
            score
        );
    }

    /// Test: get_author_stats returns authors sorted by commits descending (Medium)
    ///
    /// Verifies that the returned Vec<AuthorStats> is sorted with the
    /// author having the most commits first.
    #[test]
    #[ignore = "Requires git setup - run with --ignored"]
    fn test_get_author_stats_sorted() {
        let repo = TestRepo::new().expect("Failed to create test repo");

        // Create commits from different authors with different commit counts
        // Alice: 5 commits
        for i in 0..5 {
            repo.add_file(&format!("alice_file{}.txt", i), &format!("content{}\n", i))
                .unwrap();
            repo.commit_as(&format!("Alice commit {}", i), "Alice", "alice@example.com")
                .unwrap();
        }

        // Bob: 3 commits
        for i in 0..3 {
            repo.add_file(&format!("bob_file{}.txt", i), &format!("content{}\n", i))
                .unwrap();
            repo.commit_as(&format!("Bob commit {}", i), "Bob", "bob@example.com")
                .unwrap();
        }

        // Charlie: 1 commit
        repo.add_file("charlie_file.txt", "content\n").unwrap();
        repo.commit_as("Charlie commit", "Charlie", "charlie@example.com")
            .unwrap();

        let file_stats = get_file_churn(repo.path(), 365, &[]).expect("Should get churn");
        let author_stats =
            get_author_stats(repo.path(), 365, &file_stats).expect("Should get authors");

        // Verify we have all three authors
        assert_eq!(author_stats.len(), 3, "Should have 3 authors");

        // Verify sorted by commits descending
        for i in 1..author_stats.len() {
            assert!(
                author_stats[i - 1].commits >= author_stats[i].commits,
                "Authors should be sorted by commits descending: {} >= {} failed ({} vs {})",
                author_stats[i - 1].commits,
                author_stats[i].commits,
                author_stats[i - 1].email,
                author_stats[i].email
            );
        }

        // Verify Alice is first (most commits)
        assert_eq!(
            author_stats[0].email, "alice@example.com",
            "Alice should be first with most commits"
        );
        assert_eq!(author_stats[0].commits, 5, "Alice should have 5 commits");

        // Verify Charlie is last (fewest commits)
        assert_eq!(
            author_stats[2].email, "charlie@example.com",
            "Charlie should be last with fewest commits"
        );
        assert_eq!(author_stats[2].commits, 1, "Charlie should have 1 commit");
    }
}

// =============================================================================
// Output Formatting Tests - Phase 6
// =============================================================================

#[cfg(test)]
mod format_text_tests {
    use super::*;

    /// Helper to create a minimal valid report
    fn minimal_report() -> ChurnReport {
        ChurnReport {
            files: vec![],
            hotspots: vec![],
            authors: vec![],
            summary: ChurnSummary {
                total_files: 0,
                total_commits: 0,
                time_window_days: 30,
                total_lines_changed: 0,
                avg_commits_per_file: 0.0,
                most_churned_file: String::new(),
            },
            is_shallow: false,
            shallow_depth: None,
            warnings: vec![],
        }
    }

    /// Helper to create a report with files for testing
    fn report_with_files() -> ChurnReport {
        let files = vec![
            FileChurn {
                file: "src/core/engine.py".to_string(),
                commit_count: 47,
                lines_added: 1500,
                lines_deleted: 640,
                lines_changed: 2140,
                first_commit: Some("2025-01-01".to_string()),
                last_commit: Some("2026-01-15".to_string()),
                authors: vec![
                    "alice@example.com".to_string(),
                    "bob@example.com".to_string(),
                ],
                author_count: 2,
            },
            FileChurn {
                file: "src/api/handlers.py".to_string(),
                commit_count: 38,
                lines_added: 1000,
                lines_deleted: 520,
                lines_changed: 1520,
                first_commit: Some("2025-02-01".to_string()),
                last_commit: Some("2026-01-10".to_string()),
                authors: vec![
                    "alice@example.com".to_string(),
                    "bob@example.com".to_string(),
                    "charlie@example.com".to_string(),
                ],
                author_count: 3,
            },
        ];

        ChurnReport {
            files,
            hotspots: vec![],
            authors: vec![],
            summary: ChurnSummary {
                total_files: 2,
                total_commits: 85,
                time_window_days: 365,
                total_lines_changed: 3660,
                avg_commits_per_file: 42.5,
                most_churned_file: "src/core/engine.py".to_string(),
            },
            is_shallow: false,
            shallow_depth: None,
            warnings: vec![],
        }
    }

    /// Test: Header with analysis period is present
    #[test]
    fn test_format_text_header() {
        let report = minimal_report();
        let output = format_text_output(&report);

        assert!(
            output.contains("Code Churn Analysis"),
            "Should have header title"
        );
        assert!(output.contains("="), "Should have separator line");
    }

    /// Test: Summary statistics are displayed
    #[test]
    fn test_format_text_summary_stats() {
        let mut report = minimal_report();
        report.summary.time_window_days = 365;
        report.summary.total_files = 156;
        report.summary.total_commits = 892;
        report.summary.total_lines_changed = 45230;
        report.summary.most_churned_file = "src/core/engine.py".to_string();

        let output = format_text_output(&report);

        assert!(
            output.contains("Time window: 365 days"),
            "Should show time window"
        );
        assert!(
            output.contains("Total files changed: 156"),
            "Should show total files"
        );
        assert!(
            output.contains("Total commits: 892"),
            "Should show total commits"
        );
        assert!(
            output.contains("Total lines changed: 45230"),
            "Should show total lines"
        );
        assert!(
            output.contains("Most churned file: src/core/engine.py"),
            "Should show most churned"
        );
    }

    /// Test: Top files table is displayed with proper columns
    #[test]
    fn test_format_text_files_table() {
        let report = report_with_files();
        let output = format_text_output(&report);

        // Check table header
        assert!(
            output.contains("Top Files by Churn"),
            "Should have files section title"
        );
        assert!(output.contains("Rank"), "Should have Rank column");
        assert!(output.contains("File"), "Should have File column");
        assert!(output.contains("Commits"), "Should have Commits column");
        assert!(output.contains("Lines"), "Should have Lines column");
        assert!(output.contains("Authors"), "Should have Authors column");

        // Check file data is present
        assert!(
            output.contains("src/core/engine.py"),
            "Should show first file"
        );
        assert!(
            output.contains("47"),
            "Should show commit count for first file"
        );
        assert!(
            output.contains("src/api/handlers.py"),
            "Should show second file"
        );
    }

    /// Test: Long paths are truncated to 38 chars
    #[test]
    fn test_format_text_path_truncation() {
        let mut report = minimal_report();
        report.files = vec![FileChurn {
            file: "src/very/deeply/nested/directory/structure/with/many/levels/extremely_long_filename.py".to_string(),
            commit_count: 10,
            lines_added: 100,
            lines_deleted: 50,
            lines_changed: 150,
            first_commit: Some("2026-01-01".to_string()),
            last_commit: Some("2026-01-15".to_string()),
            authors: vec!["dev@example.com".to_string()],
            author_count: 1,
        }];
        report.summary.total_files = 1;

        let output = format_text_output(&report);

        // The path should be truncated - check that the full path is NOT present
        // but a truncated version is
        assert!(!output.contains("src/very/deeply/nested/directory/structure/with/many/levels/extremely_long_filename.py"),
            "Full path should be truncated");
        // Should contain a truncated version with ellipsis or similar
        assert!(
            output.contains("...") || output.len() < 200 || output.contains("extremely_long"),
            "Path should be truncated in some way"
        );
    }

    /// Test: Hotspots section appears when hotspots present
    #[test]
    fn test_format_text_hotspots_section() {
        let mut report = minimal_report();
        report.hotspots = vec![Hotspot {
            file: "src/core/engine.py".to_string(),
            churn_rank: 1,
            complexity_rank: 2,
            combined_score: 0.823,
            commit_count: 47,
            cyclomatic_complexity: 25,
            recommendation: "Critical: High churn + high complexity. Prioritize refactoring."
                .to_string(),
        }];

        let output = format_text_output(&report);

        assert!(output.contains("Hotspot"), "Should have Hotspots section");
        assert!(
            output.contains("src/core/engine.py"),
            "Should show hotspot file"
        );
        assert!(
            output.contains("0.823") || output.contains("0.82"),
            "Should show combined score"
        );
        assert!(
            output.contains("Critical") || output.contains("refactoring"),
            "Should show recommendation"
        );
    }

    /// Test: Authors section appears when authors present
    #[test]
    fn test_format_text_authors_section() {
        let mut report = minimal_report();
        report.authors = vec![
            AuthorStats {
                name: "Alice Smith".to_string(),
                email: "alice@example.com".to_string(),
                commits: 47,
                lines_added: 2000,
                lines_deleted: 500,
                files_touched: 23,
            },
            AuthorStats {
                name: "Bob Jones".to_string(),
                email: "bob@example.com".to_string(),
                commits: 23,
                lines_added: 1000,
                lines_deleted: 200,
                files_touched: 15,
            },
        ];

        let output = format_text_output(&report);

        assert!(
            output.contains("Author") || output.contains("author"),
            "Should have Authors section"
        );
        assert!(
            output.contains("Alice") || output.contains("alice@example.com"),
            "Should show first author"
        );
        assert!(output.contains("47"), "Should show first author's commits");
        assert!(
            output.contains("Bob") || output.contains("bob@example.com"),
            "Should show second author"
        );
    }

    /// Test: Warnings section appears when warnings present
    #[test]
    fn test_format_text_warnings_section() {
        let mut report = minimal_report();
        report.is_shallow = true;
        report.shallow_depth = Some(100);
        report.warnings = vec![
            "Repository is a shallow clone (depth ~100). Results may be incomplete.".to_string(),
        ];

        let output = format_text_output(&report);

        assert!(
            output.contains("Warning") || output.contains("warning"),
            "Should have Warnings section"
        );
        assert!(
            output.contains("shallow clone") || output.contains("depth"),
            "Should show shallow clone warning"
        );
    }

    /// Test: Empty sections are not displayed
    #[test]
    fn test_format_text_empty_sections_hidden() {
        let report = minimal_report();
        let output = format_text_output(&report);

        // With no hotspots, the Hotspot Matrix section should be absent or empty
        // With no authors, the Top Authors section should be absent or empty
        // With no warnings, the Warnings section should be absent
        let lines: Vec<&str> = output.lines().collect();

        // Count non-empty lines - should be compact
        let non_empty: Vec<&&str> = lines
            .iter()
            .filter(|l: &&&str| !l.trim().is_empty())
            .collect();

        // A minimal report should still have header and summary, but be relatively short
        assert!(
            non_empty.len() < 20,
            "Minimal report should be concise, got {} lines",
            non_empty.len()
        );
    }

    /// Test: Top 10 files limit is enforced
    #[test]
    fn test_format_text_files_limit() {
        let mut report = minimal_report();

        // Create 15 files
        for i in 1..=15 {
            report.files.push(FileChurn {
                file: format!("src/file{}.py", i),
                commit_count: 100 - i,
                lines_added: 100,
                lines_deleted: 50,
                lines_changed: 150,
                first_commit: Some("2026-01-01".to_string()),
                last_commit: Some("2026-01-15".to_string()),
                authors: vec!["dev@example.com".to_string()],
                author_count: 1,
            });
        }
        report.summary.total_files = 15;

        let output = format_text_output(&report);

        // Should show files 1-10 but not 11-15
        assert!(output.contains("file1.py"), "Should show file1");
        assert!(output.contains("file10.py"), "Should show file10");
        assert!(
            !output.contains("file11.py"),
            "Should NOT show file11 (limit is 10)"
        );
        assert!(!output.contains("file15.py"), "Should NOT show file15");
    }

    /// Test: Top 5 hotspots limit is enforced
    #[test]
    fn test_format_text_hotspots_limit() {
        let mut report = minimal_report();

        // Create 8 hotspots
        for i in 1..=8 {
            report.hotspots.push(Hotspot {
                file: format!("src/hot{}.py", i),
                churn_rank: i,
                complexity_rank: i,
                combined_score: 1.0 - (i as f64 * 0.1),
                commit_count: 50 - i,
                cyclomatic_complexity: 30 - i,
                recommendation: "Warning".to_string(),
            });
        }

        let output = format_text_output(&report);

        // Should show hotspots 1-5 but not 6-8
        assert!(output.contains("hot1.py"), "Should show hot1");
        assert!(output.contains("hot5.py"), "Should show hot5");
        assert!(
            !output.contains("hot6.py"),
            "Should NOT show hot6 (limit is 5)"
        );
        assert!(!output.contains("hot8.py"), "Should NOT show hot8");
    }

    /// Test: Top 5 authors limit is enforced
    #[test]
    fn test_format_text_authors_limit() {
        let mut report = minimal_report();

        // Create 8 authors
        for i in 1..=8 {
            report.authors.push(AuthorStats {
                name: format!("Author {}", i),
                email: format!("author{}@example.com", i),
                commits: 100 - i as u32,
                lines_added: 500,
                lines_deleted: 200,
                files_touched: 10,
            });
        }

        let output = format_text_output(&report);

        // Should show authors 1-5 but not 6-8
        assert!(
            output.contains("author1@") || output.contains("Author 1"),
            "Should show author1"
        );
        assert!(
            output.contains("author5@") || output.contains("Author 5"),
            "Should show author5"
        );
        assert!(
            !output.contains("author6@") && !output.contains("Author 6"),
            "Should NOT show author6 (limit is 5)"
        );
        assert!(
            !output.contains("author8@") && !output.contains("Author 8"),
            "Should NOT show author8"
        );
    }

    /// Test: Fixed-width columns for alignment
    #[test]
    fn test_format_text_column_alignment() {
        let report = report_with_files();
        let output = format_text_output(&report);

        // Find the table section and check that lines have consistent structure
        let lines: Vec<&str> = output.lines().collect();

        // Find lines that look like table rows (contain numbers and file paths)
        let table_rows: Vec<&str> = lines
            .iter()
            .filter(|l: &&&str| l.contains(".py") && l.chars().any(|c: char| c.is_ascii_digit()))
            .copied()
            .collect();

        assert!(!table_rows.is_empty(), "Should have table rows");

        // All table rows should have similar length (within 10 chars)
        if table_rows.len() >= 2 {
            let first_len = table_rows[0].len() as i32;
            for row in &table_rows {
                let row_len = row.len() as i32;
                let diff = (row_len - first_len).abs();
                assert!(
                    diff < 15,
                    "Table rows should have similar length. First: {}, this: {}",
                    first_len,
                    row_len
                );
            }
        }
    }
}

// =============================================================================
// Phase 2: Bot Detection Tests
// =============================================================================

mod bot_detection_tests {
    use super::*;

    #[test]
    fn test_is_bot_author_dependabot() {
        assert!(is_bot_author(
            "dependabot[bot]",
            "dependabot[bot]@users.noreply.github.com"
        ));
    }

    #[test]
    fn test_is_bot_author_renovate() {
        assert!(is_bot_author(
            "renovate[bot]",
            "renovate[bot]@users.noreply.github.com"
        ));
    }

    #[test]
    fn test_is_bot_author_github_actions() {
        assert!(is_bot_author(
            "github-actions[bot]",
            "github-actions@users.noreply.github.com"
        ));
    }

    #[test]
    fn test_is_bot_author_snyk() {
        assert!(is_bot_author("snyk-bot", "snyk-bot@snyk.io"));
    }

    #[test]
    fn test_is_bot_author_generic_bot_suffix() {
        assert!(is_bot_author("my-custom-app[bot]", "custom@example.com"));
    }

    #[test]
    fn test_is_bot_author_human_not_matched() {
        assert!(!is_bot_author("John Smith", "john@company.com"));
        assert!(!is_bot_author("Alice Developer", "alice@dev.org"));
        assert!(!is_bot_author("robot-enthusiast", "robot@company.com"));
    }

    #[test]
    fn test_is_bot_author_case_insensitive() {
        assert!(is_bot_author("Dependabot[Bot]", "DEPENDABOT@github.com"));
        assert!(is_bot_author("RENOVATE[BOT]", "renovate@example.com"));
    }

    #[test]
    fn test_is_bot_author_email_match() {
        assert!(is_bot_author(
            "Dependency Updater",
            "dependabot@users.noreply.github.com"
        ));
    }

    #[test]
    fn test_is_bot_author_no_false_positives() {
        assert!(!is_bot_author("robotics-engineer", "robotics@company.com"));
        assert!(!is_bot_author("abbott", "abbott@company.com"));
    }
}

// =============================================================================
// Phase 2: Detailed Churn Pipeline Tests
// =============================================================================

mod detailed_churn_tests {
    use super::*;

    #[test]
    #[ignore = "Requires git setup - run with --ignored"]
    fn test_get_file_churn_detailed_basic() {
        let repo = fixtures::TestRepo::new().unwrap();

        // Create a file and make some commits
        repo.add_file("src/main.rs", "fn main() {\n    println!(\"hello\");\n}\n")
            .unwrap();
        repo.commit("Initial commit").unwrap();

        repo.add_file(
            "src/main.rs",
            "fn main() {\n    println!(\"hello world\");\n    println!(\"extra line\");\n}\n",
        )
        .unwrap();
        repo.commit("Update main").unwrap();

        let (result, bot_count) = get_file_churn_detailed(repo.path(), 365, &[], false).unwrap();

        // Should find the file with 2 commits
        assert!(!result.is_empty(), "Should have at least one file");
        assert_eq!(bot_count, 0, "No bot commits expected");

        // Find our file
        let main_entry = result.values().find(|v| v.base.file.contains("main.rs"));
        assert!(main_entry.is_some(), "Should find src/main.rs in results");

        let main = main_entry.unwrap();
        assert_eq!(main.base.commit_count, 2, "Should have 2 commits");
        assert_eq!(main.commits.len(), 2, "Should have 2 commit entries");
        assert!(
            main.base.lines_changed > 0,
            "Should have non-zero lines changed"
        );
    }

    #[test]
    #[ignore = "Requires git setup - run with --ignored"]
    fn test_get_file_churn_detailed_bot_filtering() {
        let repo = fixtures::TestRepo::new().unwrap();

        // Human commit
        repo.add_file("src/lib.rs", "pub fn hello() {}\n").unwrap();
        repo.commit_as("Human commit", "Alice", "alice@dev.com")
            .unwrap();

        // Bot commit
        repo.add_file("package-lock.json", "{\"version\": 2}\n")
            .unwrap();
        repo.commit_as(
            "Bump deps",
            "dependabot[bot]",
            "dependabot@users.noreply.github.com",
        )
        .unwrap();

        // Another human commit
        repo.add_file("src/lib.rs", "pub fn hello() { println!(\"hi\"); }\n")
            .unwrap();
        repo.commit_as("Update lib", "Bob", "bob@dev.com").unwrap();

        // With bot filtering (default)
        let (_result_no_bots, bot_count_no_bots) =
            get_file_churn_detailed(repo.path(), 365, &[], false).unwrap();

        assert!(
            bot_count_no_bots >= 1,
            "Should have filtered at least 1 bot commit, got {}",
            bot_count_no_bots
        );

        // With bots included
        let (result_with_bots, bot_count_with_bots) =
            get_file_churn_detailed(repo.path(), 365, &[], true).unwrap();

        assert_eq!(
            bot_count_with_bots, 0,
            "No filtering when include_bots=true"
        );

        // The bot-included version should have the package-lock.json
        let has_lock_with_bots = result_with_bots.keys().any(|k| k.contains("package-lock"));
        assert!(
            has_lock_with_bots,
            "With bots, should include package-lock.json"
        );
    }

    #[test]
    #[ignore = "Requires git setup - run with --ignored"]
    fn test_get_file_churn_detailed_author_attribution() {
        let repo = fixtures::TestRepo::new().unwrap();

        // Multiple authors editing same file
        repo.add_file("shared.rs", "line 1\n").unwrap();
        repo.commit_as("Author 1", "Alice", "alice@dev.com")
            .unwrap();

        repo.add_file("shared.rs", "line 1\nline 2\n").unwrap();
        repo.commit_as("Author 2", "Bob", "bob@dev.com").unwrap();

        repo.add_file("shared.rs", "line 1\nline 2\nline 3\n")
            .unwrap();
        repo.commit_as("Author 1 again", "Alice", "alice@dev.com")
            .unwrap();

        let (result, _) = get_file_churn_detailed(repo.path(), 365, &[], false).unwrap();

        let shared = result.values().find(|v| v.base.file.contains("shared.rs"));
        assert!(shared.is_some(), "Should find shared.rs");

        let shared = shared.unwrap();
        // Should have 3 commits
        assert_eq!(shared.base.commit_count, 3);
        // Should have 2 unique authors
        assert_eq!(shared.base.author_count, 2, "Should have 2 unique authors");
    }

    #[test]
    #[ignore = "Requires git setup - run with --ignored"]
    fn test_get_file_churn_detailed_exclude_patterns() {
        let repo = fixtures::TestRepo::new().unwrap();

        repo.add_file("src/main.rs", "fn main() {}\n").unwrap();
        repo.add_file("Cargo.lock", "# generated\n").unwrap();
        repo.commit("Initial").unwrap();

        // Without exclude
        let (result_all, _) = get_file_churn_detailed(repo.path(), 365, &[], false).unwrap();

        // With exclude pattern
        let (result_filtered, _) =
            get_file_churn_detailed(repo.path(), 365, &["*.lock".to_string()], false).unwrap();

        // Filtered should not contain the lock file
        let has_lock_all = result_all.keys().any(|k| k.contains("Cargo.lock"));
        let has_lock_filtered = result_filtered.keys().any(|k| k.contains("Cargo.lock"));

        assert!(has_lock_all, "Unfiltered should include Cargo.lock");
        assert!(!has_lock_filtered, "Filtered should exclude Cargo.lock");
    }

    #[test]
    #[ignore = "Requires git setup - run with --ignored"]
    fn test_get_file_churn_detailed_commit_dates() {
        let repo = fixtures::TestRepo::new().unwrap();

        repo.add_file("file.txt", "v1\n").unwrap();
        repo.commit("First").unwrap();

        repo.add_file("file.txt", "v2\n").unwrap();
        repo.commit("Second").unwrap();

        let (result, _) = get_file_churn_detailed(repo.path(), 365, &[], false).unwrap();

        let entry = result.values().find(|v| v.base.file.contains("file.txt"));
        assert!(entry.is_some(), "Should find file.txt");

        let entry = entry.unwrap();
        // Each commit entry should have a non-empty date
        for commit in &entry.commits {
            assert!(!commit.date.is_empty(), "Commit date should not be empty");
        }

        // Base should have date range
        assert!(
            entry.base.first_commit.is_some(),
            "Should have first_commit date"
        );
        assert!(
            entry.base.last_commit.is_some(),
            "Should have last_commit date"
        );
    }

    #[test]
    #[ignore = "Requires git setup - run with --ignored"]
    fn test_get_file_churn_detailed_empty_repo() {
        let repo = fixtures::TestRepo::new().unwrap();

        // Make one empty initial commit so git log doesn't fail on no HEAD
        repo.commit("Initial").unwrap();

        let (result, bot_count) = get_file_churn_detailed(repo.path(), 365, &[], false).unwrap();

        assert!(result.is_empty(), "Empty repo should return no files");
        assert_eq!(bot_count, 0);
    }

    #[test]
    fn test_get_file_churn_detailed_not_git_repo() {
        let dir = tempfile::TempDir::new().unwrap();
        let result = get_file_churn_detailed(dir.path(), 365, &[], false);

        assert!(result.is_err(), "Non-git directory should return error");
        match result.unwrap_err() {
            ChurnError::NotGitRepository { .. } => {}
            other => panic!("Expected NotGitRepository, got: {:?}", other),
        }
    }

    #[test]
    fn test_get_file_churn_detailed_path_not_found() {
        let result = get_file_churn_detailed(
            std::path::Path::new("/nonexistent/path/xyz"),
            365,
            &[],
            false,
        );

        assert!(result.is_err(), "Non-existent path should return error");
        match result.unwrap_err() {
            ChurnError::PathNotFound(_) => {}
            other => panic!("Expected PathNotFound, got: {:?}", other),
        }
    }
}
