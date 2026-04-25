//! Git change detection for bugbot
//!
//! Detects files changed via git, filtered to the target language.
//! Uses direct `git` commands to list changed files -- no call graph needed.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use tldr_core::Language;

/// Result of detecting changed files in the project.
#[derive(Debug, Clone)]
pub struct ChangeDetectionResult {
    /// Files that changed and match the target language.
    pub changed_files: Vec<PathBuf>,
    /// How changes were detected (e.g. "git:staged", "git:uncommitted").
    pub detection_method: String,
}

/// Run a git command in `project` and return the listed file paths.
///
/// Each non-empty line of stdout is joined with `project` to form an absolute path.
fn git_changed_files(project: &Path, args: &[&str]) -> Result<Vec<PathBuf>> {
    let output = Command::new("git")
        .args(args)
        .current_dir(project)
        .output()
        .context("Failed to run git")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git command failed: {}", stderr);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| project.join(l))
        .collect())
}

/// Detect changed files in `project`, filtered to the given `language`.
///
/// # Arguments
/// * `project` - Project root directory (must be inside a git repo)
/// * `base_ref` - Git base reference (e.g. "HEAD", "main", "origin/main")
/// * `staged` - If true, only consider staged changes; otherwise all uncommitted
/// * `language` - Only return files matching this language's extensions
///
/// # Detection Method
/// - `staged == true`  => `"git:staged"`
/// - `staged == false` and `base_ref == "HEAD"` => `"git:uncommitted"`
/// - `staged == false` and `base_ref != "HEAD"` => `"git:{base_ref}...HEAD"`
///
/// # Returns
/// A `ChangeDetectionResult` with the filtered file list and the detection method string.
pub fn detect_changes(
    project: &Path,
    base_ref: &str,
    staged: bool,
    language: &Language,
) -> Result<ChangeDetectionResult> {
    let (raw_files, detection_method) = if staged {
        let files = git_changed_files(project, &["diff", "--name-only", "--staged"])
            .context("Failed to list staged changes")?;
        (files, "git:staged".to_string())
    } else if base_ref == "HEAD" {
        // Uncommitted = modified tracked + staged + untracked
        let mut files = git_changed_files(project, &["diff", "--name-only", "HEAD"])
            .context("Failed to list uncommitted changes")?;
        let staged_files = git_changed_files(project, &["diff", "--name-only", "--staged"])
            .context("Failed to list staged changes")?;
        let untracked = git_changed_files(project, &["ls-files", "--others", "--exclude-standard"])
            .context("Failed to list untracked files")?;
        files.extend(staged_files);
        files.extend(untracked);
        files.sort();
        files.dedup();
        (files, "git:uncommitted".to_string())
    } else {
        let range = format!("{}...HEAD", base_ref);
        let files = git_changed_files(project, &["diff", "--name-only", &range])
            .context("Failed to list base-ref changes")?;
        (files, format!("git:{}...HEAD", base_ref))
    };

    // Filter files to only those matching the target language's extensions.
    let valid_extensions = language.extensions();
    let changed_files: Vec<PathBuf> = raw_files
        .into_iter()
        .filter(|f| {
            f.extension()
                .and_then(|e| e.to_str())
                .map(|ext| {
                    let dotted = format!(".{}", ext);
                    valid_extensions.contains(&dotted.as_str())
                })
                .unwrap_or(false)
        })
        .collect();

    // Filter out paths matching .tldrignore patterns (e.g. corpus/, vendor/).
    let changed_files = tldr_core::callgraph::filter_tldrignored(project, changed_files);

    Ok(ChangeDetectionResult {
        changed_files,
        detection_method,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Helper: initialize a git repo with an initial commit in a temp directory.
    fn init_git_repo() -> TempDir {
        let tmp = TempDir::new().expect("create temp dir");
        let dir = tmp.path();

        Command::new("git")
            .args(["init"])
            .current_dir(dir)
            .output()
            .expect("git init");

        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(dir)
            .output()
            .expect("git config email");

        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir)
            .output()
            .expect("git config name");

        // Create an initial commit so HEAD exists
        std::fs::write(dir.join("README.md"), "# test\n").expect("write readme");
        Command::new("git")
            .args(["add", "."])
            .current_dir(dir)
            .output()
            .expect("git add");
        Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(dir)
            .output()
            .expect("git commit");

        tmp
    }

    #[test]
    fn test_detect_changes_no_changes_returns_empty() {
        let tmp = init_git_repo();
        let result =
            detect_changes(tmp.path(), "HEAD", false, &Language::Rust).expect("detect_changes");

        assert!(
            result.changed_files.is_empty(),
            "Expected no changed files in a clean repo, got: {:?}",
            result.changed_files
        );
        assert_eq!(result.detection_method, "git:uncommitted");
    }

    #[test]
    fn test_detect_changes_staged_method() {
        let tmp = init_git_repo();
        let result =
            detect_changes(tmp.path(), "HEAD", true, &Language::Rust).expect("detect_changes");

        assert_eq!(result.detection_method, "git:staged");
    }

    #[test]
    fn test_detect_changes_base_ref_method() {
        let tmp = init_git_repo();
        // Create a branch named "main" so the base ref is valid
        Command::new("git")
            .args(["branch", "main"])
            .current_dir(tmp.path())
            .output()
            .expect("git branch main");

        let result =
            detect_changes(tmp.path(), "main", false, &Language::Python).expect("detect_changes");

        assert_eq!(result.detection_method, "git:main...HEAD");
    }

    #[test]
    fn test_detect_changes_filters_by_language() {
        let tmp = init_git_repo();
        let dir = tmp.path();

        // Create uncommitted files of different languages
        std::fs::write(dir.join("hello.rs"), "fn main() {}\n").expect("write rs");
        std::fs::write(dir.join("hello.py"), "print('hi')\n").expect("write py");
        std::fs::write(dir.join("hello.js"), "console.log('hi')\n").expect("write js");

        // Stage them all (so git sees them as changes)
        Command::new("git")
            .args(["add", "."])
            .current_dir(dir)
            .output()
            .expect("git add");

        // Detect only Rust changes
        let result =
            detect_changes(dir, "HEAD", true, &Language::Rust).expect("detect_changes rust");

        // Only .rs files should appear
        for f in &result.changed_files {
            assert_eq!(
                f.extension().and_then(|e| e.to_str()),
                Some("rs"),
                "Expected only .rs files, got: {}",
                f.display()
            );
        }
        assert!(
            !result.changed_files.is_empty(),
            "Expected at least one .rs file in changed_files"
        );

        // Detect only Python changes
        let result =
            detect_changes(dir, "HEAD", true, &Language::Python).expect("detect_changes python");

        for f in &result.changed_files {
            assert_eq!(
                f.extension().and_then(|e| e.to_str()),
                Some("py"),
                "Expected only .py files, got: {}",
                f.display()
            );
        }
        assert!(
            !result.changed_files.is_empty(),
            "Expected at least one .py file in changed_files"
        );
    }

    #[test]
    fn test_detect_changes_uncommitted_finds_unstaged() {
        let tmp = init_git_repo();
        let dir = tmp.path();

        // Modify a tracked file (create it first, commit, then modify)
        let rs_file = dir.join("lib.rs");
        std::fs::write(&rs_file, "pub fn old() {}\n").expect("write rs");
        Command::new("git")
            .args(["add", "lib.rs"])
            .current_dir(dir)
            .output()
            .expect("git add");
        Command::new("git")
            .args(["commit", "-m", "add lib"])
            .current_dir(dir)
            .output()
            .expect("git commit");

        // Now modify it without staging
        std::fs::write(&rs_file, "pub fn new_version() {}\n").expect("overwrite rs");

        let result = detect_changes(dir, "HEAD", false, &Language::Rust).expect("detect_changes");

        assert_eq!(result.detection_method, "git:uncommitted");
        assert!(
            result.changed_files.iter().any(|f| {
                f.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n == "lib.rs")
                    .unwrap_or(false)
            }),
            "Expected lib.rs in changed files, got: {:?}",
            result.changed_files
        );
    }

    #[test]
    fn test_detect_changes_ignores_non_matching_extensions() {
        let tmp = init_git_repo();
        let dir = tmp.path();

        // Create only non-Rust files
        std::fs::write(dir.join("app.py"), "x = 1\n").expect("write py");
        std::fs::write(dir.join("app.js"), "var x = 1;\n").expect("write js");
        Command::new("git")
            .args(["add", "."])
            .current_dir(dir)
            .output()
            .expect("git add");

        let result = detect_changes(dir, "HEAD", true, &Language::Rust).expect("detect_changes");

        assert!(
            result.changed_files.is_empty(),
            "Expected no Rust files when only .py and .js were changed, got: {:?}",
            result.changed_files
        );
    }

    #[test]
    fn test_change_detection_result_fields() {
        let result = ChangeDetectionResult {
            changed_files: vec![PathBuf::from("src/main.rs")],
            detection_method: "git:staged".to_string(),
        };
        assert_eq!(result.changed_files.len(), 1);
        assert_eq!(result.detection_method, "git:staged");
    }

    #[test]
    fn test_detect_changes_respects_tldrignore() {
        let tmp = init_git_repo();
        let dir = tmp.path();

        // Create files in corpus/ (should be ignored) and src/ (should survive)
        std::fs::create_dir_all(dir.join("corpus")).unwrap();
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("corpus/vendored.py"), "x = 1\n").unwrap();
        std::fs::write(dir.join("src/main.py"), "y = 2\n").unwrap();

        // Create .tldrignore excluding corpus/
        std::fs::write(dir.join(".tldrignore"), "corpus/\n").unwrap();

        // Stage all files
        Command::new("git")
            .args(["add", "."])
            .current_dir(dir)
            .output()
            .expect("git add");

        let result = detect_changes(dir, "HEAD", true, &Language::Python).expect("detect_changes");

        // corpus/vendored.py should be excluded, only src/main.py remains
        assert!(
            !result
                .changed_files
                .iter()
                .any(|f| { f.to_string_lossy().contains("corpus") }),
            "corpus/ files should be excluded by .tldrignore, got: {:?}",
            result.changed_files
        );
        assert!(
            result.changed_files.iter().any(|f| {
                f.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n == "main.py")
                    .unwrap_or(false)
            }),
            "src/main.py should be present, got: {:?}",
            result.changed_files
        );
    }
}
