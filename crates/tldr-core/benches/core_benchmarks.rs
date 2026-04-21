//! Criterion benchmarks for tldr-core operations
//!
//! Benchmarks 7 core commands across different corpus sizes:
//! - tree: File tree traversal (I/O bound)
//! - structure: AST extraction (CPU + I/O bound)
//! - calls: Call graph building (CPU + I/O bound)
//! - cfg: Control flow graph (CPU bound)
//! - dfg: Data flow graph (CPU bound)
//! - dead: Dead code analysis (CPU + graph traversal)
//! - impact: Reverse call graph (graph traversal)

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::collections::HashSet;
use std::path::PathBuf;
use tldr_core::analysis::{detect_clones, ClonesOptions, NormalizationMode};
use tldr_core::{
    build_project_call_graph, dead_code_analysis, get_cfg_context, get_code_structure,
    get_dfg_context, get_file_tree, impact_analysis, FunctionRef, IgnoreSpec, Language,
};

/// Get the workspace root directory.
/// Cargo sets CARGO_MANIFEST_DIR to the crate's manifest directory,
/// so we go up two levels from crates/tldr-core to get workspace root.
fn workspace_root() -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest_dir)
        .parent() // crates/
        .unwrap()
        .parent() // workspace root
        .unwrap()
        .to_path_buf()
}

/// Benchmark file tree traversal on different corpus sizes
fn bench_tree(c: &mut Criterion) {
    let mut group = c.benchmark_group("tree");
    let root = workspace_root();

    // Use existing test fixtures as corpus
    let corpora = [
        ("small", "tests/fixtures/simple-project"),
        ("medium", "crates/tldr-core/src"),
        ("large", "crates"),
    ];

    for (name, path) in corpora {
        let full_path = root.join(path);
        if !full_path.exists() {
            eprintln!(
                "Skipping {}: path does not exist: {}",
                name,
                full_path.display()
            );
            continue;
        }

        group.bench_with_input(
            BenchmarkId::new("get_file_tree", name),
            &full_path,
            |b, path| {
                b.iter(|| get_file_tree(black_box(path), None, true, Some(&IgnoreSpec::default())))
            },
        );
    }
    group.finish();
}

/// Benchmark code structure extraction
fn bench_structure(c: &mut Criterion) {
    let mut group = c.benchmark_group("structure");
    let root = workspace_root();

    let corpora = [
        (
            "small_py",
            "tests/fixtures/simple-project",
            Language::Python,
        ),
        ("medium_rs", "crates/tldr-core/src", Language::Rust),
    ];

    for (name, path, lang) in corpora {
        let full_path = root.join(path);
        if !full_path.exists() {
            eprintln!(
                "Skipping {}: path does not exist: {}",
                name,
                full_path.display()
            );
            continue;
        }

        group.bench_with_input(
            BenchmarkId::new("get_code_structure", name),
            &(full_path.clone(), lang),
            |b, (path, lang)| {
                b.iter(|| {
                    get_code_structure(
                        black_box(path),
                        *lang,
                        0, // no max_results limit
                        Some(&IgnoreSpec::default()),
                    )
                })
            },
        );
    }
    group.finish();
}

/// Benchmark call graph building
fn bench_calls(c: &mut Criterion) {
    let mut group = c.benchmark_group("calls");
    group.sample_size(20); // Fewer samples for expensive operation
    let root = workspace_root();

    let corpora = [
        (
            "small_py",
            "tests/fixtures/simple-project",
            Language::Python,
        ),
        ("medium_rs", "crates/tldr-core/src", Language::Rust),
    ];

    for (name, path, lang) in corpora {
        let full_path = root.join(path);
        if !full_path.exists() {
            eprintln!(
                "Skipping {}: path does not exist: {}",
                name,
                full_path.display()
            );
            continue;
        }

        group.bench_with_input(
            BenchmarkId::new("build_call_graph", name),
            &(full_path.clone(), lang),
            |b, (path, lang)| {
                b.iter(|| {
                    build_project_call_graph(
                        black_box(path),
                        *lang,
                        None, // no workspace config
                        true, // respect_ignore
                    )
                })
            },
        );
    }
    group.finish();
}

/// Benchmark control flow graph extraction
fn bench_cfg(c: &mut Criterion) {
    let mut group = c.benchmark_group("cfg");
    let root = workspace_root();

    // Use a known Python file with functions
    let python_file = root.join("tests/fixtures/simple-project/main.py");
    if python_file.exists() {
        let file_str = python_file.to_string_lossy().to_string();
        group.bench_function("cfg_main", move |b| {
            b.iter(|| get_cfg_context(black_box(&file_str), black_box("main"), Language::Python))
        });
    }

    // Use a Rust file if available
    let rust_file = root.join("crates/tldr-core/src/types.rs");
    if rust_file.exists() {
        let file_str = rust_file.to_string_lossy().to_string();
        group.bench_function("cfg_from_extension", move |b| {
            b.iter(|| {
                get_cfg_context(
                    black_box(&file_str),
                    black_box("from_extension"),
                    Language::Rust,
                )
            })
        });
    }

    group.finish();
}

/// Benchmark data flow graph extraction
fn bench_dfg(c: &mut Criterion) {
    let mut group = c.benchmark_group("dfg");
    let root = workspace_root();

    // Use a known Python file with functions
    let python_file = root.join("tests/fixtures/simple-project/main.py");
    if python_file.exists() {
        let file_str = python_file.to_string_lossy().to_string();
        group.bench_function("dfg_main", move |b| {
            b.iter(|| get_dfg_context(black_box(&file_str), black_box("main"), Language::Python))
        });
    }

    group.finish();
}

/// Collect all function references from a call graph's edges
fn collect_all_functions(call_graph: &tldr_core::ProjectCallGraph) -> Vec<FunctionRef> {
    let mut seen: HashSet<(PathBuf, String)> = HashSet::new();
    let mut functions = Vec::new();

    for edge in call_graph.edges() {
        // Add source function
        let src_key = (edge.src_file.clone(), edge.src_func.clone());
        if !seen.contains(&src_key) {
            seen.insert(src_key);
            functions.push(FunctionRef::new(
                edge.src_file.clone(),
                edge.src_func.clone(),
            ));
        }

        // Add destination function
        let dst_key = (edge.dst_file.clone(), edge.dst_func.clone());
        if !seen.contains(&dst_key) {
            seen.insert(dst_key);
            functions.push(FunctionRef::new(
                edge.dst_file.clone(),
                edge.dst_func.clone(),
            ));
        }
    }

    functions
}

/// Benchmark dead code analysis
fn bench_dead(c: &mut Criterion) {
    let mut group = c.benchmark_group("dead");
    group.sample_size(10); // Very expensive operation
    let root = workspace_root();

    // Build call graph once for the benchmark
    let path = root.join("tests/fixtures/simple-project");
    if path.exists() {
        // Pre-build call graph
        let call_graph = build_project_call_graph(&path, Language::Python, None, true)
            .expect("Failed to build call graph for benchmark");

        // Collect all functions from the call graph edges
        let all_functions = collect_all_functions(&call_graph);

        let entry_patterns = vec!["main".to_string(), "test_".to_string()];

        group.bench_function("dead_small_py", |b| {
            b.iter(|| {
                dead_code_analysis(
                    black_box(&call_graph),
                    black_box(&all_functions),
                    Some(&entry_patterns),
                )
            })
        });
    }

    group.finish();
}

/// Benchmark impact analysis (reverse call graph traversal)
fn bench_impact(c: &mut Criterion) {
    let mut group = c.benchmark_group("impact");
    group.sample_size(20);
    let root = workspace_root();

    // Build call graph once for the benchmark
    let path = root.join("tests/fixtures/simple-project");
    if path.exists() {
        // Pre-build call graph
        let call_graph = build_project_call_graph(&path, Language::Python, None, true)
            .expect("Failed to build call graph for benchmark");

        group.bench_function("impact_helper", |b| {
            b.iter(|| {
                impact_analysis(
                    black_box(&call_graph),
                    black_box("helper"),
                    3,    // max_depth
                    None, // target_file
                )
            })
        });
    }

    group.finish();
}

/// Benchmark clone detection (Session 8)
///
/// Performance targets:
/// - 10K LOC: < 1s
/// - 50K LOC: < 5s
/// - 100K LOC: < 60s
fn bench_clones(c: &mut Criterion) {
    let mut group = c.benchmark_group("clones");
    group.sample_size(10); // Clone detection can be expensive
    let root = workspace_root();

    // Test corpora of increasing size
    let corpora = [
        ("small", "tests/fixtures/simple-project", 25, 5),
        ("medium", "crates/tldr-core/src", 25, 5),
        ("large", "crates", 50, 6), // Use stricter thresholds for large corpus
    ];

    for (name, path, min_tokens, min_lines) in corpora {
        let full_path = root.join(path);
        if !full_path.exists() {
            eprintln!(
                "Skipping {}: path does not exist: {}",
                name,
                full_path.display()
            );
            continue;
        }

        // Type-1/2 detection (exact/parameterized clones)
        let options_type12 = ClonesOptions {
            min_tokens,
            min_lines,
            threshold: 0.9, // High threshold for Type-1/2
            normalization: NormalizationMode::All,
            max_files: 500,
            max_clones: 100,
            ..Default::default()
        };

        group.bench_with_input(
            BenchmarkId::new("detect_type12", name),
            &(full_path.clone(), options_type12),
            |b, (path, options)| b.iter(|| detect_clones(black_box(path), black_box(options))),
        );

        // Type-3 detection (gapped clones) - more expensive
        let options_type3 = ClonesOptions {
            min_tokens,
            min_lines,
            threshold: 0.7, // Lower threshold for Type-3
            normalization: NormalizationMode::All,
            max_files: 200, // Limit files for Type-3 due to higher complexity
            max_clones: 50,
            ..Default::default()
        };

        group.bench_with_input(
            BenchmarkId::new("detect_type3", name),
            &(full_path.clone(), options_type3),
            |b, (path, options)| b.iter(|| detect_clones(black_box(path), black_box(options))),
        );
    }

    group.finish();
}

/// Benchmark clone detection scalability
///
/// Tests how detection time scales with codebase size.
/// Uses the entire workspace to simulate larger codebases.
fn bench_clones_scalability(c: &mut Criterion) {
    let mut group = c.benchmark_group("clones_scalability");
    group.sample_size(10); // Minimum required by criterion
    let root = workspace_root();

    // Test with different file limits to simulate scaling
    let file_limits = [50, 100, 200, 500];

    for max_files in file_limits {
        let options = ClonesOptions {
            min_tokens: 50,
            min_lines: 6,
            threshold: 0.8,
            normalization: NormalizationMode::All,
            max_files,
            max_clones: 100,
            exclude_generated: true, // Skip generated files
            ..Default::default()
        };

        group.bench_with_input(
            BenchmarkId::new("detect_files", max_files),
            &(root.clone(), options),
            |b, (path, options)| b.iter(|| detect_clones(black_box(path), black_box(options))),
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_tree,
    bench_structure,
    bench_calls,
    bench_cfg,
    bench_dfg,
    bench_dead,
    bench_impact,
    bench_clones,
    bench_clones_scalability,
);

criterion_main!(benches);
