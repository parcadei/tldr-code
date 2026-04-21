//! Embed command - Generate embeddings for code
//!
//! Generates dense embeddings for code chunks using Snowflake Arctic models.
//! Supports file-level or function-level granularity with optional caching.

use std::path::PathBuf;
use std::time::Instant;

use anyhow::Result;
use clap::Args;

use tldr_core::semantic::{
    chunk_code, CacheConfig, ChunkGranularity, ChunkOptions, EmbedReport, EmbeddedChunk, Embedder,
    EmbeddingCache, EmbeddingModel,
};

use crate::output::{OutputFormat, OutputWriter};

/// Generate embeddings for code
#[derive(Debug, Args)]
pub struct EmbedArgs {
    /// Path to file or directory to embed
    pub path: PathBuf,

    /// Output file (JSON). If not specified, prints to stdout
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Chunking granularity: "file" or "function"
    #[arg(short, long, default_value = "function")]
    pub granularity: String,

    /// Embedding model: arctic-xs, arctic-s, arctic-m, arctic-m-long, arctic-l
    #[arg(short, long, default_value = "arctic-m")]
    pub model: String,

    /// Filter by language (e.g., rust, python)
    #[arg(long)]
    pub lang: Option<Vec<String>>,

    /// Include embedding vectors in output
    #[arg(long)]
    pub include_vectors: bool,

    /// Disable embedding cache
    #[arg(long)]
    pub no_cache: bool,
}

impl EmbedArgs {
    /// Run the embed command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);
        let start = Instant::now();

        // Parse model
        let model = parse_model(&self.model)?;

        // Parse granularity
        let granularity = match self.granularity.as_str() {
            "file" => ChunkGranularity::File,
            "function" => ChunkGranularity::Function,
            _ => {
                return Err(anyhow::anyhow!(
                    "Invalid granularity '{}'. Use 'file' or 'function'.",
                    self.granularity
                ))
            }
        };

        writer.progress(&format!(
            "Embedding code in {} ({:?} granularity, {} model)...",
            self.path.display(),
            granularity,
            self.model
        ));

        // Convert language filters
        let languages = self.lang.as_ref().map(|langs| {
            langs
                .iter()
                .filter_map(|s| tldr_core::Language::from_extension(s))
                .collect()
        });

        // Chunk the code
        let chunk_opts = ChunkOptions {
            granularity,
            languages,
            ..Default::default()
        };

        let chunk_result = chunk_code(&self.path, &chunk_opts)?;

        writer.progress(&format!(
            "Found {} chunks, generating embeddings...",
            chunk_result.chunks.len()
        ));

        // Initialize cache (before embedder — skip ONNX load on 100% cache hit)
        let mut cache = if self.no_cache {
            None
        } else {
            Some(EmbeddingCache::open(CacheConfig::default())?)
        };

        let mut cache_hits = 0usize;
        let mut cache_misses = 0usize;
        let mut embedded_chunks: Vec<EmbeddedChunk> = Vec::with_capacity(chunk_result.chunks.len());

        // Phase 1: Separate cached vs uncached chunks
        let mut uncached_indices: Vec<usize> = Vec::new();

        for (i, chunk) in chunk_result.chunks.iter().enumerate() {
            if let Some(ref mut c) = cache {
                if let Some(e) = c.get(chunk, model) {
                    cache_hits += 1;
                    embedded_chunks.push(EmbeddedChunk {
                        chunk: chunk.clone(),
                        embedding: e,
                    });
                    continue;
                }
            }
            cache_misses += 1;
            // Store a placeholder; we'll fill the embedding after batch
            embedded_chunks.push(EmbeddedChunk {
                chunk: chunk.clone(),
                embedding: Vec::new(),
            });
            uncached_indices.push(i);
        }

        // Phase 2: Batch embed all uncached chunks at once (lazy model init)
        if !uncached_indices.is_empty() {
            let mut embedder = Embedder::new(model)?;
            let texts: Vec<&str> = uncached_indices
                .iter()
                .map(|&i| chunk_result.chunks[i].content.as_str())
                .collect();
            let embeddings = embedder.embed_batch(texts, true)?;

            for (idx, embedding) in uncached_indices.iter().zip(embeddings) {
                if let Some(ref mut c) = cache {
                    c.put(&chunk_result.chunks[*idx], embedding.clone(), model);
                }
                embedded_chunks[*idx].embedding = embedding;
            }
        }

        // Flush cache
        if let Some(ref mut c) = cache {
            c.flush()?;
        }

        let latency_ms = start.elapsed().as_millis() as u64;

        // Build report
        let report = EmbedReport {
            path: self.path.clone(),
            model,
            granularity,
            chunks_embedded: cache_misses,
            chunks_cached: cache_hits,
            chunks: if self.include_vectors {
                Some(embedded_chunks)
            } else {
                None
            },
            latency_ms,
        };

        writer.progress(&format!(
            "Embedded {} chunks ({} cached, {} new) in {}ms",
            cache_hits + cache_misses,
            cache_hits,
            cache_misses,
            latency_ms
        ));

        // Output based on format
        if let Some(ref output_path) = self.output {
            let file = std::fs::File::create(output_path)?;
            serde_json::to_writer_pretty(file, &report)?;
            writer.progress(&format!("Output written to {}", output_path.display()));
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
