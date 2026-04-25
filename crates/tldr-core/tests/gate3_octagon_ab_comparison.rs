//! Gate 3: Octagon Domain A/B Comparison
//!
//! Runs abstract interpretation on corpus functions and reports div-zero
//! and null-deref finding counts. Run twice — once with
//! `ENABLE_OCTAGON_DOMAIN = true`, once with `false` — to measure FP
//! reduction from the octagon relational domain.
//!
//! Run with:
//! ```sh
//! cargo test -p tldr-core --test gate3_octagon_ab_comparison -- --ignored --nocapture
//! ```

use std::path::{Path, PathBuf};

use tldr_core::dataflow::abstract_interp::ENABLE_OCTAGON_DOMAIN;
use tldr_core::{
    compute_abstract_interp, extract_file, get_cfg_context, get_dfg_context, Language, ModuleInfo,
};

/// Configuration for a single corpus repository to scan.
struct CorpusRepo {
    name: &'static str,
    dir: &'static str,
    source_subdir: &'static str,
    extension: &'static str,
    language: Language,
}

const SKIP_DIRS: &[&str] = &[
    ".venv",
    "node_modules",
    "vendor",
    "__pycache__",
    ".git",
    "test",
    "tests",
    "testdata",
    "spec",
    "bench",
    "examples",
];

fn collect_source_files(dir: &Path, extension: &str) -> Vec<PathBuf> {
    let mut result = Vec::new();
    collect_inner(dir, extension, &mut result);
    result.sort();
    result
}

fn collect_inner(dir: &Path, extension: &str, out: &mut Vec<PathBuf>) {
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
            collect_inner(&path, extension, out);
        } else if path.is_file() {
            let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
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

/// A single div-zero or null-deref finding with location.
struct Finding {
    file: String,
    function: String,
    line: usize,
    var: String,
    kind: &'static str,
}

struct RepoFindings {
    name: String,
    n_files: usize,
    n_functions: usize,
    n_analyzed: usize,
    n_div_zero: usize,
    n_null_deref: usize,
    n_skipped: usize,
    details: Vec<Finding>,
}

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
        details: Vec::new(),
    };

    for file_path in &files {
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

        // Compute a short relative path for readable output
        let short_path = file_path
            .strip_prefix(corpus_root)
            .unwrap_or(file_path)
            .to_string_lossy()
            .to_string();

        for func_name in &func_names {
            let cfg = match get_cfg_context(&path_str, func_name, repo.language) {
                Ok(c) => c,
                Err(_) => {
                    findings.n_skipped += 1;
                    continue;
                }
            };

            let dfg = match get_dfg_context(&path_str, func_name, repo.language) {
                Ok(d) => d,
                Err(_) => {
                    findings.n_skipped += 1;
                    continue;
                }
            };

            let source = std::fs::read_to_string(file_path).unwrap_or_default();
            let source_lines: Vec<&str> = source.lines().collect();

            match compute_abstract_interp(&cfg, &dfg, Some(&source_lines), lang_str) {
                Ok(ai) => {
                    findings.n_analyzed += 1;
                    findings.n_div_zero += ai.potential_div_zero.len();
                    findings.n_null_deref += ai.potential_null_deref.len();

                    for (line, var) in &ai.potential_div_zero {
                        findings.details.push(Finding {
                            file: short_path.clone(),
                            function: func_name.clone(),
                            line: *line,
                            var: var.clone(),
                            kind: "div-zero",
                        });
                    }
                    for (line, var) in &ai.potential_null_deref {
                        findings.details.push(Finding {
                            file: short_path.clone(),
                            function: func_name.clone(),
                            line: *line,
                            var: var.clone(),
                            kind: "null-deref",
                        });
                    }
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
fn gate3_octagon_corpus_findings() {
    eprintln!("╔══════════════════════════════════════════╗");
    eprintln!("║   Gate 3: Octagon Domain A/B Comparison  ║");
    eprintln!("╚══════════════════════════════════════════╝");
    eprintln!();
    eprintln!("ENABLE_OCTAGON_DOMAIN = {}", ENABLE_OCTAGON_DOMAIN);
    eprintln!();

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
        CorpusRepo {
            name: "java_petclinic",
            dir: "java_petclinic",
            source_subdir: "src",
            extension: ".java",
            language: Language::Java,
        },
        CorpusRepo {
            name: "rust_serde",
            dir: "rust_serde",
            source_subdir: "serde/src",
            extension: ".rs",
            language: Language::Rust,
        },
        CorpusRepo {
            name: "ruby_sinatra",
            dir: "ruby_sinatra",
            source_subdir: "lib",
            extension: ".rb",
            language: Language::Ruby,
        },
        CorpusRepo {
            name: "kotlin_serialization",
            dir: "kotlin_serialization",
            source_subdir: "core",
            extension: ".kt",
            language: Language::Kotlin,
        },
    ];

    let mut total_files = 0;
    let mut total_functions = 0;
    let mut total_analyzed = 0;
    let mut total_div_zero = 0;
    let mut total_null_deref = 0;
    let mut total_skipped = 0;
    let mut all_details: Vec<Finding> = Vec::new();

    for repo_cfg in &repos {
        let f = scan_repo(&corpus_root, repo_cfg);

        eprintln!(
            "  {:25} {:3} files  {:4} funcs  {:4} analyzed  {:3} skip  {:2} div0  {:2} null",
            f.name,
            f.n_files,
            f.n_functions,
            f.n_analyzed,
            f.n_skipped,
            f.n_div_zero,
            f.n_null_deref
        );

        total_files += f.n_files;
        total_functions += f.n_functions;
        total_analyzed += f.n_analyzed;
        total_div_zero += f.n_div_zero;
        total_null_deref += f.n_null_deref;
        total_skipped += f.n_skipped;
        all_details.extend(f.details);
    }

    eprintln!();
    eprintln!("── Summary ────────────────────────────────");
    eprintln!("  Files scanned:       {:>5}", total_files);
    eprintln!("  Functions found:     {:>5}", total_functions);
    eprintln!("  Functions analyzed:  {:>5}", total_analyzed);
    eprintln!("  Functions skipped:   {:>5}", total_skipped);
    eprintln!("  Div-zero findings:   {:>5}", total_div_zero);
    eprintln!("  Null-deref findings: {:>5}", total_null_deref);
    eprintln!(
        "  Total findings:      {:>5}",
        total_div_zero + total_null_deref
    );
    eprintln!();

    if !all_details.is_empty() {
        eprintln!("── Finding Details ────────────────────────");
        for d in &all_details {
            eprintln!(
                "  [{:10}] {}:{} in {}() — var '{}'",
                d.kind, d.file, d.line, d.function, d.var
            );
        }
        eprintln!();
    }

    eprintln!("ENABLE_OCTAGON_DOMAIN = {}", ENABLE_OCTAGON_DOMAIN);
    eprintln!("═══════════════════════════════════════════");
}
