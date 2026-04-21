//! Criterion benchmarks for enriched search (smart-search) performance.
//!
//! These benchmarks establish baselines and validate optimization targets:
//! - `bm25_index_build`: BM25 index construction from test project
//! - `enriched_search_fast`: Full enriched_search with include_callgraph=false
//! - `enriched_search_cached_bm25`: enriched_search with pre-built BM25 index (target API)
//! - `add_document_scaling`: Catches O(n^2) regression in add_document
//!
//! Performance targets (from spec):
//! - Steady-state query (cached BM25): < 100ms
//! - Cold query (no cache): ~365ms (acceptable)
//! - Full mode with cached call graph: < 150ms

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;
use tldr_core::search::bm25::Bm25Index;
use tldr_core::search::enriched::{enriched_search, EnrichedSearchOptions};
use tldr_core::types::Language;

/// Create the same test project used by the enriched.rs unit tests.
/// Returns (TempDir, PathBuf) where PathBuf is the project root.
///
/// Contains 3 Python files: auth.py (23 lines), routes.py (19 lines), utils.py (8 lines).
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

/// Generate a synthetic document of roughly `line_count` lines of Python-like code.
fn generate_synthetic_document(idx: usize, line_count: usize) -> String {
    let mut lines = Vec::with_capacity(line_count);
    lines.push(format!("# Module {}", idx));
    lines.push(String::new());

    let funcs_per_doc = line_count / 10; // ~10 lines per function
    for f in 0..funcs_per_doc.max(1) {
        lines.push(format!("def function_{}_{}(arg1, arg2):", idx, f));
        lines.push(format!(
            "    \"\"\"Process data for module {} function {}.\"\"\"",
            idx, f
        ));
        lines.push("    result = transform_data(arg1)".to_string());
        lines.push("    validated = validate_input(arg2)".to_string());
        lines.push("    if validated:".to_string());
        lines.push("        return process_result(result, validated)".to_string());
        lines.push("    return None".to_string());
        lines.push(String::new());
    }

    lines.join("\n")
}

// =============================================================================
// Benchmark: BM25 Index Build
// =============================================================================

/// Measures time to build a BM25 index from the test project via from_project().
/// This is the primary bottleneck in enriched_search (~365ms for real projects).
fn bench_bm25_index_build(c: &mut Criterion) {
    let mut group = c.benchmark_group("enriched_search");
    let (_dir, root) = create_test_project();

    group.bench_function("bm25_index_build", |b| {
        b.iter(|| Bm25Index::from_project(black_box(&root), Language::Python))
    });

    group.finish();
}

// =============================================================================
// Benchmark: Enriched Search Fast Path (no callgraph)
// =============================================================================

/// Measures the full enriched_search pipeline with include_callgraph=false.
/// This is the "fast path" that should be under 100ms with caching.
fn bench_enriched_search_fast(c: &mut Criterion) {
    let mut group = c.benchmark_group("enriched_search");
    let (_dir, root) = create_test_project();

    let options = EnrichedSearchOptions {
        top_k: 10,
        include_callgraph: false,
        ..Default::default()
    };

    group.bench_function("enriched_search_fast", |b| {
        b.iter(|| {
            enriched_search(
                black_box("jwt token verify"),
                black_box(&root),
                Language::Python,
                options.clone(),
            )
        })
    });

    group.finish();
}

// =============================================================================
// Benchmark: Enriched Search with Cached BM25 Index (TARGET API)
// =============================================================================

/// Measures enriched_search when a pre-built BM25 index is provided.
///
/// This benchmark tests an API that DOES NOT YET EXIST:
///   enriched_search_with_index(query, root, language, options, &index)
///
/// It will fail to compile until the cached-index API is implemented.
/// After implementation, this should complete in < 100ms (vs ~365ms without cache).
fn bench_enriched_search_cached_bm25(c: &mut Criterion) {
    let mut group = c.benchmark_group("enriched_search");
    let (_dir, root) = create_test_project();

    // Pre-build the BM25 index (one-time cost)
    let index = Bm25Index::from_project(&root, Language::Python).unwrap();

    let options = EnrichedSearchOptions {
        top_k: 10,
        include_callgraph: false,
        ..Default::default()
    };

    // TARGET API: enriched_search_with_index that accepts a pre-built index.
    // This function does not exist yet -- this benchmark will fail to compile
    // until the optimization is implemented.
    group.bench_function("enriched_search_cached_bm25", |b| {
        b.iter(|| {
            tldr_core::search::enriched::enriched_search_with_index(
                black_box("jwt token verify"),
                black_box(&root),
                Language::Python,
                options.clone(),
                black_box(&index),
            )
        })
    });

    group.finish();
}

// =============================================================================
// Benchmark: add_document Scaling (catches O(n^2) bug)
// =============================================================================

/// Measures add_document() for increasing document counts.
///
/// Current bug: avg_doc_length is recalculated by iterating ALL documents
/// on every add_document call, making N calls O(n^2) total.
///
/// Expected scaling after fix: O(n) total (constant time per add_document).
/// If N=500 takes more than ~10x the time of N=50, the O(n^2) bug is present.
fn bench_add_document_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("add_document_scaling");
    group.sample_size(20);

    let doc_counts = [10, 50, 100, 500];

    for n in doc_counts {
        // Pre-generate documents so generation time is not measured
        let documents: Vec<String> = (0..n).map(|i| generate_synthetic_document(i, 50)).collect();

        group.bench_with_input(
            BenchmarkId::new("add_n_documents", n),
            &documents,
            |b, docs| {
                b.iter(|| {
                    let mut index = Bm25Index::default();
                    for (i, doc) in docs.iter().enumerate() {
                        index.add_document(black_box(&format!("file_{}.py", i)), black_box(doc));
                    }
                    index
                })
            },
        );
    }

    group.finish();
}

criterion_group!(
    enriched_benches,
    bench_bm25_index_build,
    bench_enriched_search_fast,
    bench_enriched_search_cached_bm25,
    bench_add_document_scaling,
);

criterion_main!(enriched_benches);
