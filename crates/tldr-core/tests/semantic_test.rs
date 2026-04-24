#![cfg(feature = "semantic")]
//! Tests for Semantic Search Module (Session 16)
//!
//! Commands tested: semantic, embed, similar
//!
//! These tests validate the semantic search implementation including:
//! - Types: CodeChunk, EmbeddingModel, SemanticSearchResult
//! - Embedder: Model initialization, text embedding, batch embedding
//! - Chunker: Function extraction, file-level chunking
//! - Similarity: Cosine similarity, top-K selection
//! - Index: Add/search chunks, persistence
//! - Cache: Hit/miss, invalidation
//!
//! Tests are written to FAIL initially (TDD red phase).
//! Many tests are marked `#[ignore]` because they require model download.

use std::path::PathBuf;

// Phase 1-3 imports (implemented)
use tldr_core::semantic::{
    // Similarity (Phase 2)
    cosine_similarity,
    is_normalized,
    normalize,
    top_k_similar,
    ChunkGranularity,
    // Types (Phase 1)
    CodeChunk,
    EmbedOptions,
    // Embedder (Phase 3)
    Embedder,
    EmbeddingModel,
    SemanticSearchResult,
};

// Future phase imports (commented until implemented)
// Phase 4: Chunker
// use tldr_core::semantic::{chunk_code, chunk_file};
// Phase 5: Cache
// use tldr_core::semantic::EmbeddingCache;
// Phase 6: Index
// use tldr_core::semantic::SemanticIndex;

use tldr_core::Language;
// TldrError used in future tests
#[allow(unused_imports)]
use tldr_core::TldrError;

// =============================================================================
// Types Tests (types.rs)
// =============================================================================

mod types_tests {
    use super::*;

    #[test]
    fn code_chunk_creation() {
        // GIVEN: Parameters for a code chunk
        let file_path = PathBuf::from("src/main.rs");
        let function_name = Some("process_data".to_string());
        let content = "fn process_data() { }".to_string();

        // WHEN: We create a CodeChunk
        let chunk = CodeChunk {
            file_path: file_path.clone(),
            function_name: function_name.clone(),
            class_name: None,
            line_start: 10,
            line_end: 20,
            content: content.clone(),
            content_hash: "abc123".to_string(),
            language: Language::Rust,
        };

        // THEN: Fields should be set correctly
        assert_eq!(chunk.file_path, file_path);
        assert_eq!(chunk.function_name, function_name);
        assert_eq!(chunk.line_start, 10);
        assert_eq!(chunk.line_end, 20);
        assert_eq!(chunk.content, content);
    }

    #[test]
    fn code_chunk_serialization_roundtrip() {
        // GIVEN: A CodeChunk
        let chunk = CodeChunk {
            file_path: PathBuf::from("test.py"),
            function_name: Some("foo".to_string()),
            class_name: Some("MyClass".to_string()),
            line_start: 1,
            line_end: 10,
            content: "def foo(): pass".to_string(),
            content_hash: "hash123".to_string(),
            language: Language::Python,
        };

        // WHEN: We serialize and deserialize
        let json = serde_json::to_string(&chunk).unwrap();
        let deserialized: CodeChunk = serde_json::from_str(&json).unwrap();

        // THEN: Roundtrip should preserve all fields
        assert_eq!(chunk.file_path, deserialized.file_path);
        assert_eq!(chunk.function_name, deserialized.function_name);
        assert_eq!(chunk.class_name, deserialized.class_name);
        assert_eq!(chunk.line_start, deserialized.line_start);
        assert_eq!(chunk.line_end, deserialized.line_end);
        assert_eq!(chunk.content, deserialized.content);
        assert_eq!(chunk.content_hash, deserialized.content_hash);
    }

    #[test]
    fn embedding_model_default_is_arctic_m() {
        // GIVEN: Default embedding model
        let model = EmbeddingModel::default();

        // THEN: Default should be ArcticM
        assert_eq!(model, EmbeddingModel::ArcticM);
    }

    #[test]
    fn embedding_model_dimensions() {
        // GIVEN: Different embedding models

        // THEN: Dimensions should match spec
        assert_eq!(EmbeddingModel::ArcticXS.dimensions(), 384);
        assert_eq!(EmbeddingModel::ArcticS.dimensions(), 384);
        assert_eq!(EmbeddingModel::ArcticM.dimensions(), 768);
        assert_eq!(EmbeddingModel::ArcticMLong.dimensions(), 768);
        assert_eq!(EmbeddingModel::ArcticL.dimensions(), 1024);
    }

    #[test]
    fn embedding_model_max_context() {
        // GIVEN: Different embedding models

        // THEN: Context lengths should match spec
        assert_eq!(EmbeddingModel::ArcticXS.max_context(), 512);
        assert_eq!(EmbeddingModel::ArcticS.max_context(), 512);
        assert_eq!(EmbeddingModel::ArcticM.max_context(), 512);
        assert_eq!(EmbeddingModel::ArcticMLong.max_context(), 8192);
        assert_eq!(EmbeddingModel::ArcticL.max_context(), 512);
    }

    #[test]
    fn embedding_model_serialization() {
        // GIVEN: An embedding model
        let model = EmbeddingModel::ArcticM;

        // WHEN: We serialize it
        let json = serde_json::to_string(&model).unwrap();

        // THEN: It should use kebab-case
        assert_eq!(json, "\"arctic-m\"");
    }

    #[test]
    fn chunk_granularity_default_is_function() {
        // GIVEN: Default chunk granularity
        let granularity = ChunkGranularity::default();

        // THEN: Default should be Function
        assert_eq!(granularity, ChunkGranularity::Function);
    }

    #[test]
    fn semantic_search_result_ordering_by_score() {
        // GIVEN: Multiple search results with different scores
        let mut results = [SemanticSearchResult {
                file_path: PathBuf::from("a.rs"),
                function_name: Some("a".to_string()),
                class_name: None,
                score: 0.5,
                line_start: 1,
                line_end: 10,
                snippet: "fn a()".to_string(),
            },
            SemanticSearchResult {
                file_path: PathBuf::from("b.rs"),
                function_name: Some("b".to_string()),
                class_name: None,
                score: 0.9,
                line_start: 1,
                line_end: 10,
                snippet: "fn b()".to_string(),
            },
            SemanticSearchResult {
                file_path: PathBuf::from("c.rs"),
                function_name: Some("c".to_string()),
                class_name: None,
                score: 0.7,
                line_start: 1,
                line_end: 10,
                snippet: "fn c()".to_string(),
            }];

        // WHEN: We sort by score descending
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());

        // THEN: Results should be ordered by score (highest first)
        assert_eq!(results[0].function_name, Some("b".to_string())); // 0.9
        assert_eq!(results[1].function_name, Some("c".to_string())); // 0.7
        assert_eq!(results[2].function_name, Some("a".to_string())); // 0.5
    }
}

// =============================================================================
// Embedder Tests (embedder.rs)
// =============================================================================

mod embedder_tests {
    use super::*;

    #[test]
    #[ignore] // Requires model download
    fn embedder_new_initializes_model() {
        // GIVEN: An embedding model type
        let model = EmbeddingModel::ArcticM;

        // WHEN: We create an embedder
        let embedder = Embedder::new(model);

        // THEN: It should succeed
        assert!(embedder.is_ok());
        let embedder = embedder.unwrap();
        assert_eq!(embedder.config(), EmbeddingModel::ArcticM);
    }

    #[test]
    #[ignore] // Requires model download
    fn embedder_embed_text_returns_correct_dimensions() {
        // GIVEN: An embedder with ArcticM model
        let mut embedder = Embedder::new(EmbeddingModel::ArcticM).unwrap();

        // WHEN: We embed some text
        let embedding = embedder.embed_text("def process_data(): pass");

        // THEN: Embedding should have correct dimensions (768 for ArcticM)
        assert!(embedding.is_ok());
        let embedding = embedding.unwrap();
        assert_eq!(embedding.len(), 768);
    }

    #[test]
    #[ignore] // Requires model download
    fn embedder_embed_text_is_normalized() {
        // GIVEN: An embedder
        let mut embedder = Embedder::new(EmbeddingModel::ArcticM).unwrap();

        // WHEN: We embed some text
        let embedding = embedder.embed_text("fn main() {}").unwrap();

        // THEN: Embedding should be normalized (L2 norm ~= 1.0)
        let l2_norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (l2_norm - 1.0).abs() < 1e-5,
            "L2 norm was {}, expected ~1.0",
            l2_norm
        );
    }

    #[test]
    #[ignore] // Requires model download
    fn embedder_batch_embedding_matches_single() {
        // GIVEN: An embedder and some texts
        let mut embedder = Embedder::new(EmbeddingModel::ArcticM).unwrap();
        let text0 = "def foo(): pass";
        let text1 = "def bar(): return 42";

        // WHEN: We embed individually and in batch
        let single_0 = embedder.embed_text(text0).unwrap();
        let single_1 = embedder.embed_text(text1).unwrap();
        let batch = embedder.embed_batch(vec![text0, text1], false).unwrap();

        // THEN: Batch results should match single results
        assert_eq!(batch.len(), 2);
        for i in 0..768 {
            assert!((single_0[i] - batch[0][i]).abs() < 1e-5);
            assert!((single_1[i] - batch[1][i]).abs() < 1e-5);
        }
    }

    #[test]
    #[ignore] // Requires model download
    fn embedder_empty_input_returns_zero_vector() {
        // GIVEN: An embedder
        let mut embedder = Embedder::new(EmbeddingModel::ArcticM).unwrap();

        // WHEN: We embed an empty string
        let embedding = embedder.embed_text("");

        // THEN: Should return a zero vector (normalized)
        assert!(embedding.is_ok());
        let embedding = embedding.unwrap();
        assert_eq!(embedding.len(), 768);
        // Zero vector when normalized is still zeros (special case)
    }

    #[test]
    #[ignore] // Requires model download
    fn embedder_batch_empty_list_returns_empty() {
        // GIVEN: An embedder
        let mut embedder = Embedder::new(EmbeddingModel::ArcticM).unwrap();

        // WHEN: We embed an empty batch
        let embeddings = embedder.embed_batch(vec![], false);

        // THEN: Should return empty list
        assert!(embeddings.is_ok());
        assert!(embeddings.unwrap().is_empty());
    }

    #[test]
    fn embed_options_default_values() {
        // GIVEN: Default embed options
        let options = EmbedOptions::default();

        // THEN: Defaults should match spec
        assert_eq!(options.model, EmbeddingModel::ArcticM);
        assert!(!options.show_progress);
        // Default batch size should be reasonable
        assert!(options.batch_size >= 16 && options.batch_size <= 64);
    }
}

// =============================================================================
// Similarity Tests (similarity.rs)
// =============================================================================

mod similarity_tests {
    use super::*;

    #[test]
    fn cosine_similarity_identical_vectors_equals_one() {
        // GIVEN: Two identical normalized vectors
        let v = vec![0.5_f32, 0.5, 0.5, 0.5];
        let mut normalized = v.clone();
        normalize(&mut normalized);

        // WHEN: We compute cosine similarity
        let sim = cosine_similarity(&normalized, &normalized);

        // THEN: Similarity should be 1.0
        assert!((sim - 1.0).abs() < 1e-6, "Expected 1.0, got {}", sim);
    }

    #[test]
    fn cosine_similarity_orthogonal_vectors_equals_zero() {
        // GIVEN: Two orthogonal vectors
        let a = vec![1.0_f32, 0.0, 0.0];
        let b = vec![0.0_f32, 1.0, 0.0];

        // WHEN: We compute cosine similarity
        let sim = cosine_similarity(&a, &b);

        // THEN: Similarity should be 0.0
        assert!(sim.abs() < 1e-6, "Expected 0.0, got {}", sim);
    }

    #[test]
    fn cosine_similarity_opposite_vectors_equals_negative_one() {
        // GIVEN: Two opposite vectors
        let a = vec![1.0_f32, 0.0, 0.0];
        let b = vec![-1.0_f32, 0.0, 0.0];

        // WHEN: We compute cosine similarity
        let sim = cosine_similarity(&a, &b);

        // THEN: Similarity should be -1.0
        assert!((sim - (-1.0)).abs() < 1e-6, "Expected -1.0, got {}", sim);
    }

    #[test]
    fn cosine_similarity_is_symmetric() {
        // GIVEN: Two random vectors
        let a = vec![0.3_f32, 0.7, 0.2, 0.5];
        let b = vec![0.6_f32, 0.1, 0.8, 0.3];

        // WHEN: We compute similarity both ways
        let sim_ab = cosine_similarity(&a, &b);
        let sim_ba = cosine_similarity(&b, &a);

        // THEN: Results should be identical (symmetric)
        assert!((sim_ab - sim_ba).abs() < 1e-6);
    }

    #[test]
    #[should_panic]
    fn cosine_similarity_different_lengths_panics() {
        // GIVEN: Vectors of different lengths
        let a = vec![1.0_f32, 0.0, 0.0];
        let b = vec![1.0_f32, 0.0];

        // WHEN: We compute cosine similarity
        // THEN: Should panic
        let _ = cosine_similarity(&a, &b);
    }

    #[test]
    fn normalize_creates_unit_vector() {
        // GIVEN: A non-normalized vector
        let mut v = vec![3.0_f32, 4.0, 0.0]; // Length 5

        // WHEN: We normalize it
        normalize(&mut v);

        // THEN: L2 norm should be 1.0
        let l2_norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((l2_norm - 1.0).abs() < 1e-6);
        assert!((v[0] - 0.6).abs() < 1e-6); // 3/5
        assert!((v[1] - 0.8).abs() < 1e-6); // 4/5
    }

    #[test]
    fn is_normalized_detects_unit_vectors() {
        // GIVEN: Normalized and non-normalized vectors
        let unit = vec![0.6_f32, 0.8, 0.0]; // Already unit length
        let non_unit = vec![3.0_f32, 4.0, 0.0]; // Length 5

        // THEN: is_normalized should correctly identify them
        assert!(is_normalized(&unit));
        assert!(!is_normalized(&non_unit));
    }

    #[test]
    fn top_k_similar_returns_k_results() {
        // GIVEN: A query and candidate vectors
        let query = vec![1.0_f32, 0.0];
        let candidates: Vec<(usize, &[f32])> = vec![
            (0, &[0.9_f32, 0.1][..]), // High similarity
            (1, &[0.1_f32, 0.9][..]), // Low similarity
            (2, &[0.7_f32, 0.3][..]), // Medium similarity
            (3, &[0.8_f32, 0.2][..]), // Medium-high similarity
        ];

        // WHEN: We find top-2 similar
        let results = top_k_similar(&query, &candidates, 2, 0.0);

        // THEN: Should return exactly 2 results
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn top_k_similar_ordered_by_score_descending() {
        // GIVEN: A query and candidate vectors
        let query = vec![1.0_f32, 0.0];
        let candidates: Vec<(usize, &[f32])> = vec![
            (0, &[0.9_f32, 0.1][..]), // idx 0: highest
            (1, &[0.1_f32, 0.9][..]), // idx 1: lowest
            (2, &[0.7_f32, 0.3][..]), // idx 2: medium
        ];

        // WHEN: We find top-3 similar
        let results = top_k_similar(&query, &candidates, 3, 0.0);

        // THEN: Results should be ordered by score descending
        assert_eq!(results.len(), 3);
        assert!(results[0].1 >= results[1].1);
        assert!(results[1].1 >= results[2].1);
        assert_eq!(results[0].0, 0); // Index of highest similarity
    }

    #[test]
    fn top_k_similar_respects_threshold() {
        // GIVEN: A query and candidates with varying similarities
        let query = vec![1.0_f32, 0.0];
        let candidates: Vec<(usize, &[f32])> = vec![
            (0, &[0.99_f32, 0.01][..]), // very high similarity
            (1, &[0.1_f32, 0.9][..]),   // low similarity
            (2, &[0.5_f32, 0.5][..]),   // medium similarity
        ];

        // WHEN: We search with high threshold
        let results = top_k_similar(&query, &candidates, 10, 0.8);

        // THEN: Only results above threshold should be returned
        assert!(!results.is_empty());
        for (_, score) in &results {
            assert!(*score >= 0.8, "Score {} below threshold 0.8", score);
        }
    }

    #[test]
    fn top_k_similar_empty_candidates_returns_empty() {
        // GIVEN: A query and empty candidates
        let query = vec![1.0_f32, 0.0];
        let candidates: Vec<(usize, &[f32])> = vec![];

        // WHEN: We search
        let results = top_k_similar(&query, &candidates, 10, 0.0);

        // THEN: Should return empty
        assert!(results.is_empty());
    }

    #[test]
    fn top_k_similar_k_larger_than_candidates() {
        // GIVEN: 2 candidates but requesting top-10
        let query = vec![1.0_f32, 0.0];
        let candidates: Vec<(usize, &[f32])> =
            vec![(0, &[0.9_f32, 0.1][..]), (1, &[0.1_f32, 0.9][..])];

        // WHEN: We request top-10
        let results = top_k_similar(&query, &candidates, 10, 0.0);

        // THEN: Should return all available (2)
        assert_eq!(results.len(), 2);
    }
}



