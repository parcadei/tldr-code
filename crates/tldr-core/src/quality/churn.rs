//! Code churn analysis from git history
//!
//! This module analyzes git history to identify high-churn files (frequently changed code).
//! Combined with complexity analysis, it creates a "hotspot matrix" for prioritizing
//! refactoring efforts.
//!
//! # Implementation Status
//! - Phase 1: Helper functions (complete)
//! - Phase 2: Git command infrastructure (complete)
//! - Phase 3: File churn analysis (complete)
//! - Phase 4: Author statistics (complete)
//! - Phase 5: Hotspot analysis (stub)

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use glob::Pattern;
use serde::{Deserialize, Serialize};
use thiserror::Error;

// Default timeout for git commands: 5 minutes (PM-3)
const GIT_TIMEOUT_SECS: u64 = 300;

// =============================================================================
// Data Types (from spec section 2)
// =============================================================================

/// Churn metrics for a single file within the analysis window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileChurn {
    /// File path relative to repository root
    pub file: String,

    /// Number of commits touching this file in the time window
    pub commit_count: u32,

    /// Total lines added across all commits
    pub lines_added: u32,

    /// Total lines deleted across all commits
    pub lines_deleted: u32,

    /// Sum of lines_added + lines_deleted
    pub lines_changed: u32,

    /// ISO date (YYYY-MM-DD) of earliest commit touching this file
    /// None if no commits found (shouldn't happen in practice)
    pub first_commit: Option<String>,

    /// ISO date (YYYY-MM-DD) of most recent commit touching this file
    pub last_commit: Option<String>,

    /// Email addresses of all authors who modified this file
    pub authors: Vec<String>,

    /// Count of unique authors (equals authors.len())
    pub author_count: u32,
}

/// Statistics for a single author within the analysis window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuthorStats {
    /// Author display name from git config
    pub name: String,

    /// Author email address (canonical identifier)
    pub email: String,

    /// Total commits by this author
    pub commits: u32,

    /// Total lines added by this author
    pub lines_added: u32,

    /// Total lines deleted by this author
    pub lines_deleted: u32,

    /// Number of unique files this author modified
    pub files_touched: u32,
}

/// Hotspot combining churn frequency with cyclomatic complexity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hotspot {
    /// File path relative to repository root
    pub file: String,

    /// Rank by commit count (1 = most commits)
    pub churn_rank: u32,

    /// Rank by cyclomatic complexity (1 = highest complexity)
    pub complexity_rank: u32,

    /// Normalized combined score: (churn/max_churn) * (complexity/max_complexity)
    /// Range: 0.0 to 1.0
    pub combined_score: f64,

    /// Number of commits (same as FileChurn.commit_count)
    pub commit_count: u32,

    /// Maximum cyclomatic complexity of any function in file
    pub cyclomatic_complexity: u32,

    /// Action recommendation based on combined_score thresholds
    pub recommendation: String,
}

// =============================================================================
// Bot Detection (Phase 2: Hotspot Upgrade)
// =============================================================================

/// Default bot author patterns (case-insensitive substring match).
/// Used to filter automated commits from churn analysis.
const BOT_PATTERNS: &[&str] = &[
    "dependabot",
    "renovate",
    "github-actions",
    "[bot]",
    "snyk-bot",
    "greenkeeper",
    "depfu",
    "codecov",
    "semantic-release-bot",
];

/// Check if an author name/email matches known bot patterns.
///
/// Case-insensitive substring matching against both name and email.
/// Returns true if the author is likely a bot.
///
/// # Examples
/// ```
/// use tldr_core::quality::churn::is_bot_author;
/// assert!(is_bot_author("dependabot[bot]", "dependabot@users.noreply.github.com"));
/// assert!(!is_bot_author("John Smith", "john@company.com"));
/// ```
pub fn is_bot_author(author_name: &str, author_email: &str) -> bool {
    let name_lower = author_name.to_lowercase();
    let email_lower = author_email.to_lowercase();
    BOT_PATTERNS
        .iter()
        .any(|p| name_lower.contains(p) || email_lower.contains(p))
}

// =============================================================================
// Detailed Churn Types (Phase 2: Hotspot Upgrade)
// =============================================================================

/// Per-commit churn data for a single file.
/// Used internally for recency weighting and bot filtering.
#[derive(Debug, Clone)]
pub(crate) struct CommitChurnEntry {
    /// Commit date as full ISO 8601 string (e.g., "2026-01-15T10:30:00+00:00")
    pub date: String,
    /// Lines added in this commit for this file
    pub lines_added: u32,
    /// Lines deleted in this commit for this file
    pub lines_deleted: u32,
    /// Author email (mailmap-aware via %aE)
    pub author_email: String,
}

/// Extended file churn data with per-commit details.
/// Internal to hotspot analysis -- not exposed in public API.
#[derive(Debug, Clone)]
pub(crate) struct FileChurnDetailed {
    /// Standard aggregated churn data (for backwards compatibility)
    pub base: FileChurn,
    /// Per-commit details for recency weighting
    pub commits: Vec<CommitChurnEntry>,
}

/// Summary statistics for the churn analysis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChurnSummary {
    /// Count of files with at least one commit in window
    pub total_files: u32,

    /// Sum of commit_count across all files
    /// Note: A single commit touching 3 files counts as 3 here
    pub total_commits: u32,

    /// Analysis period in days (from --days argument)
    pub time_window_days: u32,

    /// Sum of lines_changed across all files
    pub total_lines_changed: u64,

    /// total_commits / total_files (0.0 if total_files == 0)
    pub avg_commits_per_file: f64,

    /// File path with highest commit_count
    pub most_churned_file: String,
}

/// Complete churn analysis report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChurnReport {
    /// Top K files sorted by commit_count descending
    pub files: Vec<FileChurn>,

    /// Hotspot analysis (empty unless --hotspots flag)
    pub hotspots: Vec<Hotspot>,

    /// Author statistics (empty unless --authors flag)
    pub authors: Vec<AuthorStats>,

    /// Aggregate statistics
    pub summary: ChurnSummary,

    /// True if repository is a shallow clone
    pub is_shallow: bool,

    /// Approximate depth if shallow (commit count)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shallow_depth: Option<u32>,

    /// Warning messages (e.g., shallow clone warning)
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub warnings: Vec<String>,
}

// =============================================================================
// Error Types (from spec section 9)
// =============================================================================

/// Errors specific to churn analysis
#[derive(Debug, Error)]
pub enum ChurnError {
    /// Path does not exist
    #[error("Path not found: {0}")]
    PathNotFound(PathBuf),

    /// Path is not inside a git repository
    #[error("Not a git repository: {path}")]
    NotGitRepository {
        /// Path that was expected to be inside a repository.
        path: PathBuf,
    },

    /// Git command failed
    #[error("Git command failed: {command}\n{stderr}")]
    GitError {
        /// Full git command string used for execution.
        command: String,
        /// Stderr emitted by the failed command.
        stderr: String,
        /// Exit code if the process terminated normally.
        exit_code: Option<i32>,
    },

    /// Failed to parse git output
    #[error("Failed to parse git output: {context}\nLine: {line}")]
    ParseError {
        /// Context describing what parser step failed.
        context: String,
        /// Raw line that failed to parse.
        line: String,
    },

    /// I/O error (file access, etc.)
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Complexity analysis failed (non-fatal, logged)
    #[error("Complexity analysis failed for {file}: {reason}")]
    ComplexityError {
        /// File whose complexity analysis failed.
        file: PathBuf,
        /// Human-readable reason for the failure.
        reason: String,
    },
}

// =============================================================================
// Internal Git Command Infrastructure (Phase 2)
// =============================================================================

/// Execute a git command with timeout and proper security mitigations.
///
/// # Mitigations Applied
/// - PM-1: Uses Command::arg() for each argument (no shell injection)
/// - PM-3: Adds timeout (default 5 minutes)
/// - PM-4: Adds `-c core.quotepath=false` for Unicode paths
/// - PM-5: Canonicalizes the working directory path
///
/// # Arguments
/// * `args` - Git command arguments (without "git" itself)
/// * `cwd` - Working directory for the command
///
/// # Returns
/// * `Ok(stdout)` - Command succeeded, stdout as String (trimmed)
/// * `Err(ChurnError::GitError)` - Non-zero exit or execution failed
fn run_git(args: &[&str], cwd: &Path) -> Result<String, ChurnError> {
    // PM-5: Canonicalize path before use
    let canonical_cwd = cwd.canonicalize().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            ChurnError::PathNotFound(cwd.to_path_buf())
        } else {
            ChurnError::Io(e)
        }
    })?;

    // Build command with security mitigations
    let mut cmd = Command::new("git");

    // PM-4: Add core.quotepath=false for Unicode path support
    cmd.arg("-c").arg("core.quotepath=false");

    // PM-1: Add each argument individually (no shell interpolation)
    for arg in args {
        cmd.arg(arg);
    }

    // Set working directory to canonicalized path
    cmd.current_dir(&canonical_cwd);

    // PM-3: Execute with timeout using wait_timeout approach
    // We spawn the process and use a thread to enforce timeout
    let output = {
        let child = cmd
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| ChurnError::GitError {
                command: format!("git {}", args.join(" ")),
                stderr: format!("Failed to spawn git: {}", e),
                exit_code: None,
            })?;

        // Use wait_with_output which handles the child process
        // For timeout, we use a simple approach with threads
        let timeout = Duration::from_secs(GIT_TIMEOUT_SECS);

        // Spawn a thread to wait for the child
        let (tx, rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            let result = child.wait_with_output();
            let _ = tx.send(result);
        });

        // Wait with timeout
        match rx.recv_timeout(timeout) {
            Ok(result) => {
                let _ = handle.join();
                result.map_err(|e| ChurnError::GitError {
                    command: format!("git {}", args.join(" ")),
                    stderr: format!("Failed to wait for git: {}", e),
                    exit_code: None,
                })?
            }
            Err(_) => {
                // Timeout - the thread will eventually complete but we don't wait
                return Err(ChurnError::GitError {
                    command: format!("git {}", args.join(" ")),
                    stderr: format!("Git command timed out after {} seconds", GIT_TIMEOUT_SECS),
                    exit_code: None,
                });
            }
        }
    };

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(ChurnError::GitError {
            command: format!("git {}", args.join(" ")),
            stderr,
            exit_code: output.status.code(),
        })
    }
}

// =============================================================================
// Core Analysis Functions
// =============================================================================

/// Check if a path is inside a git repository.
///
/// # Arguments
/// * `path` - Directory to check (should exist)
///
/// # Returns
/// * `Ok(true)` - Path is inside a git repository
/// * `Ok(false)` - Path exists but is not in a git repo
/// * `Err(_)` - Path doesn't exist or other I/O error
///
/// # Git Command
/// ```bash
/// git rev-parse --git-dir
/// ```
pub fn is_git_repository(path: &Path) -> Result<bool, ChurnError> {
    // Check if path exists first
    if !path.exists() {
        return Err(ChurnError::PathNotFound(path.to_path_buf()));
    }

    // PM-5: Canonicalize path (done inside run_git)
    // Run git rev-parse --git-dir
    match run_git(&["rev-parse", "--git-dir"], path) {
        Ok(_) => Ok(true),
        Err(ChurnError::GitError {
            exit_code: Some(_), ..
        }) => {
            // Non-zero exit means not a git repo
            Ok(false)
        }
        Err(ChurnError::GitError {
            exit_code: None,
            stderr,
            ..
        }) => {
            // If stderr indicates "not a git repository", return false
            if stderr.contains("not a git repository") {
                Ok(false)
            } else {
                // Other git error - propagate
                Err(ChurnError::GitError {
                    command: "git rev-parse --git-dir".to_string(),
                    stderr,
                    exit_code: None,
                })
            }
        }
        Err(e) => Err(e),
    }
}

/// Check if repository is a shallow clone.
///
/// # Arguments
/// * `path` - Repository root directory
///
/// # Returns
/// Tuple of (is_shallow, depth_if_shallow)
/// - `(false, None)` - Full clone with complete history
/// - `(true, Some(N))` - Shallow clone with ~N commits
/// - `(true, None)` - Shallow clone but depth unknown
///
/// # Detection Methods
/// 1. Run `git rev-parse --is-shallow-repository`
/// 2. If shallow, estimate depth with `git rev-list --count HEAD`
pub fn check_shallow_clone(path: &Path) -> Result<(bool, Option<u32>), ChurnError> {
    // Check if path exists first
    if !path.exists() {
        return Err(ChurnError::PathNotFound(path.to_path_buf()));
    }

    // PM-5: Canonicalize path
    let canonical_path = path.canonicalize()?;

    // Method 1: Use git rev-parse --is-shallow-repository (git 2.15+)
    let is_shallow = match run_git(&["rev-parse", "--is-shallow-repository"], &canonical_path) {
        Ok(output) => output.trim() == "true",
        Err(_) => {
            // Fallback: Check for .git/shallow file
            let shallow_file = canonical_path.join(".git").join("shallow");
            shallow_file.exists()
        }
    };

    if !is_shallow {
        return Ok((false, None));
    }

    // Get depth estimate using rev-list --count HEAD
    let depth = match run_git(&["rev-list", "--count", "HEAD"], &canonical_path) {
        Ok(output) => output.trim().parse::<u32>().ok(),
        Err(_) => None,
    };

    Ok((true, depth))
}

/// Parse git log to get churn metrics per file.
///
/// # Arguments
/// * `path` - Repository root directory
/// * `days` - Time window in days (uses git's relative date)
/// * `exclude_patterns` - Glob patterns to exclude from results
///
/// # Returns
/// HashMap mapping file path -> FileChurn
///
/// # Git Commands
///
/// ## Command 1: Commit metadata and files
/// ```bash
/// git log --since="N days ago" \
///         --pretty=format:"COMMIT:%H\x1e%aI\x1e%ae\x1e%an" \
///         --name-only
/// ```
///
/// ## Command 2: Line statistics
/// ```bash
/// git log --since="N days ago" --numstat --format=""
/// ```
pub fn get_file_churn(
    path: &Path,
    days: u32,
    exclude_patterns: &[String],
) -> Result<HashMap<String, FileChurn>, ChurnError> {
    // Check path exists
    if !path.exists() {
        return Err(ChurnError::PathNotFound(path.to_path_buf()));
    }

    // Verify it's a git repository
    if !is_git_repository(path)? {
        return Err(ChurnError::NotGitRepository {
            path: path.to_path_buf(),
        });
    }

    // Intermediate structure to accumulate per-file data
    struct FileData {
        commit_count: u32,
        lines_added: u32,
        lines_deleted: u32,
        first_commit: Option<String>,               // YYYY-MM-DD
        last_commit: Option<String>,                // YYYY-MM-DD
        authors: std::collections::HashSet<String>, // unique emails
    }

    let mut file_data: HashMap<String, FileData> = HashMap::new();

    // Build the --since argument
    let since_arg = format!("{} days ago", days);

    // Command 1: Get commit metadata and file lists
    // Format: COMMIT:<hash>\x1e<date>\x1e<email>\x1e<name>
    // Followed by file names until empty line
    let format_arg = "COMMIT:%H\x1e%aI\x1e%ae\x1e%an";
    let commit_output = match run_git(
        &[
            "log",
            &format!("--since={}", since_arg),
            &format!("--pretty=format:{}", format_arg),
            "--name-only",
        ],
        path,
    ) {
        Ok(output) => output,
        Err(ChurnError::GitError { stderr, .. }) => {
            // Handle empty repository (no commits yet) - return empty result
            if stderr.contains("does not have any commits") || stderr.contains("bad revision") {
                return Ok(HashMap::new());
            }
            // Re-return other git errors
            return Err(ChurnError::GitError {
                command: format!("git log --since=\"{}\" ...", since_arg),
                stderr,
                exit_code: None,
            });
        }
        Err(e) => return Err(e),
    };

    // Parse commit output
    // Structure:
    // COMMIT:<hash>\x1e<date>\x1e<email>\x1e<author>
    // file1.rs
    // file2.rs
    //
    // COMMIT:...
    let mut current_date: Option<String> = None;
    let mut current_email: Option<String> = None;

    for line in commit_output.lines() {
        let line = line.trim();

        if line.is_empty() {
            // Empty line separates commits
            current_date = None;
            current_email = None;
            continue;
        }

        if let Some(rest) = line.strip_prefix("COMMIT:") {
            // Parse commit line: COMMIT:<hash>\x1e<date>\x1e<email>\x1e<author>
            let parts: Vec<&str> = rest.split('\x1e').collect();

            if parts.len() >= 3 {
                // parts[0] = hash (we don't need it for churn)
                // parts[1] = ISO date like 2026-01-15T10:30:00+00:00
                // parts[2] = email
                // parts[3] = author name (optional, we use email as identifier)

                // Extract YYYY-MM-DD from ISO date
                let date_str = parts[1];
                current_date = if date_str.len() >= 10 {
                    Some(date_str[..10].to_string())
                } else {
                    None
                };
                current_email = Some(parts[2].to_string());
            }
        } else {
            // This is a file path line
            let file_path = line;

            // Skip if matches exclude pattern
            if matches_exclude_pattern(file_path, exclude_patterns) {
                continue;
            }

            // Get or create file data
            let data = file_data
                .entry(file_path.to_string())
                .or_insert_with(|| FileData {
                    commit_count: 0,
                    lines_added: 0,
                    lines_deleted: 0,
                    first_commit: None,
                    last_commit: None,
                    authors: std::collections::HashSet::new(),
                });

            // Increment commit count
            data.commit_count += 1;

            // Update date range (git log outputs newest first)
            if let Some(ref date) = current_date {
                // last_commit = first date we see (newest)
                if data.last_commit.is_none() {
                    data.last_commit = Some(date.clone());
                }
                // first_commit = always update (will end up being oldest)
                data.first_commit = Some(date.clone());
            }

            // Add author
            if let Some(ref email) = current_email {
                data.authors.insert(email.clone());
            }
        }
    }

    // Command 2: Get line statistics
    // Format: <added>\t<deleted>\t<filepath>
    // Binary files show: -\t-\t<filepath>
    let numstat_output = run_git(
        &[
            "log",
            &format!("--since={}", since_arg),
            "--numstat",
            "--format=",
        ],
        path,
    )?;

    // Parse numstat output
    for line in numstat_output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Split by tab
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 3 {
            continue;
        }

        let added_str = parts[0];
        let deleted_str = parts[1];
        let file_path = parts[2];

        // Skip if matches exclude pattern
        if matches_exclude_pattern(file_path, exclude_patterns) {
            continue;
        }

        // Skip binary files (shown as "-")
        if added_str == "-" || deleted_str == "-" {
            continue;
        }

        // Parse line counts
        let added: u32 = added_str.parse().unwrap_or(0);
        let deleted: u32 = deleted_str.parse().unwrap_or(0);

        // Update file data (may not exist if file was filtered from commit list)
        if let Some(data) = file_data.get_mut(file_path) {
            data.lines_added += added;
            data.lines_deleted += deleted;
        }
    }

    // Convert to FileChurn HashMap
    let result: HashMap<String, FileChurn> = file_data
        .into_iter()
        .map(|(file, data)| {
            let authors: Vec<String> = data.authors.into_iter().collect();
            let author_count = authors.len() as u32;

            let churn = FileChurn {
                file: file.clone(),
                commit_count: data.commit_count,
                lines_added: data.lines_added,
                lines_deleted: data.lines_deleted,
                lines_changed: data.lines_added + data.lines_deleted,
                first_commit: data.first_commit,
                last_commit: data.last_commit,
                authors,
                author_count,
            };
            (file, churn)
        })
        .collect();

    Ok(result)
}

// =============================================================================
// Detailed Churn Analysis (Phase 2: Hotspot Upgrade)
// =============================================================================

/// Get detailed file churn data with per-commit breakdowns.
///
/// Used by hotspot analysis for recency weighting and bot filtering.
/// Returns per-commit data for each file, enabling:
/// - Recency-weighted churn calculation
/// - Bot commit filtering
/// - Knowledge fragmentation computation
///
/// # Arguments
/// * `path` - Repository root directory (must be a git repository)
/// * `days` - Time window in days
/// * `exclude_patterns` - Glob patterns to exclude from results
/// * `include_bots` - If false, bot commits are filtered out
///
/// # Returns
/// Tuple of (file_data, total_bot_commits_filtered)
///
/// # Git Command
/// Uses a SINGLE git log command with streaming BufReader parsing (RISK-P1):
/// ```bash
/// git log --since="N days ago" \
///         --pretty=format:"COMMIT:%H\x1e%aI\x1e%aE\x1e%aN" \
///         --numstat --no-renames --no-merges
/// ```
///
/// # Mitigations
/// - RISK-C1: `--no-renames` prevents phantom entries from renamed files
/// - RISK-C2: `--no-merges` prevents bad numstat from merge commits
/// - RISK-C5: `splitn(3, '\t')` handles file paths containing tabs
/// - RISK-P1: Streaming BufReader instead of `wait_with_output()`
pub(crate) fn get_file_churn_detailed(
    path: &Path,
    days: u32,
    exclude_patterns: &[String],
    include_bots: bool,
) -> Result<(HashMap<String, FileChurnDetailed>, u32), ChurnError> {
    use std::io::BufRead;
    use std::process::Stdio;

    // Check path exists
    if !path.exists() {
        return Err(ChurnError::PathNotFound(path.to_path_buf()));
    }

    // Verify it's a git repository
    if !is_git_repository(path)? {
        return Err(ChurnError::NotGitRepository {
            path: path.to_path_buf(),
        });
    }

    // PM-5: Canonicalize path before use
    let canonical_path = path.canonicalize().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            ChurnError::PathNotFound(path.to_path_buf())
        } else {
            ChurnError::Io(e)
        }
    })?;

    // Build the git command
    let since_arg = format!("{} days ago", days);
    let format_arg = "COMMIT:%H\x1e%aI\x1e%aE\x1e%aN";

    let mut cmd = Command::new("git");
    cmd.arg("-c")
        .arg("core.quotepath=false")
        .arg("log")
        .arg(format!("--since={}", since_arg))
        .arg(format!("--pretty=format:{}", format_arg))
        .arg("--numstat")
        .arg("--no-renames") // RISK-C1: prevent phantom entries
        .arg("--no-merges") // RISK-C2: prevent bad numstat
        .current_dir(&canonical_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| ChurnError::GitError {
        command: "git log --numstat --no-renames --no-merges".to_string(),
        stderr: format!("Failed to spawn git: {}", e),
        exit_code: None,
    })?;

    // RISK-P1: Stream with BufReader, NOT wait_with_output()
    let stdout = child.stdout.take().ok_or_else(|| ChurnError::GitError {
        command: "git log".to_string(),
        stderr: "Failed to capture stdout".to_string(),
        exit_code: None,
    })?;
    let reader = std::io::BufReader::new(stdout);

    // Intermediate per-file accumulator
    struct FileAccum {
        commits: Vec<CommitChurnEntry>,
        authors: std::collections::HashSet<String>,
        first_commit_date: Option<String>,
        last_commit_date: Option<String>,
    }

    let mut file_data: HashMap<String, FileAccum> = HashMap::new();
    let mut total_bot_commits_filtered: u32 = 0;

    // Current commit state while parsing
    let mut current_date: Option<String> = None;
    let mut current_email: Option<String> = None;
    let mut skip_this_commit = false;

    for line_result in reader.lines() {
        let line = match line_result {
            Ok(l) => l,
            Err(_) => continue, // Skip lines that can't be read
        };

        let trimmed = line.trim();

        if trimmed.is_empty() {
            // Empty line separates commits; reset is handled by next COMMIT: line
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("COMMIT:") {
            // Parse commit header: COMMIT:<hash>\x1e<date>\x1e<email>\x1e<name>
            let parts: Vec<&str> = rest.split('\x1e').collect();

            if parts.len() >= 4 {
                // parts[0] is commit hash (not currently used)
                current_date = Some(parts[1].to_string());
                current_email = Some(parts[2].to_string());

                // Check if this is a bot commit
                let is_bot = is_bot_author(parts[3], parts[2]);
                if is_bot && !include_bots {
                    skip_this_commit = true;
                    total_bot_commits_filtered += 1;
                } else {
                    skip_this_commit = false;
                }
            } else {
                // Malformed commit line - skip
                current_date = None;
                current_email = None;
                skip_this_commit = true;
            }
            continue;
        }

        // If we're skipping this commit (bot), don't process numstat lines
        if skip_this_commit {
            continue;
        }

        // This should be a numstat line: <added>\t<deleted>\t<filepath>
        // RISK-C5: Use splitn(3, '\t') to handle file paths with tabs
        let parts: Vec<&str> = trimmed.splitn(3, '\t').collect();
        if parts.len() < 3 {
            continue; // Not a valid numstat line
        }

        let added_str = parts[0];
        let deleted_str = parts[1];
        let file_path = parts[2];

        // Skip if matches exclude pattern
        if matches_exclude_pattern(file_path, exclude_patterns) {
            continue;
        }

        // Handle binary files: numstat shows "-" for both added and deleted
        // RISK-C4: Record binary files with 0 line counts
        let lines_added: u32 = if added_str == "-" {
            0
        } else {
            added_str.parse().unwrap_or(0)
        };
        let lines_deleted: u32 = if deleted_str == "-" {
            0
        } else {
            deleted_str.parse().unwrap_or(0)
        };

        // Create commit entry for this file
        let commit_entry = CommitChurnEntry {
            date: current_date.clone().unwrap_or_default(),
            lines_added,
            lines_deleted,
            author_email: current_email.clone().unwrap_or_default(),
        };

        // Get or create file accumulator
        let accum = file_data
            .entry(file_path.to_string())
            .or_insert_with(|| FileAccum {
                commits: Vec::new(),
                authors: std::collections::HashSet::new(),
                first_commit_date: None,
                last_commit_date: None,
            });

        // Track author
        if let Some(ref email) = current_email {
            accum.authors.insert(email.clone());
        }

        // Track date range (git log outputs newest first)
        if let Some(ref date) = current_date {
            let date_short = if date.len() >= 10 { &date[..10] } else { date };
            if accum.last_commit_date.is_none() {
                accum.last_commit_date = Some(date_short.to_string());
            }
            // Always update first_commit (will end up being oldest)
            accum.first_commit_date = Some(date_short.to_string());
        }

        accum.commits.push(commit_entry);
    }

    // Wait for child process to finish
    let status = child.wait().map_err(|e| ChurnError::GitError {
        command: "git log".to_string(),
        stderr: format!("Failed to wait for git process: {}", e),
        exit_code: None,
    })?;

    // We don't fail on non-zero exit here -- empty repos produce exit code 0
    // but repos with no commits in the window may produce warnings on stderr.
    // The important thing is we got whatever output was available.
    if !status.success() {
        // Check if this is a "no commits" situation (empty result is OK)
        if file_data.is_empty() {
            return Ok((HashMap::new(), total_bot_commits_filtered));
        }
    }

    // Aggregate per-commit data into FileChurnDetailed
    let mut result: HashMap<String, FileChurnDetailed> = HashMap::new();

    // Also count per-file bot commits (for commits that were filtered globally
    // but we need per-file counts). Since we filtered at the commit level,
    // we can't easily attribute bot commits to specific files.
    // The total_bot_commits_filtered is the global count.

    for (file_path, accum) in file_data {
        // Aggregate line counts from all commits
        let total_added: u32 = accum.commits.iter().map(|c| c.lines_added).sum();
        let total_deleted: u32 = accum.commits.iter().map(|c| c.lines_deleted).sum();
        let commit_count = accum.commits.len() as u32;

        let authors: Vec<String> = accum.authors.into_iter().collect();
        let author_count = authors.len() as u32;

        let base = FileChurn {
            file: file_path.clone(),
            commit_count,
            lines_added: total_added,
            lines_deleted: total_deleted,
            lines_changed: total_added + total_deleted,
            first_commit: accum.first_commit_date,
            last_commit: accum.last_commit_date,
            authors,
            author_count,
        };

        result.insert(
            file_path,
            FileChurnDetailed {
                base,
                commits: accum.commits,
            },
        );
    }

    Ok((result, total_bot_commits_filtered))
}

/// Get author statistics from git log.
///
/// Only called when --authors flag is provided.
///
/// # Arguments
/// * `path` - Repository root directory
/// * `days` - Time window in days (uses git's relative date)
/// * `file_stats` - Pre-computed file churn data (used to count files_touched per author)
///
/// # Returns
/// Vec<AuthorStats> sorted by commits descending
///
/// # Git Commands
///
/// ## Command 1: Get commit counts per author
/// ```bash
/// git shortlog -sne --since="N days ago"
/// ```
/// Output format: `   123\tAuthor Name <email@example.com>`
///
/// ## Command 2: Get line stats per author
/// ```bash
/// git log --since="N days ago" --author="email" --numstat --format=""
/// ```
pub fn get_author_stats(
    path: &Path,
    days: u32,
    file_stats: &HashMap<String, FileChurn>,
) -> Result<Vec<AuthorStats>, ChurnError> {
    // Check path exists
    if !path.exists() {
        return Err(ChurnError::PathNotFound(path.to_path_buf()));
    }

    // Verify it's a git repository
    if !is_git_repository(path)? {
        return Err(ChurnError::NotGitRepository {
            path: path.to_path_buf(),
        });
    }

    // Build the --since argument
    let since_arg = format!("{} days ago", days);

    // Step 1: Run git shortlog for commit counts
    // Output format: "   123\tAuthor Name <email@example.com>"
    let shortlog_output = match run_git(
        &[
            "shortlog",
            "-sne",
            &format!("--since={}", since_arg),
            "HEAD",
        ],
        path,
    ) {
        Ok(output) => output,
        Err(ChurnError::GitError { stderr, .. }) => {
            // Handle empty repository or no commits in window
            if stderr.contains("does not have any commits")
                || stderr.contains("bad revision")
                || stderr.is_empty()
            {
                return Ok(Vec::new());
            }
            return Err(ChurnError::GitError {
                command: format!("git shortlog -sne --since=\"{}\"", since_arg),
                stderr,
                exit_code: None,
            });
        }
        Err(e) => return Err(e),
    };

    // If output is empty, no commits in window
    if shortlog_output.trim().is_empty() {
        return Ok(Vec::new());
    }

    // Intermediate structure to accumulate author data
    struct AuthorData {
        name: String,
        email: String,
        commits: u32,
        lines_added: u32,
        lines_deleted: u32,
    }

    let mut author_map: HashMap<String, AuthorData> = HashMap::new();

    // Step 2: Parse shortlog output
    // Format: "   123\tAuthor Name <email@example.com>"
    for line in shortlog_output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Split by tab to get count and author info
        let parts: Vec<&str> = line.splitn(2, '\t').collect();
        if parts.len() != 2 {
            continue;
        }

        let commits: u32 = match parts[0].trim().parse() {
            Ok(n) => n,
            Err(_) => continue,
        };

        let author_part = parts[1].trim();

        // Parse "Author Name <email@example.com>"
        // Find the last '<' and '>' for the email
        if let (Some(email_start), Some(email_end)) =
            (author_part.rfind('<'), author_part.rfind('>'))
        {
            if email_start < email_end {
                let name = author_part[..email_start].trim().to_string();
                let email = author_part[email_start + 1..email_end].to_string();

                author_map.insert(
                    email.clone(),
                    AuthorData {
                        name,
                        email,
                        commits,
                        lines_added: 0,
                        lines_deleted: 0,
                    },
                );
            }
        }
    }

    // Step 3: Get line stats per author using git log --author
    for data in author_map.values_mut() {
        let numstat_output = match run_git(
            &[
                "log",
                &format!("--since={}", since_arg),
                &format!("--author={}", data.email),
                "--numstat",
                "--format=",
            ],
            path,
        ) {
            Ok(output) => output,
            Err(_) => continue, // Skip this author if git log fails
        };

        // Parse numstat output
        // Format: <added>\t<deleted>\t<filepath>
        // Binary files show: -\t-\t<filepath>
        for line in numstat_output.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() < 3 {
                continue;
            }

            let added_str = parts[0];
            let deleted_str = parts[1];

            // Skip binary files (shown as "-")
            if added_str == "-" || deleted_str == "-" {
                continue;
            }

            // Parse line counts
            let added: u32 = added_str.parse().unwrap_or(0);
            let deleted: u32 = deleted_str.parse().unwrap_or(0);

            data.lines_added += added;
            data.lines_deleted += deleted;
        }
    }

    // Step 4: Calculate files_touched from file_stats
    // Count files where author email appears in the authors list
    let mut result: Vec<AuthorStats> = author_map
        .into_iter()
        .map(|(email, data)| {
            // Count files where this author appears
            let files_touched = file_stats
                .values()
                .filter(|f| f.authors.contains(&email))
                .count() as u32;

            AuthorStats {
                name: data.name,
                email: data.email,
                commits: data.commits,
                lines_added: data.lines_added,
                lines_deleted: data.lines_deleted,
                files_touched,
            }
        })
        .collect();

    // Step 5: Sort by commits descending
    result.sort_by(|a, b| b.commits.cmp(&a.commits));

    Ok(result)
}

/// Build summary statistics from file churn data.
///
/// # Formulas
/// ```text
/// total_files = len(file_stats)
/// total_commits = sum(f.commit_count for f in file_stats)
/// total_lines_changed = sum(f.lines_changed for f in file_stats)
/// avg_commits_per_file = total_commits / total_files  // 0.0 if empty
/// most_churned_file = argmax(file_stats, key=commit_count).file
/// ```
pub fn build_summary(file_stats: &HashMap<String, FileChurn>, days: u32) -> ChurnSummary {
    let total_files = file_stats.len() as u32;

    if total_files == 0 {
        return ChurnSummary {
            total_files: 0,
            total_commits: 0,
            time_window_days: days,
            total_lines_changed: 0,
            avg_commits_per_file: 0.0,
            most_churned_file: String::new(),
        };
    }

    let total_commits: u32 = file_stats.values().map(|f| f.commit_count).sum();
    let total_lines_changed: u64 = file_stats.values().map(|f| f.lines_changed as u64).sum();
    let avg_commits_per_file = total_commits as f64 / total_files as f64;

    // Find the file with the highest commit_count
    let most_churned_file = file_stats
        .values()
        .max_by_key(|f| f.commit_count)
        .map(|f| f.file.clone())
        .unwrap_or_default();

    ChurnSummary {
        total_files,
        total_commits,
        time_window_days: days,
        total_lines_changed,
        avg_commits_per_file,
        most_churned_file,
    }
}

/// Get recommendation text based on hotspot score.
///
/// # Thresholds
/// - `> 0.7`: Critical
/// - `> 0.4`: Warning
/// - `<= 0.4`: Low risk
pub fn get_recommendation(score: f64) -> &'static str {
    if score > 0.7 {
        "Critical: High churn + high complexity. Prioritize refactoring."
    } else if score > 0.4 {
        "Warning: Moderate risk. Consider simplification."
    } else {
        "Low risk."
    }
}

/// Check if a file path matches any of the exclude patterns.
///
/// Uses shell-style glob matching (fnmatch semantics):
/// - `*` matches any sequence of characters (excluding path separators in some cases)
/// - `?` matches any single character
/// - `[abc]` matches any character in the set
///
/// # Arguments
/// * `path` - File path to check
/// * `patterns` - Glob patterns to match against
///
/// # Returns
/// `true` if the path matches any pattern, `false` otherwise
pub fn matches_exclude_pattern(path: &str, patterns: &[String]) -> bool {
    for pattern_str in patterns {
        if let Ok(pattern) = Pattern::new(pattern_str) {
            if pattern.matches(path) {
                return true;
            }
        }
    }
    false
}

// =============================================================================
// Output Formatting (Phase 6)
// =============================================================================

/// Truncate a path to max_len characters, adding "..." prefix if needed.
///
/// Uses right-aligned truncation to preserve the filename.
fn truncate_path(path: &str, max_len: usize) -> String {
    if path.len() <= max_len {
        path.to_string()
    } else {
        // Keep the rightmost part (filename is most important)
        let keep_len = max_len.saturating_sub(3); // Reserve space for "..."
        let start = path.len().saturating_sub(keep_len);
        format!("...{}", &path[start..])
    }
}

/// Format report as human-readable text.
///
/// # Output Format
/// ```text
/// Code Churn Analysis
/// ==================================================
///
/// Warnings:
///   - Repository is a shallow clone (depth ~100). [...]
///
/// Time window: 365 days
/// Total files changed: 156
/// Total commits: 892
/// Total lines changed: 45230
/// Most churned file: src/core/engine.py
///
/// Top Files by Churn:
/// Rank  File                                    Commits   Lines     Authors
/// --------------------------------------------------------------------------
/// 1     src/core/engine.py                      47        2140      2
/// 2     src/api/handlers.py                     38        1520      3
/// ...
///
/// Hotspot Matrix (High Churn + High Complexity):
///   1. src/core/engine.py
///      Churn: 47 commits (rank #1)
///      Complexity: CC=25 (rank #2)
///      Score: 0.823
///      Critical: High churn + high complexity. Prioritize refactoring.
///
/// Top Authors:
///   Alice Smith <alice@example.com>
///     Commits: 47, Files: 23
///   Bob Jones <bob@example.com>
///     Commits: 23, Files: 15
/// ```
///
/// # Display Limits
/// - Files table: top 10
/// - Hotspots: top 5
/// - Authors: top 5
/// - File paths truncated to 38 chars (right-aligned truncation)
pub fn format_text_output(report: &ChurnReport) -> String {
    use std::fmt::Write;

    let mut output = String::new();

    // Header
    writeln!(output, "Code Churn Analysis").unwrap();
    writeln!(output, "==================================================").unwrap();
    writeln!(output).unwrap();

    // Warnings section (if any)
    if !report.warnings.is_empty() {
        writeln!(output, "Warnings:").unwrap();
        for warning in &report.warnings {
            writeln!(output, "  - {}", warning).unwrap();
        }
        writeln!(output).unwrap();
    }

    // Summary statistics
    writeln!(
        output,
        "Time window: {} days",
        report.summary.time_window_days
    )
    .unwrap();
    writeln!(
        output,
        "Total files changed: {}",
        report.summary.total_files
    )
    .unwrap();
    writeln!(output, "Total commits: {}", report.summary.total_commits).unwrap();
    writeln!(
        output,
        "Total lines changed: {}",
        report.summary.total_lines_changed
    )
    .unwrap();
    if !report.summary.most_churned_file.is_empty() {
        writeln!(
            output,
            "Most churned file: {}",
            report.summary.most_churned_file
        )
        .unwrap();
    }
    writeln!(output).unwrap();

    // Top Files by Churn (limit to 10)
    if !report.files.is_empty() {
        writeln!(output, "Top Files by Churn:").unwrap();
        writeln!(
            output,
            "{:<6}{:<40}{:<10}{:<10}{:<8}",
            "Rank", "File", "Commits", "Lines", "Authors"
        )
        .unwrap();
        writeln!(output, "{}", "-".repeat(74)).unwrap();

        for (i, file) in report.files.iter().take(10).enumerate() {
            let truncated = truncate_path(&file.file, 38);
            writeln!(
                output,
                "{:<6}{:<40}{:<10}{:<10}{:<8}",
                i + 1,
                truncated,
                file.commit_count,
                file.lines_changed,
                file.author_count
            )
            .unwrap();
        }
        writeln!(output).unwrap();
    }

    // Hotspot Matrix (limit to 5)
    if !report.hotspots.is_empty() {
        writeln!(output, "Hotspot Matrix (High Churn + High Complexity):").unwrap();

        for (i, hotspot) in report.hotspots.iter().take(5).enumerate() {
            writeln!(output, "  {}. {}", i + 1, hotspot.file).unwrap();
            writeln!(
                output,
                "     Churn: {} commits (rank #{})",
                hotspot.commit_count, hotspot.churn_rank
            )
            .unwrap();
            writeln!(
                output,
                "     Complexity: CC={} (rank #{})",
                hotspot.cyclomatic_complexity, hotspot.complexity_rank
            )
            .unwrap();
            writeln!(output, "     Score: {:.3}", hotspot.combined_score).unwrap();
            writeln!(output, "     {}", hotspot.recommendation).unwrap();
        }
        writeln!(output).unwrap();
    }

    // Top Authors (limit to 5)
    if !report.authors.is_empty() {
        writeln!(output, "Top Authors:").unwrap();

        for author in report.authors.iter().take(5) {
            writeln!(output, "  {} <{}>", author.name, author.email).unwrap();
            writeln!(
                output,
                "    Commits: {}, Files: {}",
                author.commits, author.files_touched
            )
            .unwrap();
        }
        writeln!(output).unwrap();
    }

    output
}
