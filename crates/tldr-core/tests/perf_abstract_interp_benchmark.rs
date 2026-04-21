//! Performance benchmark for compute_abstract_interp()
//!
//! Measures per-function timing across the 8-repo corpus and reports
//! percentiles (p50, p95, p99, max). Target budget: 5-50ms per function.
//!
//! Run with:
//! ```sh
//! cargo test -p tldr-core --test perf_abstract_interp_benchmark -- --ignored --nocapture
//! ```

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tldr_core::{
    compute_abstract_interp, extract_file, get_cfg_context, get_dfg_context, Language, ModuleInfo,
};

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

/// A single function timing measurement.
struct FuncTiming {
    repo: String,
    file: String,
    function: String,
    language: String,
    n_blocks: usize,
    n_defs: usize,
    duration: Duration,
}

fn percentile(sorted: &[Duration], pct: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let idx = ((sorted.len() as f64 - 1.0) * pct / 100.0).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn format_duration(d: Duration) -> String {
    let us = d.as_micros();
    if us < 1_000 {
        format!("{}µs", us)
    } else if us < 1_000_000 {
        format!("{:.2}ms", us as f64 / 1_000.0)
    } else {
        format!("{:.2}s", us as f64 / 1_000_000.0)
    }
}

#[test]
#[ignore]
fn perf_abstract_interp_corpus() {
    eprintln!("╔════════════════════════════════════════════════╗");
    eprintln!("║  Performance Benchmark: compute_abstract_interp ║");
    eprintln!("╚════════════════════════════════════════════════╝");
    eprintln!();

    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let corpus_root = crate_dir.join("../../corpus");
    let corpus_root = match corpus_root.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("ERROR: Cannot resolve corpus root at {:?}: {}", corpus_root, e);
            eprintln!("Test skipped (corpus not found).");
            return;
        }
    };

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

    let mut all_timings: Vec<FuncTiming> = Vec::new();
    let mut per_lang_timings: std::collections::HashMap<String, Vec<Duration>> =
        std::collections::HashMap::new();

    let overall_start = Instant::now();

    for repo_cfg in &repos {
        let repo_dir = corpus_root.join(repo_cfg.dir).join(repo_cfg.source_subdir);
        let files = collect_source_files(&repo_dir, repo_cfg.extension);
        let lang_str = repo_cfg.language.as_str();
        let mut repo_count = 0usize;

        for file_path in &files {
            let module = match extract_file(file_path, Some(&repo_dir)) {
                Ok(m) => m,
                Err(_) => continue,
            };

            let func_names = all_function_names(&module);
            let path_str = file_path.to_string_lossy();
            let short_path = file_path
                .strip_prefix(&corpus_root)
                .unwrap_or(file_path)
                .to_string_lossy()
                .to_string();

            for func_name in &func_names {
                let cfg = match get_cfg_context(&path_str, func_name, repo_cfg.language) {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                let dfg = match get_dfg_context(&path_str, func_name, repo_cfg.language) {
                    Ok(d) => d,
                    Err(_) => continue,
                };

                let source = std::fs::read_to_string(file_path).unwrap_or_default();
                let source_lines: Vec<&str> = source.lines().collect();

                let n_blocks = cfg.blocks.len();
                let n_defs = dfg.refs.iter().filter(|r| r.ref_type == tldr_core::RefType::Definition).count();

                // Time ONLY compute_abstract_interp, not CFG/DFG construction
                let start = Instant::now();
                let result = compute_abstract_interp(&cfg, &dfg, Some(&source_lines), lang_str);
                let elapsed = start.elapsed();

                if result.is_ok() {
                    all_timings.push(FuncTiming {
                        repo: repo_cfg.name.to_string(),
                        file: short_path.clone(),
                        function: func_name.clone(),
                        language: lang_str.to_string(),
                        n_blocks,
                        n_defs,
                        duration: elapsed,
                    });
                    per_lang_timings
                        .entry(lang_str.to_string())
                        .or_default()
                        .push(elapsed);
                    repo_count += 1;
                }
            }
        }

        eprintln!("  {:25} {:>5} functions timed", repo_cfg.name, repo_count);
    }

    let overall_elapsed = overall_start.elapsed();
    eprintln!();

    // Sort all durations for percentile computation
    let mut durations: Vec<Duration> = all_timings.iter().map(|t| t.duration).collect();
    durations.sort();

    let total = durations.len();
    let sum: Duration = durations.iter().sum();
    let mean = if total > 0 {
        sum / total as u32
    } else {
        Duration::ZERO
    };

    eprintln!("══ Overall Results ({} functions) ══", total);
    eprintln!("  Total wall time:    {}", format_duration(overall_elapsed));
    eprintln!("  Sum of AI times:    {}", format_duration(sum));
    eprintln!("  Mean:               {}", format_duration(mean));
    eprintln!("  p50 (median):       {}", format_duration(percentile(&durations, 50.0)));
    eprintln!("  p90:                {}", format_duration(percentile(&durations, 90.0)));
    eprintln!("  p95:                {}", format_duration(percentile(&durations, 95.0)));
    eprintln!("  p99:                {}", format_duration(percentile(&durations, 99.0)));
    eprintln!("  Max:                {}", format_duration(if durations.is_empty() {
        Duration::ZERO
    } else {
        durations[durations.len() - 1]
    }));
    eprintln!();

    // Budget check
    let p50_us = percentile(&durations, 50.0).as_micros();
    let p95_us = percentile(&durations, 95.0).as_micros();
    let p99_us = percentile(&durations, 99.0).as_micros();
    let budget_5ms = 5_000u128;
    let budget_50ms = 50_000u128;

    eprintln!("══ Budget Check (target: 5-50ms) ══");
    eprintln!(
        "  p50  {:>10} — {}",
        format_duration(percentile(&durations, 50.0)),
        if p50_us <= budget_5ms { "WELL UNDER budget" } else if p50_us <= budget_50ms { "WITHIN budget" } else { "OVER budget" }
    );
    eprintln!(
        "  p95  {:>10} — {}",
        format_duration(percentile(&durations, 95.0)),
        if p95_us <= budget_5ms { "WELL UNDER budget" } else if p95_us <= budget_50ms { "WITHIN budget" } else { "OVER budget" }
    );
    eprintln!(
        "  p99  {:>10} — {}",
        format_duration(percentile(&durations, 99.0)),
        if p99_us <= budget_5ms { "WELL UNDER budget" } else if p99_us <= budget_50ms { "WITHIN budget" } else { "OVER budget" }
    );
    eprintln!();

    // Per-language breakdown
    eprintln!("══ Per-Language Breakdown ══");
    let mut lang_names: Vec<String> = per_lang_timings.keys().cloned().collect();
    lang_names.sort();
    for lang in &lang_names {
        let mut lang_durs = per_lang_timings[lang].clone();
        lang_durs.sort();
        let n = lang_durs.len();
        let lang_sum: Duration = lang_durs.iter().sum();
        let lang_mean = if n > 0 { lang_sum / n as u32 } else { Duration::ZERO };
        eprintln!(
            "  {:12}  n={:>4}  mean={:>10}  p50={:>10}  p95={:>10}  p99={:>10}  max={:>10}",
            lang,
            n,
            format_duration(lang_mean),
            format_duration(percentile(&lang_durs, 50.0)),
            format_duration(percentile(&lang_durs, 95.0)),
            format_duration(percentile(&lang_durs, 99.0)),
            format_duration(if lang_durs.is_empty() { Duration::ZERO } else { lang_durs[lang_durs.len() - 1] }),
        );
    }
    eprintln!();

    // Top 20 slowest functions
    let mut by_duration = all_timings;
    by_duration.sort_by(|a, b| b.duration.cmp(&a.duration));

    eprintln!("══ Top 20 Slowest Functions ══");
    for (i, t) in by_duration.iter().take(20).enumerate() {
        eprintln!(
            "  {:>2}. {:>10}  {:>3} blocks  {:>4} defs  {}  {}::{}()",
            i + 1,
            format_duration(t.duration),
            t.n_blocks,
            t.n_defs,
            t.language,
            t.file.rsplit('/').next().unwrap_or(&t.file),
            t.function,
        );
    }
    eprintln!();

    // Distribution histogram
    eprintln!("══ Duration Distribution ══");
    let buckets: &[(u128, &str)] = &[
        (100, "< 100µs"),
        (500, "< 500µs"),
        (1_000, "< 1ms"),
        (5_000, "< 5ms"),
        (10_000, "< 10ms"),
        (50_000, "< 50ms"),
        (100_000, "< 100ms"),
        (u128::MAX, ">= 100ms"),
    ];
    let mut prev = 0u128;
    for &(threshold, label) in buckets {
        let count = durations
            .iter()
            .filter(|d| {
                let us = d.as_micros();
                us >= prev && (threshold == u128::MAX || us < threshold)
            })
            .count();
        let pct = if total > 0 {
            count as f64 / total as f64 * 100.0
        } else {
            0.0
        };
        let bar_len = (pct / 2.0).round() as usize;
        let bar: String = "█".repeat(bar_len);
        eprintln!("  {:>10}  {:>5} ({:>5.1}%)  {}", label, count, pct, bar);
        prev = threshold;
    }
    eprintln!();
    eprintln!("════════════════════════════════════════════════");
}
