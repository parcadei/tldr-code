//! Dominators command - Display dominator tree and dominance frontier
//!
//! Computes and displays dominator relationships for a function's CFG:
//! - Immediate dominators (idom)
//! - Dominator tree structure
//! - Dominance frontiers
//!
//! Reference: session10-spec.md Section 4.1

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use clap::Args;
use serde::Serialize;

use tldr_core::ssa::{build_dominator_tree, compute_dominance_frontier};
use tldr_core::{get_cfg_context, Language};

use crate::output::OutputFormat;

/// Compute dominator tree and dominance frontier for a function
#[derive(Debug, Args)]
pub struct DominatorsArgs {
    /// Source file to analyze
    pub file: PathBuf,

    /// Function name to analyze
    pub function: String,

    /// Programming language (auto-detected from file extension if not specified)
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,

    /// Show only immediate dominators (no tree or frontier)
    #[arg(long)]
    pub idom_only: bool,

    /// Show only dominance frontier
    #[arg(long)]
    pub frontier_only: bool,
}

/// JSON output format matching Python v2 interface
#[derive(Debug, Serialize)]
struct DominatorsOutput {
    /// Function name
    function: String,
    /// Immediate dominators: block_id -> idom_id
    idom: HashMap<usize, usize>,
    /// Dominator tree children: block_id -> [children...]
    dom_tree: HashMap<usize, Vec<usize>>,
    /// Dominance frontier: block_id -> [frontier_blocks...]
    dom_frontier: HashMap<usize, Vec<usize>>,
    /// Entry block ID
    entry_block: usize,
}

impl DominatorsArgs {
    /// Run the dominators command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        use crate::output::OutputWriter;

        let writer = OutputWriter::new(format, quiet);

        // Determine language from file extension or argument
        let language = self.lang.unwrap_or_else(|| {
            Language::from_path(&self.file).unwrap_or(Language::Python)
        });

        writer.progress(&format!(
            "Computing dominators for {} in {}...",
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

        // Get CFG
        let cfg = get_cfg_context(
            self.file.to_str().unwrap_or_default(),
            &self.function,
            language,
        )?;

        // Build dominator tree
        let dom_tree = build_dominator_tree(&cfg)?;

        // Compute dominance frontier
        let dom_frontier = compute_dominance_frontier(&cfg, &dom_tree)?;

        // Build output structure
        let mut idom_map: HashMap<usize, usize> = HashMap::new();
        let mut tree_map: HashMap<usize, Vec<usize>> = HashMap::new();

        for (block_id, node) in &dom_tree.nodes {
            if let Some(idom) = node.idom {
                idom_map.insert(*block_id, idom);
            }
            tree_map.insert(*block_id, node.children.clone());
        }

        // Convert frontier HashSet to Vec for JSON serialization
        let frontier_map: HashMap<usize, Vec<usize>> = dom_frontier
            .frontier
            .iter()
            .map(|(k, v)| {
                let mut sorted: Vec<usize> = v.iter().copied().collect();
                sorted.sort();
                (*k, sorted)
            })
            .collect();

        let output = DominatorsOutput {
            function: dom_tree.function.clone(),
            idom: idom_map,
            dom_tree: tree_map,
            dom_frontier: frontier_map,
            entry_block: dom_tree.entry,
        };

        // Output based on format
        match format {
            OutputFormat::Text => {
                let text = format_dominators_text(&output, self.idom_only, self.frontier_only);
                writer.write_text(&text)?;
            }
            OutputFormat::Json | OutputFormat::Compact => {
                if self.idom_only {
                    // Only output idom map
                    #[derive(Serialize)]
                    struct IdomOnly {
                        function: String,
                        idom: HashMap<usize, usize>,
                        entry_block: usize,
                    }
                    writer.write(&IdomOnly {
                        function: output.function,
                        idom: output.idom,
                        entry_block: output.entry_block,
                    })?;
                } else if self.frontier_only {
                    // Only output dominance frontier
                    #[derive(Serialize)]
                    struct FrontierOnly {
                        function: String,
                        dom_frontier: HashMap<usize, Vec<usize>>,
                    }
                    writer.write(&FrontierOnly {
                        function: output.function,
                        dom_frontier: output.dom_frontier,
                    })?;
                } else {
                    writer.write(&output)?;
                }
            }
            OutputFormat::Dot => {
                let dot = format_dominators_dot(&output);
                writer.write_text(&dot)?;
            }
            OutputFormat::Sarif => {
                // SARIF not applicable, fall back to JSON
                writer.write(&output)?;
            }
        }

        Ok(())
    }
}

/// Format dominators output as human-readable text
fn format_dominators_text(output: &DominatorsOutput, idom_only: bool, frontier_only: bool) -> String {
    let mut lines = Vec::new();

    lines.push(format!("Dominator Analysis: {}", output.function));
    lines.push(format!("Entry Block: {}", output.entry_block));
    lines.push(String::new());

    if !frontier_only {
        lines.push("Immediate Dominators (idom):".to_string());
        let mut idom_items: Vec<_> = output.idom.iter().collect();
        idom_items.sort_by_key(|(k, _)| *k);
        for (block, idom) in idom_items {
            lines.push(format!("  Block {} -> idom {}", block, idom));
        }
        lines.push(String::new());
    }

    if !idom_only && !frontier_only {
        lines.push("Dominator Tree:".to_string());
        let mut tree_items: Vec<_> = output.dom_tree.iter().collect();
        tree_items.sort_by_key(|(k, _)| *k);
        for (block, children) in tree_items {
            if children.is_empty() {
                lines.push(format!("  Block {} (leaf)", block));
            } else {
                lines.push(format!("  Block {} -> children {:?}", block, children));
            }
        }
        lines.push(String::new());
    }

    if !idom_only {
        lines.push("Dominance Frontier:".to_string());
        let mut frontier_items: Vec<_> = output.dom_frontier.iter().collect();
        frontier_items.sort_by_key(|(k, _)| *k);
        for (block, frontier) in frontier_items {
            if frontier.is_empty() {
                lines.push(format!("  DF[{}] = {{}}", block));
            } else {
                lines.push(format!("  DF[{}] = {:?}", block, frontier));
            }
        }
    }

    lines.join("\n")
}

/// Format dominators as DOT graph
fn format_dominators_dot(output: &DominatorsOutput) -> String {
    let mut lines = Vec::new();

    lines.push(format!("digraph dominator_tree_{} {{", output.function));
    lines.push("  rankdir=TB;".to_string());
    lines.push("  node [shape=box];".to_string());
    lines.push(String::new());

    // Entry node
    lines.push(format!(
        "  block_{} [label=\"Block {} (entry)\" style=filled fillcolor=lightblue];",
        output.entry_block, output.entry_block
    ));

    // Other nodes
    for block_id in output.dom_tree.keys() {
        if *block_id != output.entry_block {
            lines.push(format!("  block_{} [label=\"Block {}\"];", block_id, block_id));
        }
    }

    lines.push(String::new());
    lines.push("  // Dominator tree edges (solid)".to_string());

    // Dominator tree edges
    for (block, children) in &output.dom_tree {
        for child in children {
            lines.push(format!("  block_{} -> block_{};", block, child));
        }
    }

    lines.push(String::new());
    lines.push("  // Dominance frontier edges (dashed)".to_string());

    // Dominance frontier edges (dashed)
    for (block, frontier) in &output.dom_frontier {
        for f in frontier {
            lines.push(format!(
                "  block_{} -> block_{} [style=dashed color=red];",
                block, f
            ));
        }
    }

    lines.push("}".to_string());

    lines.join("\n")
}
