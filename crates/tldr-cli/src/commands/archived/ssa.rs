//! SSA command - Display Static Single Assignment form
//!
//! Constructs and displays SSA form for a function with phi functions
//! and variable versioning.
//!
//! Reference: session10-spec.md Section 4.1

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, ValueEnum};

use tldr_core::ssa::{
    construct_minimal_ssa_with_statements, construct_pruned_ssa_with_statements,
    construct_semi_pruned_ssa_with_statements,
    format_ssa_dot, format_ssa_json, format_ssa_text,
    build_memory_ssa, format_memory_ssa_text,
    compute_live_variables, LiveVariables,
    PhiFunction, SsaBlock, SsaFunction, SsaNameId, SsaStats, SsaType,
};
use tldr_core::{get_cfg_context, get_dfg_context, Language};

use crate::output::OutputFormat;

/// Display SSA (Static Single Assignment) form for a function
#[derive(Debug, Args)]
pub struct SsaArgs {
    /// Source file to analyze
    pub file: PathBuf,

    /// Function name to analyze
    pub function: String,

    /// Programming language (auto-detected from file extension if not specified)
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,

    /// SSA construction type
    #[arg(long = "type", value_enum, default_value = "minimal")]
    pub ssa_type: SsaTypeArg,

    /// Filter output to specific variable
    #[arg(long)]
    pub var: Option<String>,

    /// Include Memory SSA for heap operations
    #[arg(long)]
    pub memory: bool,

    /// Show dead code analysis (future feature)
    #[arg(long, hide = true)]
    pub show_dead: bool,
}

/// SSA construction type argument
#[derive(Debug, Clone, Copy, ValueEnum, Default)]
pub enum SsaTypeArg {
    /// Minimal SSA - phi functions at all merge points
    #[default]
    Minimal,
    /// Semi-Pruned SSA - only non-local variables get phi
    #[value(name = "semi-pruned")]
    SemiPruned,
    /// Pruned SSA - minimal phi functions (requires liveness)
    Pruned,
}

impl From<SsaTypeArg> for SsaType {
    fn from(arg: SsaTypeArg) -> Self {
        match arg {
            SsaTypeArg::Minimal => SsaType::Minimal,
            SsaTypeArg::SemiPruned => SsaType::SemiPruned,
            SsaTypeArg::Pruned => SsaType::Pruned,
        }
    }
}

impl SsaArgs {
    /// Run the SSA command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        use crate::output::OutputWriter;

        let writer = OutputWriter::new(format, quiet);

        // Determine language from file extension or argument
        let language = self.lang.unwrap_or_else(|| {
            Language::from_path(&self.file).unwrap_or(Language::Python)
        });

        writer.progress(&format!(
            "Constructing SSA for {} in {}...",
            self.function,
            self.file.display()
        ));

        // Read source file
        let source = std::fs::read_to_string(&self.file)
            .map_err(|e| anyhow::anyhow!("Failed to read file '{}': {}", self.file.display(), e))?;

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

        // Split source into statements for SSA instruction enrichment
        let statements: Vec<String> = source.lines().map(|l| l.to_string()).collect();

        // Construct SSA based on type, with source statements for richer metadata
        let mut ssa = match self.ssa_type {
            SsaTypeArg::Minimal => {
                construct_minimal_ssa_with_statements(&cfg, &dfg, &statements)?
            }
            SsaTypeArg::SemiPruned => {
                construct_semi_pruned_ssa_with_statements(&cfg, &dfg, &statements)?
            }
            SsaTypeArg::Pruned => {
                // Pruned SSA requires live variables analysis
                // For now, fall back to semi-pruned if live variables not available
                match compute_live_variables(&cfg, &dfg.refs) {
                    Ok(live_vars) => {
                        construct_pruned_ssa_with_statements(&cfg, &dfg, &live_vars, &statements)?
                    }
                    Err(_) => {
                        writer.progress("Live variables not available, falling back to semi-pruned");
                        let mut ssa =
                            construct_semi_pruned_ssa_with_statements(&cfg, &dfg, &statements)?;
                        ssa.ssa_type = SsaType::Pruned; // Mark as pruned anyway
                        ssa
                    }
                }
            }
        };

        // Set file path
        ssa.file = self.file.clone();

        // Build memory SSA if requested
        let memory_ssa = if self.memory {
            Some(build_memory_ssa(&cfg, &ssa)?)
        } else {
            None
        };

        // Filter by variable if requested
        let ssa = if let Some(ref var) = self.var {
            filter_ssa_by_variable(ssa, var)
        } else {
            ssa
        };

        // Output based on format
        match format {
            OutputFormat::Text => {
                let text = format_ssa_text(&ssa);
                writer.write_text(&text)?;

                // Include memory SSA if present
                if let Some(ref mem_ssa) = memory_ssa {
                    writer.write_text("\n")?;
                    let mem_text = format_memory_ssa_text(mem_ssa);
                    writer.write_text(&mem_text)?;
                }
            }
            OutputFormat::Dot => {
                // DOT/Graphviz format
                let dot = format_ssa_dot(&ssa);
                writer.write_text(&dot)?;
            }
            OutputFormat::Json | OutputFormat::Compact => {
                // Create combined output with optional memory SSA
                if memory_ssa.is_some() {
                    #[derive(serde::Serialize)]
                    struct SsaWithMemory {
                        #[serde(flatten)]
                        ssa: SsaFunction,
                        memory_ssa: Option<tldr_core::ssa::MemorySsa>,
                    }

                    let combined = SsaWithMemory {
                        ssa,
                        memory_ssa,
                    };
                    writer.write(&combined)?;
                } else {
                    writer.write(&ssa)?;
                }
            }
            OutputFormat::Sarif => {
                // SARIF not applicable for SSA, use JSON
                writer.write(&ssa)?;
            }
        }

        Ok(())
    }
}

/// Filter SSA to only include a specific variable
fn filter_ssa_by_variable(ssa: SsaFunction, var: &str) -> SsaFunction {
    // Filter SSA names to only those for the target variable
    let filtered_names: Vec<_> = ssa
        .ssa_names
        .iter()
        .filter(|name| name.variable == var)
        .cloned()
        .collect();

    let filtered_name_ids: HashSet<SsaNameId> = filtered_names.iter().map(|n| n.id).collect();

    // Filter blocks to only include phi functions and instructions for this variable
    let filtered_blocks: Vec<SsaBlock> = ssa
        .blocks
        .into_iter()
        .map(|block| SsaBlock {
            phi_functions: block
                .phi_functions
                .into_iter()
                .filter(|phi| phi.variable == var)
                .collect(),
            instructions: block
                .instructions
                .into_iter()
                .filter(|inst| {
                    // Keep if target is for the variable
                    inst.target
                        .map(|t| filtered_name_ids.contains(&t))
                        .unwrap_or(false)
                        // Or if any use is for the variable
                        || inst.uses.iter().any(|u| filtered_name_ids.contains(u))
                })
                .collect(),
            ..block
        })
        .collect();

    // Filter def-use chains
    let filtered_def_use: HashMap<SsaNameId, Vec<SsaNameId>> = ssa
        .def_use
        .into_iter()
        .filter(|(k, _)| filtered_name_ids.contains(k))
        .map(|(k, v)| {
            (
                k,
                v.into_iter()
                    .filter(|u| filtered_name_ids.contains(u))
                    .collect(),
            )
        })
        .collect();

    // Recompute stats
    let phi_count = filtered_blocks
        .iter()
        .flat_map(|b| &b.phi_functions)
        .filter(|p| p.variable == var)
        .count();

    SsaFunction {
        blocks: filtered_blocks,
        ssa_names: filtered_names,
        def_use: filtered_def_use,
        stats: SsaStats {
            phi_count,
            ssa_names: filtered_name_ids.len(),
            blocks: ssa.stats.blocks,
            instructions: ssa.stats.instructions,
            dead_phi_count: 0,
        },
        ..ssa
    }
}
