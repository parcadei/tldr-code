//! Integration tests for enriched search with Hybrid mode (BM25 + Regex fusion).
//!
//! These tests define the TARGET API for `SearchMode::Hybrid` support in
//! enriched search. They are expected to FAIL TO COMPILE initially because
//! `SearchMode::Hybrid` does not exist yet.
//!
//! After implementation, all tests should pass and demonstrate:
//! 1. Hybrid returns the intersection of BM25 and Regex results
//! 2. Scores follow Reciprocal Rank Fusion (RRF) formula
//! 3. Empty intersection yields empty results
//! 4. Hybrid results are a subset of pure Regex results
//! 5. Hybrid results are a subset of pure BM25 results
//! 6. Report search_mode field is "hybrid(bm25+regex)"
//! 7. top_k is respected
//! 8. Works through enriched_search_with_structure_cache too

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;
use tldr_core::search::enriched::{
    enriched_search, enriched_search_with_structure_cache, EnrichedSearchOptions, SearchMode,
};
use tldr_core::types::Language;

// =============================================================================
// Test helper: create a project with known functions for hybrid matching
// =============================================================================

/// Create a temp directory with Rust files containing known function names.
///
/// Files created:
/// - `search.rs`: functions related to "search" with `pub fn` signatures
/// - `index.rs`: functions related to "index" with `pub fn` and private fn
/// - `utils.rs`: unrelated utility functions
///
/// This gives us a controlled corpus where:
/// - BM25("search") returns search.rs functions primarily
/// - Regex("pub fn") returns all `pub fn` definitions across files
/// - Intersection: `pub fn` definitions in search-relevant files
fn create_test_project() -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    let project = dir.path().join("project");
    fs::create_dir(&project).unwrap();

    fs::write(
        project.join("search.rs"),
        r#"
/// Perform a full-text search across documents.
pub fn full_text_search(query: &str, docs: &[Document]) -> Vec<SearchResult> {
    let tokens = tokenize_query(query);
    docs.iter()
        .filter(|d| matches_any(&tokens, d))
        .map(|d| SearchResult { doc: d.clone(), score: compute_score(query, d) })
        .collect()
}

/// Search by regex pattern matching.
pub fn regex_search(pattern: &str, docs: &[Document]) -> Vec<SearchResult> {
    let re = Regex::new(pattern).unwrap();
    docs.iter()
        .filter(|d| re.is_match(&d.content))
        .map(|d| SearchResult { doc: d.clone(), score: 1.0 })
        .collect()
}

/// Internal helper: tokenize a search query into terms.
fn tokenize_query(query: &str) -> Vec<String> {
    query.split_whitespace().map(|s| s.to_lowercase()).collect()
}

/// Internal helper: check if any tokens match the document.
fn matches_any(tokens: &[String], doc: &Document) -> bool {
    tokens.iter().any(|t| doc.content.contains(t))
}

/// Compute BM25 relevance score for search ranking.
pub fn compute_search_score(query: &str, doc: &Document) -> f64 {
    let tf = query.split_whitespace()
        .filter(|t| doc.content.contains(t))
        .count() as f64;
    tf / (tf + 1.2)
}
"#,
    )
    .unwrap();

    fs::write(
        project.join("index.rs"),
        r#"
/// Build an inverted index from a collection of documents.
pub fn build_index(docs: &[Document]) -> InvertedIndex {
    let mut index = InvertedIndex::new();
    for (id, doc) in docs.iter().enumerate() {
        for token in tokenize(&doc.content) {
            index.add(token, id);
        }
    }
    index
}

/// Search the index for matching documents.
pub fn search_index(index: &InvertedIndex, query: &str) -> Vec<usize> {
    let tokens = tokenize(query);
    tokens.iter()
        .flat_map(|t| index.lookup(t))
        .collect()
}

/// Internal: tokenize content into lowercase words.
fn tokenize(content: &str) -> Vec<String> {
    content.split_whitespace().map(|s| s.to_lowercase()).collect()
}

/// Merge two indexes together.
pub fn merge_indexes(a: &InvertedIndex, b: &InvertedIndex) -> InvertedIndex {
    let mut merged = a.clone();
    for (token, ids) in b.entries() {
        for id in ids {
            merged.add(token.clone(), *id);
        }
    }
    merged
}
"#,
    )
    .unwrap();

    fs::write(
        project.join("utils.rs"),
        r#"
/// Format a timestamp as an ISO date string.
pub fn format_date(ts: u64) -> String {
    let dt = DateTime::from_timestamp(ts);
    dt.format("%Y-%m-%d").to_string()
}

/// Parse a JSON configuration file.
pub fn parse_config(path: &str) -> Config {
    let content = std::fs::read_to_string(path).unwrap();
    serde_json::from_str(&content).unwrap()
}

/// Validate an email address format.
fn validate_email(email: &str) -> bool {
    email.contains('@') && email.contains('.')
}

/// Sanitize user input to prevent injection attacks.
fn sanitize_input(input: &str) -> String {
    input.replace('<', "&lt;").replace('>', "&gt;")
}
"#,
    )
    .unwrap();

    (dir, project)
}

/// Helper: create Hybrid search options.
fn hybrid_opts(query: &str, pattern: &str, top_k: usize) -> EnrichedSearchOptions {
    EnrichedSearchOptions {
        top_k,
        include_callgraph: false,
        search_mode: SearchMode::Hybrid {
            query: query.to_string(),
            pattern: pattern.to_string(),
        },
    }
}

/// Helper: create BM25 search options.
fn bm25_opts(top_k: usize) -> EnrichedSearchOptions {
    EnrichedSearchOptions {
        top_k,
        include_callgraph: false,
        search_mode: SearchMode::Bm25,
    }
}

/// Helper: create Regex search options.
fn regex_opts(pattern: &str, top_k: usize) -> EnrichedSearchOptions {
    EnrichedSearchOptions {
        top_k,
        include_callgraph: false,
        search_mode: SearchMode::Regex(pattern.to_string()),
    }
}

// =============================================================================
// Test 1: Hybrid returns intersection of BM25 and Regex results
// =============================================================================

/// Hybrid("search", "pub fn") should return results that match BOTH
/// "search" (BM25 relevant) AND "pub fn" (regex match).
///
/// All returned results must:
/// - Contain "pub fn" in their signature (regex side)
/// - Be relevant to the query "search" (BM25 side, from search-related files)
///
/// Expected: FAILS TO COMPILE (SearchMode::Hybrid does not exist yet).
#[test]
fn test_hybrid_returns_intersection_of_bm25_and_regex() {
    let (_dir, root) = create_test_project();

    let report = enriched_search(
        "search", // query param (used by BM25 internally via Hybrid)
        &root,
        Language::Rust,
        hybrid_opts("search", "pub fn", 20),
    )
    .unwrap();

    // Should find results (there are pub fn definitions in search-relevant files)
    assert!(
        !report.results.is_empty(),
        "Hybrid('search', 'pub fn') should return results from intersection"
    );

    // Every result must have "pub fn" in its signature (regex constraint)
    for result in &report.results {
        assert!(
            result.signature.contains("pub fn"),
            "Hybrid result '{}' must match regex 'pub fn' in signature, got: '{}'",
            result.name,
            result.signature
        );
    }

    // Results should be from search-relevant files (BM25 constraint).
    // At minimum, search.rs functions should appear since they are most relevant.
    let names: Vec<&str> = report.results.iter().map(|r| r.name.as_str()).collect();
    let has_search_fn = names
        .iter()
        .any(|n| n.contains("search") || n.contains("score"));
    assert!(
        has_search_fn,
        "Hybrid results should include search-relevant functions, got: {:?}",
        names
    );
}

// =============================================================================
// Test 2: Hybrid scores use RRF (Reciprocal Rank Fusion)
// =============================================================================

/// Run Hybrid, verify that scores follow the RRF formula.
///
/// RRF_score(d) = 1/(k + rank_A(d)) + 1/(k + rank_B(d)) where k=60
///
/// Properties:
/// - Top result has higher fused score than second result
/// - Maximum possible score per list = 1/(60+1) = 1/61
/// - Maximum total RRF score = 2/61 ~ 0.03279
/// - All scores should be in range (0, 2/61]
///
/// Expected: FAILS TO COMPILE (SearchMode::Hybrid does not exist yet).
#[test]
fn test_hybrid_scores_use_rrf() {
    let (_dir, root) = create_test_project();

    let report = enriched_search(
        "search",
        &root,
        Language::Rust,
        hybrid_opts("search", "pub fn", 20),
    )
    .unwrap();

    assert!(
        report.results.len() >= 2,
        "Need at least 2 results to test RRF ordering, got {}",
        report.results.len()
    );

    // RRF max score per list = 1/(60+1) = 1/61
    // Max combined = 2/61 ~ 0.03279
    let max_rrf = 2.0 / 61.0;

    for result in &report.results {
        assert!(
            result.score > 0.0,
            "RRF score must be positive, got {} for '{}'",
            result.score,
            result.name
        );
        assert!(
            result.score <= max_rrf + f64::EPSILON,
            "RRF score must be <= 2/61 ({:.6}), got {:.6} for '{}'",
            max_rrf,
            result.score,
            result.name
        );
    }

    // Results should be sorted by score descending
    for window in report.results.windows(2) {
        assert!(
            window[0].score >= window[1].score,
            "Results must be sorted by RRF score descending: '{}' ({}) should >= '{}' ({})",
            window[0].name,
            window[0].score,
            window[1].name,
            window[1].score
        );
    }
}

// =============================================================================
// Test 3: Hybrid returns empty when no intersection
// =============================================================================

/// Hybrid("search", "ZZZZNOTFOUND") should return empty results because
/// the regex matches nothing, so the intersection is empty.
///
/// Expected: FAILS TO COMPILE (SearchMode::Hybrid does not exist yet).
#[test]
fn test_hybrid_empty_when_no_intersection() {
    let (_dir, root) = create_test_project();

    let report = enriched_search(
        "search",
        &root,
        Language::Rust,
        hybrid_opts("search", "ZZZZNOTFOUND", 20),
    )
    .unwrap();

    assert!(
        report.results.is_empty(),
        "Hybrid with impossible regex should return no results, got {}",
        report.results.len()
    );
}

// =============================================================================
// Test 4: Hybrid results are a subset of pure Regex results
// =============================================================================

/// The hybrid intersection can only shrink compared to pure regex.
/// Every result name in hybrid must also appear in the pure regex results.
///
/// Expected: FAILS TO COMPILE (SearchMode::Hybrid does not exist yet).
#[test]
fn test_hybrid_vs_pure_regex_subset() {
    let (_dir, root) = create_test_project();

    // Run pure regex search with large top_k to capture all matches
    let regex_report =
        enriched_search("", &root, Language::Rust, regex_opts("pub fn", 50)).unwrap();

    // Run hybrid search
    let hybrid_report = enriched_search(
        "search",
        &root,
        Language::Rust,
        hybrid_opts("search", "pub fn", 50),
    )
    .unwrap();

    // Build set of (file, name) from regex results
    let regex_set: HashSet<(String, String)> = regex_report
        .results
        .iter()
        .map(|r| (r.file.display().to_string(), r.name.clone()))
        .collect();

    // Every hybrid result must exist in the regex results
    for result in &hybrid_report.results {
        let key = (result.file.display().to_string(), result.name.clone());
        assert!(
            regex_set.contains(&key),
            "Hybrid result ({}, '{}') must be in pure Regex results. \
             Regex had: {:?}",
            result.file.display(),
            result.name,
            regex_set
        );
    }

    // Hybrid should have fewer or equal results (intersection shrinks)
    assert!(
        hybrid_report.results.len() <= regex_report.results.len(),
        "Hybrid ({}) should have <= results than pure Regex ({})",
        hybrid_report.results.len(),
        regex_report.results.len()
    );
}

// =============================================================================
// Test 5: Hybrid results are a subset of pure BM25 results
// =============================================================================

/// The hybrid intersection can only shrink compared to pure BM25.
/// Every result name in hybrid must also appear in the pure BM25 results.
///
/// Expected: FAILS TO COMPILE (SearchMode::Hybrid does not exist yet).
#[test]
fn test_hybrid_vs_pure_bm25_subset() {
    let (_dir, root) = create_test_project();

    // Run pure BM25 search with large top_k
    let bm25_report = enriched_search("search", &root, Language::Rust, bm25_opts(50)).unwrap();

    // Run hybrid search
    let hybrid_report = enriched_search(
        "search",
        &root,
        Language::Rust,
        hybrid_opts("search", "pub fn", 50),
    )
    .unwrap();

    // Build set of (file, name) from BM25 results
    let bm25_set: HashSet<(String, String)> = bm25_report
        .results
        .iter()
        .map(|r| (r.file.display().to_string(), r.name.clone()))
        .collect();

    // Every hybrid result must exist in the BM25 results
    for result in &hybrid_report.results {
        let key = (result.file.display().to_string(), result.name.clone());
        assert!(
            bm25_set.contains(&key),
            "Hybrid result ({}, '{}') must be in pure BM25 results. \
             BM25 had: {:?}",
            result.file.display(),
            result.name,
            bm25_set
        );
    }

    // Hybrid should have fewer or equal results (intersection shrinks)
    assert!(
        hybrid_report.results.len() <= bm25_report.results.len(),
        "Hybrid ({}) should have <= results than pure BM25 ({})",
        hybrid_report.results.len(),
        bm25_report.results.len()
    );
}

// =============================================================================
// Test 6: Report search_mode field is "hybrid(bm25+regex)"
// =============================================================================

/// The report's search_mode string should be "hybrid(bm25+regex)" to
/// distinguish it from pure "bm25+structure" or "regex+structure".
///
/// Expected: FAILS TO COMPILE (SearchMode::Hybrid does not exist yet).
#[test]
fn test_hybrid_report_search_mode_field() {
    let (_dir, root) = create_test_project();

    let report = enriched_search(
        "search",
        &root,
        Language::Rust,
        hybrid_opts("search", "pub fn", 20),
    )
    .unwrap();

    // The enriched pipeline appends "+structure" (or "+cached-structure+callgraph")
    // to the mode prefix. With include_callgraph=false and no structure cache,
    // the result is "hybrid(bm25+regex)+structure".
    assert!(
        report.search_mode.starts_with("hybrid(bm25+regex)"),
        "Hybrid search_mode should start with 'hybrid(bm25+regex)', got: '{}'",
        report.search_mode
    );
    assert_eq!(
        report.search_mode, "hybrid(bm25+regex)+structure",
        "Hybrid search_mode should be 'hybrid(bm25+regex)+structure' without callgraph, got: '{}'",
        report.search_mode
    );
}

// =============================================================================
// Test 7: Hybrid respects top_k
// =============================================================================

/// With top_k=2, at most 2 results should be returned even if the
/// intersection contains more.
///
/// Expected: FAILS TO COMPILE (SearchMode::Hybrid does not exist yet).
#[test]
fn test_hybrid_respects_top_k() {
    let (_dir, root) = create_test_project();

    let report = enriched_search(
        "search",
        &root,
        Language::Rust,
        hybrid_opts("search", "pub fn", 2),
    )
    .unwrap();

    assert!(
        report.results.len() <= 2,
        "top_k=2 should return at most 2 results, got {}",
        report.results.len()
    );
}

// =============================================================================
// Test 8: Hybrid works through enriched_search_with_structure_cache
// =============================================================================

/// Hybrid mode should also work through the structure-cache API path.
/// This ensures the Hybrid arm is handled in all enriched search entry points.
///
/// Expected: FAILS TO COMPILE (SearchMode::Hybrid does not exist yet).
#[test]
fn test_hybrid_with_structure_cache() {
    let (_dir, root) = create_test_project();

    // First, run a normal search to establish a baseline
    let baseline = enriched_search(
        "search",
        &root,
        Language::Rust,
        hybrid_opts("search", "pub fn", 20),
    )
    .unwrap();

    // Build a structure lookup from the project files.
    // We use the write/read_structure_cache round-trip to get a StructureLookup.
    let structure = tldr_core::ast::get_code_structure(&root, Language::Rust, 0, None).unwrap();

    let cache_dir = root.join(".tldr").join("cache");
    fs::create_dir_all(&cache_dir).unwrap();
    let cache_path = cache_dir.join("structure.json");

    tldr_core::search::enriched::write_structure_cache(&structure, &cache_path).unwrap();
    let lookup = tldr_core::search::enriched::read_structure_cache(&cache_path).unwrap();

    // Run hybrid search through the structure-cache path
    let cached_report = enriched_search_with_structure_cache(
        "search",
        &root,
        Language::Rust,
        hybrid_opts("search", "pub fn", 20),
        &lookup,
    )
    .unwrap();

    // Should produce results (same intersection logic)
    assert!(
        !cached_report.results.is_empty(),
        "Hybrid through structure cache should find results"
    );

    // search_mode should still indicate hybrid
    assert!(
        cached_report.search_mode.contains("hybrid"),
        "Cached hybrid search_mode should contain 'hybrid', got: '{}'",
        cached_report.search_mode
    );

    // Result names from cached path should be a subset of (or equal to) baseline
    // (Structure cache may produce slightly different enrichment but same intersection)
    let baseline_names: HashSet<&str> = baseline.results.iter().map(|r| r.name.as_str()).collect();
    let cached_names: HashSet<&str> = cached_report
        .results
        .iter()
        .map(|r| r.name.as_str())
        .collect();

    // At minimum, both should find search-related pub fn results
    let has_search_result = cached_names.iter().any(|n| n.contains("search"));
    assert!(
        has_search_result || cached_report.results.is_empty(),
        "Cached hybrid should find search-related results if any exist. \
         Baseline: {:?}, Cached: {:?}",
        baseline_names,
        cached_names
    );
}
