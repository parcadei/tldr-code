//! Live Variables command - Display live variable analysis
//!
//! Computes live variables at each program point using backward dataflow:
//! - live_in: Variables live at block entry
//! - live_out: Variables live at block exit
//!
//! Reference: session10-spec.md Section 4.2

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use clap::Args;
use serde::Serialize;

use tldr_core::ssa::compute_live_variables;
use tldr_core::{get_cfg_context, get_dfg_context, Language};

use crate::output::OutputFormat;

/// Analyze live variables for a function
#[derive(Debug, Args)]
pub struct LiveVarsArgs {
    /// Source file to analyze
    pub file: PathBuf,

    /// Function name to analyze
    pub function: String,

    /// Programming language (auto-detected from file extension if not specified)
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,

    /// Filter to specific variable
    #[arg(long)]
    pub var: Option<String>,

    /// Show only blocks where variable is live
    #[arg(long)]
    pub live_only: bool,
}

/// Block live sets for JSON output
#[derive(Debug, Serialize)]
struct BlockLiveSets {
    /// Variables live at block entry
    live_in: Vec<String>,
    /// Variables live at block exit
    live_out: Vec<String>,
}

/// JSON output format matching Python v2 interface
#[derive(Debug, Serialize)]
struct LiveVarsOutput {
    /// Function name
    function: String,
    /// Live sets per block: block_id -> {live_in, live_out}
    blocks: HashMap<String, BlockLiveSets>,
}

impl LiveVarsArgs {
    /// Run the live-vars command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        use crate::output::OutputWriter;

        let writer = OutputWriter::new(format, quiet);

        // Determine language from file extension or argument
        let language = self.lang.unwrap_or_else(|| {
            Language::from_path(&self.file).unwrap_or(Language::Python)
        });

        writer.progress(&format!(
            "Analyzing live variables for {} in {}...",
            self.function,
            self.file.display()
        ));

        // Ensure file exists
        if !self.file.exists() {
            return Err(anyhow::anyhow!(
                "File not found: {}",
                self.file.display()
            ));
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

        // Compute live variables
        let live_vars = compute_live_variables(&cfg, &dfg.refs)?;

        // Build output structure
        let mut blocks_output: HashMap<String, BlockLiveSets> = HashMap::new();

        for (block_id, live_sets) in &live_vars.blocks {
            // Convert HashSets to sorted Vecs for deterministic output
            let mut live_in: Vec<String> = live_sets.live_in.iter().cloned().collect();
            let mut live_out: Vec<String> = live_sets.live_out.iter().cloned().collect();
            live_in.sort();
            live_out.sort();

            // Filter by variable if requested
            if let Some(ref var) = self.var {
                let has_var = live_in.contains(var) || live_out.contains(var);
                if self.live_only && !has_var {
                    continue;
                }
                // If filtering by variable, only show that variable
                live_in.retain(|v| v == var);
                live_out.retain(|v| v == var);
            }

            // Skip empty blocks if live_only is set
            if self.live_only && live_in.is_empty() && live_out.is_empty() {
                continue;
            }

            blocks_output.insert(
                block_id.to_string(),
                BlockLiveSets { live_in, live_out },
            );
        }

        let output = LiveVarsOutput {
            function: live_vars.function.clone(),
            blocks: blocks_output,
        };

        // Output based on format
        match format {
            OutputFormat::Text => {
                let text = format_live_vars_text(&output, self.var.as_deref());
                writer.write_text(&text)?;
            }
            OutputFormat::Json | OutputFormat::Compact => {
                writer.write(&output)?;
            }
            OutputFormat::Dot => {
                // DOT not particularly useful for live variables, use JSON
                writer.write(&output)?;
            }
            OutputFormat::Sarif => {
                // SARIF not applicable, fall back to JSON
                writer.write(&output)?;
            }
        }

        Ok(())
    }
}

/// Format live variables output as human-readable text
fn format_live_vars_text(output: &LiveVarsOutput, var_filter: Option<&str>) -> String {
    let mut lines = Vec::new();

    lines.push(format!("Live Variables Analysis: {}", output.function));
    if let Some(var) = var_filter {
        lines.push(format!("Filtered to variable: {}", var));
    }
    lines.push(String::new());

    // Sort blocks by ID for consistent output
    let mut block_ids: Vec<_> = output.blocks.keys().collect();
    block_ids.sort_by_key(|k| k.parse::<usize>().unwrap_or(0));

    for block_id in block_ids {
        let sets = &output.blocks[block_id];
        lines.push(format!("Block {}:", block_id));

        if sets.live_in.is_empty() {
            lines.push("  live_in:  {}".to_string());
        } else {
            lines.push(format!("  live_in:  {{{}}}", sets.live_in.join(", ")));
        }

        if sets.live_out.is_empty() {
            lines.push("  live_out: {}".to_string());
        } else {
            lines.push(format!("  live_out: {{{}}}", sets.live_out.join(", ")));
        }

        lines.push(String::new());
    }

    lines.join("\n")
}
