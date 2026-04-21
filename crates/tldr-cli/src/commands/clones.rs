//! Clones command - Detect code clones in a codebase
//!
//! Identifies duplicated code fragments using token-based similarity analysis.
//! Supports Type-1 (exact), Type-2 (parameterized), and Type-3 (gapped) clones.

use std::path::PathBuf;

use anyhow::Result;
use clap::Args;

use tldr_core::analysis::{
    detect_clones, CloneType, ClonesOptions, NormalizationMode,
};

use crate::output::{
    format_clones_dot, format_clones_sarif, format_clones_text, OutputFormat, OutputWriter,
};

/// Detect code clones in a codebase
#[derive(Debug, Args)]
pub struct ClonesArgs {
    /// Path to analyze (default: current directory)
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Minimum tokens for a clone (default: 25)
    #[arg(long, default_value = "25")]
    pub min_tokens: usize,

    /// Minimum lines for a clone (default: 5)
    #[arg(long, default_value = "5")]
    pub min_lines: usize,

    /// Similarity threshold (0.0-1.0, default: 0.7)
    #[arg(short = 't', long, default_value = "0.7")]
    pub threshold: f64,

    /// Filter by clone type: 1, 2, 3, or all (default: all)
    #[arg(long, default_value = "all")]
    pub type_filter: String,

    /// Normalization mode: none, identifiers, literals, all (default: all)
    #[arg(long, default_value = "all")]
    pub normalize: String,

    /// Filter by language: python, typescript, go, rust
    #[arg(long = "language")]
    pub language: Option<String>,

    /// Output format: json, text, sarif (default: json)
    /// Use sarif for IDE/CI integration (GitHub, VS Code, etc.)
    #[arg(short, long, default_value = "json")]
    pub output: String,

    /// Show clone classes (transitive grouping)
    #[arg(long)]
    pub show_classes: bool,

    /// Include clones within the same file
    #[arg(long)]
    pub include_within_file: bool,

    /// Maximum clones to report (default: 20)
    #[arg(long, default_value = "20")]
    pub max_clones: usize,

    /// Maximum files to analyze (default: 1000)
    #[arg(long, default_value = "1000")]
    pub max_files: usize,

    /// Exclude generated files (e.g., *.pb.go, *_generated.ts, vendor/, etc.)
    #[arg(long)]
    pub exclude_generated: bool,

    /// Exclude test files (e.g., test_*.py, *_test.go, *_spec.rb, tests/, __tests__/)
    #[arg(long)]
    pub exclude_tests: bool,
}

impl ClonesArgs {
    /// Run the clones command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        writer.progress(&format!("Detecting clones in {}...", self.path.display()));

        let normalization =
            NormalizationMode::parse(&self.normalize).unwrap_or(NormalizationMode::All);

        let type_filter = parse_type_filter(&self.type_filter);

        let options = ClonesOptions {
            min_tokens: self.min_tokens,
            min_lines: self.min_lines,
            threshold: self.threshold,
            type_filter,
            normalization,
            language: self.language.clone(),
            show_classes: self.show_classes,
            include_within_file: self.include_within_file,
            max_clones: self.max_clones,
            max_files: self.max_files,
            exclude_generated: self.exclude_generated,
            exclude_tests: self.exclude_tests,
        };

        let report = detect_clones(&self.path, &options)?;

        // Determine output format from argument or global format
        let effective_format = match self.output.as_str() {
            "text" => OutputFormat::Text,
            "sarif" => OutputFormat::Sarif,
            "dot" => {
                // DOT format for graph visualization
                let dot = format_clones_dot(&report);
                writer.write_text(&dot)?;
                return Ok(());
            }
            "json" => format,
            _ => format,
        };

        match effective_format {
            OutputFormat::Text => {
                let text = format_clones_text(&report);
                writer.write_text(&text)?;
            }
            OutputFormat::Sarif => {
                let sarif = format_clones_sarif(&report);
                writer.write_text(&sarif)?;
            }
            _ => {
                writer.write(&report)?;
            }
        }

        Ok(())
    }
}

/// Parse type filter string into CloneType
fn parse_type_filter(s: &str) -> Option<CloneType> {
    match s {
        "1" => Some(CloneType::Type1),
        "2" => Some(CloneType::Type2),
        "3" => Some(CloneType::Type3),
        "all" | "" => None,
        _ => None,
    }
}

// Note: format_clones_text and format_clones_dot are imported from crate::output
// They provide improved formatting with:
// - S8-P3-T5: Human-readable clone type descriptions
// - S8-P3-T7: Helpful hints for empty results
// - S8-P3-T11: DOT output with proper escaping
