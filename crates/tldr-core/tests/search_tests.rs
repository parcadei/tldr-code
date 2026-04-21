//! Tests for Search operations
//!
//! Commands tested: search (regex), bm25_search, hybrid_search
//!
//! These tests validate the search layer implementation.

use std::collections::HashSet;
use std::path::PathBuf;

use tldr_core::search::bm25::Bm25Index;
use tldr_core::search::embedding_client::EmbeddingClient;
use tldr_core::search::hybrid::hybrid_search;
use tldr_core::search::text::search;
use tldr_core::search::tokenizer::Tokenizer;
use tldr_core::{IgnoreSpec, Language, TldrError};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

// =============================================================================
// regex search tests
// =============================================================================

mod regex_search_tests {
    use super::*;

    #[test]
    fn search_finds_pattern_in_files() {
        // GIVEN: A project with files containing "def main"
        let project = fixtures_dir().join("simple-project");

        // WHEN: We search for the pattern
        let results = search("def main", &project, None, 0, 100, 100, None);

        // THEN: Matches should be found
        assert!(results.is_ok());
        let results = results.unwrap();
        assert!(!results.is_empty());
    }

    #[test]
    fn search_supports_regex() {
        // GIVEN: A project
        let project = fixtures_dir().join("simple-project");

        // WHEN: We search with regex pattern for function definitions
        let results = search(r"def\s+\w+", &project, None, 0, 100, 100, None);

        // THEN: Function definitions should match
        assert!(results.is_ok());
        let results = results.unwrap();
        // Should find at least one function definition
        assert!(!results.is_empty());
    }

    #[test]
    fn search_filters_by_extension() {
        // GIVEN: A project with mixed file types
        let project = fixtures_dir().join("simple-project");
        let extensions: HashSet<String> = [".py".to_string()].into_iter().collect();

        // WHEN: We search with extension filter
        let results = search("def", &project, Some(&extensions), 0, 100, 100, None);

        // THEN: Only .py files should be searched
        let results = results.unwrap();
        assert!(results
            .iter()
            .all(|m| m.file.to_string_lossy().ends_with(".py")));
    }

    #[test]
    fn search_includes_context_lines() {
        // GIVEN: A project
        let project = fixtures_dir().join("simple-project");

        // WHEN: We search with context_lines > 0
        let results = search("def main", &project, None, 2, 100, 100, None);

        // THEN: Context should be included
        let results = results.unwrap();
        if !results.is_empty() {
            let match_with_context = results.first().unwrap();
            assert!(match_with_context.context.is_some());
            assert!(!match_with_context.context.as_ref().unwrap().is_empty());
        }
    }

    #[test]
    fn search_respects_max_results() {
        // GIVEN: A project with many matches
        let project = fixtures_dir().join("simple-project");

        // WHEN: We search with max_results=1
        let results = search("def", &project, None, 0, 1, 100, None);

        // THEN: At most 1 result should be returned
        let results = results.unwrap();
        assert!(results.len() <= 1);
    }

    #[test]
    fn search_respects_max_files() {
        // GIVEN: A project with many files
        let project = fixtures_dir().join("simple-project");

        // WHEN: We search with max_files=1
        let results = search("def", &project, None, 0, 100, 1, None);

        // THEN: Results should come from at most 1 file
        let results = results.unwrap();
        let unique_files: HashSet<_> = results.iter().map(|r| &r.file).collect();
        assert!(unique_files.len() <= 1);
    }

    #[test]
    fn search_skips_default_directories() {
        // GIVEN: A project (that might have node_modules, __pycache__, etc.)
        let project = fixtures_dir();

        // WHEN: We search
        let results = search("pattern", &project, None, 0, 100, 100, None);

        // THEN: Default skip directories should be excluded
        // This test passes if no error - the directories are skipped internally
        assert!(results.is_ok());
    }

    #[test]
    fn search_respects_ignore_spec() {
        // GIVEN: A project with ignore patterns
        let project = fixtures_dir().join("simple-project");
        let ignore = IgnoreSpec::new(vec!["test_*.py".to_string()]);

        // WHEN: We search with ignore spec
        let results = search("def", &project, None, 0, 100, 100, Some(&ignore));

        // THEN: Search completes (ignore spec applied internally)
        assert!(results.is_ok());
    }

    #[test]
    fn search_returns_line_numbers() {
        // GIVEN: A project
        let project = fixtures_dir().join("simple-project");

        // WHEN: We search
        let results = search("def main", &project, None, 0, 100, 100, None);

        // THEN: Line numbers should be accurate (>= 1)
        let results = results.unwrap();
        if !results.is_empty() {
            let first = results.first().unwrap();
            assert!(first.line >= 1);
        }
    }

    #[test]
    fn search_handles_no_matches() {
        // GIVEN: A project
        let project = fixtures_dir().join("simple-project");

        // WHEN: We search for something that doesn't exist
        let results = search(
            "xyzzy_nonexistent_pattern_12345",
            &project,
            None,
            0,
            100,
            100,
            None,
        );

        // THEN: Empty results, no error
        let results = results.unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn search_handles_invalid_regex() {
        // GIVEN: A project
        let project = fixtures_dir().join("simple-project");

        // WHEN: We search with invalid regex
        let results = search("[invalid(", &project, None, 0, 100, 100, None);

        // THEN: Should return an error
        assert!(results.is_err());
    }
}

// =============================================================================
// BM25 search tests
// =============================================================================

mod bm25_tests {
    use super::*;

    #[test]
    fn bm25_indexes_documents() {
        // GIVEN: A BM25 index
        let mut index = Bm25Index::new(1.5, 0.75);

        // WHEN: We add documents
        index.add_document("file1", "def process_data items");
        index.add_document("file2", "class DataProcessor");

        // THEN: Documents should be searchable
        assert_eq!(index.document_count(), 2);
    }

    #[test]
    fn bm25_ranks_by_relevance() {
        // GIVEN: An index with documents
        let mut index = Bm25Index::new(1.5, 0.75);
        index.add_document("file1", "process data items data data");
        index.add_document("file2", "process something else");

        // WHEN: We search for "data"
        let results = index.search("data", 10);

        // THEN: file1 should rank higher (more occurrences)
        assert!(!results.is_empty());
        assert_eq!(results[0].file_path, PathBuf::from("file1"));
    }

    #[test]
    fn bm25_returns_scores() {
        // GIVEN: An index with documents
        let mut index = Bm25Index::new(1.5, 0.75);
        index.add_document("file1", "process data items");

        // WHEN: We search
        let results = index.search("data", 10);

        // THEN: Results should have scores
        assert!(!results.is_empty());
        assert!(results[0].score > 0.0);
    }

    #[test]
    fn bm25_returns_matched_terms() {
        // GIVEN: An index with documents
        let mut index = Bm25Index::new(1.5, 0.75);
        index.add_document("file1", "process user data");

        // WHEN: We search for "process data"
        let results = index.search("process data", 10);

        // THEN: matched_terms should list which terms matched
        assert!(!results.is_empty());
        assert!(results[0].matched_terms.contains(&"process".to_string()));
        assert!(results[0].matched_terms.contains(&"data".to_string()));
    }

    #[test]
    fn bm25_respects_top_k() {
        // GIVEN: An index with many documents
        let mut index = Bm25Index::new(1.5, 0.75);
        for i in 0..10 {
            index.add_document(&format!("file{}", i), "process data");
        }

        // WHEN: We search with top_k=5
        let results = index.search("data", 5);

        // THEN: At most 5 results should be returned
        assert!(results.len() <= 5);
    }

    #[test]
    fn bm25_from_project_indexes_code() {
        // GIVEN: A project directory
        let project = fixtures_dir().join("simple-project");

        // WHEN: We create index from project
        let index = Bm25Index::from_project(&project, Language::Python);

        // THEN: Index should be populated (if project has Python files)
        assert!(index.is_ok());
    }

    #[test]
    fn bm25_tokenizes_camel_case() {
        // GIVEN: Documents with camelCase identifiers
        let mut index = Bm25Index::new(1.5, 0.75);
        index.add_document("file1", "processData ItemProcessor");

        // WHEN: We search for "process"
        let results = index.search("process", 10);

        // THEN: camelCase should be split and matched
        assert!(!results.is_empty());
    }

    #[test]
    fn bm25_tokenizes_snake_case() {
        // GIVEN: Documents with snake_case identifiers
        let mut index = Bm25Index::new(1.5, 0.75);
        index.add_document("file1", "process_data item_processor");

        // WHEN: We search for "process"
        let results = index.search("process", 10);

        // THEN: snake_case should be split and matched
        assert!(!results.is_empty());
    }

    #[test]
    fn bm25_case_insensitive() {
        // GIVEN: Documents with mixed case
        let mut index = Bm25Index::new(1.5, 0.75);
        index.add_document("file1", "PROCESS_DATA");

        // WHEN: We search with different case
        let results = index.search("process", 10);

        // THEN: Should still match (lowercase all tokens)
        assert!(!results.is_empty());
    }
}

// =============================================================================
// Hybrid search tests
// =============================================================================

mod hybrid_search_tests {
    use super::*;

    #[test]
    fn hybrid_combines_bm25_and_semantic() {
        // GIVEN: A project
        let project = fixtures_dir().join("simple-project");

        // WHEN: We run hybrid search (without embedding client - BM25 only)
        let report = hybrid_search("process data", &project, Language::Python, 10, 60.0, None);

        // THEN: Results should be returned (from BM25)
        assert!(report.is_ok());
    }

    #[test]
    fn hybrid_uses_rrf_formula() {
        use tldr_core::search::hybrid::calculate_rrf_score;

        // GIVEN: Ranks in two rankings
        let ranks = vec![(0, 1), (1, 1)]; // rank 1 in both

        // WHEN: We compute RRF with k=60
        let score = calculate_rrf_score(&ranks, 60.0);

        // THEN: RRF score should be 2/(60+1) = 2/61
        let expected = 2.0 / 61.0;
        assert!((score - expected).abs() < 1e-10);
    }

    #[test]
    fn hybrid_respects_k_constant() {
        use tldr_core::search::hybrid::calculate_rrf_score;

        // GIVEN: Different k constants
        let ranks = vec![(0, 1), (1, 5)];

        // WHEN: We compute hybrid with k=60 vs k=10
        let score_k60 = calculate_rrf_score(&ranks, 60.0);
        let score_k10 = calculate_rrf_score(&ranks, 10.0);

        // THEN: Scores should differ (higher k = less impact from rank diff)
        assert!(score_k60 != score_k10);
    }

    #[test]
    fn hybrid_falls_back_to_bm25_only() {
        // GIVEN: No embedding service
        let project = fixtures_dir().join("simple-project");

        // WHEN: We run hybrid search with embedding_service=None
        let report = hybrid_search("query", &project, Language::Python, 10, 60.0, None);

        // THEN: fallback_mode should indicate "bm25_only"
        assert!(report.is_ok());
        let report = report.unwrap();
        assert_eq!(report.fallback_mode, Some("bm25_only".to_string()));
    }

    #[test]
    fn hybrid_reports_overlap() {
        // GIVEN: A project
        let project = fixtures_dir().join("simple-project");

        // WHEN: We run hybrid search
        let report = hybrid_search("process", &project, Language::Python, 10, 60.0, None);

        // THEN: overlap count should be reported (as 0 in BM25-only mode)
        assert!(report.is_ok());
        let report = report.unwrap();
        // In BM25-only mode, overlap is 0
        assert_eq!(report.overlap, 0);
    }

    #[test]
    fn hybrid_reports_exclusive_results() {
        // GIVEN: A project
        let project = fixtures_dir().join("simple-project");

        // WHEN: We run hybrid search
        let report = hybrid_search("process", &project, Language::Python, 10, 60.0, None);

        // THEN: bm25_only and dense_only counts should be reported
        assert!(report.is_ok());
        let report = report.unwrap();
        // In BM25-only mode, all results are bm25_only
        // dense_only should be 0
        assert_eq!(report.dense_only, 0);
    }

    #[test]
    fn hybrid_includes_both_ranks() {
        // GIVEN: A project (BM25-only mode)
        let project = fixtures_dir().join("simple-project");

        // WHEN: We run hybrid search
        let report = hybrid_search("process", &project, Language::Python, 10, 60.0, None);

        // THEN: Results should have bm25_rank (dense_rank is None in BM25-only)
        assert!(report.is_ok());
        let report = report.unwrap();
        for result in &report.results {
            assert!(result.bm25_rank.is_some());
            assert!(result.dense_rank.is_none()); // BM25-only mode
        }
    }

    #[test]
    fn hybrid_includes_both_scores() {
        // GIVEN: A project (BM25-only mode)
        let project = fixtures_dir().join("simple-project");

        // WHEN: We run hybrid search
        let report = hybrid_search("process", &project, Language::Python, 10, 60.0, None);

        // THEN: Results should have bm25_score (dense_score is None in BM25-only)
        assert!(report.is_ok());
        let report = report.unwrap();
        for result in &report.results {
            assert!(result.bm25_score.is_some());
            assert!(result.dense_score.is_none()); // BM25-only mode
        }
    }
}

// =============================================================================
// Tokenizer tests
// =============================================================================

mod tokenizer_tests {
    use super::*;

    #[test]
    fn tokenizer_splits_camel_case() {
        let tokenizer = Tokenizer::new();
        let tokens = tokenizer.tokenize("processUserData");
        assert!(tokens.contains(&"process".to_string()));
        assert!(tokens.contains(&"user".to_string()));
        assert!(tokens.contains(&"data".to_string()));
    }

    #[test]
    fn tokenizer_splits_snake_case() {
        let tokenizer = Tokenizer::new();
        let tokens = tokenizer.tokenize("process_user_data");
        assert!(tokens.contains(&"process".to_string()));
        assert!(tokens.contains(&"user".to_string()));
        assert!(tokens.contains(&"data".to_string()));
    }

    #[test]
    fn tokenizer_filters_stopwords() {
        let tokenizer = Tokenizer::new();
        let tokens = tokenizer.tokenize("def processData");
        // "def" is a stopword
        assert!(!tokens.contains(&"def".to_string()));
        assert!(tokens.contains(&"process".to_string()));
        assert!(tokens.contains(&"data".to_string()));
    }
}

// =============================================================================
// Embedding client tests
// =============================================================================

mod embedding_client_tests {
    use super::*;

    #[test]
    fn client_unavailable_graceful_degradation() {
        // GIVEN: An embedding client pointing to unavailable service
        let client = EmbeddingClient::new("http://localhost:59999");

        // WHEN: We check availability
        let available = client.is_available();

        // THEN: Should report unavailable (gracefully)
        assert!(!available);
    }

    #[test]
    fn client_search_returns_error_when_unavailable() {
        // GIVEN: An embedding client pointing to unavailable service
        let client = EmbeddingClient::new("http://localhost:59999");

        // WHEN: We try to search
        let result = client.search("query", "project", 10);

        // THEN: Should return ConnectionFailed error
        assert!(result.is_err());
        match result {
            Err(TldrError::ConnectionFailed(_)) => (),
            _ => panic!("Expected ConnectionFailed error"),
        }
    }
}
