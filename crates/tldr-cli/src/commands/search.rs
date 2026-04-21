//! Smart Search command - Enriched BM25 search with structure + call graph context.
//!
//! Returns enriched "search result cards" containing function-level context
//! (signature, callers, callees) for each BM25 match, minimizing round-trips
//! for LLM agents exploring a codebase.

use std::path::PathBuf;

use anyhow::Result;
use clap::Args;

use tldr_core::{enriched_search, EnrichedSearchOptions, Language, SearchMode};

use crate::output::{format_enriched_search_text, OutputFormat, OutputWriter};

/// Enriched search: BM25 search with function-level context cards
#[derive(Debug, Args)]
pub struct SmartSearchArgs {
    /// Search query (natural language or code terms)
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

        // Run enriched search
        let report = enriched_search(&self.query, &self.path, language, options)?;

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
