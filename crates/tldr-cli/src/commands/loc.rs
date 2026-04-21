//! LOC command - Count lines of code with type breakdown
//!
//! Provides language-aware line counting:
//! - Code lines: Lines containing executable code
//! - Comment lines: Lines containing only comments
//! - Blank lines: Empty lines or lines with only whitespace
//!
//! # Session 15 Phase 2
//!
//! Implements spec.md Section 1 (LOC Command).
//!
//! # Invariants
//!
//! - `code_lines + comment_lines + blank_lines == total_lines`
//! - Binary files are skipped with warning
//! - Files > 10MB are skipped with warning
//!
//! # Example
//!
//! ```bash
//! # Analyze a single file
//! tldr loc src/main.rs
//!
//! # Analyze a directory with per-file breakdown
//! tldr loc src/ --by-file
//!
//! # Filter by language
//! tldr loc . --lang python
//!
//! # Exclude patterns
//! tldr loc . --exclude "*.test.py" --exclude "migrations/*"
//! ```

use std::path::PathBuf;

use anyhow::Result;
use clap::Args;

use tldr_core::metrics::loc::{analyze_loc, LocOptions, LocReport};
use tldr_core::Language;

use crate::output::{OutputFormat, OutputWriter};

/// Count lines of code with type breakdown (code, comments, blanks)
#[derive(Debug, Args)]
pub struct LocArgs {
    /// Directory or file to analyze
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Filter to specific language
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,

    /// Show per-file breakdown
    #[arg(long)]
    pub by_file: bool,

    /// Aggregate by directory
    #[arg(long)]
    pub by_dir: bool,

    /// Exclude patterns (glob syntax), can be specified multiple times
    #[arg(long, short = 'e')]
    pub exclude: Vec<String>,

    /// Include hidden files (dotfiles)
    #[arg(long)]
    pub include_hidden: bool,

    /// Ignore .gitignore rules
    #[arg(long)]
    pub no_gitignore: bool,

    /// Maximum files to process (0 = unlimited)
    #[arg(long, default_value = "0")]
    pub max_files: usize,
}

impl LocArgs {
    /// Run the LOC command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        writer.progress(&format!("Counting lines in {}...", self.path.display()));

        // Build options
        let options = LocOptions {
            lang: self.lang,
            by_file: self.by_file,
            by_dir: self.by_dir,
            exclude: self.exclude.clone(),
            include_hidden: self.include_hidden,
            gitignore: !self.no_gitignore,
            max_files: self.max_files,
            max_file_size_mb: 10, // Default 10MB limit
        };

        // Analyze
        let report = analyze_loc(&self.path, &options)?;

        // Output based on format
        if writer.is_text() {
            let text = format_loc_text(&report);
            writer.write_text(&text)?;
        } else {
            writer.write(&report)?;
        }

        Ok(())
    }
}

/// Format LOC report for human-readable text output.
/// Uses plain aligned text (no box-drawing tables) for token efficiency.
fn format_loc_text(report: &LocReport) -> String {
    use crate::output::{common_path_prefix, strip_prefix_display};
    use colored::Colorize;
    use std::path::Path;

    let mut output = String::new();

    // Summary
    let summary = &report.summary;
    output.push_str(&format!(
        "Lines of Code ({} files, {} total)\n\n",
        summary.total_files, summary.total_lines,
    ));
    output.push_str(&format!(
        "  Code:     {:>6} ({:.1}%)\n",
        summary.code_lines, summary.code_percent
    ));
    output.push_str(&format!(
        "  Comments: {:>6} ({:.1}%)\n",
        summary.comment_lines, summary.comment_percent
    ));
    output.push_str(&format!(
        "  Blank:    {:>6} ({:.1}%)\n",
        summary.blank_lines, summary.blank_percent
    ));

    // By language (plain text table)
    if !report.by_language.is_empty() {
        output.push_str("\nBy Language:\n");

        let max_lang = report
            .by_language
            .iter()
            .map(|e| e.language.len())
            .max()
            .unwrap_or(8)
            .max(8);
        output.push_str(&format!(
            "  {:<width$}  {:>5}  {:>6}  {:>6}  {:>5}  {:>6}\n",
            "Language",
            "Files",
            "Code",
            "Comment",
            "Blank",
            "Total",
            width = max_lang,
        ));

        for entry in &report.by_language {
            output.push_str(&format!(
                "  {:<width$}  {:>5}  {:>6}  {:>6}  {:>5}  {:>6}\n",
                entry.language,
                entry.files,
                entry.code_lines,
                entry.comment_lines,
                entry.blank_lines,
                entry.total_lines,
                width = max_lang,
            ));
        }
    }

    // By file (if requested and present)
    if let Some(by_file) = &report.by_file {
        if !by_file.is_empty() {
            output.push_str("\nBy File:\n");

            // Strip common path prefix
            let paths: Vec<&Path> = by_file.iter().map(|e| e.path.as_path()).collect();
            let prefix = common_path_prefix(&paths);

            let display_count = by_file.len().min(50);
            let max_path = by_file
                .iter()
                .take(display_count)
                .map(|e| strip_prefix_display(&e.path, &prefix).len())
                .max()
                .unwrap_or(4)
                .clamp(4, 50);

            output.push_str(&format!(
                "  {:<width$}  {:>4}  {:>6}  {:>6}  {:>5}  {:>6}\n",
                "File",
                "Lang",
                "Code",
                "Comment",
                "Blank",
                "Total",
                width = max_path,
            ));

            for entry in by_file.iter().take(display_count) {
                let rel = strip_prefix_display(&entry.path, &prefix);
                let display_path = if rel.len() > 50 {
                    format!("...{}", &rel[rel.len() - 47..])
                } else {
                    rel
                };
                output.push_str(&format!(
                    "  {:<width$}  {:>4}  {:>6}  {:>6}  {:>5}  {:>6}\n",
                    display_path,
                    entry.language,
                    entry.code_lines,
                    entry.comment_lines,
                    entry.blank_lines,
                    entry.total_lines,
                    width = max_path,
                ));
            }

            if by_file.len() > display_count {
                output.push_str(&format!(
                    "  ... and {} more files\n",
                    by_file.len() - display_count
                ));
            }
        }
    }

    // By directory (if requested and present)
    if let Some(by_dir) = &report.by_directory {
        if !by_dir.is_empty() {
            output.push_str("\nBy Directory:\n");

            let paths: Vec<&Path> = by_dir.iter().map(|e| e.path.as_path()).collect();
            let prefix = common_path_prefix(&paths);

            let max_dir = by_dir
                .iter()
                .take(30)
                .map(|e| strip_prefix_display(&e.path, &prefix).len())
                .max()
                .unwrap_or(4)
                .max(4);

            output.push_str(&format!(
                "  {:<width$}  {:>6}  {:>6}  {:>5}  {:>6}\n",
                "Directory",
                "Code",
                "Comment",
                "Blank",
                "Total",
                width = max_dir,
            ));

            for entry in by_dir.iter().take(30) {
                let rel = strip_prefix_display(&entry.path, &prefix);
                output.push_str(&format!(
                    "  {:<width$}  {:>6}  {:>6}  {:>5}  {:>6}\n",
                    rel,
                    entry.code_lines,
                    entry.comment_lines,
                    entry.blank_lines,
                    entry.total_lines,
                    width = max_dir,
                ));
            }
        }
    }

    // Warnings
    if !report.warnings.is_empty() {
        output.push_str(&"\nWarnings:\n".yellow().to_string());
        for warning in &report.warnings {
            output.push_str(&format!("  - {}\n", warning));
        }
    }

    output
}
