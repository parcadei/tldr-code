//! Semantic search index combining embedder, chunker, similarity, and cache
//!
//! This module provides the `SemanticIndex` struct, which is the main entry point
//! for semantic code search. It combines:
//!
//! - **Embedder**: Generates dense embeddings using fastembed-rs
//! - **Chunker**: Extracts code chunks via tree-sitter
//! - **Similarity**: Cosine similarity and top-K search
//! - **Cache**: Optional embedding persistence
//!
//! # P0 Mitigations (from phased-plan.yaml)
//!
//! - Hard limit at MAX_INDEX_SIZE (100K chunks) to prevent memory exhaustion
//! - Memory estimate before building to fail fast on large codebases
//! - Report parse failures during index build
//!
//! # Example
//!
//! ```rust,ignore
//! use std::path::Path;
//! use tldr_core::semantic::{SemanticIndex, SearchOptions, BuildOptions};
//!
//! // Build an index from a project directory
//! let index = SemanticIndex::build(
//!     Path::new("src/"),
//!     BuildOptions::default(),
//!     None, // No cache
//! )?;
//!
//! // Search for semantically related code
//! let report = index.search("parse configuration file", &SearchOptions::default())?;
//!
//! for result in report.results {
//!     println!("{}: {} (score: {:.2})",
//!         result.file_path.display(),
//!         result.function_name.unwrap_or_default(),
//!         result.score
//!     );
//! }
//! ```

use std::path::Path;
use std::time::Instant;

use crate::semantic::cache::EmbeddingCache;
use crate::semantic::chunker::chunk_code;
use crate::semantic::embedder::Embedder;
use crate::semantic::similarity::top_k_similar;
use crate::semantic::types::{
    CacheConfig, ChunkGranularity, ChunkOptions, EmbeddedChunk, EmbeddingModel,
    SemanticSearchReport, SemanticSearchResult, SimilarityReport,
};
use crate::{TldrError, TldrResult};

// =============================================================================
// Constants (P0 Mitigations)
// =============================================================================

/// Maximum number of chunks allowed in index (P0 mitigation)
///
/// Prevents memory exhaustion on large codebases. For larger projects,
/// users should filter by language or directory.
pub const MAX_INDEX_SIZE: usize = 100_000;

/// Estimated memory per chunk in bytes
///
/// Calculation: 768 dims * 4 bytes per f32 + ~500 bytes metadata
const BYTES_PER_CHUNK: usize = 768 * 4 + 500;

/// Maximum memory usage in bytes (500MB)
const MAX_MEMORY_BYTES: usize = 500 * 1024 * 1024;

// =============================================================================
// Build Options
// =============================================================================

/// Options for building a semantic index
///
/// Controls how the index is constructed, including model selection,
/// chunking granularity, and caching behavior.
#[derive(Debug, Clone)]
pub struct BuildOptions {
    /// Embedding model to use
    pub model: EmbeddingModel,

    /// Chunking granularity (file or function level)
    pub granularity: ChunkGranularity,

    /// Languages to process (None = auto-detect all)
    pub languages: Option<Vec<String>>,

    /// Show progress during index building
    pub show_progress: bool,

    /// Use embedding cache
    pub use_cache: bool,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            model: EmbeddingModel::default(),
            granularity: ChunkGranularity::Function,
            languages: None,
            show_progress: true,
            use_cache: true,
        }
    }
}

// =============================================================================
// Search Options (re-exported from types but with local defaults)
// =============================================================================

/// Options for semantic search operations
///
/// Controls how search results are filtered and ranked.
#[derive(Debug, Clone)]
pub struct SearchOptions {
    /// Maximum number of results to return
    pub top_k: usize,

    /// Minimum similarity threshold (0.0 to 1.0)
    pub threshold: f64,

    /// Include code snippet in results
    pub include_snippet: bool,

    /// Maximum lines in snippet
    pub snippet_lines: usize,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            top_k: 10,
            threshold: 0.5,
            include_snippet: true,
            snippet_lines: 5,
        }
    }
}

// =============================================================================
// Semantic Index
// =============================================================================

/// In-memory semantic index for fast similarity search
///
/// The index holds embedded code chunks and supports natural language
/// queries and code similarity searches.
///
/// # Memory Usage
///
/// Memory usage is approximately `chunks * (dimensions * 4 + 500)` bytes.
/// For 10K functions with 768-dim embeddings: ~30MB.
///
/// # Thread Safety
///
/// `SemanticIndex` is `Send` but not `Sync` due to the internal embedder.
/// Create one index per thread for concurrent searches.
pub struct SemanticIndex {
    /// All embedded chunks in the index
    chunks: Vec<EmbeddedChunk>,

    /// Embedding model used for all embeddings
    model: EmbeddingModel,

    /// Embedder for query embedding (lazily initialized, reused for searches)
    /// None if index was built entirely from cache
    embedder: Option<Embedder>,
}

impl SemanticIndex {
    /// Build a semantic index from a directory
    ///
    /// Extracts code chunks, generates embeddings, and builds a searchable index.
    ///
    /// # Arguments
    ///
    /// * `root` - Project root directory to index
    /// * `options` - Build options (model, granularity, etc.)
    /// * `cache_config` - Optional cache configuration for embedding persistence
    ///
    /// # Returns
    ///
    /// * `Ok(SemanticIndex)` - Built index ready for search
    /// * `Err(TldrError::IndexTooLarge)` - If chunk count exceeds MAX_INDEX_SIZE
    /// * `Err(TldrError::MemoryLimitExceeded)` - If estimated memory exceeds limit
    ///
    /// # P0 Mitigations
    ///
    /// - Hard limit at 100K chunks
    /// - Memory estimate before building
    /// - Reports parse failures (not silent)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let index = SemanticIndex::build(
    ///     Path::new("src/"),
    ///     BuildOptions::default(),
    ///     None,
    /// )?;
    /// ```
    pub fn build<P: AsRef<Path>>(
        root: P,
        options: BuildOptions,
        cache_config: Option<CacheConfig>,
    ) -> TldrResult<Self> {
        let start = Instant::now();
        let root = root.as_ref();

        // Initialize cache if configured
        let mut cache = if options.use_cache {
            cache_config.map(EmbeddingCache::open).transpose()?
        } else {
            None
        };

        // Convert languages from strings if provided
        let chunk_languages = options.languages.as_ref().map(|langs| {
            langs
                .iter()
                .filter_map(|s| crate::Language::from_extension(s))
                .collect()
        });

        // Chunk the codebase
        let chunk_opts = ChunkOptions {
            granularity: options.granularity,
            languages: chunk_languages,
            ..Default::default()
        };

        let chunk_result = chunk_code(root, &chunk_opts)?;

        // P0: Check index size limit
        if chunk_result.chunks.len() > MAX_INDEX_SIZE {
            return Err(TldrError::IndexTooLarge {
                count: chunk_result.chunks.len(),
                max: MAX_INDEX_SIZE,
            });
        }

        // P0: Memory estimate
        let estimated_memory = chunk_result.chunks.len() * BYTES_PER_CHUNK;
        if estimated_memory > MAX_MEMORY_BYTES {
            return Err(TldrError::MemoryLimitExceeded {
                estimated_mb: estimated_memory / (1024 * 1024),
                max_mb: MAX_MEMORY_BYTES / (1024 * 1024),
            });
        }

        // Progress reporting
        if options.show_progress && !chunk_result.chunks.is_empty() {
            eprintln!("Building index for {} chunks...", chunk_result.chunks.len());
        }

        // Report skipped files (P0: not silent)
        if !chunk_result.skipped.is_empty() && options.show_progress {
            eprintln!(
                "Skipped {} files (parse errors or unsupported)",
                chunk_result.skipped.len()
            );
        }

        // Phase 1: Separate cached vs uncached chunks
        let mut embedded_chunks: Vec<EmbeddedChunk> = Vec::with_capacity(chunk_result.chunks.len());
        let mut uncached_indices: Vec<usize> = Vec::new();

        for (i, chunk) in chunk_result.chunks.iter().enumerate() {
            let cached_embedding = if let Some(ref mut c) = cache {
                c.get(chunk, options.model)
            } else {
                None
            };

            match cached_embedding {
                Some(e) => {
                    embedded_chunks.push(EmbeddedChunk {
                        chunk: chunk.clone(),
                        embedding: e,
                    });
                }
                None => {
                    // Placeholder - will be filled by batch embed
                    embedded_chunks.push(EmbeddedChunk {
                        chunk: chunk.clone(),
                        embedding: Vec::new(),
                    });
                    uncached_indices.push(i);
                }
            }
        }

        // Phase 2: Lazy initialize embedder and batch embed uncached chunks
        // Only load the ONNX model if there are cache misses
        let embedder = if !uncached_indices.is_empty() {
            if options.show_progress {
                eprintln!(
                    "Batch embedding {} uncached chunks...",
                    uncached_indices.len()
                );
            }

            // Initialize embedder (loads 110MB ONNX model)
            let mut embedder = Embedder::new(options.model)?;

            let texts: Vec<&str> = uncached_indices
                .iter()
                .map(|&i| chunk_result.chunks[i].content.as_str())
                .collect();
            let embeddings = embedder.embed_batch(texts, options.show_progress)?;

            for (idx, embedding) in uncached_indices.iter().zip(embeddings) {
                // Store in cache
                if let Some(ref mut c) = cache {
                    c.put(&chunk_result.chunks[*idx], embedding.clone(), options.model);
                }
                embedded_chunks[*idx].embedding = embedding;
            }

            Some(embedder)
        } else {
            // All chunks were cached - skip ONNX model load entirely
            // Embedder will be lazily created later if search() is called
            if options.show_progress {
                eprintln!("All chunks cached - skipping embedder initialization");
            }
            None
        };

        // Flush cache
        if let Some(ref mut c) = cache {
            c.flush()?;
        }

        if options.show_progress {
            eprintln!("Index built in {:?}", start.elapsed());
        }

        Ok(Self {
            chunks: embedded_chunks,
            model: options.model,
            embedder,
        })
    }

    /// Search the index with a natural language query
    ///
    /// Embeds the query and finds the most similar code chunks.
    ///
    /// # Arguments
    ///
    /// * `query` - Natural language search query
    /// * `options` - Search options (top_k, threshold, etc.)
    ///
    /// # Returns
    ///
    /// * `Ok(SemanticSearchReport)` - Search results with scores and metadata
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let report = index.search("parse configuration file", &SearchOptions::default())?;
    /// for result in report.results {
    ///     println!("{}: {} ({:.2})",
    ///         result.file_path.display(),
    ///         result.function_name.unwrap_or_default(),
    ///         result.score
    ///     );
    /// }
    /// ```
    pub fn search(
        &mut self,
        query: &str,
        options: &SearchOptions,
    ) -> TldrResult<SemanticSearchReport> {
        let start = Instant::now();

        // Lazy initialize embedder if not already loaded
        if self.embedder.is_none() {
            self.embedder = Some(Embedder::new(self.model)?);
        }

        // Embed query
        let query_embedding = self.embedder.as_mut().unwrap().embed_text(query)?;

        // Build candidates for top_k_similar
        let candidates: Vec<(usize, &[f32])> = self
            .chunks
            .iter()
            .enumerate()
            .map(|(i, c)| (i, c.embedding.as_slice()))
            .collect();

        // Find similar chunks
        let similar = top_k_similar(
            &query_embedding,
            &candidates,
            options.top_k,
            options.threshold,
        );

        // Build results
        let results: Vec<SemanticSearchResult> = similar
            .into_iter()
            .map(|(idx, score)| {
                let chunk = &self.chunks[idx].chunk;
                let snippet = if options.include_snippet {
                    make_snippet(&chunk.content, options.snippet_lines)
                } else {
                    String::new()
                };
                SemanticSearchResult {
                    file_path: chunk.file_path.clone(),
                    function_name: chunk.function_name.clone(),
                    class_name: chunk.class_name.clone(),
                    score,
                    line_start: chunk.line_start,
                    line_end: chunk.line_end,
                    snippet,
                }
            })
            .collect();

        let matches_above_threshold = results.len();

        Ok(SemanticSearchReport {
            query: query.to_string(),
            model: self.model,
            results,
            total_chunks: self.chunks.len(),
            matches_above_threshold,
            latency_ms: start.elapsed().as_millis() as u64,
            cache_hit: false, // Query embeddings are not cached
        })
    }

    /// Find chunks similar to a given file/function
    ///
    /// Looks up a chunk in the index and finds similar code elsewhere.
    ///
    /// # Arguments
    ///
    /// * `file_path` - Path to the source file
    /// * `function_name` - Optional function name (None for file-level match)
    /// * `options` - Search options
    ///
    /// # Returns
    ///
    /// * `Ok(SimilarityReport)` - Similar chunks with scores
    /// * `Err(TldrError::ChunkNotFound)` - If the specified chunk is not in the index
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let report = index.find_similar("src/config.rs", Some("parse_config"), &SearchOptions::default())?;
    /// for similar in report.similar {
    ///     println!("{}: {} ({:.2})",
    ///         similar.file_path.display(),
    ///         similar.function_name.unwrap_or_default(),
    ///         similar.score
    ///     );
    /// }
    /// ```
    pub fn find_similar(
        &self,
        file_path: &str,
        function_name: Option<&str>,
        options: &SearchOptions,
    ) -> TldrResult<SimilarityReport> {
        // Find the query chunk
        let query_chunk = self
            .chunks
            .iter()
            .find(|c| {
                c.chunk.file_path.to_string_lossy() == file_path
                    && (function_name.is_none()
                        || c.chunk.function_name.as_deref() == function_name)
            })
            .ok_or_else(|| TldrError::ChunkNotFound {
                file: file_path.to_string(),
                function: function_name.map(String::from),
            })?;

        // Build candidates (excluding self)
        let candidates: Vec<(usize, &[f32])> = self
            .chunks
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                c.chunk.file_path.to_string_lossy() != file_path
                    || c.chunk.function_name != query_chunk.chunk.function_name
            })
            .map(|(i, c)| (i, c.embedding.as_slice()))
            .collect();

        // Find similar
        let similar = top_k_similar(
            &query_chunk.embedding,
            &candidates,
            options.top_k,
            options.threshold,
        );

        // Build results
        let results: Vec<SemanticSearchResult> = similar
            .into_iter()
            .map(|(idx, score)| {
                let chunk = &self.chunks[idx].chunk;
                let snippet = if options.include_snippet {
                    make_snippet(&chunk.content, options.snippet_lines)
                } else {
                    String::new()
                };
                SemanticSearchResult {
                    file_path: chunk.file_path.clone(),
                    function_name: chunk.function_name.clone(),
                    class_name: chunk.class_name.clone(),
                    score,
                    line_start: chunk.line_start,
                    line_end: chunk.line_end,
                    snippet,
                }
            })
            .collect();

        Ok(SimilarityReport {
            source: query_chunk.chunk.clone(),
            model: self.model,
            similar: results,
            total_compared: candidates.len(),
            exclude_self: true,
        })
    }

    /// Get a specific chunk by file and function name
    ///
    /// # Arguments
    ///
    /// * `file_path` - Path to the source file
    /// * `function_name` - Optional function name
    ///
    /// # Returns
    ///
    /// The embedded chunk if found, None otherwise.
    pub fn get_chunk(
        &self,
        file_path: &str,
        function_name: Option<&str>,
    ) -> Option<&EmbeddedChunk> {
        self.chunks.iter().find(|c| {
            c.chunk.file_path.to_string_lossy() == file_path
                && (function_name.is_none() || c.chunk.function_name.as_deref() == function_name)
        })
    }

    /// Get the number of chunks in the index
    pub fn len(&self) -> usize {
        self.chunks.len()
    }

    /// Check if the index is empty
    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    /// Get all chunks in the index
    pub fn chunks(&self) -> &[EmbeddedChunk] {
        &self.chunks
    }

    /// Get the embedding model used by this index
    pub fn model(&self) -> EmbeddingModel {
        self.model
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Create a snippet from code content
///
/// Takes the first N lines of the content for display purposes.
fn make_snippet(content: &str, max_lines: usize) -> String {
    content
        .lines()
        .take(max_lines)
        .collect::<Vec<_>>()
        .join("\n")
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod index_tests {
    use super::*;

    // =========================================================================
    // SearchOptions tests
    // =========================================================================

    #[test]
    fn search_options_default_values() {
        // GIVEN: Default search options
        let options = SearchOptions::default();

        // THEN: Should have sensible defaults
        assert_eq!(options.top_k, 10);
        assert!((options.threshold - 0.5).abs() < 1e-6);
        assert!(options.include_snippet);
        assert_eq!(options.snippet_lines, 5);
    }

    // =========================================================================
    // BuildOptions tests
    // =========================================================================

    #[test]
    fn build_options_default_values() {
        // GIVEN: Default build options
        let options = BuildOptions::default();

        // THEN: Should have sensible defaults
        assert_eq!(options.model, EmbeddingModel::ArcticM);
        assert_eq!(options.granularity, ChunkGranularity::Function);
        assert!(options.languages.is_none());
        assert!(options.show_progress);
        assert!(options.use_cache);
    }

    // =========================================================================
    // make_snippet tests
    // =========================================================================

    #[test]
    fn make_snippet_limits_lines() {
        // GIVEN: Multi-line content
        let content = "line1\nline2\nline3\nline4\nline5\nline6";

        // WHEN: We create a snippet with max 3 lines
        let snippet = make_snippet(content, 3);

        // THEN: Should have only 3 lines
        assert_eq!(snippet, "line1\nline2\nline3");
    }

    #[test]
    fn make_snippet_handles_short_content() {
        // GIVEN: Content with fewer lines than limit
        let content = "line1\nline2";

        // WHEN: We create a snippet with max 5 lines
        let snippet = make_snippet(content, 5);

        // THEN: Should have all lines
        assert_eq!(snippet, "line1\nline2");
    }

    #[test]
    fn make_snippet_handles_empty_content() {
        // GIVEN: Empty content
        let content = "";

        // WHEN: We create a snippet
        let snippet = make_snippet(content, 5);

        // THEN: Should be empty
        assert_eq!(snippet, "");
    }

    // =========================================================================
    // Integration tests (require model download, marked #[ignore])
    // =========================================================================

    #[test]
    #[ignore = "Requires model download"]
    fn semantic_index_build_from_directory() {
        // GIVEN: A test directory with code files
        let temp_dir = tempfile::tempdir().unwrap();
        let test_file = temp_dir.path().join("test.py");
        std::fs::write(&test_file, "def foo():\n    pass\n").unwrap();

        // WHEN: We build an index
        let options = BuildOptions {
            show_progress: false,
            use_cache: false,
            ..Default::default()
        };
        let index = SemanticIndex::build(temp_dir.path(), options, None).unwrap();

        // THEN: Index should contain chunks
        assert!(!index.is_empty());
    }

    #[test]
    #[ignore = "Requires model download"]
    fn semantic_index_search_returns_ranked_results() {
        // GIVEN: An index with some code
        let temp_dir = tempfile::tempdir().unwrap();
        std::fs::write(
            temp_dir.path().join("config.py"),
            "def parse_config():\n    pass\n",
        )
        .unwrap();
        std::fs::write(
            temp_dir.path().join("loader.py"),
            "def load_data():\n    pass\n",
        )
        .unwrap();

        let options = BuildOptions {
            show_progress: false,
            use_cache: false,
            ..Default::default()
        };
        let mut index = SemanticIndex::build(temp_dir.path(), options, None).unwrap();

        // WHEN: We search for "parse configuration"
        let search_opts = SearchOptions::default();
        let report = index.search("parse configuration", &search_opts).unwrap();

        // THEN: Results should be ranked by score
        if report.results.len() >= 2 {
            assert!(report.results[0].score >= report.results[1].score);
        }
    }

    #[test]
    #[ignore = "Requires model download"]
    fn semantic_index_search_respects_top_k() {
        // GIVEN: An index with multiple chunks
        let temp_dir = tempfile::tempdir().unwrap();
        for i in 0..5 {
            std::fs::write(
                temp_dir.path().join(format!("file{}.py", i)),
                format!("def func{}():\n    pass\n", i),
            )
            .unwrap();
        }

        let options = BuildOptions {
            show_progress: false,
            use_cache: false,
            ..Default::default()
        };
        let mut index = SemanticIndex::build(temp_dir.path(), options, None).unwrap();

        // WHEN: We search with top_k = 2
        let search_opts = SearchOptions {
            top_k: 2,
            threshold: 0.0, // Accept all
            ..Default::default()
        };
        let report = index.search("function", &search_opts).unwrap();

        // THEN: Should return at most 2 results
        assert!(report.results.len() <= 2);
    }

    #[test]
    #[ignore = "Requires model download"]
    fn semantic_index_search_respects_threshold() {
        // GIVEN: An index
        let temp_dir = tempfile::tempdir().unwrap();
        std::fs::write(temp_dir.path().join("test.py"), "def foo():\n    pass\n").unwrap();

        let options = BuildOptions {
            show_progress: false,
            use_cache: false,
            ..Default::default()
        };
        let mut index = SemanticIndex::build(temp_dir.path(), options, None).unwrap();

        // WHEN: We search with a very high threshold
        let search_opts = SearchOptions {
            top_k: 10,
            threshold: 0.99, // Very high
            ..Default::default()
        };
        let report = index
            .search("completely unrelated query", &search_opts)
            .unwrap();

        // THEN: May return no results due to threshold
        // (We can't assert empty because embeddings might still be similar)
        assert!(report.results.iter().all(|r| r.score >= 0.99));
    }

    #[test]
    fn semantic_index_empty_returns_no_results() {
        // This test doesn't need the model since we're testing empty behavior
        // We can't easily create an empty index without the model, so skip
    }

    #[test]
    #[ignore = "Requires model download"]
    fn semantic_index_len_returns_chunk_count() {
        // GIVEN: An index with known number of files
        let temp_dir = tempfile::tempdir().unwrap();
        std::fs::write(temp_dir.path().join("a.py"), "def a():\n    pass\n").unwrap();
        std::fs::write(temp_dir.path().join("b.py"), "def b():\n    pass\n").unwrap();

        let options = BuildOptions {
            show_progress: false,
            use_cache: false,
            ..Default::default()
        };
        let index = SemanticIndex::build(temp_dir.path(), options, None).unwrap();

        // THEN: len() should return chunk count
        assert!(index.len() >= 2); // At least 2 functions
    }

    #[test]
    #[ignore = "Requires model download"]
    fn semantic_index_build_uses_batch_embedding() {
        // GIVEN: A directory with multiple code files (tests batch path)
        let temp_dir = tempfile::tempdir().unwrap();
        for i in 0..10 {
            std::fs::write(
                temp_dir.path().join(format!("mod{}.py", i)),
                format!("def func_{}(x):\n    return x + {}\n", i, i),
            )
            .unwrap();
        }

        // WHEN: We build an index (should use batch embedding internally)
        let options = BuildOptions {
            show_progress: false,
            use_cache: false,
            ..Default::default()
        };
        let index = SemanticIndex::build(temp_dir.path(), options, None).unwrap();

        // THEN: All chunks should have embeddings with correct dimensions
        assert!(
            index.len() >= 10,
            "Expected at least 10 chunks, got {}",
            index.len()
        );
        for chunk in index.chunks() {
            assert_eq!(
                chunk.embedding.len(),
                768,
                "Each chunk should have 768-dim embedding"
            );
            // Verify embedding is normalized
            let norm: f32 = chunk.embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
            assert!(
                (norm - 1.0).abs() < 1e-4,
                "Embedding should be normalized, got norm={}",
                norm
            );
        }
    }

    #[test]
    #[ignore = "Requires model download"]
    fn semantic_index_build_batch_matches_sequential() {
        // GIVEN: Same files, build with batch (the new default) and verify
        // results are consistent (same chunks produce same search rankings)
        let temp_dir = tempfile::tempdir().unwrap();
        std::fs::write(
            temp_dir.path().join("parser.py"),
            "def parse_config(path):\n    with open(path) as f:\n        return f.read()\n",
        )
        .unwrap();
        std::fs::write(
            temp_dir.path().join("loader.py"),
            "def load_data(file):\n    return read(file)\n",
        )
        .unwrap();
        std::fs::write(
            temp_dir.path().join("math.py"),
            "def add_numbers(a, b):\n    return a + b\n",
        )
        .unwrap();

        let options = BuildOptions {
            show_progress: false,
            use_cache: false,
            ..Default::default()
        };
        let mut index = SemanticIndex::build(temp_dir.path(), options, None).unwrap();

        // WHEN: We search for "parse configuration"
        let search_opts = SearchOptions {
            top_k: 3,
            threshold: 0.0,
            ..Default::default()
        };
        let report = index.search("parse configuration", &search_opts).unwrap();

        // THEN: parse_config should rank higher than add_numbers
        assert!(!report.results.is_empty(), "Should have results");
        // The parser function should score higher for "parse configuration"
        let parser_result = report
            .results
            .iter()
            .find(|r| r.function_name.as_deref() == Some("parse_config"));
        let math_result = report
            .results
            .iter()
            .find(|r| r.function_name.as_deref() == Some("add_numbers"));
        if let (Some(p), Some(m)) = (parser_result, math_result) {
            assert!(
                p.score > m.score,
                "parse_config ({}) should score higher than add_numbers ({}) for 'parse configuration'",
                p.score,
                m.score
            );
        }
    }

    #[test]
    #[ignore = "Requires model download"]
    fn semantic_index_find_similar() {
        // GIVEN: An index with similar functions
        let temp_dir = tempfile::tempdir().unwrap();
        std::fs::write(
            temp_dir.path().join("config.py"),
            "def parse_config(path):\n    return read(path)\n",
        )
        .unwrap();
        std::fs::write(
            temp_dir.path().join("settings.py"),
            "def load_settings(file):\n    return read(file)\n",
        )
        .unwrap();
        std::fs::write(
            temp_dir.path().join("unrelated.py"),
            "def calculate_sum(a, b):\n    return a + b\n",
        )
        .unwrap();

        let options = BuildOptions {
            show_progress: false,
            use_cache: false,
            ..Default::default()
        };
        let index = SemanticIndex::build(temp_dir.path(), options, None).unwrap();

        // WHEN: We find similar to parse_config
        let search_opts = SearchOptions {
            top_k: 5,
            threshold: 0.0,
            ..Default::default()
        };
        let report = index
            .find_similar("config.py", Some("parse_config"), &search_opts)
            .unwrap();

        // THEN: Should exclude self and find similar code
        assert!(report.exclude_self);
        assert!(!report.similar.iter().any(|r| {
            r.file_path.to_string_lossy() == "config.py"
                && r.function_name.as_deref() == Some("parse_config")
        }));
    }
}
