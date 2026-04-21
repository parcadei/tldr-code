//! Real-world benchmark: enriched_search vs enriched_search_with_index
//! Run with: cargo run -p tldr-core --release --example real_world_bench

use std::path::PathBuf;
use std::time::{Duration, Instant};
use tldr_core::{
    enriched_search, enriched_search_with_index, Bm25Index, EnrichedSearchOptions, Language,
};

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let language = Language::Rust;
    let query = "search enriched callgraph";
    let options = EnrichedSearchOptions {
        top_k: 10,
        include_callgraph: false,
        ..Default::default()
    };

    let file_count = walkdir::WalkDir::new(&root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "rs"))
        .count();

    println!("=== Real-World Smart-Search Benchmark ===");
    println!("Project: crates/tldr-core/src/ ({} .rs files)", file_count);
    println!("Query: {:?}", query);
    println!("Language: Rust");
    println!();

    // --- Cold path: enriched_search (builds index from disk each time) ---
    let mut cold_times = Vec::new();
    let mut result_count = 0;
    let mut files_searched = 0;
    for i in 0..5 {
        let start = Instant::now();
        let report = enriched_search(query, &root, language, options.clone()).unwrap();
        let elapsed = start.elapsed();
        cold_times.push(elapsed);
        if i == 0 {
            result_count = report.results.len();
            files_searched = report.total_files_searched;
        }
    }
    let cold_med = median(&mut cold_times);
    println!(
        "Results: {} hits, {} files searched",
        result_count, files_searched
    );
    println!(
        "enriched_search (cold):       {:.2} ms  [{}]",
        ms(cold_med),
        fmt_runs(&cold_times)
    );

    // --- Build index once ---
    let build_start = Instant::now();
    let index = Bm25Index::from_project(&root, language).unwrap();
    let build_time = build_start.elapsed();
    println!(
        "Bm25Index::from_project():    {:.2} ms  ({} docs)",
        ms(build_time),
        index.document_count()
    );

    // --- Warm path: enriched_search_with_index (pre-built index) ---
    let mut warm_times = Vec::new();
    for _ in 0..10 {
        let start = Instant::now();
        let _report =
            enriched_search_with_index(query, &root, language, options.clone(), &index).unwrap();
        let elapsed = start.elapsed();
        warm_times.push(elapsed);
    }
    let warm_med = median(&mut warm_times);
    println!(
        "enriched_search_with_index:   {:.2} ms  [{}]",
        ms(warm_med),
        fmt_runs(&warm_times)
    );

    // --- Serde round-trip ---
    let ser_start = Instant::now();
    let json = serde_json::to_string(&index).unwrap();
    let ser_time = ser_start.elapsed();

    let de_start = Instant::now();
    let _index2: Bm25Index = serde_json::from_str(&json).unwrap();
    let de_time = de_start.elapsed();
    println!(
        "BM25 serialize:               {:.2} ms  ({:.0} KB)",
        ms(ser_time),
        json.len() as f64 / 1024.0
    );
    println!("BM25 deserialize:             {:.2} ms", ms(de_time));

    println!();
    println!("=== Summary ===");
    println!("Cold path (build+search):     {:.2} ms", ms(cold_med));
    println!("Index build only:             {:.2} ms", ms(build_time));
    println!("Warm path (cached index):     {:.2} ms", ms(warm_med));
    println!(
        "Speedup (cold vs warm):       {:.1}x",
        ms(cold_med) / ms(warm_med)
    );
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn median(times: &mut [Duration]) -> Duration {
    times.sort();
    times[times.len() / 2]
}

fn fmt_runs(times: &[Duration]) -> String {
    times
        .iter()
        .map(|t| format!("{:.2}", ms(*t)))
        .collect::<Vec<_>>()
        .join(", ")
}
