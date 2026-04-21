//! Semantic search module for embedding-based code search
//!
//! This module provides AI-powered semantic code search using dense embeddings
//! from the Snowflake Arctic model family. It enables:
//!
//! - Natural language queries to find semantically related code
//! - Similarity detection between code fragments
//! - Embedding generation for downstream tools
//!
//! # Architecture
//!
//! ```text
//! +-------------------+     +------------------+
//! |   SemanticIndex   |<--->|  EmbeddingCache  |
//! +-------------------+     +------------------+
//!          |                        |
//!          v                        v
//! +-------------------+     +------------------+
//! |     Embedder      |     |     Chunker      |
//! | (fastembed-rs)    |     | (tree-sitter)    |
//! +-------------------+     +------------------+
//!          |
//!          v
//! +-------------------+
//! |    Similarity     |
//! | (cosine, top-K)   |
//! +-------------------+
//! ```
//!
//! # Example
//!
//! ```rust,ignore
//! use tldr_core::semantic::{SemanticIndex, SearchOptions, ChunkOptions, EmbedOptions};
//!
//! // Build an index from a project directory
//! let index = SemanticIndex::build(
//!     Path::new("src/"),
//!     ChunkOptions::default(),
//!     EmbedOptions::default(),
//!     None, // No cache
//! )?;
//!
//! // Search for semantically related code
//! let report = index.search("parse configuration file", SearchOptions::default())?;
//!
//! for result in report.results {
//!     println!("{}: {} (score: {:.2})",
//!         result.file_path.display(),
//!         result.function_name.unwrap_or_default(),
//!         result.score
//!     );
//! }
//! ```
//!
//! # Modules
//!
//! - `types`: Core data structures (CodeChunk, EmbeddingModel, etc.)
//! - `embedder`: Embedding generation using fastembed-rs (Phase 3)
//! - `chunker`: Code chunking via tree-sitter (Phase 4)
//! - `similarity`: Cosine similarity and top-K search (Phase 2)
//! - `cache`: JSON-based embedding cache (Phase 5)
//! - `index`: In-memory semantic index (Phase 6)

pub mod types;

// Re-export all public types for convenience
pub use types::{
    CacheConfig,
    CacheStats,
    ChunkGranularity,
    ChunkOptions,
    // Core types
    CodeChunk,
    // Option types
    EmbedOptions,
    EmbedReport,
    EmbeddedChunk,
    EmbeddingModel,
    SearchOptions,
    SemanticSearchReport,
    // Result types
    SemanticSearchResult,
    SimilarityReport,
};

// Phase 2: Similarity
pub mod similarity;
pub use similarity::{cosine_similarity, is_normalized, normalize, top_k_similar};

// Placeholder re-exports for future phases
// These will be uncommented as each phase is implemented

// Phase 3: Embedder
pub mod embedder;
pub use embedder::Embedder;

// Phase 4: Chunker
pub mod chunker;
pub use chunker::{chunk_code, chunk_file, ChunkResult, SkippedFile};

// Phase 5: Cache
pub mod cache;
pub use cache::EmbeddingCache;

// Phase 6: Index
pub mod index;
pub use index::{BuildOptions, SearchOptions as IndexSearchOptions, SemanticIndex, MAX_INDEX_SIZE};
