//! Churn command - Git-based file churn analysis
//!
//! Analyzes git history to identify high-churn files (frequently changed code).
//! Combined with complexity analysis, creates a "hotspot matrix" for prioritizing
//! refactoring efforts.

use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Args;

use tldr_core::quality::churn::{
    build_summary, check_shallow_clone, get_author_stats, get_file_churn, is_git_repository,
    ChurnError, ChurnReport,
};

use crate::output::{OutputFormat, OutputWriter};

/// Analyze git-based code churn
#[derive(Debug, Args)]
pub struct ChurnArgs {
    /// Directory to analyze (default: current dir)
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Days of history to analyze
    #[arg(long, default_value = "365")]
    pub days: u32,

    /// Maximum files to show
    #[arg(long, default_value = "20")]
    pub top: usize,

    /// Exclude files matching pattern (glob syntax, can be repeated)
    #[arg(long, short = 'e')]
    pub exclude: Vec<String>,

    /// Include author statistics
    #[arg(long)]
    pub authors: bool,

    /// Deprecated: use `tldr hotspots` instead. This flag is ignored.
    #[arg(long, hide = true)]
    pub hotspots: bool,
}

impl ChurnArgs {
    /// Run the churn command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        writer.progress(&format!(
            "Analyzing churn in {} (last {} days)...",
            self.path.display(),
            self.days
        ));

        // Run the analysis
        let report = analyze_churn(
            &self.path,
            self.days,
            self.top,
            &self.exclude,
            self.authors,
            self.hotspots,
        )?;

        // Output based on format
        if writer.is_text() {
            let text = format_churn_text(&report);
            writer.write_text(&text)?;
        } else {
            writer.write(&report)?;
        }

        Ok(())
    }
}

/// Orchestrate churn analysis combining all phases.
///
/// # Arguments
/// * `path` - Directory to analyze (must be in a git repository)
/// * `days` - Time window in days
/// * `top_k` - Maximum files to return
/// * `exclude_patterns` - Glob patterns to exclude
/// * `include_authors` - Whether to include author statistics
/// * `include_hotspots` - Whether to calculate hotspot matrix
///
/// # Returns
/// Complete ChurnReport with files, optional authors, optional hotspots, and summary
pub fn analyze_churn(
    path: &Path,
    days: u32,
    top_k: usize,
    exclude_patterns: &[String],
    include_authors: bool,
    include_hotspots: bool,
) -> Result<ChurnReport, ChurnError> {
    // Check if path is a git repository
    if !is_git_repository(path)? {
        return Err(ChurnError::NotGitRepository {
            path: path.to_path_buf(),
        });
    }

    // Check for shallow clone
    let (is_shallow, shallow_depth) = check_shallow_clone(path)?;

    let mut warnings = Vec::new();
    if is_shallow {
        let depth_info = shallow_depth
            .map(|d| format!(" (~{} commits)", d))
            .unwrap_or_default();
        warnings.push(format!(
            "Repository is a shallow clone{}. Churn analysis may be incomplete.",
            depth_info
        ));
    }

    // Get file churn data
    let file_stats = get_file_churn(path, days, exclude_patterns)?;

    // Sort files by commit_count descending and take top_k
    let mut files: Vec<_> = file_stats.values().cloned().collect();
    files.sort_by(|a, b| b.commit_count.cmp(&a.commit_count));
    files.truncate(top_k);

    // Get author stats if requested
    let authors = if include_authors {
        get_author_stats(path, days, &file_stats)?
    } else {
        Vec::new()
    };

    // Hotspot analysis removed — use `tldr hotspots` instead
    if include_hotspots {
        eprintln!("Warning: `churn --hotspots` is removed. Use `tldr hotspots` instead.");
    }
    let hotspots = Vec::new();

    // Build summary
    let summary = build_summary(&file_stats, days);

    Ok(ChurnReport {
        files,
        hotspots,
        authors,
        summary,
        is_shallow,
        shallow_depth,
        warnings,
    })
}

/// Format churn report for text output (plain text, no box-drawing)
fn format_churn_text(report: &ChurnReport) -> String {
    use colored::Colorize;

    let mut output = String::new();

    // Header with summary
    output.push_str(&format!(
        "Churn Analysis ({} files, {} days)\n",
        report.summary.total_files.to_string().yellow(),
        report.summary.time_window_days
    ));
    output.push_str(&format!(
        "Total commits: {}, Lines changed: {}\n",
        report.summary.total_commits.to_string().cyan(),
        report.summary.total_lines_changed.to_string().cyan()
    ));
    output.push_str(&format!(
        "Most churned: {}\n\n",
        report.summary.most_churned_file.green()
    ));

    // Warnings
    for warning in &report.warnings {
        output.push_str(&format!("{} {}\n", "Warning:".yellow(), warning));
    }
    if !report.warnings.is_empty() {
        output.push('\n');
    }

    // Files list
    if !report.files.is_empty() {
        output.push_str(&"High-Churn Files:\n".bold().to_string());
        output.push_str(&format!(
            " {:>3}  {:>7}  {:>7}  {:>7}  {:>4}  {:>10}  {}\n",
            "#", "Commits", "+Lines", "-Lines", "Auth", "Last", "File"
        ));

        for (i, file) in report.files.iter().enumerate() {
            output.push_str(&format!(
                " {:>3}  {:>7}  {:>7}  {:>7}  {:>4}  {:>10}  {}\n",
                i + 1,
                file.commit_count,
                format!("+{}", file.lines_added),
                format!("-{}", file.lines_deleted),
                file.author_count,
                file.last_commit.as_deref().unwrap_or("-"),
                file.file
            ));
        }
        output.push('\n');
    }

    // Hotspots list
    if !report.hotspots.is_empty() {
        output.push_str(&"Hotspots (churn x complexity):\n".bold().to_string());
        output.push_str(&format!(
            " {:>3}  {:>5}  {:>7}  {:>4}  {}\n",
            "#", "Score", "Commits", "CC", "File"
        ));

        for (i, hotspot) in report.hotspots.iter().enumerate() {
            let score_str = format!("{:.2}", hotspot.combined_score);
            let score_display = if hotspot.combined_score > 0.6 {
                score_str.red().to_string()
            } else if hotspot.combined_score > 0.3 {
                score_str.yellow().to_string()
            } else {
                score_str.green().to_string()
            };

            output.push_str(&format!(
                " {:>3}  {:>5}  {:>7}  {:>4}  {}\n",
                i + 1,
                score_display,
                hotspot.commit_count,
                hotspot.cyclomatic_complexity,
                hotspot.file
            ));
        }
        output.push('\n');
    }

    // Authors list
    if !report.authors.is_empty() {
        output.push_str(&"Author Statistics:\n".bold().to_string());
        output.push_str(&format!(
            " {:>3}  {:>7}  {:>7}  {:>7}  {:>5}  {}\n",
            "#", "Commits", "+Lines", "-Lines", "Files", "Author"
        ));

        for (i, author) in report.authors.iter().enumerate() {
            output.push_str(&format!(
                " {:>3}  {:>7}  {:>7}  {:>7}  {:>5}  {}\n",
                i + 1,
                author.commits,
                format!("+{}", author.lines_added),
                format!("-{}", author.lines_deleted),
                author.files_touched,
                author.name
            ));
        }
    }

    output
}
