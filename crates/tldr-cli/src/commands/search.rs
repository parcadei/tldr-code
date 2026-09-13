//! Smart Search command - Enriched BM25 search with structure + call graph context.
//!
//! Returns enriched "search result cards" containing function-level context
//! (signature, callers, callees) for each BM25 match, minimizing round-trips
//! for LLM agents exploring a codebase.

use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Args;

use tldr_core::{
    build_project_call_graph, enriched_search, enriched_search_with_callgraph_cache,
    EnrichedSearchOptions, Language, SearchMode,
};

use crate::output::{format_enriched_search_text, OutputFormat, OutputWriter};

/// Enriched search: BM25 search with function-level context cards.
///
/// By default this command performs token-based ranking using BM25 with
/// structure and call-graph signals. Common high-frequency tokens
/// (stopwords like `fn`, `def`, `function`, `class`) are filtered out
/// of the BM25 query because they would otherwise dominate scoring
/// without adding signal.
///
/// ux-and-explain-completeness-v1 (P12.AGG12-13): when EVERY query
/// token is filtered (e.g. `fn new`, `function`, `def `), the command
/// transparently falls back to literal substring search so the query
/// still returns useful results. The report's `search_mode` field is
/// then `literal-fallback+structure` (or `+callgraph`).
///
/// Pass `--regex` to interpret the query as a regex pattern, or
/// `--hybrid <PATTERN>` to combine BM25 ranking with a regex filter.
#[derive(Debug, Args)]
pub struct SmartSearchArgs {
    /// Search query (natural language or code terms; BM25 by default,
    /// regex when `--regex` is set)
    pub query: String,

    /// Directory to search in (default: current directory)
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Programming language (auto-detect if not specified)
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,

    /// Maximum number of result cards to return
    #[arg(long, short = 'k', default_value = "10")]
    pub top_k: usize,

    /// Skip call graph enrichment (much faster, no callers/callees)
    #[arg(long)]
    pub no_callgraph: bool,

    /// Use regex pattern matching instead of BM25 ranking.
    /// The query is interpreted as a regex pattern.
    #[arg(long, conflicts_with = "hybrid")]
    pub regex: bool,

    /// Hybrid mode: combine BM25 relevance with regex filtering.
    /// The positional query is used for BM25 ranking, this pattern for regex filtering.
    #[arg(long, conflicts_with = "regex")]
    pub hybrid: Option<String>,
}

impl SmartSearchArgs {
    /// Run the search command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        // Validate path exists BEFORE language detection / progress banner
        // (lang-detect-default-v1)
        if !self.path.exists() {
            anyhow::bail!("Path not found: {}", self.path.display());
        }

        // Determine language (auto-detect from directory, default to Python)
        let language = self
            .lang
            .unwrap_or_else(|| Language::from_directory(&self.path).unwrap_or(Language::Python));

        writer.progress(&format!(
            "Smart searching for '{}' in {} ({})...",
            self.query,
            self.path.display(),
            language.as_str()
        ));

        let search_mode = if self.regex {
            SearchMode::Regex(self.query.clone())
        } else if let Some(ref pattern) = self.hybrid {
            SearchMode::Hybrid {
                query: self.query.clone(),
                pattern: pattern.clone(),
            }
        } else {
            SearchMode::Bm25
        };

        let options = EnrichedSearchOptions {
            top_k: self.top_k,
            include_callgraph: !self.no_callgraph,
            search_mode,
        };

        // Run enriched search. Call-graph enrichment costs ~60s to build, so
        // cache it on disk on first use and reuse it afterwards. The cache
        // file is the one `enriched_search_with_callgraph_cache` reads
        // (`.tldr/cache/call_graph.json`).
        let report = if options.include_callgraph {
            let cache_path = self
                .path
                .join(".tldr")
                .join("cache")
                .join("call_graph.json");
            if !cache_path.exists() {
                let _ = write_callgraph_cache(&self.path, language, &cache_path);
            }
            match enriched_search_with_callgraph_cache(
                &self.query,
                &self.path,
                language,
                options.clone(),
                &cache_path,
            ) {
                Ok(report) => report,
                // Cache missing, partial, or malformed — fall back to the
                // uncached build rather than failing the search.
                Err(_) => enriched_search(&self.query, &self.path, language, options)?,
            }
        } else {
            enriched_search(&self.query, &self.path, language, options)?
        };

        // Output based on format
        if writer.is_text() {
            let text = format_enriched_search_text(&report);
            writer.write_text(&text)?;
        } else {
            writer.write(&report)?;
        }

        Ok(())
    }
}

/// Build and persist the call-graph cache read by
/// `enriched_search_with_callgraph_cache`.
///
/// Only `from_func`/`to_func` are consumed by that reader, but the envelope
/// mirrors `warm.rs` (`from_file`/`to_file` included) so a single on-disk
/// format serves both producers.
fn write_callgraph_cache(root: &Path, language: Language, cache_path: &Path) -> Result<()> {
    let graph = build_project_call_graph(root, language, None, true)?;
    let edges: Vec<serde_json::Value> = graph
        .edges()
        .map(|e| {
            serde_json::json!({
                "from_file": e.src_file,
                "from_func": e.src_func,
                "to_file": e.dst_file,
                "to_func": e.dst_func,
            })
        })
        .collect();
    if let Some(parent) = cache_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let envelope = serde_json::json!({
        "edges": edges,
        "languages": [language.as_str()],
        "timestamp": chrono::Utc::now().timestamp(),
    });
    // Write via a same-directory temp file and rename so an interrupted write
    // can never leave a partial cache file behind for later searches to trip on.
    let tmp_path = cache_path.with_extension("json.tmp");
    std::fs::write(&tmp_path, serde_json::to_string(&envelope)?)?;
    std::fs::rename(&tmp_path, cache_path)?;
    Ok(())
}
