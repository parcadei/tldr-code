//! Integration tests for enriched search caching APIs.
//!
//! These tests define the TARGET API for BM25 index caching and call graph
//! cache integration. They are expected to FAIL TO COMPILE initially because
//! the APIs they test do not yet exist.
//!
//! After optimization, all tests should pass and demonstrate:
//! 1. Pre-built BM25 index produces identical results to cold search
//! 2. Cached call graph produces populated callers/callees
//! 3. BM25 index round-trips through serde serialization
//! 4. Call graph cache file can be read and used for enrichment

use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;
use tldr_core::search::bm25::Bm25Index;
use tldr_core::search::enriched::{
    enriched_search, enriched_search_with_index, EnrichedSearchOptions,
};
use tldr_core::types::Language;

/// Test helper: create options without call graph (fast tests)
fn opts(top_k: usize) -> EnrichedSearchOptions {
    EnrichedSearchOptions {
        top_k,
        include_callgraph: false,
        ..Default::default()
    }
}

/// Helper: create a temp directory with some Python files for testing.
/// Identical to the helper in enriched.rs tests.
fn create_test_project() -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    let project = dir.path().join("project");
    fs::create_dir(&project).unwrap();

    fs::write(
        project.join("auth.py"),
        r#"
def verify_jwt_token(request):
    """Verify JWT token from request headers."""
    token = request.headers.get("Authorization")
    if not token:
        raise AuthError("Missing token")
    claims = decode_token(token)
    check_expiry(claims)
    return claims

def decode_token(token):
    """Decode a JWT token string."""
    import jwt
    return jwt.decode(token, key="secret")

def check_expiry(claims):
    """Check if token has expired."""
    if claims["exp"] < time.time():
        raise AuthError("Token expired")

class AuthMiddleware:
    """Middleware for authentication."""
    def __init__(self, app):
        self.app = app

    def process_request(self, request):
        """Process incoming request for auth."""
        verify_jwt_token(request)
        return self.app(request)
"#,
    )
    .unwrap();

    fs::write(
        project.join("routes.py"),
        r#"
def user_routes(app):
    """Register user routes."""
    @app.route("/users")
    def list_users():
        return get_all_users()

def admin_routes(app):
    """Register admin routes."""
    @app.route("/admin")
    def admin_panel():
        return render_admin()

def get_all_users():
    """Fetch all users from database."""
    return db.query("SELECT * FROM users")

def render_admin():
    """Render admin panel."""
    return template.render("admin.html")
"#,
    )
    .unwrap();

    fs::write(
        project.join("utils.py"),
        r#"
def format_date(dt):
    """Format a datetime object."""
    return dt.strftime("%Y-%m-%d")

def parse_json(text):
    """Parse JSON string."""
    import json
    return json.loads(text)
"#,
    )
    .unwrap();

    (dir, project)
}

// =============================================================================
// Test 1: enriched_search with a pre-built (cached) BM25 index
// =============================================================================

/// Tests that enriched_search_with_index() produces the same results as
/// enriched_search() when given a pre-built BM25 index.
///
/// This validates the new API: instead of rebuilding the index from disk every
/// time, the caller can pass an already-built Bm25Index.
///
/// Expected: FAILS TO COMPILE (enriched_search_with_index does not exist yet).
#[test]
fn test_enriched_search_with_cached_bm25_index() {
    let (_dir, root) = create_test_project();
    let query = "jwt token verify";

    // Run the normal (cold) search
    let cold_report = enriched_search(query, &root, Language::Python, opts(10)).unwrap();

    // Pre-build the BM25 index
    let index = Bm25Index::from_project(&root, Language::Python).unwrap();

    // Run the cached search with the pre-built index
    let cached_report =
        enriched_search_with_index(query, &root, Language::Python, opts(10), &index).unwrap();

    // Results must be identical
    assert_eq!(
        cold_report.results.len(),
        cached_report.results.len(),
        "Cached search should return same number of results"
    );
    assert_eq!(
        cold_report.total_files_searched, cached_report.total_files_searched,
        "total_files_searched should match"
    );
    assert_eq!(
        cold_report.search_mode, cached_report.search_mode,
        "search_mode should match"
    );
    assert_eq!(
        cold_report.query, cached_report.query,
        "query should be preserved"
    );

    // Verify result names match in order
    let cold_names: Vec<&str> = cold_report
        .results
        .iter()
        .map(|r| r.name.as_str())
        .collect();
    let cached_names: Vec<&str> = cached_report
        .results
        .iter()
        .map(|r| r.name.as_str())
        .collect();
    assert_eq!(
        cold_names, cached_names,
        "Result names and ordering should be identical"
    );

    // Verify scores match
    for (cold, cached) in cold_report.results.iter().zip(cached_report.results.iter()) {
        assert!(
            (cold.score - cached.score).abs() < f64::EPSILON,
            "Scores should be identical for '{}': cold={}, cached={}",
            cold.name,
            cold.score,
            cached.score
        );
    }
}

// =============================================================================
// Test 2: enriched_search with cached call graph
// =============================================================================

/// Tests that enriched_search can use a pre-built call graph for caller/callee
/// enrichment, instead of rebuilding the full V2 call graph from scratch.
///
/// This validates the call graph cache path: read .tldr/cache/call_graph.json
/// and use it for O(ms) enrichment instead of O(50s) rebuild.
///
/// Expected: FAILS TO COMPILE (enriched_search_with_callgraph_cache does not exist yet).
#[test]
fn test_enriched_search_with_cached_callgraph() {
    let (_dir, root) = create_test_project();

    // Create a mock call graph cache in the expected location
    let cache_dir = root.join(".tldr").join("cache");
    fs::create_dir_all(&cache_dir).unwrap();

    // Write a minimal call graph cache that matches our test project
    let cache_json = serde_json::json!({
        "edges": [
            {
                "from_file": "auth.py",
                "from_func": "verify_jwt_token",
                "to_file": "auth.py",
                "to_func": "decode_token"
            },
            {
                "from_file": "auth.py",
                "from_func": "verify_jwt_token",
                "to_file": "auth.py",
                "to_func": "check_expiry"
            },
            {
                "from_file": "auth.py",
                "from_func": "process_request",
                "to_file": "auth.py",
                "to_func": "verify_jwt_token"
            }
        ],
        "languages": ["python"],
        "timestamp": 1740000000
    });

    fs::write(
        cache_dir.join("call_graph.json"),
        serde_json::to_string_pretty(&cache_json).unwrap(),
    )
    .unwrap();

    // Search with callgraph using cache
    let options = EnrichedSearchOptions {
        top_k: 10,
        include_callgraph: true,
        ..Default::default()
    };

    // TARGET API: enriched_search should detect and use the cache file
    // when include_callgraph=true, instead of rebuilding from scratch.
    //
    // This uses the existing enriched_search function signature --
    // the optimization is internal (detect .tldr/cache/call_graph.json).
    //
    // For now, we use the explicit cache API to test the behavior:
    let report = tldr_core::search::enriched::enriched_search_with_callgraph_cache(
        "jwt token verify",
        &root,
        Language::Python,
        options,
        &cache_dir.join("call_graph.json"),
    )
    .unwrap();

    assert!(!report.results.is_empty(), "Should find results");
    assert_eq!(report.search_mode, "bm25+structure+callgraph");

    // Find verify_jwt_token and check it has callees populated from cache
    let verify = report.results.iter().find(|r| r.name == "verify_jwt_token");
    assert!(verify.is_some(), "Should find verify_jwt_token in results");

    let verify = verify.unwrap();
    assert!(
        !verify.callees.is_empty(),
        "verify_jwt_token should have callees populated from cache, got: {:?}",
        verify.callees
    );

    // Verify specific callees from cache
    assert!(
        verify.callees.contains(&"decode_token".to_string()),
        "verify_jwt_token should call decode_token, got: {:?}",
        verify.callees
    );
    assert!(
        verify.callees.contains(&"check_expiry".to_string()),
        "verify_jwt_token should call check_expiry, got: {:?}",
        verify.callees
    );

    // Check callers (process_request calls verify_jwt_token)
    assert!(
        !verify.callers.is_empty(),
        "verify_jwt_token should have callers populated from cache, got: {:?}",
        verify.callers
    );
    assert!(
        verify.callers.contains(&"process_request".to_string()),
        "verify_jwt_token should be called by process_request, got: {:?}",
        verify.callers
    );
}

// =============================================================================
// Test 3: BM25 index serialization round-trip
// =============================================================================

/// Tests that a Bm25Index can be serialized (e.g., to bincode/JSON) and
/// deserialized, and that search produces identical results after round-trip.
///
/// This is essential for the daemon caching strategy: serialize the index
/// to disk, load it on the next query, avoid rebuilding from files.
///
/// Expected: FAILS TO COMPILE (Bm25Index does not implement Serialize/Deserialize yet).
#[test]
fn test_bm25_index_serialization_roundtrip() {
    let (_dir, root) = create_test_project();

    // Build index from project
    let index = Bm25Index::from_project(&root, Language::Python).unwrap();
    let doc_count = index.document_count();
    assert!(doc_count >= 3, "Should index at least 3 files");

    // Search before serialization
    let results_before = index.search("jwt token verify", 10);
    assert!(
        !results_before.is_empty(),
        "Should find results before serialization"
    );

    // Serialize to JSON (testing serde support)
    let serialized =
        serde_json::to_string(&index).expect("Bm25Index should be serializable to JSON");

    // Deserialize back
    let deserialized: Bm25Index =
        serde_json::from_str(&serialized).expect("Bm25Index should be deserializable from JSON");

    // Verify document count preserved
    assert_eq!(
        deserialized.document_count(),
        doc_count,
        "Document count should be preserved after round-trip"
    );

    // Search after deserialization
    let results_after = deserialized.search("jwt token verify", 10);

    // Results must be identical
    assert_eq!(
        results_before.len(),
        results_after.len(),
        "Same number of results after round-trip"
    );

    for (before, after) in results_before.iter().zip(results_after.iter()) {
        assert_eq!(
            before.file_path, after.file_path,
            "File paths should match after round-trip"
        );
        assert!(
            (before.score - after.score).abs() < f64::EPSILON,
            "Scores should be identical after round-trip: {} vs {}",
            before.score,
            after.score
        );
        assert_eq!(
            before.matched_terms, after.matched_terms,
            "Matched terms should be identical after round-trip"
        );
    }
}

// =============================================================================
// Test 4: Call graph cache file reading
// =============================================================================

/// Tests that the call graph cache file (.tldr/cache/call_graph.json) can be
/// read and converted into forward/reverse lookup maps for enrichment.
///
/// This validates the bridge between the daemon warm.rs cache format and the
/// enriched search call graph enrichment.
///
/// Expected: FAILS TO COMPILE (read_callgraph_cache does not exist yet).
#[test]
fn test_callgraph_cache_read() {
    let dir = TempDir::new().unwrap();
    let cache_dir = dir.path().join(".tldr").join("cache");
    fs::create_dir_all(&cache_dir).unwrap();

    // Write a realistic call graph cache
    let cache_json = serde_json::json!({
        "edges": [
            {
                "from_file": "auth.py",
                "from_func": "verify_jwt_token",
                "to_file": "auth.py",
                "to_func": "decode_token"
            },
            {
                "from_file": "auth.py",
                "from_func": "verify_jwt_token",
                "to_file": "auth.py",
                "to_func": "check_expiry"
            },
            {
                "from_file": "routes.py",
                "from_func": "user_routes",
                "to_file": "routes.py",
                "to_func": "get_all_users"
            },
            {
                "from_file": "auth.py",
                "from_func": "process_request",
                "to_file": "auth.py",
                "to_func": "verify_jwt_token"
            }
        ],
        "languages": ["python"],
        "timestamp": 1740000000
    });

    let cache_path = cache_dir.join("call_graph.json");
    fs::write(
        &cache_path,
        serde_json::to_string_pretty(&cache_json).unwrap(),
    )
    .unwrap();

    // TARGET API: read_callgraph_cache reads the warm.rs cache format
    // and returns forward/reverse maps suitable for enrichment.
    let cache = tldr_core::search::enriched::read_callgraph_cache(&cache_path).unwrap();

    // Forward graph: caller -> Vec<callee>
    let forward = &cache.forward;
    let verify_callees = forward.get("verify_jwt_token");
    assert!(
        verify_callees.is_some(),
        "Forward graph should contain verify_jwt_token"
    );
    let verify_callees = verify_callees.unwrap();
    assert!(
        verify_callees.contains(&"decode_token".to_string()),
        "verify_jwt_token should call decode_token"
    );
    assert!(
        verify_callees.contains(&"check_expiry".to_string()),
        "verify_jwt_token should call check_expiry"
    );

    // Reverse graph: callee -> Vec<caller>
    let reverse = &cache.reverse;
    let verify_callers = reverse.get("verify_jwt_token");
    assert!(
        verify_callers.is_some(),
        "Reverse graph should contain verify_jwt_token (it is called)"
    );
    let verify_callers = verify_callers.unwrap();
    assert!(
        verify_callers.contains(&"process_request".to_string()),
        "verify_jwt_token should be called by process_request"
    );

    // Check another edge
    let user_routes_callees = forward.get("user_routes");
    assert!(
        user_routes_callees.is_some(),
        "Forward graph should contain user_routes"
    );
    assert!(
        user_routes_callees
            .unwrap()
            .contains(&"get_all_users".to_string()),
        "user_routes should call get_all_users"
    );
}
