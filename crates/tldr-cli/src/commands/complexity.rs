//! Complexity command - Calculate function complexity metrics
//!
//! Returns ComplexityMetrics with cyclomatic, cognitive, nesting_depth, and lines_of_code.
//! Auto-routes through daemon when available for ~35x speedup.

use std::path::PathBuf;

use anyhow::Result;
use clap::Args;

use tldr_core::types::ComplexityMetrics;
use tldr_core::{calculate_complexity, detect_or_parse_language, validate_file_path, Language};

use crate::commands::daemon_router::{params_with_file_function, try_daemon_route};
use crate::output::{format_complexity_text, OutputFormat, OutputWriter};

/// Calculate complexity metrics for a function
#[derive(Debug, Args)]
pub struct ComplexityArgs {
    /// file containing the function
    pub file: PathBuf,

    /// Function name to analyze
    pub function: String,

    /// Programming language (auto-detect if not specified)
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,
}

impl ComplexityArgs {
    /// Run the complexity command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        // Validate file path exists (M28: shared validator - returns PathNotFound error)
        let validated_path = validate_file_path(self.file.to_str().unwrap_or_default(), None)?;

        // Try daemon first for cached result (use file's parent as project root)
        let project = validated_path.parent().unwrap_or(&validated_path);
        if let Some(result) = try_daemon_route::<ComplexityMetrics>(
            project,
            "complexity",
            params_with_file_function(&validated_path, &self.function),
        ) {
            if writer.is_text() {
                writer.write_text(&format_complexity_text(&result))?;
            } else {
                writer.write(&result)?;
            }
            return Ok(());
        }

        // Fallback to direct compute

        // Detect or parse language (uses shared validator M28)
        let language =
            detect_or_parse_language(self.lang.as_ref().map(|l| l.as_str()), &validated_path)?;

        writer.progress(&format!(
            "Calculating complexity for {} in {} ({:?})...",
            self.function,
            validated_path.display(),
            language
        ));

        // Calculate complexity - the function takes file path as string
        let result = calculate_complexity(
            validated_path.to_str().unwrap_or_default(),
            &self.function,
            language,
        )?;

        // Output based on format
        if writer.is_text() {
            writer.write_text(&format_complexity_text(&result))?;
        } else {
            writer.write(&result)?;
        }

        Ok(())
    }
}
