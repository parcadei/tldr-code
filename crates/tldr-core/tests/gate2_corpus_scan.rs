//! Gate 2 Corpus Scanner Integration Test
//!
//! Scans corpus source files, runs abstract interpretation on every function,
//! and counts `potential_div_zero` + `potential_null_deref` findings.
//! Used for A/B comparison of the guard narrowing feature flag.
//!
//! Run with:
//! ```sh
//! cargo test -p tldr-core --test gate2_corpus_scan -- --ignored --nocapture
//! ```

use std::path::{Path, PathBuf};

use tldr_core::dataflow::abstract_interp::ENABLE_GUARD_NARROWING;
use tldr_core::{
    compute_abstract_interp, extract_file, get_cfg_context, get_dfg_context, Language, ModuleInfo,
};

/// Configuration for a single corpus repository to scan.
struct CorpusRepo {
    /// Short name used in summary output.
    name: &'static str,
    /// Relative path from `corpus/` to the repo root.
    dir: &'static str,
    /// Glob-style sub-path within the repo where source files live.
    /// We walk this directory recursively.
    source_subdir: &'static str,
    /// File extension (with leading dot) for source files.
    extension: &'static str,
    /// Language for CFG/DFG/AI.
    language: Language,
}

/// Directories to skip when walking source trees.
const SKIP_DIRS: &[&str] = &[
    ".venv",
    "node_modules",
    "vendor",
    "__pycache__",
    ".git",
    "test",
    "tests",
    "testdata",
];

/// Recursively collect files with a given extension from `dir`,
/// skipping test files and excluded directories.
fn collect_source_files(dir: &Path, extension: &str) -> Vec<PathBuf> {
    let mut result = Vec::new();
    collect_source_files_inner(dir, extension, &mut result);
    result.sort();
    result
}

fn collect_source_files_inner(dir: &Path, extension: &str, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if SKIP_DIRS.contains(&dir_name) {
                continue;
            }
            collect_source_files_inner(&path, extension, out);
        } else if path.is_file() {
            let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // Skip test files (Go convention: *_test.go, Python: test_*.py)
            if file_name.ends_with("_test.go") || file_name.starts_with("test_") {
                continue;
            }
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                let dotted = format!(".{}", ext);
                if dotted == extension {
                    out.push(path);
                }
            }
        }
    }
}

/// Extract all function names from a ModuleInfo, including class/struct methods.
fn all_function_names(module: &ModuleInfo) -> Vec<String> {
    let mut names = Vec::new();
    for f in &module.functions {
        names.push(f.name.clone());
    }
    for class in &module.classes {
        for method in &class.methods {
            names.push(method.name.clone());
        }
    }
    names
}

/// Per-repo finding counts.
struct RepoFindings {
    name: String,
    n_files: usize,
    n_functions: usize,
    n_analyzed: usize,
    n_div_zero: usize,
    n_null_deref: usize,
    n_skipped: usize,
}

/// Analyze a single corpus repo: walk files, extract functions, run abstract interp.
fn scan_repo(corpus_root: &Path, repo: &CorpusRepo) -> RepoFindings {
    let repo_dir = corpus_root.join(repo.dir).join(repo.source_subdir);

    let files = collect_source_files(&repo_dir, repo.extension);

    let mut findings = RepoFindings {
        name: repo.name.to_string(),
        n_files: files.len(),
        n_functions: 0,
        n_analyzed: 0,
        n_div_zero: 0,
        n_null_deref: 0,
        n_skipped: 0,
    };

    for file_path in &files {
        // Extract module info to get function names
        let module = match extract_file(file_path, Some(&repo_dir)) {
            Ok(m) => m,
            Err(_) => {
                findings.n_skipped += 1;
                continue;
            }
        };

        let func_names = all_function_names(&module);
        findings.n_functions += func_names.len();

        let path_str = file_path.to_string_lossy();
        let lang_str = repo.language.as_str();

        for func_name in &func_names {
            // Build CFG
            let cfg = match get_cfg_context(&path_str, func_name, repo.language) {
                Ok(c) => c,
                Err(_) => {
                    findings.n_skipped += 1;
                    continue;
                }
            };

            // Build DFG
            let dfg = match get_dfg_context(&path_str, func_name, repo.language) {
                Ok(d) => d,
                Err(_) => {
                    findings.n_skipped += 1;
                    continue;
                }
            };

            // Read source lines for abstract interp
            let source = std::fs::read_to_string(file_path).unwrap_or_default();
            let source_lines: Vec<&str> = source.lines().collect();

            // Run abstract interpretation
            match compute_abstract_interp(&cfg, &dfg, Some(&source_lines), lang_str) {
                Ok(ai) => {
                    findings.n_analyzed += 1;
                    findings.n_div_zero += ai.potential_div_zero.len();
                    findings.n_null_deref += ai.potential_null_deref.len();
                }
                Err(_) => {
                    findings.n_skipped += 1;
                }
            }
        }
    }

    findings
}

#[test]
#[ignore]
fn gate2_corpus_findings_count() {
    eprintln!("=== Gate 2 Corpus Scanner ===");
    eprintln!("ENABLE_GUARD_NARROWING = {}", ENABLE_GUARD_NARROWING);
    eprintln!();

    // Resolve corpus root: CARGO_MANIFEST_DIR points to crates/tldr-core,
    // corpus/ is at workspace root (../../corpus/).
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let corpus_root = crate_dir.join("../../corpus");
    let corpus_root = match corpus_root.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "ERROR: Cannot resolve corpus root at {:?}: {}",
                corpus_root, e
            );
            eprintln!("Test skipped (corpus not found).");
            return;
        }
    };
    eprintln!("Corpus root: {}", corpus_root.display());
    eprintln!();

    let repos = vec![
        CorpusRepo {
            name: "python_requests",
            dir: "python_requests",
            source_subdir: "src/requests",
            extension: ".py",
            language: Language::Python,
        },
        CorpusRepo {
            name: "go_gin",
            dir: "go_gin",
            source_subdir: ".",
            extension: ".go",
            language: Language::Go,
        },
        CorpusRepo {
            name: "typescript_ky",
            dir: "typescript_ky",
            source_subdir: "source",
            extension: ".ts",
            language: Language::TypeScript,
        },
        CorpusRepo {
            name: "javascript_express",
            dir: "javascript_express",
            source_subdir: "lib",
            extension: ".js",
            language: Language::JavaScript,
        },
    ];

    let mut total_files = 0;
    let mut total_functions = 0;
    let mut total_analyzed = 0;
    let mut total_div_zero = 0;
    let mut total_null_deref = 0;
    let mut total_skipped = 0;

    for repo_cfg in &repos {
        let f = scan_repo(&corpus_root, repo_cfg);

        eprintln!(
            "{}: {} files, {} functions discovered, {} analyzed, {} skipped, {} div-zero, {} null-deref",
            f.name, f.n_files, f.n_functions, f.n_analyzed, f.n_skipped, f.n_div_zero, f.n_null_deref
        );

        total_files += f.n_files;
        total_functions += f.n_functions;
        total_analyzed += f.n_analyzed;
        total_div_zero += f.n_div_zero;
        total_null_deref += f.n_null_deref;
        total_skipped += f.n_skipped;
    }

    eprintln!();
    eprintln!("=== TOTALS ===");
    eprintln!("Files scanned:      {}", total_files);
    eprintln!("Functions found:    {}", total_functions);
    eprintln!("Functions analyzed: {}", total_analyzed);
    eprintln!("Functions skipped:  {}", total_skipped);
    eprintln!("Div-zero findings:  {}", total_div_zero);
    eprintln!("Null-deref findings:{}", total_null_deref);
    eprintln!("Total findings:     {}", total_div_zero + total_null_deref);
    eprintln!("ENABLE_GUARD_NARROWING = {}", ENABLE_GUARD_NARROWING);
    eprintln!("=== Done ===");
}
