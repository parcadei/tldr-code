//! Similar command - Find similar code fragments
//!
//! Finds code that is semantically similar to a given file or function.
//! Uses dense embeddings to compute similarity scores.

use std::path::PathBuf;

use anyhow::Result;
use clap::Args;

use tldr_core::semantic::{
    BuildOptions, CacheConfig, ChunkGranularity, EmbeddingModel, IndexSearchOptions, SemanticIndex,
};

use crate::output::{OutputFormat, OutputWriter};

/// Find similar code fragments
#[derive(Debug, Args)]
pub struct SimilarArgs {
    /// Source file to find similar code for
    pub file: PathBuf,

    /// Specific function name (optional, searches whole file if not specified)
    #[arg(short = 'F', long)]
    pub function: Option<String>,

    /// Maximum number of results
    #[arg(short = 'n', long, default_value = "5")]
    pub top: usize,

    /// Minimum similarity threshold
    #[arg(short = 't', long, default_value = "0.7")]
    pub threshold: f64,

    /// Path to search for similar code (default: current directory)
    #[arg(short, long, default_value = ".")]
    pub path: PathBuf,

    /// Embedding model: arctic-xs, arctic-s, arctic-m, arctic-m-long, arctic-l
    #[arg(short, long, default_value = "arctic-m")]
    pub model: String,

    /// Include self in results (by default, the query is excluded)
    #[arg(long)]
    pub include_self: bool,

    /// Disable embedding cache
    #[arg(long)]
    pub no_cache: bool,
}

impl SimilarArgs {
    /// Run the similar command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        // Parse model
        let model = parse_model(&self.model)?;

        // Canonicalize file path for matching
        let canonical_file = self
            .file
            .canonicalize()
            .unwrap_or_else(|_| self.file.clone());
        let file_str = canonical_file.display().to_string();

        // Smart search path: if --path is the default "." and the input file is
        // an absolute path, use the file's parent directory to avoid indexing the
        // entire cwd (which may be an enormous repo).
        let effective_path =
            if self.path == std::path::Path::new(".") && canonical_file.is_absolute() {
                canonical_file
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| self.path.clone())
            } else {
                self.path.clone()
            };

        writer.progress(&format!(
            "Finding code similar to {}{}...",
            self.file.display(),
            self.function
                .as_ref()
                .map(|f| format!("::{}", f))
                .unwrap_or_default()
        ));

        // Build options
        let build_opts = BuildOptions {
            model,
            granularity: ChunkGranularity::Function,
            languages: None,
            show_progress: !quiet,
            use_cache: !self.no_cache,
        };

        // Cache config
        let cache_config = if self.no_cache {
            None
        } else {
            Some(CacheConfig::default())
        };

        // Build index using effective path
        let index = SemanticIndex::build(&effective_path, build_opts, cache_config)?;

        writer.progress(&format!(
            "Searching {} chunks for similar code...",
            index.len()
        ));

        // Search options
        let search_opts = IndexSearchOptions {
            top_k: self.top,
            threshold: self.threshold,
            include_snippet: true,
            snippet_lines: 5,
        };

        // Find similar
        let report = index.find_similar(&file_str, self.function.as_deref(), &search_opts)?;

        // Output based on format
        if writer.is_text() {
            let text = format_similar_text(&report);
            writer.write_text(&text)?;
        } else {
            writer.write(&report)?;
        }

        Ok(())
    }
}

/// Parse model string into EmbeddingModel
fn parse_model(model_str: &str) -> Result<EmbeddingModel> {
    match model_str {
        "arctic-xs" | "xs" => Ok(EmbeddingModel::ArcticXS),
        "arctic-s" | "s" => Ok(EmbeddingModel::ArcticS),
        "arctic-m" | "m" => Ok(EmbeddingModel::ArcticM),
        "arctic-m-long" | "m-long" => Ok(EmbeddingModel::ArcticMLong),
        "arctic-l" | "l" => Ok(EmbeddingModel::ArcticL),
        _ => Err(anyhow::anyhow!(
            "Invalid model '{}'. Options: arctic-xs, arctic-s, arctic-m, arctic-m-long, arctic-l",
            model_str
        )),
    }
}

/// Format similarity report for text output
fn format_similar_text(report: &tldr_core::semantic::SimilarityReport) -> String {
    use colored::Colorize;

    let mut output = String::new();

    // Source info
    let source_name = report.source.function_name.as_deref().unwrap_or("<file>");
    let source_class = report
        .source
        .class_name
        .as_ref()
        .map(|c| format!("{}::", c))
        .unwrap_or_default();

    output.push_str(&format!(
        "{}: {}:{}{}\n",
        "Finding similar to".bold(),
        report.source.file_path.display().to_string().green(),
        source_class,
        source_name.blue()
    ));
    output.push_str(&format!(
        "Model: {} | Compared: {} chunks | Exclude self: {}\n\n",
        format!("{:?}", report.model).yellow(),
        report.total_compared,
        report.exclude_self
    ));

    if report.similar.is_empty() {
        output.push_str("No similar code found above threshold.\n");
    } else {
        output.push_str(&format!(
            "{} ({} found):\n\n",
            "Similar code".bold(),
            report.similar.len()
        ));

        for (i, result) in report.similar.iter().enumerate() {
            let func_name = result.function_name.as_deref().unwrap_or("<file>");
            let class_prefix = result
                .class_name
                .as_ref()
                .map(|c| format!("{}::", c))
                .unwrap_or_default();

            output.push_str(&format!(
                "{}. {}:{}{} (score: {:.2})\n",
                i + 1,
                result.file_path.display().to_string().green(),
                class_prefix,
                func_name.blue(),
                result.score
            ));
            output.push_str(&format!(
                "   Lines {}-{}\n",
                result.line_start, result.line_end
            ));

            if !result.snippet.is_empty() {
                output.push_str(&format!("   {}\n", result.snippet.dimmed()));
            }
            output.push('\n');
        }
    }

    output
}
