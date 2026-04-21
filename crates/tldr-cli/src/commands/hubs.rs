//! Hubs command - Detect high-centrality hub functions
//!
//! Identifies "hub" functions that are change amplifiers - modifications to them
//! affect many other parts of the codebase. Uses graph centrality algorithms
//! to quantify risk.
//!
//! # Algorithms
//!
//! - `in_degree`: How many functions call this one (dependencies)
//! - `out_degree`: How many functions this one calls (complexity)
//! - `pagerank`: Recursive importance based on caller importance
//! - `betweenness`: How often this lies on shortest paths (bottleneck)
//!
//! # Premortem Mitigations
//! - T14: CLI registration follows existing pattern
//! - T16: Small graph (<10 nodes) messaging
//! - T18: Text formatting follows spec style guide

use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, ValueEnum};

use tldr_core::analysis::hubs::{compute_hub_report, HubAlgorithm};
use tldr_core::callgraph::{build_forward_graph, build_reverse_graph, collect_nodes};
use tldr_core::{build_project_call_graph, Language};

use crate::output::{format_hubs_text, OutputFormat, OutputWriter};

/// Algorithm selection for CLI (mirrors HubAlgorithm)
#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub enum AlgorithmArg {
    /// All algorithms: in_degree, out_degree, pagerank, betweenness
    #[default]
    All,
    /// In-degree only (fast)
    Indegree,
    /// Out-degree only (fast)
    Outdegree,
    /// PageRank only
    Pagerank,
    /// Betweenness only (slow for large graphs)
    Betweenness,
}

impl From<AlgorithmArg> for HubAlgorithm {
    fn from(arg: AlgorithmArg) -> Self {
        match arg {
            AlgorithmArg::All => HubAlgorithm::All,
            AlgorithmArg::Indegree => HubAlgorithm::InDegree,
            AlgorithmArg::Outdegree => HubAlgorithm::OutDegree,
            AlgorithmArg::Pagerank => HubAlgorithm::PageRank,
            AlgorithmArg::Betweenness => HubAlgorithm::Betweenness,
        }
    }
}

/// Detect hub functions using centrality analysis
#[derive(Debug, Args)]
pub struct HubsArgs {
    /// Project root directory (default: current directory)
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Number of top hubs to return
    #[arg(long, default_value = "10")]
    pub top: usize,

    /// Centrality algorithm to use
    #[arg(long, value_enum, default_value = "all")]
    pub algorithm: AlgorithmArg,

    /// Minimum composite score threshold (0.0-1.0)
    #[arg(long)]
    pub threshold: Option<f64>,

    /// Programming language (auto-detect if not specified)
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,
}

impl HubsArgs {
    /// Run the hubs command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        // Validate path exists
        if !self.path.exists() {
            anyhow::bail!("Path not found: {}", self.path.display());
        }

        // Validate threshold if provided
        if let Some(thresh) = self.threshold {
            if !(0.0..=1.0).contains(&thresh) {
                anyhow::bail!("Threshold must be between 0.0 and 1.0, got {}", thresh);
            }
        }

        // Determine language (auto-detect from directory, default to Python)
        let language = self
            .lang
            .unwrap_or_else(|| Language::from_directory(&self.path).unwrap_or(Language::Python));

        writer.progress(&format!(
            "Building call graph for {} ({:?})...",
            self.path.display(),
            language
        ));

        // Build call graph
        let graph = build_project_call_graph(&self.path, language, None, true)?;

        writer.progress("Computing hub centrality metrics...");

        // Build graph representations
        let forward = build_forward_graph(&graph);
        let reverse = build_reverse_graph(&graph);
        let nodes = collect_nodes(&graph);

        // Compute hub report
        let report = compute_hub_report(
            &nodes,
            &forward,
            &reverse,
            self.algorithm.into(),
            self.top,
            self.threshold,
        );

        // Output based on format
        if writer.is_text() {
            let text = format_hubs_text(&report);
            writer.write_text(&text)?;
        } else {
            writer.write(&report)?;
        }

        Ok(())
    }
}
