//! Integration tests for the semantic module
//!
//! Tests cover:
//! - EmbeddingModel types and dimensions
//! - CodeChunk creation and serialization
//! - Cosine similarity calculations
//! - Vector normalization
//! - Top-K similarity selection
//! - Chunker functionality
//! - Cache operations
//! - Index building and searching
//!
//! Note: Tests requiring model downloads are marked with #[ignore]

use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

use tldr_core::semantic::{
    // Functions
    cosine_similarity,
    is_normalized,
    normalize,
    top_k_similar,
    CacheConfig,
    ChunkGranularity,
    // Types
    CodeChunk,
    EmbeddedChunk,
    Embedder,
    EmbeddingCache,
    EmbeddingModel,
    SemanticIndex,
    SemanticSearchResult,
};

use tldr_core::semantic::index::{BuildOptions, SearchOptions as IndexSearchOptions};
use tldr_core::types::Language;

// ============================================================================
// Test Helpers
// ============================================================================

fn create_test_dir() -> TempDir {
    TempDir::new().unwrap()
}

fn write_file(dir: &TempDir, name: &str, content: &str) -> PathBuf {
    let path = dir.path().join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, content).unwrap();
    path
}

fn create_test_chunk() -> CodeChunk {
    CodeChunk {
        file_path: PathBuf::from("src/main.rs"),
        function_name: Some("process_data".to_string()),
        class_name: None,
        line_start: 10,
        line_end: 25,
        content: "fn process_data() { println!(\"hello\"); }".to_string(),
        content_hash: "abc123".to_string(),
        language: Language::Rust,
    }
}

// ============================================================================
// EmbeddingModel Tests
// ============================================================================

#[test]
fn test_embedding_model_default() {
    let model = EmbeddingModel::default();
    assert_eq!(model, EmbeddingModel::ArcticM);
}

#[test]
fn test_embedding_model_dimensions() {
    assert_eq!(EmbeddingModel::ArcticXS.dimensions(), 384);
    assert_eq!(EmbeddingModel::ArcticS.dimensions(), 384);
    assert_eq!(EmbeddingModel::ArcticM.dimensions(), 768);
    assert_eq!(EmbeddingModel::ArcticMLong.dimensions(), 768);
    assert_eq!(EmbeddingModel::ArcticL.dimensions(), 1024);
}

#[test]
fn test_embedding_model_max_context() {
    assert_eq!(EmbeddingModel::ArcticXS.max_context(), 512);
    assert_eq!(EmbeddingModel::ArcticS.max_context(), 512);
    assert_eq!(EmbeddingModel::ArcticM.max_context(), 512);
    assert_eq!(EmbeddingModel::ArcticMLong.max_context(), 8192);
    assert_eq!(EmbeddingModel::ArcticL.max_context(), 512);
}

#[test]
fn test_embedding_model_model_name() {
    assert!(EmbeddingModel::ArcticXS
        .model_name()
        .contains("arctic-embed-xs"));
    assert!(EmbeddingModel::ArcticM
        .model_name()
        .contains("arctic-embed-m"));
    assert!(EmbeddingModel::ArcticL
        .model_name()
        .contains("arctic-embed-l"));
}

#[test]
fn test_embedding_model_serialization() {
    let model = EmbeddingModel::ArcticM;
    let json = serde_json::to_string(&model).unwrap();
    assert_eq!(json, "\"arctic-m\"");

    let deserialized: EmbeddingModel = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, model);
}

// ============================================================================
// ChunkGranularity Tests
// ============================================================================

#[test]
fn test_chunk_granularity_default() {
    let granularity = ChunkGranularity::default();
    assert_eq!(granularity, ChunkGranularity::Function);
}

#[test]
fn test_chunk_granularity_serialization() {
    let file = ChunkGranularity::File;
    let json = serde_json::to_string(&file).unwrap();
    assert_eq!(json, "\"file\"");

    let func = ChunkGranularity::Function;
    let json = serde_json::to_string(&func).unwrap();
    assert_eq!(json, "\"function\"");
}

// ============================================================================
// CodeChunk Tests
// ============================================================================

#[test]
fn test_code_chunk_creation() {
    let chunk = create_test_chunk();

    assert_eq!(chunk.file_path, PathBuf::from("src/main.rs"));
    assert_eq!(chunk.function_name, Some("process_data".to_string()));
    assert_eq!(chunk.line_start, 10);
    assert_eq!(chunk.line_end, 25);
    assert!(!chunk.content.is_empty());
    assert_eq!(chunk.content_hash, "abc123");
    assert_eq!(chunk.language, Language::Rust);
}

#[test]
fn test_code_chunk_serialization_roundtrip() {
    let chunk = create_test_chunk();

    let json = serde_json::to_string(&chunk).unwrap();
    let deserialized: CodeChunk = serde_json::from_str(&json).unwrap();

    assert_eq!(chunk.file_path, deserialized.file_path);
    assert_eq!(chunk.function_name, deserialized.function_name);
    assert_eq!(chunk.class_name, deserialized.class_name);
    assert_eq!(chunk.line_start, deserialized.line_start);
    assert_eq!(chunk.line_end, deserialized.line_end);
    assert_eq!(chunk.content, deserialized.content);
    assert_eq!(chunk.content_hash, deserialized.content_hash);
    assert_eq!(chunk.language, deserialized.language);
}

#[test]
fn test_code_chunk_file_level() {
    let chunk = CodeChunk {
        file_path: PathBuf::from("src/lib.rs"),
        function_name: None, // File-level chunk
        class_name: None,
        line_start: 1,
        line_end: 100,
        content: "// Entire file content".to_string(),
        content_hash: "file_hash".to_string(),
        language: Language::Rust,
    };

    assert!(chunk.function_name.is_none());
}

// ============================================================================
// EmbeddedChunk Tests
// ============================================================================

#[test]
fn test_embedded_chunk_creation() {
    let chunk = create_test_chunk();
    let embedding = vec![0.1_f32; 768];

    let embedded = EmbeddedChunk { chunk, embedding };

    assert_eq!(embedded.embedding.len(), 768);
    assert_eq!(embedded.chunk.language, Language::Rust);
}

#[test]
fn test_embedded_chunk_serialization() {
    let chunk = create_test_chunk();
    let embedding = vec![0.1_f32, 0.2_f32, 0.3_f32];

    let embedded = EmbeddedChunk { chunk, embedding };
    let json = serde_json::to_string(&embedded).unwrap();
    let deserialized: EmbeddedChunk = serde_json::from_str(&json).unwrap();

    assert_eq!(embedded.embedding, deserialized.embedding);
}

// ============================================================================
// Cosine Similarity Tests
// ============================================================================

#[test]
fn test_cosine_similarity_identical_vectors() {
    let v = vec![0.5_f32, 0.5, 0.5, 0.5];
    let mut normalized = v.clone();
    normalize(&mut normalized);

    let sim = cosine_similarity(&normalized, &normalized);

    assert!((sim - 1.0).abs() < 1e-6, "Expected 1.0, got {}", sim);
}

#[test]
fn test_cosine_similarity_orthogonal_vectors() {
    let a = vec![1.0_f32, 0.0, 0.0];
    let b = vec![0.0_f32, 1.0, 0.0];

    let sim = cosine_similarity(&a, &b);

    assert!(sim.abs() < 1e-6, "Expected 0.0, got {}", sim);
}

#[test]
fn test_cosine_similarity_opposite_vectors() {
    let a = vec![1.0_f32, 0.0, 0.0];
    let b = vec![-1.0_f32, 0.0, 0.0];

    let sim = cosine_similarity(&a, &b);

    assert!((sim - (-1.0)).abs() < 1e-6, "Expected -1.0, got {}", sim);
}

#[test]
fn test_cosine_similarity_symmetric() {
    let a = vec![0.3_f32, 0.7, 0.2, 0.5];
    let b = vec![0.6_f32, 0.1, 0.8, 0.3];

    let sim_ab = cosine_similarity(&a, &b);
    let sim_ba = cosine_similarity(&b, &a);

    assert!((sim_ab - sim_ba).abs() < 1e-6);
}

#[test]
#[should_panic(expected = "Vectors must have same length")]
fn test_cosine_similarity_different_lengths_panics() {
    let a = vec![1.0_f32, 0.0, 0.0];
    let b = vec![1.0_f32, 0.0];

    let _ = cosine_similarity(&a, &b);
}

#[test]
fn test_cosine_similarity_zero_vectors() {
    let zero = vec![0.0_f32, 0.0, 0.0];
    let normal = vec![1.0_f32, 0.0, 0.0];

    // Zero vs zero
    let sim1 = cosine_similarity(&zero, &zero);
    assert!(sim1.abs() < 1e-6);
    assert!(!sim1.is_nan());

    // Zero vs normal
    let sim2 = cosine_similarity(&zero, &normal);
    assert!(sim2.abs() < 1e-6);
    assert!(!sim2.is_nan());
}

// ============================================================================
// Normalize Tests
// ============================================================================

#[test]
fn test_normalize_creates_unit_vector() {
    let mut v = vec![3.0_f32, 4.0, 0.0]; // Length 5

    normalize(&mut v);

    let l2_norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!((l2_norm - 1.0).abs() < 1e-6);
    assert!((v[0] - 0.6).abs() < 1e-6); // 3/5
    assert!((v[1] - 0.8).abs() < 1e-6); // 4/5
}

#[test]
fn test_normalize_zero_vector() {
    let mut v = vec![0.0_f32, 0.0, 0.0];

    normalize(&mut v);

    // Should stay all zeros
    for x in &v {
        assert!(x.abs() < 1e-6);
        assert!(!x.is_nan());
    }
}

#[test]
fn test_normalize_already_normalized() {
    let mut v = vec![0.6_f32, 0.8, 0.0];
    let original = v.clone();

    normalize(&mut v);

    for (a, b) in v.iter().zip(original.iter()) {
        assert!((a - b).abs() < 1e-6);
    }
}

// ============================================================================
// IsNormalized Tests
// ============================================================================

#[test]
fn test_is_normalized_detects_unit_vectors() {
    let unit = vec![0.6_f32, 0.8, 0.0];
    let non_unit = vec![3.0_f32, 4.0, 0.0];

    assert!(is_normalized(&unit));
    assert!(!is_normalized(&non_unit));
}

#[test]
fn test_is_normalized_false_for_zero() {
    let zero = vec![0.0_f32, 0.0, 0.0];
    assert!(!is_normalized(&zero));
}

// ============================================================================
// Top-K Similar Tests
// ============================================================================

#[test]
fn test_top_k_similar_returns_k_results() {
    let query = vec![1.0_f32, 0.0];
    let candidates: Vec<(usize, &[f32])> = vec![
        (0, &[0.9_f32, 0.1][..]),
        (1, &[0.1_f32, 0.9][..]),
        (2, &[0.7_f32, 0.3][..]),
        (3, &[0.8_f32, 0.2][..]),
    ];

    let results = top_k_similar(&query, &candidates, 2, 0.0);

    assert_eq!(results.len(), 2);
}

#[test]
fn test_top_k_similar_ordered_by_score() {
    let query = vec![1.0_f32, 0.0];
    let candidates: Vec<(usize, &[f32])> = vec![
        (0, &[0.9_f32, 0.1][..]),
        (1, &[0.1_f32, 0.9][..]),
        (2, &[0.7_f32, 0.3][..]),
    ];

    let results = top_k_similar(&query, &candidates, 3, 0.0);

    assert_eq!(results.len(), 3);
    assert!(results[0].1 >= results[1].1);
    assert!(results[1].1 >= results[2].1);
    assert_eq!(results[0].0, 0);
}

#[test]
fn test_top_k_similar_respects_threshold() {
    let query = vec![1.0_f32, 0.0];
    let candidates: Vec<(usize, &[f32])> = vec![
        (0, &[0.99_f32, 0.01][..]),
        (1, &[0.1_f32, 0.9][..]),
        (2, &[0.5_f32, 0.5][..]),
    ];

    let results = top_k_similar(&query, &candidates, 10, 0.8);

    assert!(!results.is_empty());
    for (_, score) in &results {
        assert!(*score >= 0.8);
    }
}

#[test]
fn test_top_k_similar_empty_candidates() {
    let query = vec![1.0_f32, 0.0];
    let candidates: Vec<(usize, &[f32])> = vec![];

    let results = top_k_similar(&query, &candidates, 10, 0.0);

    assert!(results.is_empty());
}

#[test]
fn test_top_k_similar_k_larger_than_candidates() {
    let query = vec![1.0_f32, 0.0];
    let candidates: Vec<(usize, &[f32])> = vec![(0, &[0.9_f32, 0.1][..]), (1, &[0.1_f32, 0.9][..])];

    let results = top_k_similar(&query, &candidates, 10, 0.0);

    assert_eq!(results.len(), 2);
}

#[test]
fn test_top_k_similar_k_zero() {
    let query = vec![1.0_f32, 0.0];
    let candidates: Vec<(usize, &[f32])> = vec![(0, &[0.9_f32, 0.1][..])];

    let results = top_k_similar(&query, &candidates, 0, 0.0);

    assert!(results.is_empty());
}

#[test]
fn test_top_k_similar_all_below_threshold() {
    let query = vec![1.0_f32, 0.0];
    let candidates: Vec<(usize, &[f32])> =
        vec![(0, &[0.0_f32, 1.0][..]), (1, &[-1.0_f32, 0.0][..])];

    let results = top_k_similar(&query, &candidates, 10, 0.5);

    assert!(results.is_empty());
}

// ============================================================================
// Options Tests
// ============================================================================

// EmbedOptions and ChunkOptions are internal types not publicly exported

// SearchOptions is exported as IndexSearchOptions from semantic::index

#[test]
fn test_cache_config_default() {
    let config = CacheConfig::default();
    assert!(config.max_size_mb > 0);
    assert!(config.ttl_days > 0);
}

// ============================================================================
// Chunker Tests
// ============================================================================

// Note: chunk_code and chunk_file are not publicly exported from the semantic module.
// They are internal implementation details used by SemanticIndex::build.

// ============================================================================
// Embedder Tests (require model - marked as ignore)
// ============================================================================

#[test]
#[ignore = "Requires model download (~110MB)"]
fn test_embedder_new() {
    let result = Embedder::new(EmbeddingModel::ArcticM);
    assert!(result.is_ok());

    let embedder = result.unwrap();
    assert_eq!(embedder.config(), EmbeddingModel::ArcticM);
    assert_eq!(embedder.dimensions(), 768);
}

#[test]
#[ignore = "Requires model download (~110MB)"]
fn test_embedder_embed_text() {
    let mut embedder = Embedder::new(EmbeddingModel::ArcticM).expect("Failed to init");

    let embedding = embedder
        .embed_text("fn process_data() { }")
        .expect("Failed to embed");

    assert_eq!(embedding.len(), 768);
    assert!(is_normalized(&embedding));
}

#[test]
#[ignore = "Requires model download (~110MB)"]
fn test_embedder_empty_text() {
    let mut embedder = Embedder::new(EmbeddingModel::ArcticM).expect("Failed to init");

    let embedding = embedder.embed_text("").expect("Failed to embed empty");

    assert_eq!(embedding.len(), 768);
    assert!(embedding.iter().all(|&x| x == 0.0));
}

#[test]
#[ignore = "Requires model download (~30MB)"]
fn test_embedder_xs_model() {
    let mut embedder = Embedder::new(EmbeddingModel::ArcticXS).expect("Failed to init XS");

    let embedding = embedder.embed_text("test").expect("Failed to embed");

    assert_eq!(embedding.len(), 384);
    assert!(is_normalized(&embedding));
}

// ============================================================================
// EmbeddingCache Tests
// ============================================================================

#[test]
fn test_embedding_cache_open() {
    let dir = create_test_dir();
    let config = CacheConfig {
        cache_dir: dir.path().to_path_buf(),
        max_size_mb: 100,
        ttl_days: 7,
    };

    let result = EmbeddingCache::open(config);
    assert!(result.is_ok());
}

// ============================================================================
// SemanticIndex Tests (require model - marked as ignore)
// ============================================================================

#[test]
#[ignore = "Requires model download (~110MB)"]
fn test_semantic_index_build() {
    let dir = create_test_dir();
    write_file(&dir, "test.py", "def process_data():\n    return 42");

    let build_opts = BuildOptions::default();
    let result = SemanticIndex::build(dir.path(), build_opts, None);
    assert!(result.is_ok());
}

#[test]
#[ignore = "Requires model download (~110MB)"]
fn test_semantic_index_search() {
    let dir = create_test_dir();
    write_file(&dir, "test.py", "def process_data():\n    return 42");

    let mut index = SemanticIndex::build(dir.path(), BuildOptions::default(), None)
        .expect("Failed to build index");

    let result = index.search("process data", &IndexSearchOptions::default());
    assert!(result.is_ok());
}

// ============================================================================
// SemanticSearchResult Tests
// ============================================================================

#[test]
fn test_semantic_search_result_ordering() {
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
        }];

    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());

    assert_eq!(results[0].function_name, Some("b".to_string()));
    assert_eq!(results[1].function_name, Some("a".to_string()));
}

// ============================================================================
// Integration Tests
// ============================================================================

#[test]
fn test_semantic_workflow_simulation() {
    // Simulate the semantic search workflow without actual embedding
    let _chunk1 = CodeChunk {
        file_path: PathBuf::from("src/utils.py"),
        function_name: Some("process_data".to_string()),
        class_name: None,
        line_start: 1,
        line_end: 10,
        content: "def process_data(data):\n    return data".to_string(),
        content_hash: "hash1".to_string(),
        language: Language::Python,
    };

    let _chunk2 = CodeChunk {
        file_path: PathBuf::from("src/main.py"),
        function_name: Some("analyze_data".to_string()),
        class_name: None,
        line_start: 1,
        line_end: 10,
        content: "def analyze_data(data):\n    return data * 2".to_string(),
        content_hash: "hash2".to_string(),
        language: Language::Python,
    };

    // Create normalized embeddings (simulated)
    let mut embedding1 = vec![0.9_f32, 0.1, 0.0, 0.0];
    normalize(&mut embedding1);

    let mut embedding2 = vec![0.1_f32, 0.9, 0.0, 0.0];
    normalize(&mut embedding2);

    // Create query
    let query = vec![1.0_f32, 0.0, 0.0, 0.0];

    // Find similar
    let candidates: Vec<(usize, &[f32])> = vec![(0, &embedding1), (1, &embedding2)];

    let results = top_k_similar(&query, &candidates, 2, 0.0);

    assert!(!results.is_empty());
    // chunk1 should be more similar to query
    assert_eq!(results[0].0, 0);
}
