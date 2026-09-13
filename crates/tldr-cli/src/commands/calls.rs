//! Calls command - Build call graph
//!
//! Builds and displays the cross-file call graph for a project.
//! Auto-routes through daemon when available for ~35x speedup.

use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Args;
use serde::{Deserialize, Serialize};

use tldr_core::callgraph::cross_file_types::CallType;
use tldr_core::callgraph::{build_project_call_graph_v2, BuildConfig};
use tldr_core::Language;

use crate::commands::daemon_router::{params_with_calls, try_daemon_route};
use crate::output::{format_calls_dot, DotCallEdge, OutputFormat, OutputWriter};

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

    /// Maximum items (edges and nodes) to include in output (default: 200)
    #[arg(long, default_value = "200")]
    pub max_items: usize,
}

/// Call graph output format
///
/// med-low-schema-cleanup-v1 (N12): the redundant `edge_count` and
/// `node_count` keys were removed. `total_edges` + `shown_edges` +
/// `truncated` is the single canonical pair (mirrors what `references`
/// and `dead` use); `node_count` was always equal to `nodes.len()` so
/// consumers can derive it locally.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct CallGraphOutput {
    root: PathBuf,
    /// Resolved language. `None` (serialized as JSON `null`) when the
    /// caller passed no `--lang` flag and `Language::from_directory`
    /// found no analyzable files (e.g. the path is an empty directory).
    ///
    /// schema-cleanup-v2 (P2.BUG-10): pre-fix the type was `Language`
    /// and the `unwrap_or(Language::Python)` autodetect fallback caused
    /// an empty directory to be reported as `language: "python"` —
    /// silently picking a default that misrepresented the input. Now
    /// the field is `Option<Language>` and the autodetect failure
    /// surfaces as JSON `null`, which downstream consumers can branch
    /// on without parsing English error strings.
    language: Option<Language>,
    nodes: Vec<String>,
    edges: Vec<EdgeOutput>,
    /// Whether the output was truncated due to max_items limit
    ///
    /// (path-and-schema-cleanup-v3 P3.BUG-N5) Always emitted — including
    /// when `false` — so schema consumers do not need to handle the
    /// absent-key case. Previously elided via `skip_serializing_if`, but
    /// downstream tooling (and `references`, `dead`, `dice`, etc.) all
    /// treat `truncated` as a stable boolean key. Set when either the edge
    /// list or the node list exceeds `max_items`.
    #[serde(default)]
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

        // Validate path exists BEFORE language detection / progress banner
        // (lang-detect-default-v1)
        if !self.path.exists() {
            anyhow::bail!("Path not found: {}", self.path.display());
        }

        // Determine language. schema-cleanup-v2 (P2.BUG-10): when the
        // caller did not pass `--lang` AND `Language::from_directory`
        // detects nothing (e.g. empty directory), preserve the absence
        // as `None` rather than silently falling back to Python — that
        // fallback caused empty-dir scans to be reported as
        // `language: "python"` with zero edges, which misrepresented
        // the input. The build path below treats `None` as Python for
        // call-graph construction (the call-graph builder requires a
        // language) but the JSON `language` field reflects the actual
        // detection result.
        let detected_language = self
            .lang
            .or_else(|| Language::from_directory(&self.path));
        let language = detected_language.unwrap_or(Language::Python);

        // Try daemon first for cached result
        if let Some(output) = try_daemon_route::<CallGraphOutput>(
            &self.path,
            "calls",
            params_with_calls(
                &self.path,
                self.lang.map(|l| l.as_str()),
                self.respect_ignore,
                self.max_items,
            ),
        ) {
            // Output based on format
            if writer.is_text() {
                let mut text = String::new();
                let lang_label = output
                    .language
                    .map(|l| l.as_str().to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                text.push_str(&format!(
                    "Call Graph for {} ({})\n",
                    output.root.display(),
                    lang_label,
                ));
                text.push_str(&format!("Edges: {}\n\n", output.total_edges));

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
            } else if writer.is_dot() {
                // surface-gaps-v1 (BUG-19): DOT support for the daemon path.
                let srcs: Vec<String> = output
                    .edges
                    .iter()
                    .map(|e| format!("{}:{}", e.src_file.display(), e.src_func))
                    .collect();
                let dsts: Vec<String> = output
                    .edges
                    .iter()
                    .map(|e| format!("{}:{}", e.dst_file.display(), e.dst_func))
                    .collect();
                let labels: Vec<String> = output
                    .edges
                    .iter()
                    .map(|e| format!("{:?}", e.call_type))
                    .collect();
                let dot_edges: Vec<DotCallEdge<'_>> = (0..output.edges.len())
                    .map(|i| DotCallEdge {
                        src: srcs[i].as_str(),
                        dst: dsts[i].as_str(),
                        label: Some(labels[i].as_str()),
                    })
                    .collect();
                let dot = format_calls_dot(&dot_edges);
                writer.write_text(&dot)?;
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

        // Build call graph (V2 canonical). Shared with the daemon `Calls`
        // handler so both serialise the identical shape — the daemon used to
        // emit a different type (`{"edges":…}` only), which failed to
        // deserialize and forced a silent local rebuild on every call.
        let output = compute_call_graph_output(
            &self.path,
            language,
            detected_language,
            self.respect_ignore,
            self.max_items,
        )?;

        // Output based on format
        if writer.is_dot() {
            // surface-gaps-v1 (BUG-19): direct-compute DOT path.
            let srcs: Vec<String> = output
                .edges
                .iter()
                .map(|e| format!("{}:{}", e.src_file.display(), e.src_func))
                .collect();
            let dsts: Vec<String> = output
                .edges
                .iter()
                .map(|e| format!("{}:{}", e.dst_file.display(), e.dst_func))
                .collect();
            let labels: Vec<String> = output
                .edges
                .iter()
                .map(|e| format!("{:?}", e.call_type))
                .collect();
            let dot_edges: Vec<DotCallEdge<'_>> = (0..output.edges.len())
                .map(|i| DotCallEdge {
                    src: srcs[i].as_str(),
                    dst: dsts[i].as_str(),
                    label: Some(labels[i].as_str()),
                })
                .collect();
            let dot = format_calls_dot(&dot_edges);
            writer.write_text(&dot)?;
            return Ok(());
        }
        if writer.is_text() {
            let mut text = String::new();
            let lang_label = detected_language
                .map(|l| l.as_str().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            text.push_str(&format!(
                "Call Graph for {} ({})\n",
                self.path.display(),
                lang_label,
            ));
            text.push_str(&format!("Edges: {}\n\n", output.total_edges));

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

/// Compute the canonical call-graph output for a project.
///
/// Shared by the CLI's direct path and the daemon `Calls` handler. The daemon
/// previously serialised a different type (`build_project_call_graph`, whose
/// JSON carries only an `edges` key), so the CLI's `try_daemon_route` could
/// never deserialize the response and silently fell back to a full local
/// rebuild on every invocation — which made `warm` useless.
pub(crate) fn compute_call_graph_output(
    path: &Path,
    build_language: Language,
    detected_language: Option<Language>,
    respect_ignore: bool,
    max_items: usize,
) -> Result<CallGraphOutput> {
    let config = BuildConfig {
        language: build_language.as_str().to_string(),
        respect_ignore,
        use_type_resolution: true,
        ..Default::default()
    };
    let ir = build_project_call_graph_v2(path, config)?;
    // Bypass compat layer - output ir.edges directly with normalized paths
    let root = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
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
    let truncated = total_edges > max_items;
    let mut edges = edges;
    if edges.len() > max_items {
        // Sort by source file + function as a simple importance metric
        edges.sort_by(|a, b| {
            let a_key = format!("{}:{}", a.src_file.display(), a.src_func);
            let b_key = format!("{}:{}", b.src_file.display(), b.src_func);
            a_key.cmp(&b_key)
        });
        edges.truncate(max_items);
    }
    let shown_edges = edges.len();

    // Build unique node set from truncated edges AND from every defined
    // function in the project (BUG-AGG12-4): including defined funcs as graph
    // nodes gives every language a faithful node count.
    let mut node_set = std::collections::BTreeSet::new();
    for edge in &edges {
        node_set.insert(format!("{}:{}", edge.src_file.display(), edge.src_func));
        node_set.insert(format!("{}:{}", edge.dst_file.display(), edge.dst_func));
    }
    for (file_path, file_ir) in &ir.files {
        let rel = file_path.strip_prefix(&root).unwrap_or(file_path);
        for func in &file_ir.funcs {
            let qualified = if let Some(class) = &func.class_name {
                format!("{}.{}", class, func.name)
            } else {
                func.name.clone()
            };
            node_set.insert(format!("{}:{}", rel.display(), qualified));
        }
    }
    let mut nodes: Vec<String> = node_set.into_iter().collect();
    // Cap nodes with the same `max_items` budget as edges. Repo-wide `calls`
    // otherwise returned every function in the project (20k+ nodes / >500k
    // tokens) because `--max-items` only limited edges, not the node list.
    let nodes_truncated = nodes.len() > max_items;
    if nodes_truncated {
        nodes.truncate(max_items);
    }

    Ok(CallGraphOutput {
        root: path.to_path_buf(),
        language: detected_language,
        nodes,
        edges,
        truncated: truncated || nodes_truncated,
        total_edges,
        shown_edges,
    })
}

/// Re-apply the `max_items` truncation to an already-built output.
///
/// Used by the daemon to cache a full graph once (for `impact`) and derive the
/// truncated `calls` shape from it without rebuilding.
pub(crate) fn truncate_output(mut output: CallGraphOutput, max_items: usize) -> CallGraphOutput {
    output.total_edges = output.edges.len();
    let edges_truncated = output.total_edges > max_items;
    if output.edges.len() > max_items {
        output.edges.sort_by(|a, b| {
            let a_key = format!("{}:{}", a.src_file.display(), a.src_func);
            let b_key = format!("{}:{}", b.src_file.display(), b.src_func);
            a_key.cmp(&b_key)
        });
        output.edges.truncate(max_items);
    }
    output.shown_edges = output.edges.len();

    let nodes_truncated = output.nodes.len() > max_items;
    if nodes_truncated {
        output.nodes.truncate(max_items);
    }
    output.truncated = edges_truncated || nodes_truncated;
    output
}

/// Rebuild the V1 call graph consumed by `impact_analysis` from a cached
/// `CallGraphOutput`, so `impact` can reuse the graph `warm`/`calls` built
/// instead of paying another ~60s cold build.
pub(crate) fn project_graph_from_output(output: &CallGraphOutput) -> tldr_core::ProjectCallGraph {
    let mut graph = tldr_core::ProjectCallGraph::new();
    for e in &output.edges {
        graph.add_edge(tldr_core::CallEdge {
            src_file: e.src_file.clone(),
            src_func: e.src_func.clone(),
            dst_file: e.dst_file.clone(),
            dst_func: e.dst_func.clone(),
        });
    }
    graph
}
