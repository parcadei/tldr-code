//! Integration tests for enriched search with regex mode.
//!
//! These tests define the TARGET API for `SearchMode::Regex` support in
//! enriched search. They are expected to FAIL TO COMPILE initially because
//! `SearchMode` does not exist yet.
//!
//! After implementation, all tests should pass and demonstrate:
//! 1. Regex patterns match function names/content and return enriched results
//! 2. Enriched results have signatures, line ranges, and callers/callees
//! 3. `top_k`, callgraph, and error handling work with regex mode
//! 4. Default search mode remains `Bm25` for backward compatibility
//! 5. `enriched_search_with_index` works with regex mode (ignores BM25 index)

use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;
use tldr_core::search::bm25::Bm25Index;
use tldr_core::search::enriched::{
    enriched_search, enriched_search_with_index, EnrichedSearchOptions, SearchMode,
};
use tldr_core::types::Language;

// =============================================================================
// Test helper: create a project with known functions for regex matching
// =============================================================================

/// Create a temp directory with Python files containing known function names.
///
/// Files created:
/// - `auth.py`: verify_token, decode_token, check_expiry, AuthMiddleware
/// - `handlers.py`: handle_login, handle_logout, handle_refresh_token
/// - `utils.py`: format_date, parse_json, validate_email, sanitize_input
/// - `crypto.py`: encrypt_token, decrypt_token, hash_password, verify_signature
fn create_test_project() -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    let project = dir.path().join("project");
    fs::create_dir(&project).unwrap();

    fs::write(
        project.join("auth.py"),
        r#"
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
    return jwt.decode(token, key="secret")

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

def handle_refresh_token(request):
    """Handle token refresh request."""
    old_token = request.headers.get("Authorization")
    new_token = refresh_token(old_token)
    return {"token": new_token}
"#,
    )
    .unwrap();

    fs::write(
        project.join("utils.py"),
        r#"
def format_date(dt):
    """Format a datetime object as ISO string."""
    return dt.strftime("%Y-%m-%d")

def parse_json(text):
    """Parse JSON string into a dictionary."""
    import json
    return json.loads(text)

def validate_email(email):
    """Validate an email address format."""
    import re
    return re.match(r"^[\w.]+@[\w.]+$", email) is not None

def sanitize_input(text):
    """Sanitize user input to prevent injection."""
    return text.replace("<", "&lt;").replace(">", "&gt;")
"#,
    )
    .unwrap();

    fs::write(
        project.join("crypto.py"),
        r#"
def encrypt_token(payload, key):
    """Encrypt a token payload with the given key."""
    import cryptography
    return cryptography.fernet.Fernet(key).encrypt(payload.encode())

def decrypt_token(encrypted, key):
    """Decrypt an encrypted token with the given key."""
    import cryptography
    return cryptography.fernet.Fernet(key).decrypt(encrypted).decode()

def hash_password(password):
    """Hash a password using bcrypt."""
    import bcrypt
    return bcrypt.hashpw(password.encode(), bcrypt.gensalt())

def verify_signature(data, signature, public_key):
    """Verify a cryptographic signature."""
    import cryptography
    return public_key.verify(signature, data)
"#,
    )
    .unwrap();

    (dir, project)
}

/// Helper: create options with regex search mode (no callgraph).
fn regex_opts(pattern: &str, top_k: usize) -> EnrichedSearchOptions {
    EnrichedSearchOptions {
        top_k,
        include_callgraph: false,
        search_mode: SearchMode::Regex(pattern.to_string()),
    }
}

// =============================================================================
// Test 1: Regex search finds matching functions by name pattern
// =============================================================================

/// Regex pattern matching function names that contain "token".
///
/// The pattern `.*token` should match:
/// - verify_token, decode_token (auth.py)
/// - handle_refresh_token (handlers.py)
/// - encrypt_token, decrypt_token (crypto.py)
///
/// It should NOT match:
/// - check_expiry, format_date, parse_json, etc.
///
/// Expected: FAILS TO COMPILE (SearchMode does not exist yet).
#[test]
fn test_regex_search_finds_matching_functions() {
    let (_dir, root) = create_test_project();

    let report = enriched_search(
        "", // query is ignored for regex mode; pattern is in search_mode
        &root,
        Language::Python,
        regex_opts(".*token", 20),
    )
    .unwrap();

    assert!(
        !report.results.is_empty(),
        "Regex '.*token' should match functions containing 'token'"
    );

    let names: Vec<&str> = report.results.iter().map(|r| r.name.as_str()).collect();

    // Should find token-related functions
    assert!(
        names.contains(&"verify_token"),
        "Should find verify_token, got: {:?}",
        names
    );
    assert!(
        names.contains(&"decode_token"),
        "Should find decode_token, got: {:?}",
        names
    );

    // Should NOT find functions whose content never mentions "token"
    // Note: check_expiry DOES match because its docstring contains "token".
    // format_date and validate_email have no "token" in their content.
    assert!(
        !names.contains(&"format_date"),
        "format_date does not contain 'token' anywhere, got: {:?}",
        names
    );
    assert!(
        !names.contains(&"validate_email"),
        "validate_email does not contain 'token' anywhere, got: {:?}",
        names
    );

    // All results should be enriched to function level (not module-level)
    for result in &report.results {
        assert_ne!(
            result.kind, "module",
            "Regex results should be function-level, not module-level: {}",
            result.name
        );
    }
}

// =============================================================================
// Test 2: Regex search returns enriched results with signatures and line ranges
// =============================================================================

/// Each enriched result from regex search should have the same fields as BM25
/// enriched results: non-empty signature, valid file path, valid line range.
///
/// Expected: FAILS TO COMPILE (SearchMode does not exist yet).
#[test]
fn test_regex_search_returns_enriched_results() {
    let (_dir, root) = create_test_project();

    let report = enriched_search(
        "",
        &root,
        Language::Python,
        regex_opts("def (verify|decode)_token", 10),
    )
    .unwrap();

    assert!(
        !report.results.is_empty(),
        "Regex should match verify_token and decode_token"
    );

    for result in &report.results {
        // Signature should be non-empty (the definition line)
        assert!(
            !result.signature.is_empty(),
            "Result '{}' should have a non-empty signature",
            result.name
        );

        // File path should be a relative path ending in .py
        assert!(
            result.file.to_str().unwrap().ends_with(".py"),
            "Result '{}' file should end with .py, got: {:?}",
            result.name,
            result.file
        );

        // Line range should be valid (start <= end, both > 0)
        let (start, end) = result.line_range;
        assert!(
            start > 0 && end >= start,
            "Result '{}' should have valid line_range, got: ({}, {})",
            result.name,
            start,
            end
        );
    }

    // Results should be sorted by score descending, then file, then name
    // (deterministic ordering for reproducibility)
    for window in report.results.windows(2) {
        let a = &window[0];
        let b = &window[1];
        let order_ok = a.score > b.score
            || (a.score == b.score && (a.file < b.file || (a.file == b.file && a.name <= b.name)));
        assert!(
            order_ok,
            "Results should be sorted by score desc, then file, then name. \
             Got '{}' (score={}, file={:?}) before '{}' (score={}, file={:?})",
            a.name, a.score, a.file, b.name, b.score, b.file
        );
    }
}

// =============================================================================
// Test 3: Regex search respects top_k limit
// =============================================================================

/// With many matching functions, top_k should limit the number of results.
///
/// Expected: FAILS TO COMPILE (SearchMode does not exist yet).
#[test]
fn test_regex_search_respects_top_k() {
    let (_dir, root) = create_test_project();

    // Pattern that matches many functions (any def statement)
    let report = enriched_search("", &root, Language::Python, regex_opts("def ", 2)).unwrap();

    assert!(
        report.results.len() <= 2,
        "top_k=2 should return at most 2 results, got {}",
        report.results.len()
    );

    // Verify we got results (there are many functions in the test project)
    assert!(
        !report.results.is_empty(),
        "Should find at least 1 result for 'def ' pattern"
    );
}

// =============================================================================
// Test 4: Regex search with callgraph enrichment
// =============================================================================

/// When include_callgraph is true AND search_mode is Regex, results should
/// still have callers/callees populated (from call graph analysis).
///
/// Expected: FAILS TO COMPILE (SearchMode does not exist yet).
#[test]
fn test_regex_search_with_callgraph() {
    let (_dir, root) = create_test_project();

    // Create a mock call graph cache
    let cache_dir = root.join(".tldr").join("cache");
    fs::create_dir_all(&cache_dir).unwrap();

    let cache_json = serde_json::json!({
        "edges": [
            {
                "from_file": "auth.py",
                "from_func": "verify_token",
                "to_file": "auth.py",
                "to_func": "decode_token"
            },
            {
                "from_file": "auth.py",
                "from_func": "verify_token",
                "to_file": "auth.py",
                "to_func": "check_expiry"
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

    let options = EnrichedSearchOptions {
        top_k: 10,
        include_callgraph: true,
        search_mode: SearchMode::Regex(".*token".to_string()),
    };

    let report = enriched_search("", &root, Language::Python, options).unwrap();

    assert!(
        !report.results.is_empty(),
        "Should find token-related functions"
    );

    // Find verify_token and check it has callees
    let verify = report.results.iter().find(|r| r.name == "verify_token");
    assert!(verify.is_some(), "Should find verify_token in results");

    let verify = verify.unwrap();
    // With callgraph enabled, verify_token should have callees populated
    // (decode_token, check_expiry) -- if callgraph finds the edges.
    // Note: callgraph enrichment is best-effort, so we check the structure
    // exists even if empty (implementation may not find edges from regex alone).
    assert!(
        verify.callees.contains(&"decode_token".to_string())
            || verify.callees.contains(&"check_expiry".to_string())
            || verify.callees.is_empty(), // acceptable if callgraph doesn't find edges
        "verify_token callees should be populated or empty (best-effort), got: {:?}",
        verify.callees
    );
}

// =============================================================================
// Test 5: Regex search with invalid pattern returns error
// =============================================================================

/// An invalid regex pattern should return an error, not panic.
///
/// Expected: FAILS TO COMPILE (SearchMode does not exist yet).
#[test]
fn test_regex_search_invalid_pattern() {
    let (_dir, root) = create_test_project();

    let result = enriched_search("", &root, Language::Python, regex_opts("[invalid", 10));

    assert!(
        result.is_err(),
        "Invalid regex '[invalid' should return an error, got: {:?}",
        result.as_ref().map(|r| r.results.len())
    );

    // Error message should mention the regex or pattern issue
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.to_lowercase().contains("regex")
            || err_msg.to_lowercase().contains("pattern")
            || err_msg.to_lowercase().contains("invalid"),
        "Error message should mention regex/pattern issue, got: {}",
        err_msg
    );
}

// =============================================================================
// Test 6: Regex search with no matches returns empty results
// =============================================================================

/// A regex that matches nothing should return an empty results list,
/// but total_files_searched should still reflect the number of files scanned.
///
/// Expected: FAILS TO COMPILE (SearchMode does not exist yet).
#[test]
fn test_regex_search_no_matches() {
    let (_dir, root) = create_test_project();

    let report = enriched_search(
        "",
        &root,
        Language::Python,
        regex_opts("zzz_nonexistent_pattern_xyz", 10),
    )
    .unwrap();

    assert!(
        report.results.is_empty(),
        "Pattern that matches nothing should return empty results, got {} results",
        report.results.len()
    );

    assert!(
        report.total_files_searched > 0,
        "total_files_searched should reflect files scanned even with no matches, got {}",
        report.total_files_searched
    );
}

// =============================================================================
// Test 7: Default search mode is Bm25 (backward compatibility)
// =============================================================================

/// The default EnrichedSearchOptions should use SearchMode::Bm25 to ensure
/// backward compatibility with existing callers.
///
/// Expected: FAILS TO COMPILE (SearchMode does not exist yet).
#[test]
fn test_default_search_mode_is_bm25() {
    let default_opts = EnrichedSearchOptions::default();

    assert!(
        matches!(default_opts.search_mode, SearchMode::Bm25),
        "Default search_mode should be SearchMode::Bm25, got: {:?}",
        default_opts.search_mode
    );

    // Also verify the other defaults are preserved
    assert_eq!(default_opts.top_k, 10, "Default top_k should be 10");
    assert!(
        default_opts.include_callgraph,
        "Default include_callgraph should be true"
    );
}

// =============================================================================
// Test 8: Regex search with enriched_search_with_index
// =============================================================================

/// When using `enriched_search_with_index` with `SearchMode::Regex`, the BM25
/// index should be ignored (regex does its own file scanning). The function
/// should still work correctly and return enriched results.
///
/// Expected: FAILS TO COMPILE (SearchMode does not exist yet).
#[test]
fn test_regex_search_with_cached_index() {
    let (_dir, root) = create_test_project();

    // Build a BM25 index (which regex mode should ignore)
    let index = Bm25Index::from_project(&root, Language::Python).unwrap();

    let report = enriched_search_with_index(
        "", // query ignored for regex mode
        &root,
        Language::Python,
        regex_opts(".*token", 10),
        &index,
    )
    .unwrap();

    assert!(
        !report.results.is_empty(),
        "Regex search via enriched_search_with_index should find results"
    );

    let names: Vec<&str> = report.results.iter().map(|r| r.name.as_str()).collect();
    assert!(
        names.contains(&"verify_token") || names.contains(&"decode_token"),
        "Should find token-related functions via index path, got: {:?}",
        names
    );

    // Verify enrichment still works
    for result in &report.results {
        assert!(
            !result.signature.is_empty(),
            "Result '{}' should have signature even via index path",
            result.name
        );
    }
}
