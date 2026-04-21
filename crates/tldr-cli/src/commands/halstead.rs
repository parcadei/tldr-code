//! Halstead metrics command - Calculate Halstead complexity metrics per function
//!
//! Exposes Halstead software science metrics as a standalone command with:
//! - Per-function granularity
//! - Threshold-based recommendations
//! - Optional operator/operand listing
//! - File or directory analysis

use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Args;
use colored::Colorize;

use tldr_core::metrics::halstead::{
    analyze_halstead, merge_halstead_reports, HalsteadOptions, HalsteadReport, ThresholdStatus,
};
use tldr_core::metrics::{walk_source_files, WalkOptions};
use tldr_core::{detect_or_parse_language, validate_file_path, Language};

use crate::output::{common_path_prefix, strip_prefix_display, OutputFormat, OutputWriter};

/// Calculate Halstead complexity metrics
#[derive(Debug, Args)]
pub struct HalsteadArgs {
    /// File or directory to analyze
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Specific function to analyze (analyzes all if not specified)
    #[arg(long)]
    pub function: Option<String>,

    /// Programming language (auto-detect if not specified)
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,

    /// Show list of operators found
    #[arg(long)]
    pub show_operators: bool,

    /// Show list of operands found
    #[arg(long)]
    pub show_operands: bool,

    /// Volume threshold for warnings (default: 1000)
    #[arg(long, default_value = "1000")]
    pub threshold_volume: f64,

    /// Difficulty threshold for warnings (default: 20)
    #[arg(long, default_value = "20")]
    pub threshold_difficulty: f64,

    /// Maximum functions to report (0 = all)
    #[arg(long, default_value = "0")]
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

impl HalsteadArgs {
    /// Run the halstead command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        let options = HalsteadOptions {
            function: self.function.clone(),
            volume_threshold: self.threshold_volume,
            difficulty_threshold: self.threshold_difficulty,
            show_operators: self.show_operators,
            show_operands: self.show_operands,
            top: self.top,
        };

        let report = if self.path.is_file() {
            // Single file: preserve exact current behavior
            let validated_path = validate_file_path(self.path.to_str().unwrap_or_default(), None)?;
            let language =
                detect_or_parse_language(self.lang.as_ref().map(|l| l.as_str()), &validated_path)?;

            writer.progress(&format!(
                "Calculating Halstead metrics for {} ({:?})...",
                validated_path.display(),
                language
            ));

            analyze_halstead(&validated_path, Some(language), options)?
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
                "Calculating Halstead metrics for {} files in {}...",
                files.len(),
                self.path.display()
            ));

            let mut reports = Vec::new();
            let mut extra_warnings = walk_warnings;

            for file in &files {
                // Detect language per-file (walker already filtered to supported extensions)
                let language = match Language::from_path(file) {
                    Some(l) => l,
                    None => {
                        extra_warnings
                            .push(format!("Skipping {}: unsupported language", file.display()));
                        continue;
                    }
                };

                // Clone options because analyze_halstead takes ownership
                match analyze_halstead(file, Some(language), options.clone()) {
                    Ok(report) => reports.push(report),
                    Err(e) => {
                        extra_warnings.push(format!("Failed to analyze {}: {}", file.display(), e));
                    }
                }
            }

            let mut merged = merge_halstead_reports(reports, &options);
            let mut all_warnings = extra_warnings;
            all_warnings.append(&mut merged.warnings);
            merged.warnings = all_warnings;
            merged
        } else {
            return Err(anyhow::anyhow!(
                "Path does not exist: {}",
                self.path.display()
            ));
        };

        // Output based on format
        if writer.is_text() {
            self.print_text_report(&report, &writer)?;
        } else {
            writer.write(&report)?;
        }

        Ok(())
    }

    fn print_text_report(&self, report: &HalsteadReport, writer: &OutputWriter) -> Result<()> {
        // Header
        writer.write_text(&format!(
            "\n{}\n",
            "Halstead Metrics Report".bold().underline()
        ))?;

        // Summary
        writer.write_text(&format!(
            "\n{} ({} functions analyzed)\n",
            "Summary".bold(),
            report.summary.total_functions
        ))?;
        writer.write_text(&format!(
            "  Avg Volume:     {:.2}\n",
            report.summary.avg_volume
        ))?;
        writer.write_text(&format!(
            "  Avg Difficulty: {:.2}\n",
            report.summary.avg_difficulty
        ))?;
        writer.write_text(&format!(
            "  Avg Effort:     {:.2}\n",
            report.summary.avg_effort
        ))?;
        writer.write_text(&format!(
            "  Est. Bugs:      {:.3}\n",
            report.summary.total_estimated_bugs
        ))?;

        if report.summary.violations_count > 0 {
            writer.write_text(&format!(
                "  {}: {}\n",
                "Violations".red(),
                report.summary.violations_count
            ))?;
        }

        // Functions table
        writer.write_text(&format!("\n{}\n", "Functions".bold()))?;
        writer.write_text(&format!(
            "  {:<30} {:>8} {:>8} {:>10} {:>12} {:>10} {:>8}\n",
            "Name", "n1", "n2", "Volume", "Difficulty", "Effort", "Status"
        ))?;
        writer.write_text(&format!("{}\n", "-".repeat(98)))?;

        for func in &report.functions {
            let status = format_status(&func.thresholds.volume_status);
            let name = if func.name.len() > 30 {
                format!("{}...", &func.name[..27])
            } else {
                func.name.clone()
            };

            writer.write_text(&format!(
                "  {:<30} {:>8} {:>8} {:>10.2} {:>12.2} {:>10.0} {:>8}\n",
                name,
                func.metrics.n1,
                func.metrics.n2,
                func.metrics.volume,
                func.metrics.difficulty,
                func.metrics.effort,
                status
            ))?;

            // Show operators/operands if requested
            if let Some(ref operators) = func.operators {
                writer.write_text(&format!(
                    "    Operators: {}\n",
                    operators.join(", ").dimmed()
                ))?;
            }
            if let Some(ref operands) = func.operands {
                writer.write_text(&format!("    Operands: {}\n", operands.join(", ").dimmed()))?;
            }
        }

        // Violations with relative path display
        if !report.violations.is_empty() {
            // Compute common prefix for relative path display
            let violation_paths: Vec<&Path> = report
                .violations
                .iter()
                .map(|v| Path::new(v.file.as_str()))
                .collect();
            let prefix = if violation_paths.is_empty() {
                PathBuf::new()
            } else {
                common_path_prefix(&violation_paths)
            };

            writer.write_text(&format!("\n{}\n", "Threshold Violations".red().bold()))?;
            for violation in &report.violations {
                let rel_path = strip_prefix_display(Path::new(&violation.file), &prefix);
                writer.write_text(&format!(
                    "  {} in {}: {} = {:.2} (threshold: {:.2})\n",
                    violation.name.yellow(),
                    rel_path,
                    violation.metric,
                    violation.value,
                    violation.threshold
                ))?;
            }
        }

        // Warnings section
        if !report.warnings.is_empty() {
            writer.write_text(&format!("\n{}\n", "Warnings".yellow().bold()))?;
            for warning in &report.warnings {
                writer.write_text(&format!("  {}\n", warning))?;
            }
        }

        Ok(())
    }
}

fn format_status(status: &ThresholdStatus) -> String {
    match status {
        ThresholdStatus::Good => "good".green().to_string(),
        ThresholdStatus::Warning => "warning".yellow().to_string(),
        ThresholdStatus::Bad => "bad".red().to_string(),
    }
}
