//! Maintainability command - Calculate Maintainability Index
//!
//! Returns MaintainabilityReport with MI scores per file and summary.
//! Auto-routes through daemon when available for ~35x speedup.

use std::path::PathBuf;

use anyhow::Result;
use clap::Args;

use tldr_core::quality::maintainability::{maintainability_index, MaintainabilityReport};
use tldr_core::{detect_or_parse_language, validate_file_path, Language};

use crate::commands::daemon_router::{params_with_path, try_daemon_route};
use crate::output::{format_maintainability_text, OutputFormat, OutputWriter};

/// Calculate Maintainability Index for files
#[derive(Debug, Args)]
pub struct MaintainabilityArgs {
    /// File or directory to analyze
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Include detailed Halstead metrics in output
    #[arg(long, short = 'H')]
    pub halstead: bool,

    /// Programming language filter (auto-detect if not specified)
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,
}

impl MaintainabilityArgs {
    /// Run the maintainability command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        // Validate path exists (M28: shared validator)
        let validated_path = validate_file_path(
            self.path.to_str().unwrap_or_default(),
            None,
        )?;

        // Try daemon first for cached result
        if let Some(report) = try_daemon_route::<MaintainabilityReport>(
            &validated_path,
            "maintainability",
            params_with_path(Some(&validated_path)),
        ) {
            // Output summary in progress
            writer.progress(&format!(
                "Analyzed {} files, average MI: {:.1} (grade: {})",
                report.summary.files_analyzed,
                report.summary.average_mi,
                grade_from_mi(report.summary.average_mi)
            ));
            if writer.is_text() {
                writer.write_text(&format_maintainability_text(&report))?;
            } else {
                writer.write(&report)?;
            }
            return Ok(());
        }

        // Fallback to direct compute

        // Get language filter if specified
        let language = if let Some(ref lang) = self.lang {
            Some(detect_or_parse_language(
                Some(lang.as_str()),
                &validated_path,
            )?)
        } else {
            None
        };

        writer.progress(&format!(
            "Calculating Maintainability Index for {}...",
            validated_path.display()
        ));

        // Calculate MI
        let report = maintainability_index(&validated_path, self.halstead, language)?;

        // Output summary in progress
        writer.progress(&format!(
            "Analyzed {} files, average MI: {:.1} (grade: {})",
            report.summary.files_analyzed,
            report.summary.average_mi,
            grade_from_mi(report.summary.average_mi)
        ));

        // Output based on format
        if writer.is_text() {
            writer.write_text(&format_maintainability_text(&report))?;
        } else {
            writer.write(&report)?;
        }

        Ok(())
    }
}

/// Get letter grade from MI score
fn grade_from_mi(mi: f64) -> char {
    if mi > 85.0 {
        'A'
    } else if mi > 65.0 {
        'B'
    } else if mi > 45.0 {
        'C'
    } else if mi > 25.0 {
        'D'
    } else {
        'F'
    }
}
