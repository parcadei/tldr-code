//! Patterns command - Detect design patterns and coding conventions
//!
//! Analyzes a codebase to detect patterns including:
//! - Soft delete patterns
//! - Error handling patterns
//! - Naming conventions
//! - Resource management patterns
//! - Validation patterns
//! - Test idioms
//! - Import patterns
//! - Type coverage
//! - API conventions
//! - Async patterns

use std::path::PathBuf;

use anyhow::Result;
use clap::Args;

use tldr_core::patterns::format::format_pattern_report_text;
use tldr_core::{detect_patterns_with_config, Language, PatternCategory, PatternConfig};

use crate::output::{OutputFormat, OutputWriter};

/// Detect design patterns and coding conventions
#[derive(Debug, Args)]
pub struct PatternsArgs {
    /// Path to file or directory to analyze (default: current directory)
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Programming language (auto-detect if not specified)
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,

    /// Filter to specific pattern category
    #[arg(long, short = 'c', value_parser = parse_category)]
    pub category: Option<PatternCategory>,

    /// Minimum confidence threshold (0.0-1.0)
    #[arg(long, default_value = "0.5")]
    pub min_confidence: f64,

    /// Maximum files to analyze (0 = unlimited)
    #[arg(long, default_value = "1000")]
    pub max_files: usize,

    /// Skip LLM constraint generation
    #[arg(long)]
    pub no_constraints: bool,
}

fn parse_category(s: &str) -> Result<PatternCategory, String> {
    s.parse()
}

impl PatternsArgs {
    /// Run the patterns command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        writer.progress(&format!("Analyzing patterns in {}...", self.path.display()));

        // Build config
        let config = PatternConfig {
            min_confidence: self.min_confidence,
            max_files: self.max_files,
            evidence_limit: 3,
            categories: self.category.map(|c| vec![c]).unwrap_or_default(),
            generate_constraints: !self.no_constraints,
        };

        // Run pattern detection
        let report = detect_patterns_with_config(&self.path, self.lang, config)?;

        // Output based on format
        if writer.is_text() {
            let text = format_pattern_report_text(&report);
            writer.write_text(&text)?;
        } else {
            writer.write(&report)?;
        }

        Ok(())
    }
}
