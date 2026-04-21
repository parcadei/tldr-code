//! Whatbreaks command - unified impact analysis wrapper
//!
//! Auto-detects target type (function/file/module) and runs appropriate
//! sub-analyses to answer: "What breaks if I change X?"
//!
//! # Sub-Analyses by Target Type
//!
//! - **Function**: Runs `impact` analysis to find callers
//! - **File**: Runs `importers` + `change-impact` analysis
//! - **Module**: Runs `importers` analysis
//!
//! # Premortem Mitigations
//! - T14: CLI registration follows existing pattern
//! - T15: --type flag for disambiguation
//! - T18: Text formatting follows spec style guide

use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, ValueEnum};

use tldr_core::analysis::whatbreaks::{
    whatbreaks_analysis, TargetType, WhatbreaksOptions,
};
use tldr_core::Language;

use crate::output::{format_whatbreaks_text, OutputFormat, OutputWriter};

/// Target type selection for CLI (T15 mitigation)
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum TargetTypeArg {
    /// Function name - run impact analysis
    Function,
    /// File path - run importers + change-impact
    File,
    /// Module name - run importers
    Module,
}

impl From<TargetTypeArg> for TargetType {
    fn from(arg: TargetTypeArg) -> Self {
        match arg {
            TargetTypeArg::Function => TargetType::Function,
            TargetTypeArg::File => TargetType::File,
            TargetTypeArg::Module => TargetType::Module,
        }
    }
}

/// Analyze what breaks if a target is changed
///
/// Automatically detects whether target is a function, file, or module
/// and runs appropriate sub-analyses.
#[derive(Debug, Args)]
pub struct WhatbreaksArgs {
    /// Target to analyze (function name, file path, or module name)
    pub target: String,

    /// Project root directory (default: current directory)
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Force target type (overrides auto-detection)
    #[arg(long = "type", short = 't', value_enum)]
    pub target_type: Option<TargetTypeArg>,

    /// Maximum depth for impact/caller traversal
    #[arg(long, short = 'd', default_value = "3")]
    pub depth: usize,

    /// Skip slow analyses (diff-impact)
    #[arg(long)]
    pub quick: bool,

    /// Programming language (auto-detect if not specified)
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,
}

impl WhatbreaksArgs {
    /// Run the whatbreaks command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        // Validate path exists
        if !self.path.exists() {
            anyhow::bail!("Path not found: {}", self.path.display());
        }

        writer.progress(&format!(
            "Analyzing what breaks if '{}' changes...",
            self.target
        ));

        // Build options
        let options = WhatbreaksOptions {
            depth: self.depth,
            quick: self.quick,
            language: self.lang,
            force_type: self.target_type.map(|t| t.into()),
        };

        // Run analysis
        let report = whatbreaks_analysis(&self.target, &self.path, &options)?;

        writer.progress(&format!(
            "Target type: {} ({})",
            report.target_type, report.detection_reason
        ));

        // Output based on format
        if writer.is_text() {
            let text = format_whatbreaks_text(&report);
            writer.write_text(&text)?;
        } else {
            writer.write(&report)?;
        }

        Ok(())
    }
}
