//! Reaching Definitions command - Display reaching definitions analysis
//!
//! Provides data flow analysis showing which variable definitions reach
//! each use point in a function.
//!
//! Reference: session10-spec.md Section 4.2

use std::path::PathBuf;

use anyhow::Result;
use clap::Args;

use tldr_core::dfg::{
    build_reaching_defs_report, filter_reaching_defs_by_variable, format_reaching_defs_json,
    format_reaching_defs_text_with_options, ReachingDefsFormatOptions, ReachingDefsReport,
};
use tldr_core::{get_cfg_context, get_dfg_context, Language};

use crate::output::OutputFormat;

/// Analyze reaching definitions for a function
#[derive(Debug, Args)]
pub struct ReachingDefsArgs {
    /// Source file to analyze
    pub file: PathBuf,

    /// Function name to analyze
    pub function: String,

    /// Programming language (auto-detected from file extension if not specified)
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,

    /// Filter output to specific variable
    #[arg(long)]
    pub var: Option<String>,

    /// Show definitions reaching specific line
    #[arg(long)]
    pub line: Option<usize>,

    /// Show def-use chains (enabled by default)
    #[arg(long, default_value = "true")]
    pub show_chains: bool,

    /// Flag potentially uninitialized uses (enabled by default)
    #[arg(long, default_value = "true")]
    pub show_uninitialized: bool,

    /// Show IN/OUT sets per block
    #[arg(long)]
    pub show_in_out: bool,

    /// Show only def-use/use-def chains, hide header, blocks, and statistics
    #[arg(long)]
    pub chains_only: bool,

    /// Function parameters (comma-separated, for uninit detection)
    #[arg(long)]
    pub params: Option<String>,
}

impl ReachingDefsArgs {
    /// Run the reaching-defs command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        use crate::output::OutputWriter;

        let writer = OutputWriter::new(format, quiet);

        // Determine language from file extension or argument
        let language = self
            .lang
            .unwrap_or_else(|| Language::from_path(&self.file).unwrap_or(Language::Python));

        writer.progress(&format!(
            "Analyzing reaching definitions for {} in {}...",
            self.function,
            self.file.display()
        ));

        // Read source file - ensure it exists
        if !self.file.exists() {
            return Err(anyhow::anyhow!("File not found: {}", self.file.display()));
        }

        // Get CFG and DFG
        let cfg = get_cfg_context(
            self.file.to_str().unwrap_or_default(),
            &self.function,
            language,
        )?;

        let dfg = get_dfg_context(
            self.file.to_str().unwrap_or_default(),
            &self.function,
            language,
        )?;

        // Build the report
        let mut report = build_reaching_defs_report(&cfg, &dfg.refs, self.file.clone());

        // Filter by variable if requested
        if let Some(ref var) = self.var {
            report = filter_reaching_defs_by_variable(&report, var);
        }

        // Filter by line if requested
        if let Some(line) = self.line {
            report = filter_report_by_line(&report, line as u32);
        }

        // Build format options from CLI flags
        // --chains-only: show ONLY chains (no header, no blocks, no stats)
        // --show-in-out: show per-block GEN/KILL/IN/OUT details
        // Default (neither flag): header + chains + stats (no blocks)
        // Both flags: chains_only takes precedence
        let format_options = if self.chains_only {
            ReachingDefsFormatOptions::chains_only()
        } else {
            ReachingDefsFormatOptions {
                show_blocks: self.show_in_out,
                show_chains: self.show_chains,
                show_uninitialized: self.show_uninitialized,
                show_header: true,
                show_stats: true,
            }
        };

        // Output based on format
        match format {
            OutputFormat::Text => {
                let text = format_reaching_defs_text_with_options(&report, &format_options);
                writer.write_text(&text)?;
            }
            OutputFormat::Json | OutputFormat::Compact => {
                let json = format_reaching_defs_json(&report)
                    .map_err(|e| anyhow::anyhow!("JSON serialization failed: {}", e))?;
                writer.write_text(&json)?;
            }
            OutputFormat::Dot => {
                // DOT not supported for reaching defs, fall back to JSON
                let json = format_reaching_defs_json(&report)
                    .map_err(|e| anyhow::anyhow!("JSON serialization failed: {}", e))?;
                writer.write_text(&json)?;
            }
            OutputFormat::Sarif => {
                // SARIF not supported, fall back to JSON
                let json = format_reaching_defs_json(&report)
                    .map_err(|e| anyhow::anyhow!("JSON serialization failed: {}", e))?;
                writer.write_text(&json)?;
            }
        }

        Ok(())
    }
}

/// Filter the report to show only definitions reaching a specific line.
///
/// This shows:
/// - Use-def chains where the use is at the specified line
/// - Def-use chains where the definition reaches the specified line
fn filter_report_by_line(report: &ReachingDefsReport, line: u32) -> ReachingDefsReport {
    use std::collections::HashSet;

    // Find use-def chains where the use is at this line
    let relevant_use_def_chains: Vec<_> = report
        .use_def_chains
        .iter()
        .filter(|c| c.use_site.line == line)
        .cloned()
        .collect();

    // Collect all definition lines from relevant use-def chains
    let relevant_def_lines: HashSet<u32> = relevant_use_def_chains
        .iter()
        .flat_map(|c| c.reaching_defs.iter().map(|d| d.line))
        .collect();

    // Filter def-use chains to those relevant definitions
    let relevant_def_use_chains: Vec<_> = report
        .def_use_chains
        .iter()
        .filter(|c| relevant_def_lines.contains(&c.definition.line))
        .cloned()
        .collect();

    // Filter uninitialized uses to this line
    let relevant_uninitialized: Vec<_> = report
        .uninitialized
        .iter()
        .filter(|u| u.line == line)
        .cloned()
        .collect();

    ReachingDefsReport {
        function: report.function.clone(),
        file: report.file.clone(),
        blocks: report.blocks.clone(), // Keep all blocks for context
        def_use_chains: relevant_def_use_chains,
        use_def_chains: relevant_use_def_chains,
        uninitialized: relevant_uninitialized,
        stats: report.stats.clone(),
        uncertain_defs: report.uncertain_defs.clone(),
        confidence: report.confidence,
    }
}
