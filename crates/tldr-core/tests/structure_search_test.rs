//! Integration tests for enriched_search_with_structure_cache.
//!
//! These tests verify that enriched search can consume a pre-built structure
//! cache to avoid tree-sitter re-parsing during search enrichment.

use std::path::PathBuf;

use tempfile::TempDir;
use tldr_core::{
    enriched_search, enriched_search_with_structure_cache, get_code_structure,
    read_structure_cache, write_structure_cache, EnrichedSearchOptions, Language, SearchMode,
};

/// Helper: create options without call graph
fn opts(top_k: usize) -> EnrichedSearchOptions {
    EnrichedSearchOptions {
        top_k,
        include_callgraph: false,
        search_mode: SearchMode::Bm25,
    }
}

#[test]
fn test_enriched_search_with_structure_cache_returns_results() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("project");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("lib.py"),
        r#"def search_files(pattern):
    """Search for files matching pattern."""
    return []

def filter_results(results, predicate):
    """Filter search results."""
    return [r for r in results if predicate(r)]

class SearchEngine:
    def run_query(self, query):
        return search_files(query)
"#,
    )
    .unwrap();

    // Build structure cache
    let structure = get_code_structure(&src, Language::Python, 0, None).unwrap();
    let cache_path = dir.path().join("structure.json");
    write_structure_cache(&structure, &cache_path).unwrap();
    let lookup = read_structure_cache(&cache_path).unwrap();

    let options = EnrichedSearchOptions {
        top_k: 10,
        include_callgraph: false,
        search_mode: SearchMode::Bm25,
    };

    let report =
        enriched_search_with_structure_cache("search", &src, Language::Python, options, &lookup)
            .unwrap();
    assert!(
        !report.results.is_empty(),
        "Should find results for 'search'"
    );

    // Results should have function-level info
    for result in &report.results {
        assert!(!result.name.is_empty());
        assert!(result.line_range.0 > 0);
        assert!(result.line_range.1 >= result.line_range.0);
    }
}

#[test]
fn test_enriched_search_with_structure_cache_matches_uncached() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("project");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("math.py"),
        r#"def add(a, b):
    return a + b

def multiply(a, b):
    return a * b

def divide(a, b):
    if b == 0:
        raise ValueError("division by zero")
    return a / b
"#,
    )
    .unwrap();

    let structure = get_code_structure(&src, Language::Python, 0, None).unwrap();
    let cache_path = dir.path().join("structure.json");
    write_structure_cache(&structure, &cache_path).unwrap();
    let lookup = read_structure_cache(&cache_path).unwrap();

    let options_cached = EnrichedSearchOptions {
        top_k: 10,
        include_callgraph: false,
        search_mode: SearchMode::Bm25,
    };
    let options_uncached = EnrichedSearchOptions {
        top_k: 10,
        include_callgraph: false,
        search_mode: SearchMode::Bm25,
    };

    let cached = enriched_search_with_structure_cache(
        "divide",
        &src,
        Language::Python,
        options_cached,
        &lookup,
    )
    .unwrap();
    let uncached = enriched_search("divide", &src, Language::Python, options_uncached).unwrap();

    // Same result names and kinds
    let mut cached_names: Vec<_> = cached
        .results
        .iter()
        .map(|r| (r.name.clone(), r.kind.clone()))
        .collect();
    let mut uncached_names: Vec<_> = uncached
        .results
        .iter()
        .map(|r| (r.name.clone(), r.kind.clone()))
        .collect();
    cached_names.sort();
    uncached_names.sort();
    assert_eq!(
        cached_names, uncached_names,
        "Cached and uncached should return same results"
    );
}

#[test]
fn test_enriched_search_with_structure_cache_regex_mode() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("project");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("handlers.py"),
        r#"def handle_request(req):
    return process(req)

def handle_error(err):
    log(err)
    return None
"#,
    )
    .unwrap();

    let structure = get_code_structure(&src, Language::Python, 0, None).unwrap();
    let cache_path = dir.path().join("structure.json");
    write_structure_cache(&structure, &cache_path).unwrap();
    let lookup = read_structure_cache(&cache_path).unwrap();

    let options = EnrichedSearchOptions {
        top_k: 10,
        include_callgraph: false,
        search_mode: SearchMode::Regex("handle_\\w+".to_string()),
    };

    let report =
        enriched_search_with_structure_cache("handle", &src, Language::Python, options, &lookup)
            .unwrap();
    assert!(!report.results.is_empty(), "Regex mode should find results");
    assert!(
        report.search_mode.contains("regex"),
        "Search mode should indicate regex"
    );
    assert!(
        report.search_mode.contains("cached-structure"),
        "Search mode should indicate cached-structure"
    );
}

#[test]
fn test_enriched_search_with_structure_cache_falls_back_on_miss() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("project");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.py"), "def alpha():\n    return 1\n").unwrap();
    std::fs::write(src.join("b.py"), "def beta():\n    return 2\n").unwrap();

    // Build full structure cache, then remove b.py from the lookup
    let structure = get_code_structure(&src, Language::Python, 0, None).unwrap();
    let cache_path = dir.path().join("structure.json");
    write_structure_cache(&structure, &cache_path).unwrap();
    let mut lookup = read_structure_cache(&cache_path).unwrap();
    // Remove b.py from the lookup to simulate a cache miss
    lookup.by_file.remove(&PathBuf::from("b.py"));

    let options = EnrichedSearchOptions {
        top_k: 10,
        include_callgraph: false,
        search_mode: SearchMode::Bm25,
    };

    // Search for "beta return" which is only in b.py (not cached)
    let report = enriched_search_with_structure_cache(
        "beta return",
        &src,
        Language::Python,
        options,
        &lookup,
    )
    .unwrap();
    // Should still find results via fallback to tree-sitter
    assert!(
        !report.results.is_empty(),
        "Should fall back to tree-sitter for uncached files"
    );
}

#[test]
fn test_enriched_search_with_structure_cache_search_mode_string() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("project");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("x.py"), "def foo():\n    return 42\n").unwrap();

    let structure = get_code_structure(&src, Language::Python, 0, None).unwrap();
    let cache_path = dir.path().join("structure.json");
    write_structure_cache(&structure, &cache_path).unwrap();
    let lookup = read_structure_cache(&cache_path).unwrap();

    // BM25 mode
    let report_bm25 =
        enriched_search_with_structure_cache("foo", &src, Language::Python, opts(10), &lookup)
            .unwrap();
    assert!(
        report_bm25.search_mode.contains("cached-structure"),
        "BM25 mode should contain 'cached-structure', got '{}'",
        report_bm25.search_mode
    );
    assert!(
        report_bm25.search_mode.contains("bm25"),
        "BM25 mode should contain 'bm25', got '{}'",
        report_bm25.search_mode
    );
}
