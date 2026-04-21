//! Cognitive complexity command - Calculate SonarQube cognitive complexity
//!
//! Analyzes cognitive complexity for functions in a file or directory,
//! with threshold checking.
//! Extends the basic complexity command with detailed cognitive metrics.

use std::path::PathBuf;

use anyhow::Result;
use clap::Args;

use tldr_core::{
    metrics::{
        analyze_cognitive, merge_cognitive_reports, walk_source_files, CognitiveOptions,
        WalkOptions,
    },
    validate_file_path, Language,
};

use crate::output::{format_cognitive_text, OutputFormat, OutputWriter};

/// Calculate cognitive complexity for functions (SonarQube algorithm)
#[derive(Debug, Args)]
pub struct CognitiveArgs {
    /// File or directory to analyze
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Specific function to analyze (analyzes all if not specified)
    /// Note: --function is the long form; -f short flag is NOT used to avoid collision with --format
    #[arg(long)]
    pub function: Option<String>,

    /// Programming language (auto-detect if not specified)
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,

    /// Complexity threshold for violations (default: 15)
    #[arg(long, default_value = "15")]
    pub threshold: u32,

    /// High threshold for severe violations (default: 25)
    #[arg(long, default_value = "25")]
    pub high_threshold: u32,

    /// Show line-by-line complexity contributors
    #[arg(long)]
    pub show_contributors: bool,

    /// Include cyclomatic complexity comparison
    #[arg(long)]
    pub include_cyclomatic: bool,

    /// Maximum functions to report (0 = all)
    #[arg(long, default_value = "50")]
    pub top: usize,

    /// Exclude patterns (glob syntax), can be specified multiple times
    #[arg(long, short = 'e')]
    pub exclude: Vec<String>,

    /// Include hidden files (dotfiles)
    #[arg(long)]
    pub include_hidden: bool,

    /// Maximum files to process (0 = unlimited)
    #[arg(long, default_value = "0")]
    pub max_files: usize,
}

impl CognitiveArgs {
    /// Run the cognitive complexity command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        // Build options
        let options = CognitiveOptions::new()
            .with_function(self.function.clone())
            .with_threshold(self.threshold)
            .with_high_threshold(self.high_threshold)
            .with_contributors(self.show_contributors)
            .with_cyclomatic(self.include_cyclomatic)
            .with_top(self.top);

        let report = if self.path.is_file() {
            // Single file: preserve exact current behavior
            let validated_path = validate_file_path(self.path.to_str().unwrap_or_default(), None)?;

            writer.progress(&format!(
                "Calculating cognitive complexity for {}...",
                validated_path.display()
            ));

            analyze_cognitive(&validated_path, &options)?
        } else if self.path.is_dir() {
            // Directory: walk -> analyze each -> merge
            let walk_options = WalkOptions {
                lang: self.lang,
                exclude: self.exclude.clone(),
                include_hidden: self.include_hidden,
                gitignore: true,
                max_files: self.max_files,
            };

            let (files, walk_warnings) = walk_source_files(&self.path, &walk_options)?;

            writer.progress(&format!(
                "Analyzing {} files in {}...",
                files.len(),
                self.path.display()
            ));

            let mut reports = Vec::new();
            let mut extra_warnings = walk_warnings;

            for file in &files {
                match analyze_cognitive(file, &options) {
                    Ok(report) => reports.push(report),
                    Err(e) => {
                        extra_warnings.push(format!("Failed to analyze {}: {}", file.display(), e));
                    }
                }
            }

            let mut merged = merge_cognitive_reports(reports, &options);
            // Prepend walk warnings and per-file error warnings
            let mut all_warnings = extra_warnings;
            all_warnings.append(&mut merged.warnings);
            merged.warnings = all_warnings;
            merged
        } else {
            // Path does not exist (or is a special file)
            return Err(anyhow::anyhow!(
                "Path does not exist: {}",
                self.path.display()
            ));
        };

        // Output based on format
        if writer.is_text() {
            writer.write_text(&format_cognitive_text(&report))?;
        } else {
            writer.write(&report)?;
        }

        Ok(())
    }
}
