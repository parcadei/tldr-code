//! Structure cache benchmark: uncached vs cached enriched search
//! Run with: cargo run -p tldr-core --release --example structure_cache_bench

use std::path::PathBuf;
use std::time::{Duration, Instant};
use tldr_core::{
    enriched_search, enriched_search_with_structure_cache, get_code_structure,
    read_structure_cache, write_structure_cache, EnrichedSearchOptions, Language, SearchMode,
};

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let language = Language::Rust;

    let file_count = walkdir::WalkDir::new(&root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "rs"))
        .count();

    println!("=== Structure Cache Benchmark ===");
    println!("Project: crates/tldr-core/src/ ({} .rs files)", file_count);
    println!();

    // Build structure cache
    let build_start = Instant::now();
    let structure = get_code_structure(&root, language, 0, None).unwrap();
    let build_time = build_start.elapsed();
    let total_defs: usize = structure.files.iter().map(|f| f.definitions.len()).sum();
    println!(
        "get_code_structure():         {:.2} ms  ({} files, {} definitions)",
        ms(build_time),
        structure.files.len(),
        total_defs
    );

    let cache_path = std::env::temp_dir().join("tldr_structure_cache_bench.json");
    let write_start = Instant::now();
    write_structure_cache(&structure, &cache_path).unwrap();
    let write_time = write_start.elapsed();
    let cache_size = std::fs::metadata(&cache_path).unwrap().len();
    println!(
        "write_structure_cache():      {:.2} ms  ({:.0} KB)",
        ms(write_time),
        cache_size as f64 / 1024.0
    );

    let read_start = Instant::now();
    let lookup = read_structure_cache(&cache_path).unwrap();
    let read_time = read_start.elapsed();
    println!(
        "read_structure_cache():       {:.2} ms  ({} files in lookup)",
        ms(read_time),
        lookup.by_file.len()
    );
    println!();

    // Queries to benchmark
    let queries: Vec<(&str, &str)> = vec![
        ("impl\\s+\\w+\\s+for\\s+\\w+", "broad (impl for)"),
        ("pub\\s+fn", "broad (pub fn)"),
        ("fn\\s+search", "focused (fn search)"),
        ("BM25", "literal (BM25)"),
        ("WarmCallEdge", "rare (WarmCallEdge)"),
    ];

    println!(
        "{:<32} {:>12} {:>12} {:>8}",
        "Query", "Uncached", "Cached", "Speedup"
    );
    println!("{}", "-".repeat(68));

    for (pattern, label) in &queries {
        let options_uncached = EnrichedSearchOptions {
            top_k: 10,
            include_callgraph: false,
            search_mode: SearchMode::Regex(pattern.to_string()),
        };

        // Uncached (5 runs, take median)
        let mut uncached_times = Vec::new();
        for _ in 0..5 {
            let start = Instant::now();
            let _report =
                enriched_search(pattern, &root, language, options_uncached.clone()).unwrap();
            uncached_times.push(start.elapsed());
        }
        let uncached_med = median(&mut uncached_times);

        // Cached (10 runs, take median)
        let options_cached = EnrichedSearchOptions {
            top_k: 10,
            include_callgraph: false,
            search_mode: SearchMode::Regex(pattern.to_string()),
        };
        let mut cached_times = Vec::new();
        for _ in 0..10 {
            let start = Instant::now();
            let _report = enriched_search_with_structure_cache(
                pattern,
                &root,
                language,
                options_cached.clone(),
                &lookup,
            )
            .unwrap();
            cached_times.push(start.elapsed());
        }
        let cached_med = median(&mut cached_times);

        let speedup = ms(uncached_med) / ms(cached_med);
        println!(
            "{:<32} {:>9.2} ms {:>9.2} ms {:>7.1}x",
            label,
            ms(uncached_med),
            ms(cached_med),
            speedup
        );
    }

    println!();
    println!("=== Cache Overhead ===");
    println!("Structure build:              {:.2} ms", ms(build_time));
    println!(
        "Cache write:                  {:.2} ms ({:.0} KB)",
        ms(write_time),
        cache_size as f64 / 1024.0
    );
    println!("Cache read:                   {:.2} ms", ms(read_time));

    // Cleanup
    let _ = std::fs::remove_file(&cache_path);
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn median(times: &mut [Duration]) -> Duration {
    times.sort();
    times[times.len() / 2]
}
