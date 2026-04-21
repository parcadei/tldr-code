//! Search functionality for TLDR
//!
//! This module provides text search capabilities:
//! - **Regex search**: Pattern matching across files
//! - **BM25 search**: Keyword search with relevance ranking
//! - **Hybrid search**: RRF fusion of BM25 + semantic embeddings
//!
//! # Mitigations Addressed
//! - M8: Embedding service connection failure - graceful degradation to BM25-only
//! - M11: BM25 tokenization differences - port exact Python tokenization logic

pub mod bm25;
pub mod embedding_client;
pub mod enriched;
pub mod hybrid;
pub mod text;
pub mod tokenizer;

// Re-export main types and functions
pub use bm25::{Bm25Index, Bm25Result};
pub use embedding_client::EmbeddingClient;
pub use enriched::{
    enriched_search, enriched_search_with_callgraph_cache, enriched_search_with_index,
    enriched_search_with_structure_cache, read_callgraph_cache, read_structure_cache,
    search_with_inner, write_structure_cache, CallGraphLookup, EnrichedResult,
    EnrichedSearchOptions, EnrichedSearchReport, SearchMode, StructureLookup,
};
pub use hybrid::{hybrid_search, HybridResult, HybridSearchReport};
pub use text::{search, SearchMatch};
pub use tokenizer::Tokenizer;
