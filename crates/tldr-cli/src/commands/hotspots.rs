//! Hotspots command - Identify high-risk code regions
//!
//! Combines git churn data with cognitive complexity to identify
//! code that is both frequently changed and complex (hotspots).

use std::path::PathBuf;

use anyhow::Result;
use clap::Args;

use tldr_core::quality::hotspots::{analyze_hotspots, HotspotsOptions, HotspotsReport};

use crate::output::{OutputFormat, OutputWriter};

/// Identify churn x complexity hotspots
#[derive(Debug, Args)]
pub struct HotspotsArgs {
    /// Directory to analyze (default: current directory)
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Days of git history to analyze
    #[arg(long, default_value = "365")]
    pub days: u32,

    /// Number of hotspots to return
    #[arg(long, default_value = "20")]
    pub top: usize,

    /// Analyze at function level (default: file level)
    #[arg(long)]
    pub by_function: bool,

    /// Include complexity trend analysis
    #[arg(long)]
    pub show_trend: bool,

    /// Minimum commits to be considered a hotspot
    #[arg(long, default_value = "3")]
    pub min_commits: u32,

    /// Exclude patterns (glob syntax, can be repeated)
    #[arg(long, short = 'e')]
    pub exclude: Vec<String>,

    /// Minimum hotspot score threshold (0.0 to 1.0)
    #[arg(long)]
    pub threshold: Option<f64>,

    /// Since date (ISO format, e.g., 2024-01-01)
    #[arg(long)]
    pub since: Option<String>,

    /// Exponential decay half-life in days (default: 90, 0 = no decay)
    #[arg(long, default_value = "90")]
    pub recency_halflife: f64,

    /// Include bot/automated commits in churn analysis (default: filtered)
    #[arg(long)]
    pub include_bots: bool,
}

impl HotspotsArgs {
    /// Run the hotspots command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        writer.progress(&format!(
            "Analyzing hotspots in {} (last {} days)...",
            self.path.display(),
            self.days
        ));

        // Build options
        let mut options = HotspotsOptions::new()
            .with_days(self.days)
            .with_top(self.top)
            .with_min_commits(self.min_commits)
            .with_by_function(self.by_function)
            .with_show_trend(self.show_trend)
            .with_exclude(self.exclude.clone());

        if let Some(threshold) = self.threshold {
            options = options.with_threshold(threshold);
        }

        if let Some(ref since) = self.since {
            options = options.with_since(since.clone());
        }

        options = options
            .with_recency_halflife(self.recency_halflife)
            .with_include_bots(self.include_bots);

        // Run analysis
        let report = analyze_hotspots(&self.path, &options)?;

        // Output based on format
        if writer.is_text() {
            let text = format_hotspots_text(&report);
            writer.write_text(&text)?;
        } else {
            writer.write(&report)?;
        }

        Ok(())
    }
}

/// Format hotspots report for text output (plain text, no box-drawing)
fn format_hotspots_text(report: &HotspotsReport) -> String {
    use colored::Colorize;

    let mut output = String::new();

    // Header with summary
    output.push_str(&format!(
        "Hotspots Analysis ({} files, {} days)\n",
        report.summary.total_files_analyzed.to_string().yellow(),
        report.metadata.days
    ));
    output.push_str(&format!(
        "Total commits: {}, Concentration: {:.1}%\n",
        report.summary.total_commits.to_string().cyan(),
        report.summary.hotspot_concentration
    ));
    if let Some(bot_filtered) = report.summary.total_bot_commits_filtered {
        if bot_filtered > 0 {
            output.push_str(&format!(
                "Bot commits filtered: {}\n",
                bot_filtered.to_string().dimmed()
            ));
        }
    }
    output.push_str(&format!(
        "Mode: {}, Algorithm: v{}\n\n",
        if report.metadata.by_function {
            "function-level"
        } else {
            "file-level"
        },
        report.metadata.algorithm_version
    ));

    // Warnings
    for warning in &report.warnings {
        output.push_str(&format!("{} {}\n", "Warning:".yellow(), warning));
    }
    if !report.warnings.is_empty() {
        output.push('\n');
    }

    // Hotspots table (plain text)
    if !report.hotspots.is_empty() {
        output.push_str(
            &"Hotspots (high churn + high complexity):\n"
                .bold()
                .to_string(),
        );

        if report.metadata.by_function {
            output.push_str(&format!(
                " {:>3}  {:>5}  {:>5}  {:>5}  {:>7}  {:>4}  {:>8}  {:<20}  {}\n",
                "#", "Score", "Churn", "Cmplx", "Commits", "Cog", "Priority", "Function", "File"
            ));
            for (i, h) in report.hotspots.iter().enumerate() {
                let priority = short_priority(&h.recommendation);
                output.push_str(&format!(
                    " {:>3}  {:>5.2}  {:>5.2}  {:>5.2}  {:>7}  {:>4}  {:>8}  {:<20}  {}\n",
                    i + 1,
                    h.hotspot_score,
                    h.churn_score,
                    h.complexity_score,
                    h.commit_count,
                    h.complexity,
                    priority,
                    h.function.as_deref().unwrap_or("-"),
                    h.file
                ));
            }
        } else {
            output.push_str(&format!(
                " {:>3}  {:>5}  {:>5}  {:>5}  {:>7}  {:>4}  {:>8}  {}\n",
                "#", "Score", "Churn", "Cmplx", "Commits", "Cog", "Priority", "File"
            ));
            for (i, h) in report.hotspots.iter().enumerate() {
                let priority = short_priority(&h.recommendation);
                output.push_str(&format!(
                    " {:>3}  {:>5.2}  {:>5.2}  {:>5.2}  {:>7}  {:>4}  {:>8}  {}\n",
                    i + 1,
                    h.hotspot_score,
                    h.churn_score,
                    h.complexity_score,
                    h.commit_count,
                    h.complexity,
                    priority,
                    h.file
                ));
            }
        }
        output.push('\n');
    } else {
        output.push_str("No hotspots found.\n\n");
    }

    // Summary recommendation
    output.push_str(&format!(
        "{}: {}\n",
        "Summary".bold(),
        report.summary.recommendation
    ));

    output
}

/// Extract short priority label from full recommendation string.
fn short_priority(recommendation: &str) -> &'static str {
    if recommendation.starts_with("Critical") {
        "Critical"
    } else if recommendation.starts_with("High") {
        "High"
    } else if recommendation.starts_with("Medium") {
        "Medium"
    } else {
        "Monitor"
    }
}
