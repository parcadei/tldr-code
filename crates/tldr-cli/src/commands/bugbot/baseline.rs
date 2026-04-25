//! Git baseline extraction for bugbot
//!
//! Retrieves the "before" version of changed files from git so the analysis
//! pipeline can compare baseline vs current and detect regressions.

use std::io::Write;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};
use tempfile::NamedTempFile;

/// Result of checking baseline status for a file.
#[derive(Debug)]
pub enum BaselineStatus {
    /// File exists at the baseline ref; contains the original content.
    Exists(String),
    /// File is new -- it did not exist at the baseline ref.
    NewFile,
    /// `git show` failed for an unexpected reason (stderr captured).
    GitShowFailed(String),
}

/// Get the content of a file at the given git ref.
///
/// # Arguments
/// * `project` - Project root directory (must be inside a git repo).
/// * `file`    - Path to the file (absolute or relative to `project`).
/// * `base_ref`- Git ref to read from, e.g. `"HEAD"`, `"main"`.
///
/// # Returns
/// * `BaselineStatus::Exists(content)` when the file existed at `base_ref`.
/// * `BaselineStatus::NewFile` when `git show` reports the path does not exist.
/// * `BaselineStatus::GitShowFailed(stderr)` on other git failures.
pub fn get_baseline_content(project: &Path, file: &Path, base_ref: &str) -> Result<BaselineStatus> {
    // Compute relative path from project root.
    // If the file is already relative (or outside the project) we fall through.
    let relative = file.strip_prefix(project).unwrap_or(file);

    // On all platforms git expects forward-slash separators in `ref:path`.
    let relative_str = relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("/");

    let output = Command::new("git")
        .args(["show", &format!("{}:{}", base_ref, relative_str)])
        .current_dir(project)
        .output()
        .context("Failed to run git show")?;

    if output.status.success() {
        let content =
            String::from_utf8(output.stdout).context("git show output is not valid UTF-8")?;
        Ok(BaselineStatus::Exists(content))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("does not exist")
            || stderr.contains("not exist in")
            || stderr.contains("exists on disk, but not in")
            || stderr.contains("did not match any")
        {
            Ok(BaselineStatus::NewFile)
        } else {
            Ok(BaselineStatus::GitShowFailed(stderr.to_string()))
        }
    }
}

/// Write baseline content to a temporary file with the correct extension.
///
/// The extension is preserved so that tree-sitter can detect the language
/// when parsing the temporary file.  The caller must keep the returned
/// `NamedTempFile` handle alive -- dropping it deletes the file.
pub fn write_baseline_tmpfile(content: &str, file_path: &Path) -> Result<NamedTempFile> {
    let extension = file_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("txt");

    let mut tmpfile = tempfile::Builder::new()
        .prefix("bugbot_baseline_")
        .suffix(&format!(".{}", extension))
        .tempfile()
        .context("Failed to create temp file for baseline")?;

    tmpfile
        .write_all(content.as_bytes())
        .context("Failed to write baseline content to temp file")?;
    tmpfile.flush()?;

    Ok(tmpfile)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Helper: initialize a git repo with an initial commit in a temp directory.
    fn init_git_repo() -> tempfile::TempDir {
        let tmp = tempfile::TempDir::new().expect("create temp dir");
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

        // Create an initial commit so HEAD exists.
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
    fn test_get_baseline_existing_file() {
        let tmp = init_git_repo();
        let dir = tmp.path();

        // Commit a file with known content.
        let original = "fn original() {}\n";
        std::fs::write(dir.join("lib.rs"), original).expect("write lib.rs");
        Command::new("git")
            .args(["add", "lib.rs"])
            .current_dir(dir)
            .output()
            .expect("git add");
        Command::new("git")
            .args(["commit", "-m", "add lib.rs"])
            .current_dir(dir)
            .output()
            .expect("git commit");

        // Modify the file (uncommitted).
        std::fs::write(dir.join("lib.rs"), "fn modified() {}\n").expect("overwrite lib.rs");

        // Baseline at HEAD should return the original content.
        let status =
            get_baseline_content(dir, &dir.join("lib.rs"), "HEAD").expect("get_baseline_content");

        match status {
            BaselineStatus::Exists(content) => {
                assert_eq!(
                    content, original,
                    "Baseline should return the committed content"
                );
            }
            other => panic!("Expected BaselineStatus::Exists, got: {:?}", other),
        }
    }

    #[test]
    fn test_get_baseline_new_file() {
        let tmp = init_git_repo();
        let dir = tmp.path();

        // Create a file that has never been committed.
        std::fs::write(dir.join("brand_new.rs"), "fn new() {}\n").expect("write new file");

        let status = get_baseline_content(dir, &dir.join("brand_new.rs"), "HEAD")
            .expect("get_baseline_content");

        match status {
            BaselineStatus::NewFile => {} // expected
            other => panic!("Expected BaselineStatus::NewFile, got: {:?}", other),
        }
    }

    #[test]
    fn test_get_baseline_deleted_file() {
        let tmp = init_git_repo();
        let dir = tmp.path();

        // Commit a file.
        let original = "fn to_delete() {}\n";
        std::fs::write(dir.join("doomed.rs"), original).expect("write doomed.rs");
        Command::new("git")
            .args(["add", "doomed.rs"])
            .current_dir(dir)
            .output()
            .expect("git add");
        Command::new("git")
            .args(["commit", "-m", "add doomed.rs"])
            .current_dir(dir)
            .output()
            .expect("git commit");

        // Delete the file from the working tree.
        std::fs::remove_file(dir.join("doomed.rs")).expect("delete doomed.rs");

        // Baseline at HEAD should still return the committed content.
        let status = get_baseline_content(dir, &dir.join("doomed.rs"), "HEAD")
            .expect("get_baseline_content");

        match status {
            BaselineStatus::Exists(content) => {
                assert_eq!(
                    content, original,
                    "Baseline should return the committed content even after deletion"
                );
            }
            other => panic!("Expected BaselineStatus::Exists, got: {:?}", other),
        }
    }

    #[test]
    fn test_tmpfile_has_correct_extension() {
        let tmpfile =
            write_baseline_tmpfile("content", &PathBuf::from("src/lib.rs")).expect("write tmpfile");

        let path = tmpfile.path();
        let ext = path.extension().and_then(|e| e.to_str());
        assert_eq!(ext, Some("rs"), "Temp file should have .rs extension");
    }

    #[test]
    fn test_tmpfile_content_matches() {
        let content = "fn hello() { println!(\"world\"); }\n";
        let tmpfile =
            write_baseline_tmpfile(content, &PathBuf::from("example.py")).expect("write tmpfile");

        let read_back = std::fs::read_to_string(tmpfile.path()).expect("read tmpfile");
        assert_eq!(
            read_back, content,
            "Content read back from temp file should match what was written"
        );
    }
}
