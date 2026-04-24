//! Git operations module
//!
//! Provides git-related utilities for churn analysis and other git-based commands.

use std::path::Path;
use std::process::Command;

use crate::error::TldrError;
use crate::TldrResult;

/// Check if a path is inside a git repository
pub fn is_git_repository(path: &Path) -> bool {
    Command::new("git")
        .arg("rev-parse")
        .arg("--git-dir")
        .current_dir(path)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Check if repository is a shallow clone
pub fn is_shallow_clone(path: &Path) -> bool {
    Command::new("git")
        .arg("rev-parse")
        .arg("--is-shallow-repository")
        .current_dir(path)
        .output()
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "true")
        .unwrap_or(false)
}

/// Get git log output with specified format
pub fn git_log(
    path: &Path,
    since_days: u32,
    format: &str,
    extra_args: &[&str],
) -> TldrResult<String> {
    let since = format!("--since={} days ago", since_days);
    let format_arg = format!("--pretty=format:{}", format);

    let mut cmd = Command::new("git");
    cmd.arg("log")
        .arg(&since)
        .arg(&format_arg)
        .current_dir(path);

    for arg in extra_args {
        cmd.arg(arg);
    }

    let output = cmd
        .output()
        .map_err(|e| TldrError::GitError(format!("Failed to run git log: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TldrError::GitError(format!("git log failed: {}", stderr)));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Get git log with numstat (lines added/deleted per file)
pub fn git_log_numstat(path: &Path, since_days: u32) -> TldrResult<String> {
    let since = format!("--since={} days ago", since_days);

    let output = Command::new("git")
        .arg("log")
        .arg(&since)
        .arg("--numstat")
        .arg("--format=")
        .current_dir(path)
        .output()
        .map_err(|e| TldrError::GitError(format!("Failed to run git log: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TldrError::GitError(format!(
            "git log --numstat failed: {}",
            stderr
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
