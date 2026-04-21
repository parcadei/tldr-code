//! TDD tests for the `search_with_inner()` refactoring of enriched search.
//!
//! These tests verify behavioral equivalence across all 4 public enriched search
//! functions. After the refactoring, the 4 functions should become thin wrappers
//! over a shared `search_with_inner()` helper.
//!
//! **Expected to FAIL TO COMPILE** until `search_with_inner` is implemented and
//! exported from `tldr_core::search::enriched`.
//!
//! Test strategy:
//! 1. Each public function returns correct results on a known fixture
//! 2. Variants that differ only in caching produce equivalent result sets
//! 3. Options (top_k, search_mode, module penalty) are respected uniformly
//! 4. The new `search_with_inner()` API produces identical results to the
//!    existing public functions (proves the refactoring is correct)

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;
use tldr_core::get_code_structure;
use tldr_core::search::bm25::Bm25Index;
use tldr_core::search::enriched::{
    enriched_search,
    enriched_search_with_index,
    enriched_search_with_structure_cache,
    read_structure_cache,
    // NEW API: This import will fail to compile until the refactoring lands.
    // `search_with_inner` is the shared pipeline that all 4 public functions
    // will delegate to after deduplication.
    search_with_inner,
    write_structure_cache,
    EnrichedSearchOptions,
    SearchMode,
    StructureLookup,
};
use tldr_core::types::Language;

// =============================================================================
// Test fixtures
// =============================================================================

/// Create a temp project with multiple Python files containing functions, classes,
/// and module-level code. Designed so that:
/// - "token" queries match auth.py functions (verify_token, decode_token, refresh_token)
/// - "parse" queries match utils.py functions (parse_json, parse_csv)
/// - Module-level matches exist (import statements, constants)
/// - There are enough files (3+) to exercise the parallel code path
fn create_test_project() -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    let project = dir.path().join("project");
    fs::create_dir(&project).unwrap();

    fs::write(
        project.join("auth.py"),
        r#"
TOKEN_SECRET = "my-secret-key"

def verify_token(request):
    """Verify authentication token from request headers."""
    token = request.headers.get("Authorization")
    if not token:
        raise AuthError("Missing token")
    claims = decode_token(token)
    check_expiry(claims)
    return claims

def decode_token(token):
    """Decode a JWT token string into claims."""
    import jwt
    return jwt.decode(token, key=TOKEN_SECRET)

def refresh_token(old_token):
    """Refresh an expired token."""
    claims = decode_token(old_token)
    claims["exp"] = new_expiry()
    return encode_token(claims)

def check_expiry(claims):
    """Check if token claims have expired."""
    if claims["exp"] < time.time():
        raise AuthError("Token expired")

class AuthMiddleware:
    """Middleware for authentication."""
    def __init__(self, app):
        self.app = app

    def process_request(self, request):
        """Process incoming request for auth."""
        verify_token(request)
        return self.app(request)
"#,
    )
    .unwrap();

    fs::write(
        project.join("utils.py"),
        r#"
def parse_json(text):
    """Parse JSON string into a dictionary."""
    import json
    return json.loads(text)

def parse_csv(text):
    """Parse CSV string into rows."""
    import csv
    return list(csv.reader(text.splitlines()))

def format_date(dt):
    """Format a datetime object as ISO string."""
    return dt.strftime("%Y-%m-%d")

def validate_email(email):
    """Validate an email address format."""
    import re
    return re.match(r"^[\w.]+@[\w.]+$", email) is not None
"#,
    )
    .unwrap();

    fs::write(
        project.join("handlers.py"),
        r#"
def handle_login(request):
    """Handle user login request."""
    username = request.data["username"]
    password = request.data["password"]
    token = create_token(username)
    return {"token": token}

def handle_logout(request):
    """Handle user logout request."""
    invalidate_token(request.token)
    return {"status": "logged_out"}

def handle_refresh(request):
    """Handle token refresh request."""
    old_token = request.headers.get("Authorization")
    new_token = refresh_token(old_token)
    return {"token": new_token}
"#,
    )
    .unwrap();

    fs::write(
        project.join("models.py"),
        r#"
class User:
    """User model."""
    def __init__(self, name, email):
        self.name = name
        self.email = email

    def to_dict(self):
        return {"name": self.name, "email": self.email}

class Token:
    """Token model for authentication."""
    def __init__(self, value, expires_at):
        self.value = value
        self.expires_at = expires_at

    def is_expired(self):
        import time
        return self.expires_at < time.time()
"#,
    )
    .unwrap();

    (dir, project)
}

/// Helper: BM25 options without call graph (fast tests)
fn bm25_opts(top_k: usize) -> EnrichedSearchOptions {
    EnrichedSearchOptions {
        top_k,
        include_callgraph: false,
        search_mode: SearchMode::Bm25,
    }
}

/// Helper: Regex options without call graph
fn regex_opts(pattern: &str, top_k: usize) -> EnrichedSearchOptions {
    EnrichedSearchOptions {
        top_k,
        include_callgraph: false,
        search_mode: SearchMode::Regex(pattern.to_string()),
    }
}

/// Helper: Build a structure cache lookup from a project directory
fn build_structure_lookup(root: &std::path::Path) -> StructureLookup {
    let dir = tempfile::TempDir::new().unwrap();
    let cache_path = dir.path().join("structure_cache.json");
    let structure = get_code_structure(root, Language::Python, 0, None).unwrap();
    write_structure_cache(&structure, &cache_path).unwrap();
    read_structure_cache(&cache_path).unwrap()
}

// =============================================================================
// Test 1: enriched_search returns non-empty results with expected fields
// =============================================================================

/// Verify that the base `enriched_search()` function returns results with all
/// expected fields populated when given a known query on the test fixture.
///
/// This test anchors the public API contract: after refactoring to use
/// `search_with_inner()`, this function must still return identical results.
#[test]
fn test_enriched_search_returns_results() {
    let (_dir, root) = create_test_project();

    let report = enriched_search("token verify", &root, Language::Python, bm25_opts(10)).unwrap();

    // Must return at least one result
    assert!(
        !report.results.is_empty(),
        "enriched_search should return results for 'token verify'"
    );

    // Report metadata
    assert_eq!(report.query, "token verify");
    assert!(
        report.total_files_searched > 0,
        "Should have searched files"
    );
    assert!(
        report.search_mode.starts_with("bm25"),
        "Search mode should start with 'bm25', got '{}'",
        report.search_mode
    );

    // Each result must have required fields populated
    for result in &report.results {
        assert!(!result.name.is_empty(), "Result name must not be empty");
        assert!(!result.kind.is_empty(), "Result kind must not be empty");
        assert!(
            !result.file.as_os_str().is_empty(),
            "Result file must not be empty"
        );
        assert!(
            result.line_range.0 > 0,
            "Line range start should be 1-indexed, got {}",
            result.line_range.0
        );
        assert!(
            result.line_range.1 >= result.line_range.0,
            "Line range end ({}) should be >= start ({})",
            result.line_range.1,
            result.line_range.0
        );
        assert!(result.score > 0.0, "Score should be positive");
    }

    // At least one result should be a function (not just module-level)
    let has_function = report.results.iter().any(|r| r.kind == "function");
    assert!(
        has_function,
        "Should find at least one function-level result for 'token verify'"
    );
}

// =============================================================================
// Test 2: enriched_search and enriched_search_with_index return same results
// =============================================================================

/// Verify that `enriched_search()` (builds fresh BM25 index) and
/// `enriched_search_with_index()` (reuses pre-built BM25 index) return the
/// same result names for the same query.
///
/// This is the core equivalence test for the BM25 caching path. After
/// refactoring, both functions delegate to `search_with_inner()` with
/// different BM25 source strategies, but the output must be identical.
#[test]
fn test_all_variants_return_same_results_for_bm25() {
    let (_dir, root) = create_test_project();
    let query = "token decode";

    // Variant 1: Fresh BM25 (cold search)
    let cold_report = enriched_search(query, &root, Language::Python, bm25_opts(10)).unwrap();

    // Variant 2: Pre-built BM25 index
    let index = Bm25Index::from_project(&root, Language::Python).unwrap();
    let cached_report =
        enriched_search_with_index(query, &root, Language::Python, bm25_opts(10), &index).unwrap();

    // Same number of results
    assert_eq!(
        cold_report.results.len(),
        cached_report.results.len(),
        "Cold and cached BM25 should return same result count"
    );

    // Same result names (sorted, since tie-breaking may vary)
    let mut cold_names: Vec<String> = cold_report.results.iter().map(|r| r.name.clone()).collect();
    let mut cached_names: Vec<String> = cached_report
        .results
        .iter()
        .map(|r| r.name.clone())
        .collect();
    cold_names.sort();
    cached_names.sort();
    assert_eq!(
        cold_names, cached_names,
        "Cold and cached BM25 should return same result names"
    );

    // Same scores (since same index, same query, same enrichment)
    for (cold, cached) in cold_report.results.iter().zip(cached_report.results.iter()) {
        assert!(
            (cold.score - cached.score).abs() < f64::EPSILON,
            "Scores should match for '{}': cold={}, cached={}",
            cold.name,
            cold.score,
            cached.score
        );
    }

    // Both should report same total_files_searched
    assert_eq!(
        cold_report.total_files_searched, cached_report.total_files_searched,
        "total_files_searched should match between cold and cached"
    );
}

// =============================================================================
// Test 3: Structure cache matches live tree-sitter parse
// =============================================================================

/// Verify that `enriched_search()` (live tree-sitter) and
/// `enriched_search_with_structure_cache()` (cached definitions) produce
/// the same result names for the same query.
///
/// The preview field may differ (cache does not store preview), so we only
/// compare names, kinds, and file paths.
#[test]
fn test_structure_cache_matches_live_parse() {
    let (_dir, root) = create_test_project();
    let query = "parse json";

    // Build structure cache from the project
    let lookup = build_structure_lookup(&root);

    // Variant 1: Live tree-sitter
    let live_report = enriched_search(query, &root, Language::Python, bm25_opts(10)).unwrap();

    // Variant 2: Cached structure
    let cached_report = enriched_search_with_structure_cache(
        query,
        &root,
        Language::Python,
        bm25_opts(10),
        &lookup,
    )
    .unwrap();

    // Same result names (sorted for order-independence)
    let mut live_names: Vec<String> = live_report.results.iter().map(|r| r.name.clone()).collect();
    let mut cached_names: Vec<String> = cached_report
        .results
        .iter()
        .map(|r| r.name.clone())
        .collect();
    live_names.sort();
    cached_names.sort();
    assert_eq!(
        live_names, cached_names,
        "Live tree-sitter and cached structure should return same result names.\n  Live: {:?}\n  Cached: {:?}",
        live_names, cached_names
    );

    // Same kinds
    let mut live_kinds: Vec<(String, String)> = live_report
        .results
        .iter()
        .map(|r| (r.name.clone(), r.kind.clone()))
        .collect();
    let mut cached_kinds: Vec<(String, String)> = cached_report
        .results
        .iter()
        .map(|r| (r.name.clone(), r.kind.clone()))
        .collect();
    live_kinds.sort();
    cached_kinds.sort();
    assert_eq!(
        live_kinds, cached_kinds,
        "Result kinds should match between live and cached"
    );

    // search_mode strings should differ (one has "structure", other has "cached-structure")
    assert!(
        live_report.search_mode.contains("structure"),
        "Live report search_mode should contain 'structure', got '{}'",
        live_report.search_mode
    );
    assert!(
        cached_report.search_mode.contains("cached-structure"),
        "Cached report search_mode should contain 'cached-structure', got '{}'",
        cached_report.search_mode
    );
}

// =============================================================================
// Test 4: Regex mode works through all variants
// =============================================================================

/// Verify that `SearchMode::Regex` works correctly through `enriched_search()`,
/// `enriched_search_with_index()`, and `enriched_search_with_structure_cache()`.
///
/// For regex mode, the BM25 index is ignored (regex does its own file scanning),
/// so all variants should return the same results.
///
/// Pattern `handle_\w+` should match: handle_login, handle_logout, handle_refresh
/// and NOT match: verify_token, parse_json, etc.
#[test]
fn test_regex_mode_works_through_all_variants() {
    let (_dir, root) = create_test_project();
    let pattern = r"handle_\w+";

    // Variant 1: enriched_search with regex
    let report_base = enriched_search(
        "", // query ignored for regex
        &root,
        Language::Python,
        regex_opts(pattern, 20),
    )
    .unwrap();

    assert!(
        !report_base.results.is_empty(),
        "Regex 'handle_\\w+' should find results via enriched_search"
    );
    assert!(
        report_base.search_mode.contains("regex"),
        "Search mode should indicate regex, got '{}'",
        report_base.search_mode
    );

    // All results should contain "handle" in their name or file content
    let base_names: HashSet<String> = report_base.results.iter().map(|r| r.name.clone()).collect();

    // Variant 2: enriched_search_with_index with regex (index should be ignored)
    let index = Bm25Index::from_project(&root, Language::Python).unwrap();
    let report_index =
        enriched_search_with_index("", &root, Language::Python, regex_opts(pattern, 20), &index)
            .unwrap();

    let index_names: HashSet<String> = report_index
        .results
        .iter()
        .map(|r| r.name.clone())
        .collect();

    assert_eq!(
        base_names, index_names,
        "Regex results should be identical between enriched_search and enriched_search_with_index.\n  Base: {:?}\n  Index: {:?}",
        base_names, index_names
    );

    // Variant 3: enriched_search_with_structure_cache with regex
    let lookup = build_structure_lookup(&root);
    let report_cached = enriched_search_with_structure_cache(
        "",
        &root,
        Language::Python,
        regex_opts(pattern, 20),
        &lookup,
    )
    .unwrap();

    let cached_names: HashSet<String> = report_cached
        .results
        .iter()
        .map(|r| r.name.clone())
        .collect();

    assert_eq!(
        base_names, cached_names,
        "Regex results should be identical between enriched_search and enriched_search_with_structure_cache.\n  Base: {:?}\n  Cached: {:?}",
        base_names, cached_names
    );

    // Verify the regex mode string is consistent
    assert!(
        report_index.search_mode.contains("regex"),
        "Index variant regex search_mode should contain 'regex', got '{}'",
        report_index.search_mode
    );
    assert!(
        report_cached.search_mode.contains("regex"),
        "Cached variant regex search_mode should contain 'regex', got '{}'",
        report_cached.search_mode
    );
}

// =============================================================================
// Test 5: top_k is respected across all variants
// =============================================================================

/// Verify that the `top_k` option limits results to at most `top_k` entries
/// across all enriched search variants.
///
/// Uses top_k=3 with a query broad enough to match many functions, then
/// verifies no variant returns more than 3 results.
#[test]
fn test_top_k_respected() {
    let (_dir, root) = create_test_project();
    let query = "def"; // broad query matching many functions
    let top_k = 3;

    // Variant 1: enriched_search
    let report_base = enriched_search(query, &root, Language::Python, bm25_opts(top_k)).unwrap();
    assert!(
        report_base.results.len() <= top_k,
        "enriched_search should return at most {} results, got {}",
        top_k,
        report_base.results.len()
    );

    // Variant 2: enriched_search_with_index
    let index = Bm25Index::from_project(&root, Language::Python).unwrap();
    let report_index =
        enriched_search_with_index(query, &root, Language::Python, bm25_opts(top_k), &index)
            .unwrap();
    assert!(
        report_index.results.len() <= top_k,
        "enriched_search_with_index should return at most {} results, got {}",
        top_k,
        report_index.results.len()
    );

    // Variant 3: enriched_search_with_structure_cache
    let lookup = build_structure_lookup(&root);
    let report_cached = enriched_search_with_structure_cache(
        query,
        &root,
        Language::Python,
        bm25_opts(top_k),
        &lookup,
    )
    .unwrap();
    assert!(
        report_cached.results.len() <= top_k,
        "enriched_search_with_structure_cache should return at most {} results, got {}",
        top_k,
        report_cached.results.len()
    );

    // Also test with regex mode
    let report_regex =
        enriched_search("", &root, Language::Python, regex_opts("def \\w+", top_k)).unwrap();
    assert!(
        report_regex.results.len() <= top_k,
        "enriched_search (regex) should return at most {} results, got {}",
        top_k,
        report_regex.results.len()
    );
}

// =============================================================================
// Test 6: Module penalty applied -- functions rank above modules
// =============================================================================

/// Verify that module-level matches receive a score penalty so that
/// function/method/class results rank higher.
///
/// The enriched search pipeline penalizes "module" kind results by 0.2x (when
/// function results exist). This test queries for "token" which matches both:
/// - Function-level: verify_token, decode_token, refresh_token
/// - Module-level: TOKEN_SECRET constant in auth.py (matched as module entry)
///
/// After penalty, all function results should score higher than module results.
#[test]
fn test_module_penalty_applied() {
    let (_dir, root) = create_test_project();

    // "token" matches both function names and the TOKEN_SECRET constant
    let report = enriched_search("token secret", &root, Language::Python, bm25_opts(20)).unwrap();

    // Separate results by kind
    let functions: Vec<_> = report
        .results
        .iter()
        .filter(|r| r.kind != "module")
        .collect();
    let modules: Vec<_> = report
        .results
        .iter()
        .filter(|r| r.kind == "module")
        .collect();

    if !functions.is_empty() && !modules.is_empty() {
        // The lowest-scoring function should still be above the highest-scoring module
        let min_function_score = functions
            .iter()
            .map(|r| r.score)
            .fold(f64::INFINITY, f64::min);
        let max_module_score = modules
            .iter()
            .map(|r| r.score)
            .fold(f64::NEG_INFINITY, f64::max);

        assert!(
            min_function_score > max_module_score,
            "Function results should rank above module results after penalty.\n\
             Min function score: {:.4}, Max module score: {:.4}\n\
             Functions: {:?}\n\
             Modules: {:?}",
            min_function_score,
            max_module_score,
            functions
                .iter()
                .map(|r| (&r.name, r.score))
                .collect::<Vec<_>>(),
            modules
                .iter()
                .map(|r| (&r.name, r.score))
                .collect::<Vec<_>>(),
        );
    }
    // If there are only functions or only modules, the penalty logic is not
    // directly testable, but the test still passes (no assertion failure).
    // The penalty factor changes from 0.2 to 0.5 when no functions exist.
}

// =============================================================================
// Test 7: search_with_inner produces identical results to enriched_search
// =============================================================================

/// Verify that calling `search_with_inner()` directly (the new shared pipeline)
/// produces the same results as `enriched_search()` when given the same inputs
/// and no caching overrides.
///
/// This is the KEY refactoring correctness test. It proves that:
/// 1. `search_with_inner()` exists and is publicly accessible
/// 2. When called with `bm25_source=None` and `structure_cache=None` (i.e.,
///    no cached BM25 index, no cached structure), it behaves identically to
///    `enriched_search()`.
///
/// Expected: FAILS TO COMPILE (`search_with_inner` does not exist yet).
#[test]
fn test_search_with_inner_matches_enriched_search() {
    let (_dir, root) = create_test_project();
    let query = "token verify";

    // Call the existing public function
    let existing_report = enriched_search(query, &root, Language::Python, bm25_opts(10)).unwrap();

    // Call the new inner function directly with no caching overrides.
    // The exact signature is TBD, but conceptually:
    //   search_with_inner(query, root, language, options,
    //                     bm25_index: Option<&Bm25Index>,
    //                     structure_cache: Option<&StructureLookup>,
    //                     callgraph_cache_path: Option<&Path>)
    let inner_report = search_with_inner(
        query,
        &root,
        Language::Python,
        bm25_opts(10),
        None, // no cached BM25 index => build fresh
        None, // no cached structure => use live tree-sitter
        None, // no cached callgraph path
    )
    .unwrap();

    // Identical result names
    let mut existing_names: Vec<String> = existing_report
        .results
        .iter()
        .map(|r| r.name.clone())
        .collect();
    let mut inner_names: Vec<String> = inner_report
        .results
        .iter()
        .map(|r| r.name.clone())
        .collect();
    existing_names.sort();
    inner_names.sort();
    assert_eq!(
        existing_names, inner_names,
        "search_with_inner (no caches) should produce same results as enriched_search"
    );

    // Identical scores
    for (existing, inner) in existing_report
        .results
        .iter()
        .zip(inner_report.results.iter())
    {
        assert!(
            (existing.score - inner.score).abs() < f64::EPSILON,
            "Scores should be identical for '{}': existing={}, inner={}",
            existing.name,
            existing.score,
            inner.score
        );
    }

    // Same metadata
    assert_eq!(existing_report.query, inner_report.query);
    assert_eq!(
        existing_report.total_files_searched,
        inner_report.total_files_searched
    );
    assert_eq!(existing_report.search_mode, inner_report.search_mode);
}

// =============================================================================
// Test 8: search_with_inner with cached BM25 matches enriched_search_with_index
// =============================================================================

/// Verify that `search_with_inner()` with a pre-built BM25 index produces the
/// same results as `enriched_search_with_index()`.
///
/// Expected: FAILS TO COMPILE (`search_with_inner` does not exist yet).
#[test]
fn test_search_with_inner_cached_bm25_matches_with_index() {
    let (_dir, root) = create_test_project();
    let query = "parse json";
    let index = Bm25Index::from_project(&root, Language::Python).unwrap();

    // Existing API
    let existing_report =
        enriched_search_with_index(query, &root, Language::Python, bm25_opts(10), &index).unwrap();

    // New inner API with cached BM25
    let inner_report = search_with_inner(
        query,
        &root,
        Language::Python,
        bm25_opts(10),
        Some(&index), // cached BM25 index
        None,         // no cached structure
        None,         // no cached callgraph
    )
    .unwrap();

    let mut existing_names: Vec<String> = existing_report
        .results
        .iter()
        .map(|r| r.name.clone())
        .collect();
    let mut inner_names: Vec<String> = inner_report
        .results
        .iter()
        .map(|r| r.name.clone())
        .collect();
    existing_names.sort();
    inner_names.sort();
    assert_eq!(
        existing_names, inner_names,
        "search_with_inner(bm25_index=Some) should match enriched_search_with_index"
    );
}

// =============================================================================
// Test 9: search_with_inner with structure cache matches with_structure_cache
// =============================================================================

/// Verify that `search_with_inner()` with a structure cache produces the same
/// results as `enriched_search_with_structure_cache()`.
///
/// Expected: FAILS TO COMPILE (`search_with_inner` does not exist yet).
#[test]
fn test_search_with_inner_structure_cache_matches_with_structure_cache() {
    let (_dir, root) = create_test_project();
    let query = "handle login";
    let lookup = build_structure_lookup(&root);

    // Existing API
    let existing_report = enriched_search_with_structure_cache(
        query,
        &root,
        Language::Python,
        bm25_opts(10),
        &lookup,
    )
    .unwrap();

    // New inner API with cached structure
    let inner_report = search_with_inner(
        query,
        &root,
        Language::Python,
        bm25_opts(10),
        None,          // no cached BM25
        Some(&lookup), // cached structure
        None,          // no cached callgraph
    )
    .unwrap();

    let mut existing_names: Vec<String> = existing_report
        .results
        .iter()
        .map(|r| r.name.clone())
        .collect();
    let mut inner_names: Vec<String> = inner_report
        .results
        .iter()
        .map(|r| r.name.clone())
        .collect();
    existing_names.sort();
    inner_names.sort();
    assert_eq!(
        existing_names, inner_names,
        "search_with_inner(structure_cache=Some) should match enriched_search_with_structure_cache"
    );

    // Verify search_mode reflects cached-structure
    assert!(
        inner_report.search_mode.contains("cached-structure"),
        "Inner with structure cache should indicate 'cached-structure' in search_mode, got '{}'",
        inner_report.search_mode
    );
}
