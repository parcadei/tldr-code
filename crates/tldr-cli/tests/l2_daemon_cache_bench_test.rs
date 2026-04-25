//! L2 Daemon Cache Benchmark Tests
//!
//! Hypothesis H2: "The daemon QueryCache can serve cached call graph and IR
//! queries with <50ms latency, making it viable as the L2 caching backend."
//!
//! These tests measure actual latency of QueryCache operations to determine
//! whether it can serve L2 analysis queries (call_graph, cfg, dfg, etc.)
//! within acceptable bounds.
//!
//! Test categories:
//! 1. Cold query latency (cache miss)
//! 2. Warm query latency (cache hit, deserialization cost)
//! 3. Call graph cache store/retrieve
//! 4. Per-function IR cache (CFG/DFG per function) at scale
//! 5. Dirty file invalidation propagation speed
//! 6. Memory footprint for realistic IR payloads
//! 7. Persistence round-trip (save/load)
//! 8. Concurrent access latency under contention

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use tempfile::tempdir;

use tldr_cli::commands::daemon::salsa::{hash_args, hash_path, QueryCache, QueryKey};

// =============================================================================
// Simulated L2 Data Structures
// =============================================================================
// These represent the kinds of data L2 analysis would cache.

/// Simulated call graph edge
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
struct CallEdge {
    from_file: String,
    from_func: String,
    to_file: String,
    to_func: String,
    call_site_line: usize,
}

/// Simulated project call graph (what `tldr calls` would produce)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
struct ProjectCallGraph {
    edges: Vec<CallEdge>,
    files: Vec<String>,
    functions: usize,
    languages: Vec<String>,
}

/// Simulated CFG basic block
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
struct BasicBlock {
    id: usize,
    start_line: usize,
    end_line: usize,
    statements: Vec<String>,
    successors: Vec<usize>,
    predecessors: Vec<usize>,
}

/// Simulated control flow graph for a function
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
struct FunctionCfg {
    file: String,
    function_name: String,
    blocks: Vec<BasicBlock>,
    entry_block: usize,
    exit_blocks: Vec<usize>,
}

/// Simulated data flow fact
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
struct DataFlowFact {
    variable: String,
    defined_at: usize,
    used_at: Vec<usize>,
    flow_type: String,
}

/// Simulated data flow graph for a function
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
struct FunctionDfg {
    file: String,
    function_name: String,
    facts: Vec<DataFlowFact>,
    taint_sources: Vec<String>,
    taint_sinks: Vec<String>,
}

// =============================================================================
// Test Data Generators
// =============================================================================

/// Generate a realistic-sized call graph (~245 files, ~2500 edges)
fn generate_project_call_graph(num_files: usize, edges_per_file: usize) -> ProjectCallGraph {
    let files: Vec<String> = (0..num_files)
        .map(|i| format!("src/module_{}/file_{}.rs", i / 10, i))
        .collect();

    let mut edges = Vec::new();
    for (i, file) in files.iter().enumerate() {
        for j in 0..edges_per_file {
            let target_idx = (i + j + 1) % num_files;
            edges.push(CallEdge {
                from_file: file.clone(),
                from_func: format!("func_{}", j),
                to_file: files[target_idx].clone(),
                to_func: format!("target_func_{}", j),
                call_site_line: j * 10 + 5,
            });
        }
    }

    ProjectCallGraph {
        functions: num_files * edges_per_file,
        files,
        edges,
        languages: vec!["rust".to_string()],
    }
}

/// Generate a realistic CFG for a function (~8-15 basic blocks)
fn generate_function_cfg(file: &str, func_name: &str, num_blocks: usize) -> FunctionCfg {
    let blocks: Vec<BasicBlock> = (0..num_blocks)
        .map(|i| BasicBlock {
            id: i,
            start_line: i * 5 + 1,
            end_line: i * 5 + 4,
            statements: (0..3)
                .map(|s| format!("let x_{} = compute_{}(arg);", s, s))
                .collect(),
            successors: if i < num_blocks - 1 {
                vec![i + 1]
            } else {
                vec![]
            },
            predecessors: if i > 0 { vec![i - 1] } else { vec![] },
        })
        .collect();

    FunctionCfg {
        file: file.to_string(),
        function_name: func_name.to_string(),
        entry_block: 0,
        exit_blocks: vec![num_blocks - 1],
        blocks,
    }
}

/// Generate a realistic DFG for a function (~10-20 data flow facts)
fn generate_function_dfg(file: &str, func_name: &str, num_facts: usize) -> FunctionDfg {
    let facts: Vec<DataFlowFact> = (0..num_facts)
        .map(|i| DataFlowFact {
            variable: format!("var_{}", i),
            defined_at: i * 3 + 1,
            used_at: vec![i * 3 + 2, i * 3 + 5, i * 3 + 8],
            flow_type: if i % 3 == 0 {
                "taint".to_string()
            } else {
                "data".to_string()
            },
        })
        .collect();

    FunctionDfg {
        file: file.to_string(),
        function_name: func_name.to_string(),
        taint_sources: vec!["user_input".to_string(), "request_body".to_string()],
        taint_sinks: vec!["sql_query".to_string(), "html_output".to_string()],
        facts,
    }
}

// =============================================================================
// Helper: Timing with statistics
// =============================================================================

struct TimingStats {
    samples: Vec<f64>,
}

impl TimingStats {
    fn new() -> Self {
        Self {
            samples: Vec::new(),
        }
    }

    fn record(&mut self, duration_us: f64) {
        self.samples.push(duration_us);
    }

    fn mean_us(&self) -> f64 {
        if self.samples.is_empty() {
            return 0.0;
        }
        self.samples.iter().sum::<f64>() / self.samples.len() as f64
    }

    fn median_us(&self) -> f64 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let mut sorted = self.samples.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mid = sorted.len() / 2;
        if sorted.len() % 2 == 0 {
            (sorted[mid - 1] + sorted[mid]) / 2.0
        } else {
            sorted[mid]
        }
    }

    fn p99_us(&self) -> f64 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let mut sorted = self.samples.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let idx = ((sorted.len() as f64) * 0.99) as usize;
        sorted[idx.min(sorted.len() - 1)]
    }

    fn max_us(&self) -> f64 {
        self.samples.iter().copied().fold(0.0_f64, |a, b| a.max(b))
    }

    fn count(&self) -> usize {
        self.samples.len()
    }
}

impl std::fmt::Display for TimingStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "n={}, mean={:.1}us, median={:.1}us, p99={:.1}us, max={:.1}us",
            self.count(),
            self.mean_us(),
            self.median_us(),
            self.p99_us(),
            self.max_us()
        )
    }
}

// =============================================================================
// Benchmark 1: Cold Query Latency (Cache Miss)
// =============================================================================

#[test]
fn bench_cold_query_latency_cache_miss() {
    let cache = QueryCache::new(10_000);
    let mut stats = TimingStats::new();

    // Measure 1000 cache misses
    for i in 0..1000 {
        let key = QueryKey::new("calls", hash_args(&("project", i)));
        let start = Instant::now();
        let result: Option<ProjectCallGraph> = cache.get(&key);
        let elapsed = start.elapsed();

        assert!(result.is_none(), "Expected cache miss");
        stats.record(elapsed.as_nanos() as f64 / 1000.0);
    }

    println!("\n=== BENCHMARK: Cold Query Latency (Cache Miss) ===");
    println!("  {}", stats);

    // Cache miss should be sub-microsecond (just a DashMap lookup + stats update)
    assert!(
        stats.median_us() < 10.0,
        "Cache miss median {:.1}us exceeds 10us threshold",
        stats.median_us()
    );
    assert!(
        stats.p99_us() < 100.0,
        "Cache miss p99 {:.1}us exceeds 100us threshold",
        stats.p99_us()
    );
}

// =============================================================================
// Benchmark 2: Warm Query Latency (Cache Hit - Small Payload)
// =============================================================================

#[test]
fn bench_warm_query_latency_small_payload() {
    let cache = QueryCache::new(10_000);

    // Pre-populate with small data (single function CFG)
    let cfg = generate_function_cfg("src/lib.rs", "main", 8);
    let key = QueryKey::new("cfg", hash_args(&("src/lib.rs", "main")));
    let input_hash = hash_path(Path::new("src/lib.rs"));
    cache.insert(key.clone(), &cfg, vec![input_hash]);

    let mut stats = TimingStats::new();

    // Measure 1000 cache hits
    for _ in 0..1000 {
        let start = Instant::now();
        let result: Option<FunctionCfg> = cache.get(&key);
        let elapsed = start.elapsed();

        assert!(result.is_some(), "Expected cache hit");
        stats.record(elapsed.as_nanos() as f64 / 1000.0);
    }

    println!("\n=== BENCHMARK: Warm Query Latency (Small Payload - 8-block CFG) ===");
    println!("  {}", stats);

    let serialized_size = serde_json::to_vec(&cfg).unwrap().len();
    println!("  Payload size: {} bytes", serialized_size);

    // Small payload hit should be well under 1ms
    assert!(
        stats.median_us() < 500.0,
        "Small payload hit median {:.1}us exceeds 500us threshold",
        stats.median_us()
    );
    assert!(
        stats.p99_us() < 2_000.0,
        "Small payload hit p99 {:.1}us exceeds 2ms threshold",
        stats.p99_us()
    );
}

// =============================================================================
// Benchmark 3: Call Graph Cache (Large Payload)
// =============================================================================

#[test]
fn bench_call_graph_cache_large_payload() {
    let cache = QueryCache::new(10_000);

    // Generate a realistic project call graph (~245 files, ~10 edges each = 2450 edges)
    let call_graph = generate_project_call_graph(245, 10);
    let key = QueryKey::new("calls", hash_args(&("project_root",)));
    let input_hashes: Vec<u64> = call_graph
        .files
        .iter()
        .map(|f| hash_path(Path::new(f)))
        .collect();

    // Measure insert latency
    let insert_start = Instant::now();
    cache.insert(key.clone(), &call_graph, input_hashes);
    let insert_elapsed = insert_start.elapsed();

    let serialized_size = serde_json::to_vec(&call_graph).unwrap().len();

    println!("\n=== BENCHMARK: Call Graph Cache (Large Payload) ===");
    println!("  Edges: {}", call_graph.edges.len());
    println!("  Files: {}", call_graph.files.len());
    println!(
        "  Serialized size: {} bytes ({:.1} KB)",
        serialized_size,
        serialized_size as f64 / 1024.0
    );
    println!(
        "  Insert latency: {:.1}us ({:.3}ms)",
        insert_elapsed.as_nanos() as f64 / 1000.0,
        insert_elapsed.as_secs_f64() * 1000.0
    );

    // Measure retrieve latency
    let mut stats = TimingStats::new();
    for _ in 0..100 {
        let start = Instant::now();
        let result: Option<ProjectCallGraph> = cache.get(&key);
        let elapsed = start.elapsed();

        assert!(result.is_some(), "Expected cache hit for call graph");
        stats.record(elapsed.as_nanos() as f64 / 1000.0);
    }

    println!("  Retrieve: {}", stats);

    // Call graph retrieval: target is <50ms. The payload is large (hundreds of KB)
    // so JSON deserialization will dominate.
    let threshold_us = 50_000.0; // 50ms in microseconds
    assert!(
        stats.median_us() < threshold_us,
        "Call graph retrieval median {:.1}us ({:.1}ms) exceeds 50ms threshold",
        stats.median_us(),
        stats.median_us() / 1000.0
    );
}

// =============================================================================
// Benchmark 4: Per-Function IR Cache at Scale (50 functions)
// =============================================================================

#[test]
fn bench_per_function_ir_cache_50_functions() {
    let cache = QueryCache::new(10_000);

    // Pre-populate cache with 50 functions' CFG + DFG
    let mut cfg_keys = Vec::new();
    let mut dfg_keys = Vec::new();
    let mut total_cfg_bytes = 0usize;
    let mut total_dfg_bytes = 0usize;

    let populate_start = Instant::now();
    for i in 0..50 {
        let file = format!("src/module_{}.rs", i);
        let func = format!("process_{}", i);
        let input_hash = hash_path(Path::new(&file));

        // CFG with 8-15 blocks
        let num_blocks = 8 + (i % 8);
        let cfg = generate_function_cfg(&file, &func, num_blocks);
        let cfg_key = QueryKey::new("cfg", hash_args(&(&file, &func)));
        total_cfg_bytes += serde_json::to_vec(&cfg).unwrap().len();
        cache.insert(cfg_key.clone(), &cfg, vec![input_hash]);
        cfg_keys.push(cfg_key);

        // DFG with 10-20 facts
        let num_facts = 10 + (i % 11);
        let dfg = generate_function_dfg(&file, &func, num_facts);
        let dfg_key = QueryKey::new("dfg", hash_args(&(&file, &func)));
        total_dfg_bytes += serde_json::to_vec(&dfg).unwrap().len();
        cache.insert(dfg_key.clone(), &dfg, vec![input_hash]);
        dfg_keys.push(dfg_key);
    }
    let populate_elapsed = populate_start.elapsed();

    println!("\n=== BENCHMARK: Per-Function IR Cache (50 functions, CFG+DFG) ===");
    println!(
        "  Cache entries: {} ({} CFGs + {} DFGs)",
        cache.len(),
        cfg_keys.len(),
        dfg_keys.len()
    );
    println!(
        "  Total CFG bytes: {} ({:.1} KB)",
        total_cfg_bytes,
        total_cfg_bytes as f64 / 1024.0
    );
    println!(
        "  Total DFG bytes: {} ({:.1} KB)",
        total_dfg_bytes,
        total_dfg_bytes as f64 / 1024.0
    );
    println!(
        "  Total cached: {:.1} KB",
        (total_cfg_bytes + total_dfg_bytes) as f64 / 1024.0
    );
    println!(
        "  Populate time: {:.3}ms",
        populate_elapsed.as_secs_f64() * 1000.0
    );

    // Measure CFG lookup latency
    let mut cfg_stats = TimingStats::new();
    for key in &cfg_keys {
        let start = Instant::now();
        let result: Option<FunctionCfg> = cache.get(key);
        let elapsed = start.elapsed();
        assert!(result.is_some());
        cfg_stats.record(elapsed.as_nanos() as f64 / 1000.0);
    }

    // Measure DFG lookup latency
    let mut dfg_stats = TimingStats::new();
    for key in &dfg_keys {
        let start = Instant::now();
        let result: Option<FunctionDfg> = cache.get(key);
        let elapsed = start.elapsed();
        assert!(result.is_some());
        dfg_stats.record(elapsed.as_nanos() as f64 / 1000.0);
    }

    println!("  CFG lookup: {}", cfg_stats);
    println!("  DFG lookup: {}", dfg_stats);

    // Per-function IR lookup should be well under 1ms
    assert!(
        cfg_stats.median_us() < 1_000.0,
        "CFG lookup median {:.1}us exceeds 1ms",
        cfg_stats.median_us()
    );
    assert!(
        dfg_stats.median_us() < 1_000.0,
        "DFG lookup median {:.1}us exceeds 1ms",
        dfg_stats.median_us()
    );
}

// =============================================================================
// Benchmark 5: Memory Footprint for 50 Functions' IR
// =============================================================================

#[test]
fn bench_memory_footprint_50_functions() {
    let cache = QueryCache::new(10_000);

    let mut total_serialized_bytes = 0usize;

    for i in 0..50 {
        let file = format!("src/module_{}.rs", i);
        let func = format!("process_{}", i);
        let input_hash = hash_path(Path::new(&file));

        // CFG
        let cfg = generate_function_cfg(&file, &func, 12);
        let cfg_bytes = serde_json::to_vec(&cfg).unwrap();
        total_serialized_bytes += cfg_bytes.len();
        let cfg_key = QueryKey::new("cfg", hash_args(&(&file, &func)));
        cache.insert(cfg_key, &cfg, vec![input_hash]);

        // DFG
        let dfg = generate_function_dfg(&file, &func, 15);
        let dfg_bytes = serde_json::to_vec(&dfg).unwrap();
        total_serialized_bytes += dfg_bytes.len();
        let dfg_key = QueryKey::new("dfg", hash_args(&(&file, &func)));
        cache.insert(dfg_key, &dfg, vec![input_hash]);
    }

    println!("\n=== BENCHMARK: Memory Footprint (50 functions, CFG+DFG) ===");
    println!("  Cache entries: {}", cache.len());
    println!(
        "  Total serialized JSON bytes: {} ({:.1} KB)",
        total_serialized_bytes,
        total_serialized_bytes as f64 / 1024.0
    );
    println!(
        "  Average per function (CFG+DFG): {:.0} bytes",
        total_serialized_bytes as f64 / 50.0
    );

    // For a 245-file project, extrapolate
    let extrapolated_kb = (total_serialized_bytes as f64 / 50.0) * 245.0 / 1024.0;
    println!(
        "  Extrapolated for 245 files: {:.1} KB ({:.1} MB)",
        extrapolated_kb,
        extrapolated_kb / 1024.0
    );

    // Memory should be reasonable - well under 100MB for a full project
    assert!(
        extrapolated_kb < 100_000.0,
        "Extrapolated memory {:.1} KB exceeds 100MB",
        extrapolated_kb
    );

    // Individual entry size should be manageable
    let avg_per_func = total_serialized_bytes as f64 / 50.0;
    println!("  Average bytes per function: {:.0}", avg_per_func);

    // Each function's IR should be under 10KB on average
    assert!(
        avg_per_func < 10_000.0,
        "Average per-function IR size {:.0} bytes exceeds 10KB",
        avg_per_func
    );
}

// =============================================================================
// Benchmark 6: Dirty File Invalidation Speed
// =============================================================================

#[test]
fn bench_dirty_file_invalidation_speed() {
    let cache = QueryCache::new(10_000);

    // Populate cache: 100 files, 3 queries per file (cfg, dfg, structure)
    let mut file_hashes = Vec::new();
    for i in 0..100 {
        let file = format!("src/file_{}.rs", i);
        let file_hash = hash_path(Path::new(&file));
        file_hashes.push(file_hash);

        for query_type in &["cfg", "dfg", "structure"] {
            let key = QueryKey::new(*query_type, hash_args(&(&file, query_type)));
            cache.insert(key, &format!("data_{}_{}", i, query_type), vec![file_hash]);
        }
    }

    assert_eq!(cache.len(), 300);

    println!("\n=== BENCHMARK: Dirty File Invalidation Speed ===");
    println!("  Cache entries before: {}", cache.len());

    // Measure invalidation of a single file (should remove 3 entries)
    let mut single_stats = TimingStats::new();
    for i in 0..50 {
        // Re-populate entries for this file
        let file = format!("src/file_{}.rs", i);
        let file_hash = file_hashes[i];
        for query_type in &["cfg", "dfg", "structure"] {
            let key = QueryKey::new(*query_type, hash_args(&(&file, query_type)));
            cache.insert(key, &format!("data_{}_{}", i, query_type), vec![file_hash]);
        }

        let start = Instant::now();
        let invalidated = cache.invalidate_by_input(file_hash);
        let elapsed = start.elapsed();

        assert_eq!(
            invalidated, 3,
            "Expected 3 entries invalidated for file {}",
            i
        );
        single_stats.record(elapsed.as_nanos() as f64 / 1000.0);
    }

    println!("  Single file invalidation (3 entries): {}", single_stats);

    // Measure bulk invalidation (10 files at once)
    // Re-populate fully
    for i in 0..100 {
        let file = format!("src/file_{}.rs", i);
        let file_hash = file_hashes[i];
        for query_type in &["cfg", "dfg", "structure"] {
            let key = QueryKey::new(*query_type, hash_args(&(&file, query_type)));
            cache.insert(key, &format!("data_{}_{}", i, query_type), vec![file_hash]);
        }
    }

    let bulk_start = Instant::now();
    let mut total_invalidated = 0;
    for i in 0..10 {
        total_invalidated += cache.invalidate_by_input(file_hashes[i]);
    }
    let bulk_elapsed = bulk_start.elapsed();

    println!(
        "  Bulk invalidation (10 files, {} entries): {:.1}us ({:.3}ms)",
        total_invalidated,
        bulk_elapsed.as_nanos() as f64 / 1000.0,
        bulk_elapsed.as_secs_f64() * 1000.0
    );

    // Single file invalidation should be sub-millisecond
    assert!(
        single_stats.median_us() < 1_000.0,
        "Single file invalidation median {:.1}us exceeds 1ms",
        single_stats.median_us()
    );
}

// =============================================================================
// Benchmark 7: Persistence Round-Trip (Save + Load)
// =============================================================================

#[test]
fn bench_persistence_round_trip() {
    let dir = tempdir().unwrap();
    let cache_path = dir.path().join("l2_cache.bin");

    let cache = QueryCache::new(10_000);

    // Populate with realistic L2 data (50 functions)
    for i in 0..50 {
        let file = format!("src/module_{}.rs", i);
        let func = format!("process_{}", i);
        let input_hash = hash_path(Path::new(&file));

        let cfg = generate_function_cfg(&file, &func, 12);
        let cfg_key = QueryKey::new("cfg", hash_args(&(&file, &func)));
        cache.insert(cfg_key, &cfg, vec![input_hash]);

        let dfg = generate_function_dfg(&file, &func, 15);
        let dfg_key = QueryKey::new("dfg", hash_args(&(&file, &func)));
        cache.insert(dfg_key, &dfg, vec![input_hash]);
    }

    // Also add a call graph
    let call_graph = generate_project_call_graph(50, 5);
    let cg_key = QueryKey::new("calls", hash_args(&("project",)));
    cache.insert(cg_key.clone(), &call_graph, vec![]);

    let total_entries = cache.len();

    println!("\n=== BENCHMARK: Persistence Round-Trip ===");
    println!("  Entries to persist: {}", total_entries);

    // Measure save
    let save_start = Instant::now();
    cache.save_to_file(&cache_path).unwrap();
    let save_elapsed = save_start.elapsed();

    let file_size = std::fs::metadata(&cache_path).unwrap().len();
    println!(
        "  Save: {:.3}ms ({} bytes, {:.1} KB on disk)",
        save_elapsed.as_secs_f64() * 1000.0,
        file_size,
        file_size as f64 / 1024.0
    );

    // Measure load
    let load_start = Instant::now();
    let loaded = QueryCache::load_from_file(&cache_path).unwrap();
    let load_elapsed = load_start.elapsed();

    println!(
        "  Load: {:.3}ms ({} entries restored)",
        load_elapsed.as_secs_f64() * 1000.0,
        loaded.len()
    );

    assert_eq!(loaded.len(), total_entries);

    // Verify data integrity after round-trip
    let result: Option<ProjectCallGraph> = loaded.get(&cg_key);
    assert!(
        result.is_some(),
        "Call graph should survive persistence round-trip"
    );
    let restored_cg = result.unwrap();
    assert_eq!(restored_cg.edges.len(), call_graph.edges.len());

    // Persistence should be fast enough for shutdown/startup
    // Target: save < 500ms, load < 500ms for 100 entries
    assert!(
        save_elapsed.as_secs_f64() < 0.5,
        "Save took {:.3}ms, exceeds 500ms threshold",
        save_elapsed.as_secs_f64() * 1000.0
    );
    assert!(
        load_elapsed.as_secs_f64() < 0.5,
        "Load took {:.3}ms, exceeds 500ms threshold",
        load_elapsed.as_secs_f64() * 1000.0
    );
}

// =============================================================================
// Benchmark 8: Concurrent Access Under Contention
// =============================================================================

#[test]
fn bench_concurrent_access_latency() {
    use std::sync::Arc;

    let cache = Arc::new(QueryCache::new(10_000));

    // Pre-populate with 100 entries
    for i in 0..100 {
        let key = QueryKey::new("cfg", hash_args(&(i,)));
        let cfg = generate_function_cfg(&format!("file_{}.rs", i), &format!("func_{}", i), 10);
        cache.insert(key, &cfg, vec![]);
    }

    // Spawn 4 reader threads and 1 writer thread concurrently
    let num_readers = 4;
    let reads_per_thread = 250;
    let writes_per_thread = 50;

    let mut handles = Vec::new();

    // Reader threads
    for thread_id in 0..num_readers {
        let cache_clone = Arc::clone(&cache);
        let handle = std::thread::spawn(move || {
            let mut stats = TimingStats::new();
            for i in 0..reads_per_thread {
                let key_idx = (thread_id * reads_per_thread + i) % 100;
                let key = QueryKey::new("cfg", hash_args(&(key_idx,)));
                let start = Instant::now();
                let _result: Option<FunctionCfg> = cache_clone.get(&key);
                let elapsed = start.elapsed();
                stats.record(elapsed.as_nanos() as f64 / 1000.0);
            }
            stats
        });
        handles.push(("reader", handle));
    }

    // Writer thread
    {
        let cache_clone = Arc::clone(&cache);
        let handle = std::thread::spawn(move || {
            let mut stats = TimingStats::new();
            for i in 0..writes_per_thread {
                let key = QueryKey::new("cfg", hash_args(&(100 + i,)));
                let cfg = generate_function_cfg(
                    &format!("new_file_{}.rs", i),
                    &format!("new_func_{}", i),
                    10,
                );
                let start = Instant::now();
                cache_clone.insert(key, &cfg, vec![]);
                let elapsed = start.elapsed();
                stats.record(elapsed.as_nanos() as f64 / 1000.0);
            }
            stats
        });
        handles.push(("writer", handle));
    }

    println!(
        "\n=== BENCHMARK: Concurrent Access ({} readers + 1 writer) ===",
        num_readers
    );

    for (role, handle) in handles {
        let stats = handle.join().unwrap();
        println!("  {} thread: {}", role, stats);

        // Under contention, latency should still be reasonable
        // DashMap is designed for concurrent access, so overhead should be minimal
        assert!(
            stats.p99_us() < 10_000.0,
            "{} thread p99 {:.1}us exceeds 10ms under contention",
            role,
            stats.p99_us()
        );
    }
}

// =============================================================================
// Benchmark 9: Query Type Coverage (what L2 needs vs what cache supports)
// =============================================================================

#[test]
fn bench_l2_query_type_coverage() {
    let cache = QueryCache::new(10_000);

    // The QueryCache is generic - it can store any Serialize/Deserialize type.
    // Let's verify all L2-relevant query types work:

    println!("\n=== BENCHMARK: L2 Query Type Coverage ===");

    // 1. Call graph (project-level)
    let cg = generate_project_call_graph(10, 5);
    let cg_key = QueryKey::new("calls", hash_args(&("project",)));
    cache.insert(cg_key.clone(), &cg, vec![]);
    let result: Option<ProjectCallGraph> = cache.get(&cg_key);
    assert!(result.is_some(), "calls query type: SUPPORTED");
    println!("  calls (call graph):     SUPPORTED");

    // 2. CFG per function
    let cfg = generate_function_cfg("test.rs", "main", 10);
    let cfg_key = QueryKey::new("cfg", hash_args(&("test.rs", "main")));
    cache.insert(cfg_key.clone(), &cfg, vec![]);
    let result: Option<FunctionCfg> = cache.get(&cfg_key);
    assert!(result.is_some(), "cfg query type: SUPPORTED");
    println!("  cfg (control flow):     SUPPORTED");

    // 3. DFG per function
    let dfg = generate_function_dfg("test.rs", "main", 15);
    let dfg_key = QueryKey::new("dfg", hash_args(&("test.rs", "main")));
    cache.insert(dfg_key.clone(), &dfg, vec![]);
    let result: Option<FunctionDfg> = cache.get(&dfg_key);
    assert!(result.is_some(), "dfg query type: SUPPORTED");
    println!("  dfg (data flow):        SUPPORTED");

    // 4. Impact analysis (reverse call graph)
    let impact_data: HashMap<String, Vec<String>> = {
        let mut m = HashMap::new();
        m.insert(
            "target_func".to_string(),
            vec!["caller_1".to_string(), "caller_2".to_string()],
        );
        m
    };
    let impact_key = QueryKey::new("impact", hash_args(&("target_func", 2)));
    cache.insert(impact_key.clone(), &impact_data, vec![]);
    let result: Option<HashMap<String, Vec<String>>> = cache.get(&impact_key);
    assert!(result.is_some(), "impact query type: SUPPORTED");
    println!("  impact (reverse calls): SUPPORTED");

    // 5. Dead code analysis
    let dead_funcs: Vec<String> = vec!["unused_func_1".to_string(), "unused_func_2".to_string()];
    let dead_key = QueryKey::new("dead", hash_args(&("project",)));
    cache.insert(dead_key.clone(), &dead_funcs, vec![]);
    let result: Option<Vec<String>> = cache.get(&dead_key);
    assert!(result.is_some(), "dead query type: SUPPORTED");
    println!("  dead (dead code):       SUPPORTED");

    // 6. Program slice
    let slice_data: Vec<usize> = vec![1, 5, 12, 18, 25]; // affected line numbers
    let slice_key = QueryKey::new("slice", hash_args(&("test.rs", "main", 25)));
    cache.insert(slice_key.clone(), &slice_data, vec![]);
    let result: Option<Vec<usize>> = cache.get(&slice_key);
    assert!(result.is_some(), "slice query type: SUPPORTED");
    println!("  slice (program slice):  SUPPORTED");

    // 7. Structure / extract (L1 query that L2 depends on)
    let structure_data = serde_json::json!({
        "functions": [{"name": "main", "line": 1}],
        "classes": [],
        "imports": ["std::io"]
    });
    let struct_key = QueryKey::new("structure", hash_args(&("test.rs",)));
    cache.insert(struct_key.clone(), &structure_data, vec![]);
    let result: Option<serde_json::Value> = cache.get(&struct_key);
    assert!(result.is_some(), "structure query type: SUPPORTED");
    println!("  structure (L1 base):    SUPPORTED");

    println!("  ---");
    println!("  All 7 L2 query types: SUPPORTED (generic cache accepts any Serialize type)");
}

// =============================================================================
// Benchmark 10: Invalidation Cascade Correctness
// =============================================================================

#[test]
fn bench_invalidation_cascade_correctness() {
    let cache = QueryCache::new(10_000);

    // Simulate real scenario: file A is modified, which should invalidate:
    // - CFG for all functions in file A
    // - DFG for all functions in file A
    // - Call graph (depends on all files)
    // - But NOT: CFG/DFG for functions in file B

    let file_a = "src/module_a.rs";
    let file_b = "src/module_b.rs";
    let hash_a = hash_path(Path::new(file_a));
    let hash_b = hash_path(Path::new(file_b));

    // File A: 3 functions
    for i in 0..3 {
        let func = format!("func_a_{}", i);
        let cfg = generate_function_cfg(file_a, &func, 8);
        let cfg_key = QueryKey::new("cfg", hash_args(&(file_a, &func)));
        cache.insert(cfg_key, &cfg, vec![hash_a]);

        let dfg = generate_function_dfg(file_a, &func, 10);
        let dfg_key = QueryKey::new("dfg", hash_args(&(file_a, &func)));
        cache.insert(dfg_key, &dfg, vec![hash_a]);
    }

    // File B: 3 functions
    for i in 0..3 {
        let func = format!("func_b_{}", i);
        let cfg = generate_function_cfg(file_b, &func, 8);
        let cfg_key = QueryKey::new("cfg", hash_args(&(file_b, &func)));
        cache.insert(cfg_key, &cfg, vec![hash_b]);

        let dfg = generate_function_dfg(file_b, &func, 10);
        let dfg_key = QueryKey::new("dfg", hash_args(&(file_b, &func)));
        cache.insert(dfg_key, &dfg, vec![hash_b]);
    }

    // Call graph depends on both files
    let cg = generate_project_call_graph(10, 3);
    let cg_key = QueryKey::new("calls", hash_args(&("project",)));
    cache.insert(cg_key.clone(), &cg, vec![hash_a, hash_b]);

    assert_eq!(cache.len(), 13); // 6 (A) + 6 (B) + 1 (CG)

    println!("\n=== BENCHMARK: Invalidation Cascade Correctness ===");
    println!("  Cache entries before invalidation: {}", cache.len());

    // Invalidate file A
    let start = Instant::now();
    let invalidated = cache.invalidate_by_input(hash_a);
    let elapsed = start.elapsed();

    println!(
        "  Invalidated {} entries in {:.1}us",
        invalidated,
        elapsed.as_nanos() as f64 / 1000.0
    );
    println!("  Cache entries after: {}", cache.len());

    // Should have invalidated: 3 CFGs (A) + 3 DFGs (A) + 1 call graph = 7
    assert_eq!(
        invalidated, 7,
        "Expected 7 invalidated (6 function IR + 1 call graph)"
    );
    assert_eq!(cache.len(), 6, "Expected 6 remaining (file B's 6 entries)");

    // File B entries should still be accessible
    for i in 0..3 {
        let func = format!("func_b_{}", i);
        let cfg_key = QueryKey::new("cfg", hash_args(&(file_b, &func)));
        let result: Option<FunctionCfg> = cache.get(&cfg_key);
        assert!(
            result.is_some(),
            "File B's func_{} CFG should survive invalidation",
            i
        );
    }

    println!("  File B entries: all 6 intact (CORRECT)");
}

// =============================================================================
// Benchmark 11: Scaling Test (245 files, full project IR)
// =============================================================================

#[test]
fn bench_full_project_scale_245_files() {
    let cache = QueryCache::new(10_000);

    // Simulate a full 245-file Rust project with L2 analysis cached
    let num_files = 245;
    let funcs_per_file = 4; // Average functions per file

    let populate_start = Instant::now();
    let mut total_bytes = 0usize;

    for i in 0..num_files {
        let file = format!("src/crate/module_{}/file_{}.rs", i / 20, i);
        let file_hash = hash_path(Path::new(&file));

        for j in 0..funcs_per_file {
            let func = format!("func_{}_{}", i, j);

            // CFG
            let cfg = generate_function_cfg(&file, &func, 10);
            let cfg_bytes = serde_json::to_vec(&cfg).unwrap();
            total_bytes += cfg_bytes.len();
            let cfg_key = QueryKey::new("cfg", hash_args(&(&file, &func)));
            cache.insert(cfg_key, &cfg, vec![file_hash]);

            // DFG
            let dfg = generate_function_dfg(&file, &func, 12);
            let dfg_bytes = serde_json::to_vec(&dfg).unwrap();
            total_bytes += dfg_bytes.len();
            let dfg_key = QueryKey::new("dfg", hash_args(&(&file, &func)));
            cache.insert(dfg_key, &dfg, vec![file_hash]);
        }
    }

    // Add project-level call graph
    let cg = generate_project_call_graph(num_files, 8);
    let cg_bytes = serde_json::to_vec(&cg).unwrap();
    total_bytes += cg_bytes.len();
    let cg_key = QueryKey::new("calls", hash_args(&("project",)));
    cache.insert(cg_key.clone(), &cg, vec![]);

    let populate_elapsed = populate_start.elapsed();

    let expected_entries = num_files * funcs_per_file * 2 + 1; // CFG + DFG per func + 1 CG

    println!("\n=== BENCHMARK: Full Project Scale (245 files) ===");
    println!("  Files: {}", num_files);
    println!("  Functions: {}", num_files * funcs_per_file);
    println!(
        "  Cache entries: {} (expected {})",
        cache.len(),
        expected_entries
    );
    println!(
        "  Total serialized bytes: {:.1} KB ({:.1} MB)",
        total_bytes as f64 / 1024.0,
        total_bytes as f64 / (1024.0 * 1024.0)
    );
    println!(
        "  Populate time: {:.1}ms",
        populate_elapsed.as_secs_f64() * 1000.0
    );

    assert_eq!(cache.len(), expected_entries);

    // Random access pattern: look up 100 random functions
    let mut lookup_stats = TimingStats::new();
    for i in 0..100 {
        let file_idx = (i * 7) % num_files; // pseudo-random distribution
        let func_idx = i % funcs_per_file;
        let file = format!("src/crate/module_{}/file_{}.rs", file_idx / 20, file_idx);
        let func = format!("func_{}_{}", file_idx, func_idx);
        let cfg_key = QueryKey::new("cfg", hash_args(&(&file, &func)));

        let start = Instant::now();
        let result: Option<FunctionCfg> = cache.get(&cfg_key);
        let elapsed = start.elapsed();

        assert!(
            result.is_some(),
            "Expected hit for file {} func {}",
            file_idx,
            func_idx
        );
        lookup_stats.record(elapsed.as_nanos() as f64 / 1000.0);
    }

    println!("  Random lookup (100 queries): {}", lookup_stats);

    // Look up call graph
    let cg_start = Instant::now();
    let cg_result: Option<ProjectCallGraph> = cache.get(&cg_key);
    let cg_elapsed = cg_start.elapsed();

    assert!(cg_result.is_some());
    println!(
        "  Call graph lookup: {:.1}us ({:.3}ms), {} edges",
        cg_elapsed.as_nanos() as f64 / 1000.0,
        cg_elapsed.as_secs_f64() * 1000.0,
        cg_result.unwrap().edges.len()
    );

    // Even at full project scale, lookups should be under 50ms
    assert!(
        lookup_stats.p99_us() < 50_000.0,
        "Full-scale lookup p99 {:.1}us ({:.1}ms) exceeds 50ms",
        lookup_stats.p99_us(),
        lookup_stats.p99_us() / 1000.0
    );
}

// =============================================================================
// Benchmark 12: Cache Key Collision Safety
// =============================================================================

#[test]
fn bench_cache_key_collision_safety() {
    let cache = QueryCache::new(10_000);

    // Ensure different query types for the same file/function don't collide
    let file = "src/lib.rs";
    let func = "process";

    let cfg = generate_function_cfg(file, func, 10);
    let dfg = generate_function_dfg(file, func, 15);

    // Keys use different query_name, so they should not collide
    let cfg_key = QueryKey::new("cfg", hash_args(&(file, func)));
    let dfg_key = QueryKey::new("dfg", hash_args(&(file, func)));

    cache.insert(cfg_key.clone(), &cfg, vec![]);
    cache.insert(dfg_key.clone(), &dfg, vec![]);

    assert_eq!(cache.len(), 2, "CFG and DFG should be separate entries");

    let cfg_result: Option<FunctionCfg> = cache.get(&cfg_key);
    let dfg_result: Option<FunctionDfg> = cache.get(&dfg_key);

    assert!(cfg_result.is_some(), "CFG should be retrievable");
    assert!(dfg_result.is_some(), "DFG should be retrievable");

    assert_eq!(
        cfg_result.unwrap().blocks.len(),
        10,
        "CFG data should be correct"
    );
    assert_eq!(
        dfg_result.unwrap().facts.len(),
        15,
        "DFG data should be correct"
    );

    println!("\n=== BENCHMARK: Cache Key Collision Safety ===");
    println!("  Different query types for same file/func: NO COLLISION (correct)");
}
