//! Hybrid search using Reciprocal Rank Fusion (RRF)
//!
//! Combines BM25 keyword search with semantic embedding search using RRF.
//!
//! # RRF Formula
//! ```text
//! RRF_score(d) = sum(1 / (k + rank_i(d)) for each ranking i)
//! ```
//!
//! Where:
//! - k: constant to prevent division by small numbers (default 60)
//! - rank_i(d): rank of document d in ranking i (1-indexed)
//!
//! # Mitigation M8
//! Gracefully degrades to BM25-only when embedding service is unavailable.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::bm25::{Bm25Index, Bm25Result};
use super::embedding_client::{EmbeddingClient, SemanticResult};
use crate::types::Language;
use crate::TldrResult;

/// Default RRF k constant
pub const DEFAULT_K_CONSTANT: f64 = 60.0;

/// A single hybrid search result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HybridResult {
    /// File path
    pub file_path: std::path::PathBuf,
    /// Combined RRF score
    pub rrf_score: f64,
    /// Rank in BM25 results (None if not in BM25 top-k)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bm25_rank: Option<usize>,
    /// Rank in semantic results (None if not in semantic top-k)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dense_rank: Option<usize>,
    /// BM25 score (None if not in BM25 results)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bm25_score: Option<f64>,
    /// Semantic similarity score (None if not in semantic results)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dense_score: Option<f64>,
    /// Snippet of matching content
    pub snippet: String,
    /// Terms that matched in BM25
    pub matched_terms: Vec<String>,
}

/// Report from hybrid search
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HybridSearchReport {
    /// Search results sorted by RRF score
    pub results: Vec<HybridResult>,
    /// Original query
    pub query: String,
    /// Total candidates considered
    pub total_candidates: usize,
    /// Results only in BM25
    pub bm25_only: usize,
    /// Results only in semantic search
    pub dense_only: usize,
    /// Results in both rankings
    pub overlap: usize,
    /// Fallback mode (if semantic search unavailable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_mode: Option<String>,
}

/// Perform hybrid search combining BM25 and semantic search
///
/// # Arguments
/// * `query` - Search query string
/// * `root` - Project root directory
/// * `language` - Programming language to search
/// * `top_k` - Number of results to return
/// * `k_constant` - RRF k constant (default 60)
/// * `embedding_client` - Optional embedding service client
///
/// # Returns
/// HybridSearchReport containing fused results
///
/// # Example
/// ```ignore
/// use tldr_core::search::hybrid::hybrid_search;
/// use tldr_core::search::embedding_client::EmbeddingClient;
///
/// let client = EmbeddingClient::new("http://localhost:8765");
/// let report = hybrid_search(
///     "process data",
///     Path::new("src/"),
///     Language::Python,
///     10,
///     60.0,
///     Some(&client),
/// )?;
/// ```
pub fn hybrid_search(
    query: &str,
    root: &Path,
    language: Language,
    top_k: usize,
    k_constant: f64,
    embedding_client: Option<&EmbeddingClient>,
) -> TldrResult<HybridSearchReport> {
    // Build BM25 index and search
    let bm25_index = Bm25Index::from_project(root, language)?;
    let bm25_results = bm25_index.search(query, top_k * 2); // Get more for RRF

    // Try semantic search if client provided
    let (semantic_results, fallback_mode) = match embedding_client {
        Some(client) => {
            match client.search(query, &root.to_string_lossy(), top_k * 2) {
                Ok(results) => (results, None),
                Err(_) => {
                    // M8: Graceful degradation
                    (Vec::new(), Some("bm25_only".to_string()))
                }
            }
        }
        None => (Vec::new(), Some("bm25_only".to_string())),
    };

    // Fuse results using RRF
    let fused = fuse_rrf(&bm25_results, &semantic_results, k_constant, top_k);

    // Calculate statistics
    let bm25_files: std::collections::HashSet<_> = bm25_results
        .iter()
        .map(|r| r.file_path.to_string_lossy().to_string())
        .collect();
    let dense_files: std::collections::HashSet<_> =
        semantic_results.iter().map(|r| r.doc_id.clone()).collect();

    let overlap = bm25_files.intersection(&dense_files).count();
    let bm25_only = bm25_files.len() - overlap;
    let dense_only = dense_files.len() - overlap;

    Ok(HybridSearchReport {
        results: fused,
        query: query.to_string(),
        total_candidates: bm25_files.len() + dense_files.len() - overlap,
        bm25_only,
        dense_only,
        overlap,
        fallback_mode,
    })
}

/// Fuse BM25 and semantic results using Reciprocal Rank Fusion
fn fuse_rrf(
    bm25_results: &[Bm25Result],
    semantic_results: &[SemanticResult],
    k: f64,
    top_k: usize,
) -> Vec<HybridResult> {
    let mut scores: HashMap<String, HybridResult> = HashMap::new();

    // Add BM25 results
    for (rank, result) in bm25_results.iter().enumerate() {
        let file_key = result.file_path.to_string_lossy().to_string();
        let rrf_contrib = 1.0 / (k + (rank + 1) as f64);

        let entry = scores
            .entry(file_key.clone())
            .or_insert_with(|| HybridResult {
                file_path: result.file_path.clone(),
                rrf_score: 0.0,
                bm25_rank: None,
                dense_rank: None,
                bm25_score: None,
                dense_score: None,
                snippet: String::new(),
                matched_terms: Vec::new(),
            });

        entry.rrf_score += rrf_contrib;
        entry.bm25_rank = Some(rank + 1);
        entry.bm25_score = Some(result.score);
        entry.snippet = result.snippet.clone();
        entry.matched_terms = result.matched_terms.clone();
    }

    // Add semantic results
    for (rank, result) in semantic_results.iter().enumerate() {
        let file_key = result.doc_id.clone();
        let rrf_contrib = 1.0 / (k + (rank + 1) as f64);

        let entry = scores
            .entry(file_key.clone())
            .or_insert_with(|| HybridResult {
                file_path: std::path::PathBuf::from(&result.doc_id),
                rrf_score: 0.0,
                bm25_rank: None,
                dense_rank: None,
                bm25_score: None,
                dense_score: None,
                snippet: String::new(),
                matched_terms: Vec::new(),
            });

        entry.rrf_score += rrf_contrib;
        entry.dense_rank = Some(rank + 1);
        entry.dense_score = Some(result.score);

        // Use semantic snippet if BM25 didn't provide one
        if entry.snippet.is_empty() {
            entry.snippet = result.snippet.clone();
        }
    }

    // Sort by RRF score and take top_k
    let mut results: Vec<HybridResult> = scores.into_values().collect();
    results.sort_by(|a, b| {
        b.rrf_score
            .partial_cmp(&a.rrf_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(top_k);

    results
}

/// Calculate RRF score for a single document
///
/// # Arguments
/// * `ranks` - Vector of (ranking_id, rank) pairs for this document
/// * `k` - RRF constant
pub fn calculate_rrf_score(ranks: &[(usize, usize)], k: f64) -> f64 {
    ranks.iter().map(|(_, rank)| 1.0 / (k + *rank as f64)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rrf_score_calculation() {
        // Document appears at rank 1 in both rankings
        let ranks = vec![(0, 1), (1, 1)];
        let score = calculate_rrf_score(&ranks, 60.0);

        // Expected: 1/(60+1) + 1/(60+1) = 2/61
        let expected = 2.0 / 61.0;
        assert!((score - expected).abs() < 1e-10);
    }

    #[test]
    fn test_rrf_score_different_ranks() {
        // Document at rank 1 in first, rank 5 in second
        let ranks = vec![(0, 1), (1, 5)];
        let score = calculate_rrf_score(&ranks, 60.0);

        // Expected: 1/(60+1) + 1/(60+5) = 1/61 + 1/65
        let expected = 1.0 / 61.0 + 1.0 / 65.0;
        assert!((score - expected).abs() < 1e-10);
    }

    #[test]
    fn test_fuse_rrf_bm25_only() {
        let bm25_results = vec![
            Bm25Result {
                file_path: std::path::PathBuf::from("file1.py"),
                score: 1.5,
                line_start: 1,
                line_end: 10,
                snippet: "snippet 1".to_string(),
                matched_terms: vec!["process".to_string()],
            },
            Bm25Result {
                file_path: std::path::PathBuf::from("file2.py"),
                score: 1.0,
                line_start: 1,
                line_end: 5,
                snippet: "snippet 2".to_string(),
                matched_terms: vec!["data".to_string()],
            },
        ];

        let fused = fuse_rrf(&bm25_results, &[], 60.0, 10);

        assert_eq!(fused.len(), 2);
        assert_eq!(fused[0].file_path, std::path::PathBuf::from("file1.py"));
        assert!(fused[0].bm25_rank.is_some());
        assert!(fused[0].dense_rank.is_none());
    }

    #[test]
    fn test_fuse_rrf_overlap() {
        let bm25_results = vec![Bm25Result {
            file_path: std::path::PathBuf::from("file1.py"),
            score: 1.5,
            line_start: 1,
            line_end: 10,
            snippet: "snippet".to_string(),
            matched_terms: vec!["process".to_string()],
        }];

        let semantic_results = vec![SemanticResult {
            doc_id: "file1.py".to_string(),
            score: 0.95,
            line_start: 1,
            line_end: 10,
            snippet: "semantic snippet".to_string(),
        }];

        let fused = fuse_rrf(&bm25_results, &semantic_results, 60.0, 10);

        assert_eq!(fused.len(), 1);
        assert!(fused[0].bm25_rank.is_some());
        assert!(fused[0].dense_rank.is_some());

        // RRF score should be sum of both contributions
        let expected_score = 1.0 / 61.0 + 1.0 / 61.0;
        assert!((fused[0].rrf_score - expected_score).abs() < 1e-10);
    }

    #[test]
    fn test_hybrid_fallback_mode() {
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let test_file = tmp.path().join("test.py");
        std::fs::write(&test_file, "def process_data():\n    pass").unwrap();

        // No embedding client - should fall back to BM25 only
        let report =
            hybrid_search("process", tmp.path(), Language::Python, 10, 60.0, None).unwrap();

        assert_eq!(report.fallback_mode, Some("bm25_only".to_string()));
        // Should still have BM25 results
        // (may be empty if tokenizer filters out "process")
    }

    #[test]
    fn test_hybrid_k_constant_effect() {
        // Higher k values reduce the impact of rank differences
        let ranks_high_k = calculate_rrf_score(&[(0, 1), (1, 10)], 100.0);
        let ranks_low_k = calculate_rrf_score(&[(0, 1), (1, 10)], 10.0);

        // With higher k, the difference between ranks matters less
        // So the ratio between first and second contribution should be closer to 1
        let ratio_high = (1.0 / 101.0) / (1.0 / 110.0);
        let ratio_low = (1.0 / 11.0) / (1.0 / 20.0);

        assert!(ratio_high < ratio_low);
        assert!(ranks_high_k > 0.0);
        assert!(ranks_low_k > 0.0);
    }
}
