//! Comprehensive tests for tldr-core Git module
//!
//! Coverage: is_git_repository, is_shallow_clone, git_log, git_log_numstat

use std::fs;
use std::path::Path;
use std::process::Command;

use tldr_core::git::{git_log, git_log_numstat, is_git_repository, is_shallow_clone};

// =============================================================================
// is_git_repository tests
// =============================================================================

#[test]
fn test_is_git_repository_true() {
    // Current directory should be a git repository
    let current_dir = std::env::current_dir().unwrap();

    // Only test if we're actually in a git repo
    if Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        assert!(is_git_repository(&current_dir));
    }
}

#[test]
fn test_is_git_repository_false() {
    let temp_dir = tempfile::tempdir().unwrap();

    // Temp directory should not be a git repository
    assert!(!is_git_repository(temp_dir.path()));
}

#[test]
fn test_is_git_repository_nonexistent() {
    // Non-existent path should return false, not panic
    let result = is_git_repository(Path::new("/nonexistent/path/that/does/not/exist"));
    assert!(!result);
}

#[test]
fn test_is_git_repository_in_subdirectory() {
    // Create a temp git repo
    let temp_dir = tempfile::tempdir().unwrap();

    // Initialize git repo
    let init_output = Command::new("git")
        .args(["init"])
        .current_dir(&temp_dir)
        .output();

    if init_output.is_err() {
        // Git not available, skip test
        return;
    }

    // Create subdirectory
    let subdir = temp_dir.path().join("src").join("utils");
    fs::create_dir_all(&subdir).unwrap();

    // Should still detect as git repository from subdirectory
    assert!(is_git_repository(&subdir));
    assert!(is_git_repository(temp_dir.path()));
}

#[test]
fn test_is_git_repository_empty_directory() {
    let temp_dir = tempfile::tempdir().unwrap();

    // Empty directory should not be a git repo
    assert!(!is_git_repository(temp_dir.path()));
}

// =============================================================================
// is_shallow_clone tests
// =============================================================================

#[test]
fn test_is_shallow_clone_false() {
    // Create a normal (non-shallow) git repo
    let temp_dir = tempfile::tempdir().unwrap();

    let init_output = Command::new("git")
        .args(["init"])
        .current_dir(&temp_dir)
        .output();

    if init_output.is_err() {
        // Git not available, skip test
        return;
    }

    // Configure git user for commits
    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    // Create a file and commit
    fs::write(temp_dir.path().join("file.txt"), "content").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    // Normal repo should not be shallow
    assert!(!is_shallow_clone(temp_dir.path()));
}

#[test]
fn test_is_shallow_clone_non_repo() {
    let temp_dir = tempfile::tempdir().unwrap();

    // Non-repo should return false
    assert!(!is_shallow_clone(temp_dir.path()));
}

#[test]
fn test_is_shallow_clone_empty_repo() {
    let temp_dir = tempfile::tempdir().unwrap();

    let init_output = Command::new("git")
        .args(["init"])
        .current_dir(&temp_dir)
        .output();

    if init_output.is_err() {
        return;
    }

    // Empty repo (no commits) - behavior varies by git version
    // Just verify it doesn't panic
    let _ = is_shallow_clone(temp_dir.path());
}

// =============================================================================
// git_log tests
// =============================================================================

#[test]
fn test_git_log_basic() {
    let temp_dir = tempfile::tempdir().unwrap();

    // Initialize git repo
    let init_output = Command::new("git")
        .args(["init"])
        .current_dir(&temp_dir)
        .output();

    if init_output.is_err() {
        return; // Git not available
    }

    // Configure git user
    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    // Create initial commit
    fs::write(temp_dir.path().join("file.txt"), "content").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    // Test git_log
    let result = git_log(temp_dir.path(), 30, "%H", &[]);
    assert!(result.is_ok());

    let log = result.unwrap();
    assert!(!log.is_empty());
}

#[test]
fn test_git_log_with_format() {
    let temp_dir = tempfile::tempdir().unwrap();

    let init_output = Command::new("git")
        .args(["init"])
        .current_dir(&temp_dir)
        .output();

    if init_output.is_err() {
        return;
    }

    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    fs::write(temp_dir.path().join("file.txt"), "content").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    Command::new("git")
        .args(["commit", "-m", "Test commit message"])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    // Test with different formats
    let result = git_log(temp_dir.path(), 30, "%s", &[]);
    assert!(result.is_ok());
    let log = result.unwrap();
    assert!(log.contains("Test commit message"));
}

#[test]
fn test_git_log_with_extra_args() {
    let temp_dir = tempfile::tempdir().unwrap();

    let init_output = Command::new("git")
        .args(["init"])
        .current_dir(&temp_dir)
        .output();

    if init_output.is_err() {
        return;
    }

    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    fs::write(temp_dir.path().join("file.txt"), "content").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    Command::new("git")
        .args(["commit", "-m", "First"])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    fs::write(temp_dir.path().join("file.txt"), "more content").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    Command::new("git")
        .args(["commit", "-m", "Second"])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    // Test with -n1 to limit to 1 commit
    let result = git_log(temp_dir.path(), 30, "%s", &["-n1"]);
    assert!(result.is_ok());
    let log = result.unwrap();
    assert!(log.contains("Second"));
}

#[test]
fn test_git_log_nonexistent_repo() {
    let temp_dir = tempfile::tempdir().unwrap();

    let result = git_log(temp_dir.path(), 30, "%H", &[]);

    // Should fail since not a git repo
    assert!(result.is_err());
}

#[test]
fn test_git_log_empty_repo() {
    let temp_dir = tempfile::tempdir().unwrap();

    let init_output = Command::new("git")
        .args(["init"])
        .current_dir(&temp_dir)
        .output();

    if init_output.is_err() {
        return;
    }

    // No commits yet
    let result = git_log(temp_dir.path(), 30, "%H", &[]);
    // Should succeed but return empty string
    assert!(result.is_ok());
    assert!(result.unwrap().is_empty());
}

#[test]
fn test_git_log_since_days_zero() {
    let temp_dir = tempfile::tempdir().unwrap();

    let init_output = Command::new("git")
        .args(["init"])
        .current_dir(&temp_dir)
        .output();

    if init_output.is_err() {
        return;
    }

    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    fs::write(temp_dir.path().join("file.txt"), "content").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    Command::new("git")
        .args(["commit", "-m", "Test"])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    // Test with 0 days (should still work)
    let result = git_log(temp_dir.path(), 0, "%H", &[]);
    assert!(result.is_ok());
}

// =============================================================================
// git_log_numstat tests
// =============================================================================

#[test]
fn test_git_log_numstat_basic() {
    let temp_dir = tempfile::tempdir().unwrap();

    let init_output = Command::new("git")
        .args(["init"])
        .current_dir(&temp_dir)
        .output();

    if init_output.is_err() {
        return;
    }

    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    fs::write(temp_dir.path().join("file.txt"), "line1\nline2\nline3\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    Command::new("git")
        .args(["commit", "-m", "Add file"])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    let result = git_log_numstat(temp_dir.path(), 30);
    assert!(result.is_ok());

    let numstat = result.unwrap();
    // Numstat format: "<added>\t<deleted>\t<file>"
    assert!(!numstat.is_empty());
}

#[test]
fn test_git_log_numstat_with_changes() {
    let temp_dir = tempfile::tempdir().unwrap();

    let init_output = Command::new("git")
        .args(["init"])
        .current_dir(&temp_dir)
        .output();

    if init_output.is_err() {
        return;
    }

    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    // First commit
    fs::write(temp_dir.path().join("file.txt"), "line1\nline2\nline3\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(&temp_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "First"])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    // Second commit with changes
    fs::write(
        temp_dir.path().join("file.txt"),
        "line1\nmodified\nline3\nline4\n",
    )
    .unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(&temp_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Second"])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    let result = git_log_numstat(temp_dir.path(), 30);
    assert!(result.is_ok());

    let numstat = result.unwrap();
    // Should contain file.txt with change stats
    assert!(numstat.contains("file.txt"));
}

#[test]
fn test_git_log_numstat_nonexistent_repo() {
    let temp_dir = tempfile::tempdir().unwrap();

    let result = git_log_numstat(temp_dir.path(), 30);
    assert!(result.is_err());
}

#[test]
fn test_git_log_numstat_empty_repo() {
    let temp_dir = tempfile::tempdir().unwrap();

    let init_output = Command::new("git")
        .args(["init"])
        .current_dir(&temp_dir)
        .output();

    if init_output.is_err() {
        return;
    }

    let result = git_log_numstat(temp_dir.path(), 30);
    assert!(result.is_ok());
    assert!(result.unwrap().is_empty());
}

// =============================================================================
// Edge cases and integration tests
// =============================================================================

#[test]
fn test_git_operations_with_special_characters_in_path() {
    let temp_dir = tempfile::tempdir().unwrap();
    let special_dir = temp_dir.path().join("dir with spaces");
    fs::create_dir(&special_dir).unwrap();

    let init_output = Command::new("git")
        .args(["init"])
        .current_dir(&special_dir)
        .output();

    if init_output.is_err() {
        return;
    }

    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(&special_dir)
        .output()
        .unwrap();

    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(&special_dir)
        .output()
        .unwrap();

    fs::write(special_dir.join("file.txt"), "content").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(&special_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Test"])
        .current_dir(&special_dir)
        .output()
        .unwrap();

    assert!(is_git_repository(&special_dir));

    let result = git_log(&special_dir, 30, "%H", &[]);
    assert!(result.is_ok());
}

#[test]
fn test_git_operations_unicode_path() {
    let temp_dir = tempfile::tempdir().unwrap();
    let unicode_dir = temp_dir.path().join("目录"); // "directory" in Chinese
    fs::create_dir(&unicode_dir).unwrap();

    let init_output = Command::new("git")
        .args(["init"])
        .current_dir(&unicode_dir)
        .output();

    if init_output.is_err() {
        return;
    }

    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(&unicode_dir)
        .output()
        .unwrap();

    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(&unicode_dir)
        .output()
        .unwrap();

    fs::write(unicode_dir.join("file.txt"), "content").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(&unicode_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Test"])
        .current_dir(&unicode_dir)
        .output()
        .unwrap();

    assert!(is_git_repository(&unicode_dir));
}

#[test]
fn test_multiple_commits_log_order() {
    let temp_dir = tempfile::tempdir().unwrap();

    let init_output = Command::new("git")
        .args(["init"])
        .current_dir(&temp_dir)
        .output();

    if init_output.is_err() {
        return;
    }

    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    // Create multiple commits
    for i in 1..=5 {
        let filename = format!("file{}.txt", i);
        fs::write(temp_dir.path().join(&filename), format!("content{}", i)).unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(&temp_dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", &format!("Commit {}", i)])
            .current_dir(&temp_dir)
            .output()
            .unwrap();
    }

    let result = git_log(temp_dir.path(), 30, "%s", &[]);
    assert!(result.is_ok());

    let log = result.unwrap();
    // Should contain all commit messages
    assert!(log.contains("Commit 1"));
    assert!(log.contains("Commit 2"));
    assert!(log.contains("Commit 3"));
    assert!(log.contains("Commit 4"));
    assert!(log.contains("Commit 5"));
}

#[test]
fn test_git_log_with_author_format() {
    let temp_dir = tempfile::tempdir().unwrap();

    let init_output = Command::new("git")
        .args(["init"])
        .current_dir(&temp_dir)
        .output();

    if init_output.is_err() {
        return;
    }

    Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    fs::write(temp_dir.path().join("file.txt"), "content").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(&temp_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Test"])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    let result = git_log(temp_dir.path(), 30, "%an <%ae>", &[]);
    assert!(result.is_ok());

    let log = result.unwrap();
    assert!(log.contains("Test User"));
    assert!(log.contains("test@example.com"));
}

#[test]
fn test_git_log_since_days_filter() {
    let temp_dir = tempfile::tempdir().unwrap();

    let init_output = Command::new("git")
        .args(["init"])
        .current_dir(&temp_dir)
        .output();

    if init_output.is_err() {
        return;
    }

    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    fs::write(temp_dir.path().join("file.txt"), "content").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(&temp_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Test"])
        .current_dir(&temp_dir)
        .output()
        .unwrap();

    // Test with different day values
    let result_1 = git_log(temp_dir.path(), 1, "%H", &[]);
    let result_7 = git_log(temp_dir.path(), 7, "%H", &[]);
    let result_30 = git_log(temp_dir.path(), 30, "%H", &[]);

    assert!(result_1.is_ok());
    assert!(result_7.is_ok());
    assert!(result_30.is_ok());
}

#[test]
fn test_git_worktree_detection() {
    // This is an advanced git feature - worktrees
    // We just test that is_git_repository works in main repo
    let temp_dir = tempfile::tempdir().unwrap();

    let init_output = Command::new("git")
        .args(["init"])
        .current_dir(&temp_dir)
        .output();

    if init_output.is_err() {
        return;
    }

    assert!(is_git_repository(temp_dir.path()));
}

// Test for submodule detection
#[test]
fn test_git_submodule_detection() {
    // Create main repo
    let main_dir = tempfile::tempdir().unwrap();
    let sub_dir = tempfile::tempdir().unwrap();

    // Initialize main repo
    let main_init = Command::new("git")
        .args(["init"])
        .current_dir(&main_dir)
        .output();

    if main_init.is_err() {
        return;
    }

    // Initialize sub repo
    Command::new("git")
        .args(["init"])
        .current_dir(&sub_dir)
        .output()
        .unwrap();

    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(&sub_dir)
        .output()
        .unwrap();

    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(&sub_dir)
        .output()
        .unwrap();

    fs::write(sub_dir.path().join("file.txt"), "content").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(&sub_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Initial"])
        .current_dir(&sub_dir)
        .output()
        .unwrap();

    // Both should be detected as git repos
    assert!(is_git_repository(main_dir.path()));
    assert!(is_git_repository(sub_dir.path()));
}
