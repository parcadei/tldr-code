//! Calls command - Build call graph
//!
//! Builds and displays the cross-file call graph for a project.
//! Auto-routes through daemon when available for ~35x speedup.

use std::path::PathBuf;

use anyhow::Result;
use clap::Args;
use serde::{Deserialize, Serialize};

fn is_false(v: &bool) -> bool {
    !*v
}

use tldr_core::callgraph::cross_file_types::CallType;
use tldr_core::callgraph::{build_project_call_graph_v2, BuildConfig};
use tldr_core::Language;

use crate::commands::daemon_router::{params_with_path, try_daemon_route};
use crate::output::{OutputFormat, OutputWriter};

/// Build and display cross-file call graph
#[derive(Debug, Args)]
pub struct CallsArgs {
    /// Project root directory (default: current directory)
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Programming language (auto-detected if not specified)
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,

    /// Respect .gitignore and .tldrignore patterns
    #[arg(long, default_value = "true")]
    pub respect_ignore: bool,

    /// Maximum items (edges) to include in output (default: 200)
    #[arg(long, default_value = "200")]
    pub max_items: usize,
}

/// Call graph output format
#[derive(Debug, Serialize, Deserialize)]
struct CallGraphOutput {
    root: PathBuf,
    language: Language,
    edge_count: usize,
    node_count: usize,
    nodes: Vec<String>,
    edges: Vec<EdgeOutput>,
    /// Whether the output was truncated due to max_items limit
    #[serde(skip_serializing_if = "is_false", default)]
    truncated: bool,
    /// Total number of edges before truncation
    total_edges: usize,
    /// Number of edges shown after truncation
    shown_edges: usize,
}

#[derive(Debug, Serialize, Deserialize)]
struct EdgeOutput {
    src_file: PathBuf,
    src_func: String,
    dst_file: PathBuf,
    dst_func: String,
    call_type: CallType,
}

impl CallsArgs {
    /// Run the calls command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        // Determine language (auto-detect from directory, default to Python)
        let language = self
            .lang
            .unwrap_or_else(|| Language::from_directory(&self.path).unwrap_or(Language::Python));

        // Try daemon first for cached result
        if let Some(output) = try_daemon_route::<CallGraphOutput>(
            &self.path,
            "calls",
            params_with_path(Some(&self.path)),
        ) {
            // Output based on format
            if writer.is_text() {
                let mut text = String::new();
                text.push_str(&format!(
                    "Call Graph for {} ({:?})\n",
                    output.root.display(),
                    output.language
                ));
                text.push_str(&format!("Edges: {}\n\n", output.edge_count));

                for edge in &output.edges {
                    text.push_str(&format!(
                        "{}:{} -> {}:{}\n",
                        edge.src_file.display(),
                        edge.src_func,
                        edge.dst_file.display(),
                        edge.dst_func
                    ));
                }

                writer.write_text(&text)?;
                return Ok(());
            } else {
                writer.write(&output)?;
                return Ok(());
            }
        }

        // Fallback to direct compute
        writer.progress(&format!(
            "Building call graph for {} ({:?})...",
            self.path.display(),
            language
        ));

        // Build call graph (V2 canonical)
        let config = BuildConfig {
            language: language.as_str().to_string(),
            respect_ignore: self.respect_ignore,
            use_type_resolution: true,
            ..Default::default()
        };
        let ir = build_project_call_graph_v2(&self.path, config)?;
        // Bypass compat layer - output ir.edges directly with normalized paths
        let root = self
            .path
            .canonicalize()
            .unwrap_or_else(|_| self.path.clone());
        let edges: Vec<EdgeOutput> = ir
            .edges
            .iter()
            .map(|e| {
                let src = e.src_file.strip_prefix(&root).unwrap_or(&e.src_file);
                let dst = e.dst_file.strip_prefix(&root).unwrap_or(&e.dst_file);
                EdgeOutput {
                    src_file: src.to_path_buf(),
                    src_func: e.src_func.clone(),
                    dst_file: dst.to_path_buf(),
                    dst_func: e.dst_func.clone(),
                    call_type: e.call_type,
                }
            })
            .collect();

        // Sort and truncate edges by max_items
        let total_edges = edges.len();
        let truncated = total_edges > self.max_items;
        let mut edges = edges;
        if edges.len() > self.max_items {
            // Sort by source file + function as a simple importance metric
            edges.sort_by(|a, b| {
                let a_key = format!("{}:{}", a.src_file.display(), a.src_func);
                let b_key = format!("{}:{}", b.src_file.display(), b.src_func);
                a_key.cmp(&b_key)
            });
            edges.truncate(self.max_items);
        }
        let shown_edges = edges.len();

        // Build unique node set from truncated edges
        let mut node_set = std::collections::BTreeSet::new();
        for edge in &edges {
            node_set.insert(format!("{}:{}", edge.src_file.display(), edge.src_func));
            node_set.insert(format!("{}:{}", edge.dst_file.display(), edge.dst_func));
        }
        let nodes: Vec<String> = node_set.into_iter().collect();

        let output = CallGraphOutput {
            root: self.path.clone(),
            language,
            edge_count: total_edges,
            node_count: nodes.len(),
            nodes,
            edges,
            truncated,
            total_edges,
            shown_edges,
        };

        // Output based on format
        if writer.is_text() {
            let mut text = String::new();
            text.push_str(&format!(
                "Call Graph for {} ({:?})\n",
                self.path.display(),
                language
            ));
            text.push_str(&format!("Edges: {}\n\n", output.edge_count));

            for edge in &output.edges {
                text.push_str(&format!(
                    "{}:{} -> {}:{}\n",
                    edge.src_file.display(),
                    edge.src_func,
                    edge.dst_file.display(),
                    edge.dst_func
                ));
            }

            writer.write_text(&text)?;
        } else {
            writer.write(&output)?;
        }

        Ok(())
    }
}
