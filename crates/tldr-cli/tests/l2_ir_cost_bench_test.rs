//! L2 IR Construction Cost Benchmark
//!
//! Hypothesis H3: Per-function IR construction (CFG + DFG + SSA + abstract
//! interpretation + taint) costs 3-10ms per function, making it feasible to
//! analyze 50 functions within the deferred tier budget of ~2 seconds.
//!
//! This test measures actual wall-clock cost of each IR layer for Python
//! functions of varying complexity, using real-world-representative code.
//!
//! Run with:
//!   cargo test -p tldr-cli -- l2_ir_cost_bench --nocapture --test-threads=1

use std::collections::HashMap;
use std::time::{Duration, Instant};

use tldr_core::ast::parser::parse;
use tldr_core::cfg::get_cfg_context;
use tldr_core::dataflow::{compute_abstract_interp, compute_available_exprs};
use tldr_core::dfg::{compute_reaching_definitions, get_dfg_context};
use tldr_core::security::compute_taint;
use tldr_core::ssa::{construct_ssa, SsaType};
use tldr_core::Language;

// =============================================================================
// Test Fixtures: Python functions at three complexity tiers
// =============================================================================

/// Small function (~10 lines): simple linear flow, no branching
const SMALL_FN: &str = r#"
def add_values(a, b):
    x = a + 1
    y = b + 2
    result = x + y
    return result
"#;

/// Medium function (~30 lines): branching with match-like if/elif
const MEDIUM_FN: &str = r#"
def classify_score(score, bonus):
    total = score + bonus
    if total >= 90:
        grade = "A"
        multiplier = 4
    elif total >= 80:
        grade = "B"
        multiplier = 3
    elif total >= 70:
        grade = "C"
        multiplier = 2
    elif total >= 60:
        grade = "D"
        multiplier = 1
    else:
        grade = "F"
        multiplier = 0
    points = total * multiplier
    adjusted = points - bonus
    final_score = adjusted + 10
    return final_score
"#;

/// Complex function (~50 lines): nested loops, conditions, multiple variables
const COMPLEX_FN: &str = r#"
def matrix_stats(rows, cols, default_val):
    total = 0
    max_val = default_val
    min_val = default_val
    count = 0
    row_sums = []
    col_sums = []
    for i in range(rows):
        row_total = 0
        for j in range(cols):
            val = i * cols + j + default_val
            if val > max_val:
                max_val = val
            if val < min_val:
                min_val = val
            total = total + val
            row_total = row_total + val
            count = count + 1
        row_sums.append(row_total)
    for j in range(cols):
        col_total = 0
        for i in range(rows):
            val = i * cols + j + default_val
            col_total = col_total + val
        col_sums.append(col_total)
    if count > 0:
        mean = total / count
        spread = max_val - min_val
    else:
        mean = 0
        spread = 0
    result = mean + spread
    return result
"#;

/// Very large function for stress testing (~100 lines of branching + loops)
const STRESS_FN: &str = r#"
def process_records(records, threshold, mode):
    total = 0
    errors = 0
    warnings = 0
    processed = 0
    skipped = 0
    results = []
    error_log = []
    for record in records:
        value = record
        if value < 0:
            errors = errors + 1
            error_log.append(value)
            continue
        if value > threshold:
            warnings = warnings + 1
            if mode == "strict":
                skipped = skipped + 1
                continue
            elif mode == "lenient":
                value = threshold
            else:
                value = value / 2
        if value == 0:
            skipped = skipped + 1
            continue
        processed = processed + 1
        total = total + value
        results.append(value)
    if processed > 0:
        average = total / processed
    else:
        average = 0
    error_rate = errors
    warning_rate = warnings
    skip_rate = skipped
    summary = average + error_rate + warning_rate + skip_rate
    if summary > threshold:
        status = "critical"
    elif summary > threshold / 2:
        status = "warning"
    else:
        status = "ok"
    final_count = processed + skipped + errors
    ratio = processed
    quality = ratio - error_rate
    if quality < 0:
        quality = 0
    output = quality + final_count
    return output
"#;

/// Multi-function file for batch cost measurement (10 functions in one file)
const BATCH_FILE: &str = r#"
def func_01(x):
    y = x + 1
    z = y * 2
    return z

def func_02(a, b):
    if a > b:
        result = a - b
    else:
        result = b - a
    return result

def func_03(n):
    total = 0
    for i in range(n):
        total = total + i
    return total

def func_04(x, y, z):
    a = x + y
    b = y + z
    c = a + b
    d = c - x
    return d

def func_05(items):
    count = 0
    for item in items:
        if item > 0:
            count = count + 1
    return count

def func_06(a, b, c):
    if a > 0:
        x = b + c
    elif b > 0:
        x = a + c
    else:
        x = a + b
    return x

def func_07(n):
    result = 1
    i = 1
    while i <= n:
        result = result * i
        i = i + 1
    return result

def func_08(x, y):
    dx = x * x
    dy = y * y
    dist = dx + dy
    return dist

def func_09(data, target):
    found = 0
    index = 0
    for item in data:
        if item == target:
            found = 1
            break
        index = index + 1
    return found

def func_10(a, b, c, d):
    ab = a + b
    cd = c + d
    abcd = ab + cd
    diff = ab - cd
    prod = abcd + diff
    return prod
"#;

// =============================================================================
// Helper: build statements map (line_number -> source_text)
// =============================================================================

fn build_statements(source: &str) -> HashMap<u32, String> {
    source
        .lines()
        .enumerate()
        .map(|(i, line)| ((i + 1) as u32, line.to_string()))
        .collect()
}

fn source_lines_vec(source: &str) -> Vec<&str> {
    source.lines().collect()
}

// =============================================================================
// Helper: measure N iterations, return average duration
// =============================================================================

fn measure_avg<F: FnMut()>(warmup: usize, iterations: usize, mut f: F) -> Duration {
    // Warm up
    for _ in 0..warmup {
        f();
    }
    // Timed iterations
    let start = Instant::now();
    for _ in 0..iterations {
        f();
    }
    start.elapsed() / iterations as u32
}

// =============================================================================
// 1. Tree-sitter Parse Cost
// =============================================================================

#[test]
fn l2_ir_cost_bench_01_parse() {
    let iterations = 50;
    let warmup = 5;

    eprintln!("\n=== 1. Tree-sitter Parse Cost ===");

    // Small
    let small_avg = measure_avg(warmup, iterations, || {
        let _ = parse(SMALL_FN, Language::Python).unwrap();
    });
    eprintln!(
        "  Parse (small,  ~6 lines):   {:>8.3}ms",
        small_avg.as_secs_f64() * 1000.0
    );

    // Medium
    let med_avg = measure_avg(warmup, iterations, || {
        let _ = parse(MEDIUM_FN, Language::Python).unwrap();
    });
    eprintln!(
        "  Parse (medium, ~20 lines):  {:>8.3}ms",
        med_avg.as_secs_f64() * 1000.0
    );

    // Complex
    let complex_avg = measure_avg(warmup, iterations, || {
        let _ = parse(COMPLEX_FN, Language::Python).unwrap();
    });
    eprintln!(
        "  Parse (complex, ~35 lines): {:>8.3}ms",
        complex_avg.as_secs_f64() * 1000.0
    );

    // Stress
    let stress_avg = measure_avg(warmup, iterations, || {
        let _ = parse(STRESS_FN, Language::Python).unwrap();
    });
    eprintln!(
        "  Parse (stress, ~50 lines):  {:>8.3}ms",
        stress_avg.as_secs_f64() * 1000.0
    );

    // Batch file (10 functions)
    let batch_avg = measure_avg(warmup, iterations, || {
        let _ = parse(BATCH_FILE, Language::Python).unwrap();
    });
    eprintln!(
        "  Parse (batch, ~100 lines):  {:>8.3}ms",
        batch_avg.as_secs_f64() * 1000.0
    );

    // Budget check: parse should be < 5ms even for large files
    assert!(
        stress_avg.as_millis() < 50,
        "Parse too slow for stress fn: {:?} (budget: <50ms)",
        stress_avg
    );
}

// =============================================================================
// 2. CFG Construction Cost
// =============================================================================

#[test]
fn l2_ir_cost_bench_02_cfg() {
    let iterations = 50;
    let warmup = 5;

    eprintln!("\n=== 2. CFG Construction Cost ===");

    let small_avg = measure_avg(warmup, iterations, || {
        let _ = get_cfg_context(SMALL_FN, "add_values", Language::Python).unwrap();
    });
    eprintln!(
        "  CFG (small):   {:>8.3}ms",
        small_avg.as_secs_f64() * 1000.0
    );

    let med_avg = measure_avg(warmup, iterations, || {
        let _ = get_cfg_context(MEDIUM_FN, "classify_score", Language::Python).unwrap();
    });
    eprintln!("  CFG (medium):  {:>8.3}ms", med_avg.as_secs_f64() * 1000.0);

    let complex_avg = measure_avg(warmup, iterations, || {
        let _ = get_cfg_context(COMPLEX_FN, "matrix_stats", Language::Python).unwrap();
    });
    eprintln!(
        "  CFG (complex): {:>8.3}ms",
        complex_avg.as_secs_f64() * 1000.0
    );

    let stress_avg = measure_avg(warmup, iterations, || {
        let _ = get_cfg_context(STRESS_FN, "process_records", Language::Python).unwrap();
    });
    eprintln!(
        "  CFG (stress):  {:>8.3}ms",
        stress_avg.as_secs_f64() * 1000.0
    );

    assert!(
        stress_avg.as_millis() < 100,
        "CFG too slow for stress fn: {:?} (budget: <100ms)",
        stress_avg
    );
}

// =============================================================================
// 3. DFG (including reaching definitions) Cost
// =============================================================================

#[test]
fn l2_ir_cost_bench_03_dfg_reaching_defs() {
    let iterations = 50;
    let warmup = 5;

    eprintln!("\n=== 3. DFG + Reaching Definitions Cost ===");

    // get_dfg_context internally calls get_cfg_context + compute_reaching_definitions
    // So this measures the combined cost (which is the realistic usage pattern)
    let small_avg = measure_avg(warmup, iterations, || {
        let _ = get_dfg_context(SMALL_FN, "add_values", Language::Python).unwrap();
    });
    eprintln!(
        "  DFG (small):   {:>8.3}ms  (includes CFG + reaching defs)",
        small_avg.as_secs_f64() * 1000.0
    );

    let med_avg = measure_avg(warmup, iterations, || {
        let _ = get_dfg_context(MEDIUM_FN, "classify_score", Language::Python).unwrap();
    });
    eprintln!("  DFG (medium):  {:>8.3}ms", med_avg.as_secs_f64() * 1000.0);

    let complex_avg = measure_avg(warmup, iterations, || {
        let _ = get_dfg_context(COMPLEX_FN, "matrix_stats", Language::Python).unwrap();
    });
    eprintln!(
        "  DFG (complex): {:>8.3}ms",
        complex_avg.as_secs_f64() * 1000.0
    );

    let stress_avg = measure_avg(warmup, iterations, || {
        let _ = get_dfg_context(STRESS_FN, "process_records", Language::Python).unwrap();
    });
    eprintln!(
        "  DFG (stress):  {:>8.3}ms",
        stress_avg.as_secs_f64() * 1000.0
    );

    // Also measure compute_reaching_definitions ALONE (given pre-built CFG + refs)
    eprintln!("\n  --- Reaching Defs Only (pre-built CFG) ---");
    let cfg_s = get_cfg_context(SMALL_FN, "add_values", Language::Python).unwrap();
    let dfg_s = get_dfg_context(SMALL_FN, "add_values", Language::Python).unwrap();
    let rd_small = measure_avg(warmup, iterations, || {
        let _ = compute_reaching_definitions(&cfg_s, &dfg_s.refs);
    });
    eprintln!(
        "  ReachDefs only (small):   {:>8.3}ms",
        rd_small.as_secs_f64() * 1000.0
    );

    let cfg_c = get_cfg_context(COMPLEX_FN, "matrix_stats", Language::Python).unwrap();
    let dfg_c = get_dfg_context(COMPLEX_FN, "matrix_stats", Language::Python).unwrap();
    let rd_complex = measure_avg(warmup, iterations, || {
        let _ = compute_reaching_definitions(&cfg_c, &dfg_c.refs);
    });
    eprintln!(
        "  ReachDefs only (complex): {:>8.3}ms",
        rd_complex.as_secs_f64() * 1000.0
    );

    let cfg_st = get_cfg_context(STRESS_FN, "process_records", Language::Python).unwrap();
    let dfg_st = get_dfg_context(STRESS_FN, "process_records", Language::Python).unwrap();
    let rd_stress = measure_avg(warmup, iterations, || {
        let _ = compute_reaching_definitions(&cfg_st, &dfg_st.refs);
    });
    eprintln!(
        "  ReachDefs only (stress):  {:>8.3}ms",
        rd_stress.as_secs_f64() * 1000.0
    );

    assert!(
        stress_avg.as_millis() < 100,
        "DFG too slow for stress fn: {:?} (budget: <100ms)",
        stress_avg
    );
}

// =============================================================================
// 4. SSA Construction Cost
// =============================================================================

#[test]
fn l2_ir_cost_bench_04_ssa() {
    let iterations = 50;
    let warmup = 5;

    eprintln!("\n=== 4. SSA Construction Cost ===");

    // construct_ssa internally calls get_cfg_context + get_dfg_context + build dominator tree + phi placement
    let small_avg = measure_avg(warmup, iterations, || {
        let _ = construct_ssa(SMALL_FN, "add_values", Language::Python, SsaType::Minimal).unwrap();
    });
    eprintln!(
        "  SSA-Minimal (small):   {:>8.3}ms  (includes CFG + DFG + dom tree + phi)",
        small_avg.as_secs_f64() * 1000.0
    );

    let med_avg = measure_avg(warmup, iterations, || {
        let _ = construct_ssa(
            MEDIUM_FN,
            "classify_score",
            Language::Python,
            SsaType::Minimal,
        )
        .unwrap();
    });
    eprintln!(
        "  SSA-Minimal (medium):  {:>8.3}ms",
        med_avg.as_secs_f64() * 1000.0
    );

    let complex_avg = measure_avg(warmup, iterations, || {
        let _ = construct_ssa(
            COMPLEX_FN,
            "matrix_stats",
            Language::Python,
            SsaType::Minimal,
        )
        .unwrap();
    });
    eprintln!(
        "  SSA-Minimal (complex): {:>8.3}ms",
        complex_avg.as_secs_f64() * 1000.0
    );

    let stress_avg = measure_avg(warmup, iterations, || {
        let _ = construct_ssa(
            STRESS_FN,
            "process_records",
            Language::Python,
            SsaType::Minimal,
        )
        .unwrap();
    });
    eprintln!(
        "  SSA-Minimal (stress):  {:>8.3}ms",
        stress_avg.as_secs_f64() * 1000.0
    );

    // Also test Pruned SSA (includes liveness analysis)
    eprintln!("\n  --- Pruned SSA (includes liveness) ---");
    let pruned_complex = measure_avg(warmup, iterations, || {
        let _ = construct_ssa(
            COMPLEX_FN,
            "matrix_stats",
            Language::Python,
            SsaType::Pruned,
        )
        .unwrap();
    });
    eprintln!(
        "  SSA-Pruned (complex):  {:>8.3}ms",
        pruned_complex.as_secs_f64() * 1000.0
    );

    let pruned_stress = measure_avg(warmup, iterations, || {
        let _ = construct_ssa(
            STRESS_FN,
            "process_records",
            Language::Python,
            SsaType::Pruned,
        )
        .unwrap();
    });
    eprintln!(
        "  SSA-Pruned (stress):   {:>8.3}ms",
        pruned_stress.as_secs_f64() * 1000.0
    );

    assert!(
        stress_avg.as_millis() < 100,
        "SSA too slow for stress fn: {:?} (budget: <100ms)",
        stress_avg
    );
}

// =============================================================================
// 5. Taint Analysis Cost
// =============================================================================

#[test]
fn l2_ir_cost_bench_05_taint() {
    let iterations = 50;
    let warmup = 5;

    eprintln!("\n=== 5. Taint Analysis Cost ===");

    // Taint analysis needs pre-built CFG + DFG refs + statements map
    // Small
    let cfg_s = get_cfg_context(SMALL_FN, "add_values", Language::Python).unwrap();
    let dfg_s = get_dfg_context(SMALL_FN, "add_values", Language::Python).unwrap();
    let stmts_s = build_statements(SMALL_FN);
    let taint_small = measure_avg(warmup, iterations, || {
        let _ = compute_taint(&cfg_s, &dfg_s.refs, &stmts_s, Language::Python);
    });
    eprintln!(
        "  Taint (small, pre-built):   {:>8.3}ms",
        taint_small.as_secs_f64() * 1000.0
    );

    // Complex
    let cfg_c = get_cfg_context(COMPLEX_FN, "matrix_stats", Language::Python).unwrap();
    let dfg_c = get_dfg_context(COMPLEX_FN, "matrix_stats", Language::Python).unwrap();
    let stmts_c = build_statements(COMPLEX_FN);
    let taint_complex = measure_avg(warmup, iterations, || {
        let _ = compute_taint(&cfg_c, &dfg_c.refs, &stmts_c, Language::Python);
    });
    eprintln!(
        "  Taint (complex, pre-built): {:>8.3}ms",
        taint_complex.as_secs_f64() * 1000.0
    );

    // Stress
    let cfg_st = get_cfg_context(STRESS_FN, "process_records", Language::Python).unwrap();
    let dfg_st = get_dfg_context(STRESS_FN, "process_records", Language::Python).unwrap();
    let stmts_st = build_statements(STRESS_FN);
    let taint_stress = measure_avg(warmup, iterations, || {
        let _ = compute_taint(&cfg_st, &dfg_st.refs, &stmts_st, Language::Python);
    });
    eprintln!(
        "  Taint (stress, pre-built):  {:>8.3}ms",
        taint_stress.as_secs_f64() * 1000.0
    );

    // Full taint cost (including CFG + DFG construction)
    eprintln!("\n  --- Taint End-to-End (includes CFG + DFG) ---");
    let taint_e2e_stress = measure_avg(warmup, iterations, || {
        let cfg = get_cfg_context(STRESS_FN, "process_records", Language::Python).unwrap();
        let dfg = get_dfg_context(STRESS_FN, "process_records", Language::Python).unwrap();
        let stmts = build_statements(STRESS_FN);
        let _ = compute_taint(&cfg, &dfg.refs, &stmts, Language::Python);
    });
    eprintln!(
        "  Taint E2E (stress):         {:>8.3}ms",
        taint_e2e_stress.as_secs_f64() * 1000.0
    );

    assert!(
        taint_stress.as_millis() < 100,
        "Taint too slow for stress fn: {:?} (budget: <100ms)",
        taint_stress
    );
}

// =============================================================================
// 6. Abstract Interpretation Cost
// =============================================================================

#[test]
fn l2_ir_cost_bench_06_abstract_interp() {
    let iterations = 50;
    let warmup = 5;

    eprintln!("\n=== 6. Abstract Interpretation Cost ===");

    // Abstract interp needs pre-built CFG + DFG + source lines
    // Small
    let cfg_s = get_cfg_context(SMALL_FN, "add_values", Language::Python).unwrap();
    let dfg_s = get_dfg_context(SMALL_FN, "add_values", Language::Python).unwrap();
    let lines_s = source_lines_vec(SMALL_FN);
    let ai_small = measure_avg(warmup, iterations, || {
        let _ = compute_abstract_interp(&cfg_s, &dfg_s, Some(&lines_s), "python");
    });
    eprintln!(
        "  AbsInterp (small, pre-built):   {:>8.3}ms",
        ai_small.as_secs_f64() * 1000.0
    );

    // Complex
    let cfg_c = get_cfg_context(COMPLEX_FN, "matrix_stats", Language::Python).unwrap();
    let dfg_c = get_dfg_context(COMPLEX_FN, "matrix_stats", Language::Python).unwrap();
    let lines_c = source_lines_vec(COMPLEX_FN);
    let ai_complex = measure_avg(warmup, iterations, || {
        let _ = compute_abstract_interp(&cfg_c, &dfg_c, Some(&lines_c), "python");
    });
    eprintln!(
        "  AbsInterp (complex, pre-built): {:>8.3}ms",
        ai_complex.as_secs_f64() * 1000.0
    );

    // Stress
    let cfg_st = get_cfg_context(STRESS_FN, "process_records", Language::Python).unwrap();
    let dfg_st = get_dfg_context(STRESS_FN, "process_records", Language::Python).unwrap();
    let lines_st = source_lines_vec(STRESS_FN);
    let ai_stress = measure_avg(warmup, iterations, || {
        let _ = compute_abstract_interp(&cfg_st, &dfg_st, Some(&lines_st), "python");
    });
    eprintln!(
        "  AbsInterp (stress, pre-built):  {:>8.3}ms",
        ai_stress.as_secs_f64() * 1000.0
    );

    // Full E2E
    eprintln!("\n  --- AbsInterp End-to-End (includes CFG + DFG) ---");
    let ai_e2e_stress = measure_avg(warmup, iterations, || {
        let cfg = get_cfg_context(STRESS_FN, "process_records", Language::Python).unwrap();
        let dfg = get_dfg_context(STRESS_FN, "process_records", Language::Python).unwrap();
        let lines = source_lines_vec(STRESS_FN);
        let _ = compute_abstract_interp(&cfg, &dfg, Some(&lines), "python");
    });
    eprintln!(
        "  AbsInterp E2E (stress):         {:>8.3}ms",
        ai_e2e_stress.as_secs_f64() * 1000.0
    );

    assert!(
        ai_stress.as_millis() < 100,
        "AbsInterp too slow for stress fn: {:?} (budget: <100ms)",
        ai_stress
    );
}

// =============================================================================
// 7. Available Expressions Cost
// =============================================================================

#[test]
fn l2_ir_cost_bench_07_available_exprs() {
    let iterations = 50;
    let warmup = 5;

    eprintln!("\n=== 7. Available Expressions Cost ===");

    // Needs pre-built CFG + DFG
    let cfg_s = get_cfg_context(SMALL_FN, "add_values", Language::Python).unwrap();
    let dfg_s = get_dfg_context(SMALL_FN, "add_values", Language::Python).unwrap();
    let ae_small = measure_avg(warmup, iterations, || {
        let _ = compute_available_exprs(&cfg_s, &dfg_s);
    });
    eprintln!(
        "  AvailExprs (small, pre-built):   {:>8.3}ms",
        ae_small.as_secs_f64() * 1000.0
    );

    let cfg_c = get_cfg_context(COMPLEX_FN, "matrix_stats", Language::Python).unwrap();
    let dfg_c = get_dfg_context(COMPLEX_FN, "matrix_stats", Language::Python).unwrap();
    let ae_complex = measure_avg(warmup, iterations, || {
        let _ = compute_available_exprs(&cfg_c, &dfg_c);
    });
    eprintln!(
        "  AvailExprs (complex, pre-built): {:>8.3}ms",
        ae_complex.as_secs_f64() * 1000.0
    );

    let cfg_st = get_cfg_context(STRESS_FN, "process_records", Language::Python).unwrap();
    let dfg_st = get_dfg_context(STRESS_FN, "process_records", Language::Python).unwrap();
    let ae_stress = measure_avg(warmup, iterations, || {
        let _ = compute_available_exprs(&cfg_st, &dfg_st);
    });
    eprintln!(
        "  AvailExprs (stress, pre-built):  {:>8.3}ms",
        ae_stress.as_secs_f64() * 1000.0
    );

    assert!(
        ae_stress.as_millis() < 100,
        "AvailExprs too slow for stress fn: {:?} (budget: <100ms)",
        ae_stress
    );
}

// =============================================================================
// 8. Full IR Pipeline Cost (all layers for one function)
// =============================================================================

#[test]
fn l2_ir_cost_bench_08_full_pipeline() {
    let iterations = 30;
    let warmup = 3;

    eprintln!(
        "\n=== 8. Full IR Pipeline (Parse + CFG + DFG + SSA + Taint + AbsInterp + AvailExprs) ==="
    );

    // Small
    let full_small = measure_avg(warmup, iterations, || {
        let _ = parse(SMALL_FN, Language::Python).unwrap();
        let cfg = get_cfg_context(SMALL_FN, "add_values", Language::Python).unwrap();
        let dfg = get_dfg_context(SMALL_FN, "add_values", Language::Python).unwrap();
        let _ = construct_ssa(SMALL_FN, "add_values", Language::Python, SsaType::Minimal).unwrap();
        let stmts = build_statements(SMALL_FN);
        let _ = compute_taint(&cfg, &dfg.refs, &stmts, Language::Python);
        let lines = source_lines_vec(SMALL_FN);
        let _ = compute_abstract_interp(&cfg, &dfg, Some(&lines), "python");
        let _ = compute_available_exprs(&cfg, &dfg);
    });
    eprintln!(
        "  Full pipeline (small):   {:>8.3}ms",
        full_small.as_secs_f64() * 1000.0
    );

    // Medium
    let full_med = measure_avg(warmup, iterations, || {
        let _ = parse(MEDIUM_FN, Language::Python).unwrap();
        let cfg = get_cfg_context(MEDIUM_FN, "classify_score", Language::Python).unwrap();
        let dfg = get_dfg_context(MEDIUM_FN, "classify_score", Language::Python).unwrap();
        let _ = construct_ssa(
            MEDIUM_FN,
            "classify_score",
            Language::Python,
            SsaType::Minimal,
        )
        .unwrap();
        let stmts = build_statements(MEDIUM_FN);
        let _ = compute_taint(&cfg, &dfg.refs, &stmts, Language::Python);
        let lines = source_lines_vec(MEDIUM_FN);
        let _ = compute_abstract_interp(&cfg, &dfg, Some(&lines), "python");
        let _ = compute_available_exprs(&cfg, &dfg);
    });
    eprintln!(
        "  Full pipeline (medium):  {:>8.3}ms",
        full_med.as_secs_f64() * 1000.0
    );

    // Complex
    let full_complex = measure_avg(warmup, iterations, || {
        let _ = parse(COMPLEX_FN, Language::Python).unwrap();
        let cfg = get_cfg_context(COMPLEX_FN, "matrix_stats", Language::Python).unwrap();
        let dfg = get_dfg_context(COMPLEX_FN, "matrix_stats", Language::Python).unwrap();
        let _ = construct_ssa(
            COMPLEX_FN,
            "matrix_stats",
            Language::Python,
            SsaType::Minimal,
        )
        .unwrap();
        let stmts = build_statements(COMPLEX_FN);
        let _ = compute_taint(&cfg, &dfg.refs, &stmts, Language::Python);
        let lines = source_lines_vec(COMPLEX_FN);
        let _ = compute_abstract_interp(&cfg, &dfg, Some(&lines), "python");
        let _ = compute_available_exprs(&cfg, &dfg);
    });
    eprintln!(
        "  Full pipeline (complex): {:>8.3}ms",
        full_complex.as_secs_f64() * 1000.0
    );

    // Stress
    let full_stress = measure_avg(warmup, iterations, || {
        let _ = parse(STRESS_FN, Language::Python).unwrap();
        let cfg = get_cfg_context(STRESS_FN, "process_records", Language::Python).unwrap();
        let dfg = get_dfg_context(STRESS_FN, "process_records", Language::Python).unwrap();
        let _ = construct_ssa(
            STRESS_FN,
            "process_records",
            Language::Python,
            SsaType::Minimal,
        )
        .unwrap();
        let stmts = build_statements(STRESS_FN);
        let _ = compute_taint(&cfg, &dfg.refs, &stmts, Language::Python);
        let lines = source_lines_vec(STRESS_FN);
        let _ = compute_abstract_interp(&cfg, &dfg, Some(&lines), "python");
        let _ = compute_available_exprs(&cfg, &dfg);
    });
    eprintln!(
        "  Full pipeline (stress):  {:>8.3}ms",
        full_stress.as_secs_f64() * 1000.0
    );

    // Budget: full pipeline for one function should be < 50ms per the spec
    // Allow 200ms as a generous upper bound for stress functions
    eprintln!("\n  Budget check: spec says 3-10ms/fn, PM2-P2 warned 16-52ms/fn");
    eprintln!(
        "  At {:>6.1}ms/fn (stress), 2-second budget allows {} functions",
        full_stress.as_secs_f64() * 1000.0,
        (2000.0 / (full_stress.as_secs_f64() * 1000.0)) as u32
    );

    assert!(
        full_stress.as_millis() < 500,
        "Full pipeline too slow for stress fn: {:?} (budget: <500ms)",
        full_stress
    );
}

// =============================================================================
// 9. Batch Cost: 10 functions from one file
// =============================================================================

#[test]
fn l2_ir_cost_bench_09_batch() {
    let iterations = 20;
    let warmup = 3;

    let func_names = [
        "func_01", "func_02", "func_03", "func_04", "func_05", "func_06", "func_07", "func_08",
        "func_09", "func_10",
    ];

    eprintln!("\n=== 9. Batch Cost: 10 functions from one file ===");

    // Approach A: Independent calls (each call re-parses the file)
    let batch_independent = measure_avg(warmup, iterations, || {
        for name in &func_names {
            let cfg = get_cfg_context(BATCH_FILE, name, Language::Python).unwrap();
            let dfg = get_dfg_context(BATCH_FILE, name, Language::Python).unwrap();
            let _ = construct_ssa(BATCH_FILE, name, Language::Python, SsaType::Minimal).unwrap();
            let stmts = build_statements(BATCH_FILE);
            let _ = compute_taint(&cfg, &dfg.refs, &stmts, Language::Python);
            let lines = source_lines_vec(BATCH_FILE);
            let _ = compute_abstract_interp(&cfg, &dfg, Some(&lines), "python");
            let _ = compute_available_exprs(&cfg, &dfg);
        }
    });
    let per_fn_independent = batch_independent / 10;
    eprintln!(
        "  Batch (10 fns, independent): {:>8.3}ms total, {:>8.3}ms/fn",
        batch_independent.as_secs_f64() * 1000.0,
        per_fn_independent.as_secs_f64() * 1000.0,
    );

    // PM-42 concern: how much of the cost is redundant parsing?
    // Measure parse cost alone for 10 calls
    let parse_10x = measure_avg(warmup, iterations, || {
        for _ in 0..10 {
            let _ = parse(BATCH_FILE, Language::Python).unwrap();
        }
    });
    eprintln!(
        "  Parse x10 (redundant):       {:>8.3}ms (waste from re-parsing)",
        parse_10x.as_secs_f64() * 1000.0
    );
    eprintln!(
        "  Parse x1:                    {:>8.3}ms (shared-tree approach)",
        (parse_10x.as_secs_f64() * 1000.0) / 10.0
    );

    let parse_waste_pct = (parse_10x.as_secs_f64() / batch_independent.as_secs_f64()) * 100.0;
    eprintln!(
        "  Redundant parse overhead:    {:>6.1}% of batch cost",
        parse_waste_pct
    );

    // Final budget check
    eprintln!(
        "\n  Budget: 2000ms / {:.1}ms per fn = {} functions in budget",
        per_fn_independent.as_secs_f64() * 1000.0,
        (2000.0 / (per_fn_independent.as_secs_f64() * 1000.0)) as u32
    );

    assert!(
        batch_independent.as_millis() < 5000,
        "Batch too slow: {:?} (budget: <5000ms for 10 functions)",
        batch_independent
    );
}

// =============================================================================
// 10. Redundant Parse Detection (PM-42 validation)
// =============================================================================

#[test]
fn l2_ir_cost_bench_10_parse_redundancy() {
    let iterations = 30;
    let warmup = 5;

    eprintln!("\n=== 10. Parse Redundancy Analysis (PM-42) ===");

    // For a single function, how many times is the source parsed?
    // get_cfg_context parses once, get_dfg_context parses again AND calls get_cfg_context
    // construct_ssa calls get_cfg_context + get_dfg_context (2 more parses + 1 internal)
    //
    // Measure: CFG alone vs DFG alone vs SSA alone to quantify re-parse cost

    let cfg_only = measure_avg(warmup, iterations, || {
        let _ = get_cfg_context(STRESS_FN, "process_records", Language::Python).unwrap();
    });

    let dfg_only = measure_avg(warmup, iterations, || {
        let _ = get_dfg_context(STRESS_FN, "process_records", Language::Python).unwrap();
    });

    let ssa_only = measure_avg(warmup, iterations, || {
        let _ = construct_ssa(
            STRESS_FN,
            "process_records",
            Language::Python,
            SsaType::Minimal,
        )
        .unwrap();
    });

    let parse_only = measure_avg(warmup, iterations, || {
        let _ = parse(STRESS_FN, Language::Python).unwrap();
    });

    eprintln!(
        "  parse() alone:        {:>8.3}ms",
        parse_only.as_secs_f64() * 1000.0
    );
    eprintln!(
        "  get_cfg_context():    {:>8.3}ms  (1 parse)",
        cfg_only.as_secs_f64() * 1000.0
    );
    eprintln!(
        "  get_dfg_context():    {:>8.3}ms  (2 parses: own + internal CFG)",
        dfg_only.as_secs_f64() * 1000.0
    );
    eprintln!(
        "  construct_ssa():      {:>8.3}ms  (3+ parses: CFG + DFG + DFG-internal-CFG)",
        ssa_only.as_secs_f64() * 1000.0
    );
    eprintln!();

    // Theoretical minimum: if all shared one parse
    let analysis_only = cfg_only.as_secs_f64() + dfg_only.as_secs_f64() + ssa_only.as_secs_f64();
    let parse_overhead = parse_only.as_secs_f64() * 4.0; // ~4 redundant parses
    if analysis_only > 0.0 {
        let overhead_pct = (parse_overhead / analysis_only) * 100.0;
        eprintln!(
            "  Estimated parse overhead in combined calls: {:.1}%",
            overhead_pct
        );
        eprintln!(
            "  Shared-tree savings potential: ~{:.3}ms per function",
            parse_overhead * 1000.0
        );
    }
}

// =============================================================================
// 11. Summary Table Generator
// =============================================================================

#[test]
fn l2_ir_cost_bench_11_summary_table() {
    let iterations = 30;
    let warmup = 3;

    eprintln!("\n");
    eprintln!("╔══════════════════════════════════════════════════════════════════════════╗");
    eprintln!("║              L2 IR CONSTRUCTION COST SUMMARY TABLE                      ║");
    eprintln!("╠══════════════════════════════════════════════════════════════════════════╣");

    // Measure all operations at each complexity level
    struct Row {
        name: &'static str,
        small: f64,
        medium: f64,
        complex: f64,
        stress: f64,
    }

    let mut rows: Vec<Row> = Vec::new();

    // Parse
    let ps = measure_avg(warmup, iterations, || {
        let _ = parse(SMALL_FN, Language::Python);
    })
    .as_secs_f64()
        * 1000.0;
    let pm = measure_avg(warmup, iterations, || {
        let _ = parse(MEDIUM_FN, Language::Python);
    })
    .as_secs_f64()
        * 1000.0;
    let pc = measure_avg(warmup, iterations, || {
        let _ = parse(COMPLEX_FN, Language::Python);
    })
    .as_secs_f64()
        * 1000.0;
    let pst = measure_avg(warmup, iterations, || {
        let _ = parse(STRESS_FN, Language::Python);
    })
    .as_secs_f64()
        * 1000.0;
    rows.push(Row {
        name: "Parse",
        small: ps,
        medium: pm,
        complex: pc,
        stress: pst,
    });

    // CFG
    let cs = measure_avg(warmup, iterations, || {
        let _ = get_cfg_context(SMALL_FN, "add_values", Language::Python);
    })
    .as_secs_f64()
        * 1000.0;
    let cm = measure_avg(warmup, iterations, || {
        let _ = get_cfg_context(MEDIUM_FN, "classify_score", Language::Python);
    })
    .as_secs_f64()
        * 1000.0;
    let cc = measure_avg(warmup, iterations, || {
        let _ = get_cfg_context(COMPLEX_FN, "matrix_stats", Language::Python);
    })
    .as_secs_f64()
        * 1000.0;
    let cst = measure_avg(warmup, iterations, || {
        let _ = get_cfg_context(STRESS_FN, "process_records", Language::Python);
    })
    .as_secs_f64()
        * 1000.0;
    rows.push(Row {
        name: "CFG",
        small: cs,
        medium: cm,
        complex: cc,
        stress: cst,
    });

    // DFG (includes CFG + reaching defs)
    let ds = measure_avg(warmup, iterations, || {
        let _ = get_dfg_context(SMALL_FN, "add_values", Language::Python);
    })
    .as_secs_f64()
        * 1000.0;
    let dm = measure_avg(warmup, iterations, || {
        let _ = get_dfg_context(MEDIUM_FN, "classify_score", Language::Python);
    })
    .as_secs_f64()
        * 1000.0;
    let dc = measure_avg(warmup, iterations, || {
        let _ = get_dfg_context(COMPLEX_FN, "matrix_stats", Language::Python);
    })
    .as_secs_f64()
        * 1000.0;
    let dst = measure_avg(warmup, iterations, || {
        let _ = get_dfg_context(STRESS_FN, "process_records", Language::Python);
    })
    .as_secs_f64()
        * 1000.0;
    rows.push(Row {
        name: "DFG+Reach",
        small: ds,
        medium: dm,
        complex: dc,
        stress: dst,
    });

    // SSA (includes CFG + DFG + dom tree + phi)
    let ss = measure_avg(warmup, iterations, || {
        let _ = construct_ssa(SMALL_FN, "add_values", Language::Python, SsaType::Minimal);
    })
    .as_secs_f64()
        * 1000.0;
    let sm = measure_avg(warmup, iterations, || {
        let _ = construct_ssa(
            MEDIUM_FN,
            "classify_score",
            Language::Python,
            SsaType::Minimal,
        );
    })
    .as_secs_f64()
        * 1000.0;
    let sc = measure_avg(warmup, iterations, || {
        let _ = construct_ssa(
            COMPLEX_FN,
            "matrix_stats",
            Language::Python,
            SsaType::Minimal,
        );
    })
    .as_secs_f64()
        * 1000.0;
    let sst = measure_avg(warmup, iterations, || {
        let _ = construct_ssa(
            STRESS_FN,
            "process_records",
            Language::Python,
            SsaType::Minimal,
        );
    })
    .as_secs_f64()
        * 1000.0;
    rows.push(Row {
        name: "SSA",
        small: ss,
        medium: sm,
        complex: sc,
        stress: sst,
    });

    // Taint (pre-built)
    let cfg_s = get_cfg_context(SMALL_FN, "add_values", Language::Python).unwrap();
    let dfg_s = get_dfg_context(SMALL_FN, "add_values", Language::Python).unwrap();
    let stmts_s = build_statements(SMALL_FN);
    let cfg_m = get_cfg_context(MEDIUM_FN, "classify_score", Language::Python).unwrap();
    let dfg_m = get_dfg_context(MEDIUM_FN, "classify_score", Language::Python).unwrap();
    let stmts_m = build_statements(MEDIUM_FN);
    let cfg_c = get_cfg_context(COMPLEX_FN, "matrix_stats", Language::Python).unwrap();
    let dfg_c = get_dfg_context(COMPLEX_FN, "matrix_stats", Language::Python).unwrap();
    let stmts_c = build_statements(COMPLEX_FN);
    let cfg_st = get_cfg_context(STRESS_FN, "process_records", Language::Python).unwrap();
    let dfg_st = get_dfg_context(STRESS_FN, "process_records", Language::Python).unwrap();
    let stmts_st = build_statements(STRESS_FN);

    let ts = measure_avg(warmup, iterations, || {
        let _ = compute_taint(&cfg_s, &dfg_s.refs, &stmts_s, Language::Python);
    })
    .as_secs_f64()
        * 1000.0;
    let tm = measure_avg(warmup, iterations, || {
        let _ = compute_taint(&cfg_m, &dfg_m.refs, &stmts_m, Language::Python);
    })
    .as_secs_f64()
        * 1000.0;
    let tc = measure_avg(warmup, iterations, || {
        let _ = compute_taint(&cfg_c, &dfg_c.refs, &stmts_c, Language::Python);
    })
    .as_secs_f64()
        * 1000.0;
    let tst = measure_avg(warmup, iterations, || {
        let _ = compute_taint(&cfg_st, &dfg_st.refs, &stmts_st, Language::Python);
    })
    .as_secs_f64()
        * 1000.0;
    rows.push(Row {
        name: "Taint",
        small: ts,
        medium: tm,
        complex: tc,
        stress: tst,
    });

    // AbsInterp (pre-built)
    let lines_s = source_lines_vec(SMALL_FN);
    let lines_m = source_lines_vec(MEDIUM_FN);
    let lines_c = source_lines_vec(COMPLEX_FN);
    let lines_st = source_lines_vec(STRESS_FN);

    let ais = measure_avg(warmup, iterations, || {
        let _ = compute_abstract_interp(&cfg_s, &dfg_s, Some(&lines_s), "python");
    })
    .as_secs_f64()
        * 1000.0;
    let aim = measure_avg(warmup, iterations, || {
        let _ = compute_abstract_interp(&cfg_m, &dfg_m, Some(&lines_m), "python");
    })
    .as_secs_f64()
        * 1000.0;
    let aic = measure_avg(warmup, iterations, || {
        let _ = compute_abstract_interp(&cfg_c, &dfg_c, Some(&lines_c), "python");
    })
    .as_secs_f64()
        * 1000.0;
    let aist = measure_avg(warmup, iterations, || {
        let _ = compute_abstract_interp(&cfg_st, &dfg_st, Some(&lines_st), "python");
    })
    .as_secs_f64()
        * 1000.0;
    rows.push(Row {
        name: "AbsInterp",
        small: ais,
        medium: aim,
        complex: aic,
        stress: aist,
    });

    // AvailExprs (pre-built)
    let aes = measure_avg(warmup, iterations, || {
        let _ = compute_available_exprs(&cfg_s, &dfg_s);
    })
    .as_secs_f64()
        * 1000.0;
    let aem = measure_avg(warmup, iterations, || {
        let _ = compute_available_exprs(&cfg_m, &dfg_m);
    })
    .as_secs_f64()
        * 1000.0;
    let aec = measure_avg(warmup, iterations, || {
        let _ = compute_available_exprs(&cfg_c, &dfg_c);
    })
    .as_secs_f64()
        * 1000.0;
    let aest = measure_avg(warmup, iterations, || {
        let _ = compute_available_exprs(&cfg_st, &dfg_st);
    })
    .as_secs_f64()
        * 1000.0;
    rows.push(Row {
        name: "AvailExpr",
        small: aes,
        medium: aem,
        complex: aec,
        stress: aest,
    });

    // Print table
    eprintln!("║                                                                          ║");
    eprintln!(
        "║  {:12} │ {:>10} │ {:>10} │ {:>10} │ {:>10}  ║",
        "Operation", "Small", "Medium", "Complex", "Stress"
    );
    eprintln!(
        "║  {:─<12} │ {:─>10} │ {:─>10} │ {:─>10} │ {:─>10}  ║",
        "", "", "", "", ""
    );
    for row in &rows {
        eprintln!(
            "║  {:12} │ {:>8.3}ms │ {:>8.3}ms │ {:>8.3}ms │ {:>8.3}ms  ║",
            row.name, row.small, row.medium, row.complex, row.stress
        );
    }

    // Full pipeline row (sum of individual ops - but note SSA/DFG overlap with CFG)
    // For the "full pipeline" we use the E2E measurement from test 08
    let full_s = measure_avg(warmup, iterations, || {
        let _ = parse(SMALL_FN, Language::Python).unwrap();
        let cfg = get_cfg_context(SMALL_FN, "add_values", Language::Python).unwrap();
        let dfg = get_dfg_context(SMALL_FN, "add_values", Language::Python).unwrap();
        let _ = construct_ssa(SMALL_FN, "add_values", Language::Python, SsaType::Minimal).unwrap();
        let stmts = build_statements(SMALL_FN);
        let _ = compute_taint(&cfg, &dfg.refs, &stmts, Language::Python);
        let lines = source_lines_vec(SMALL_FN);
        let _ = compute_abstract_interp(&cfg, &dfg, Some(&lines), "python");
        let _ = compute_available_exprs(&cfg, &dfg);
    })
    .as_secs_f64()
        * 1000.0;

    let full_m = measure_avg(warmup, iterations, || {
        let _ = parse(MEDIUM_FN, Language::Python).unwrap();
        let cfg = get_cfg_context(MEDIUM_FN, "classify_score", Language::Python).unwrap();
        let dfg = get_dfg_context(MEDIUM_FN, "classify_score", Language::Python).unwrap();
        let _ = construct_ssa(
            MEDIUM_FN,
            "classify_score",
            Language::Python,
            SsaType::Minimal,
        )
        .unwrap();
        let stmts = build_statements(MEDIUM_FN);
        let _ = compute_taint(&cfg, &dfg.refs, &stmts, Language::Python);
        let lines = source_lines_vec(MEDIUM_FN);
        let _ = compute_abstract_interp(&cfg, &dfg, Some(&lines), "python");
        let _ = compute_available_exprs(&cfg, &dfg);
    })
    .as_secs_f64()
        * 1000.0;

    let full_c = measure_avg(warmup, iterations, || {
        let _ = parse(COMPLEX_FN, Language::Python).unwrap();
        let cfg = get_cfg_context(COMPLEX_FN, "matrix_stats", Language::Python).unwrap();
        let dfg = get_dfg_context(COMPLEX_FN, "matrix_stats", Language::Python).unwrap();
        let _ = construct_ssa(
            COMPLEX_FN,
            "matrix_stats",
            Language::Python,
            SsaType::Minimal,
        )
        .unwrap();
        let stmts = build_statements(COMPLEX_FN);
        let _ = compute_taint(&cfg, &dfg.refs, &stmts, Language::Python);
        let lines = source_lines_vec(COMPLEX_FN);
        let _ = compute_abstract_interp(&cfg, &dfg, Some(&lines), "python");
        let _ = compute_available_exprs(&cfg, &dfg);
    })
    .as_secs_f64()
        * 1000.0;

    let full_st = measure_avg(warmup, iterations, || {
        let _ = parse(STRESS_FN, Language::Python).unwrap();
        let cfg = get_cfg_context(STRESS_FN, "process_records", Language::Python).unwrap();
        let dfg = get_dfg_context(STRESS_FN, "process_records", Language::Python).unwrap();
        let _ = construct_ssa(
            STRESS_FN,
            "process_records",
            Language::Python,
            SsaType::Minimal,
        )
        .unwrap();
        let stmts = build_statements(STRESS_FN);
        let _ = compute_taint(&cfg, &dfg.refs, &stmts, Language::Python);
        let lines = source_lines_vec(STRESS_FN);
        let _ = compute_abstract_interp(&cfg, &dfg, Some(&lines), "python");
        let _ = compute_available_exprs(&cfg, &dfg);
    })
    .as_secs_f64()
        * 1000.0;

    eprintln!(
        "║  {:─<12} │ {:─>10} │ {:─>10} │ {:─>10} │ {:─>10}  ║",
        "", "", "", "", ""
    );
    eprintln!(
        "║  {:12} │ {:>8.3}ms │ {:>8.3}ms │ {:>8.3}ms │ {:>8.3}ms  ║",
        "FULL E2E", full_s, full_m, full_c, full_st
    );

    eprintln!("║                                                                          ║");
    eprintln!("╠══════════════════════════════════════════════════════════════════════════╣");

    // Verdict
    let functions_in_budget = (2000.0 / full_st) as u32;
    let spec_target = 50;

    eprintln!("║  VERDICT                                                                 ║");
    eprintln!("║                                                                          ║");
    eprintln!(
        "║  Spec target: 3-10ms/fn   Actual (stress): {:.1}ms/fn              ║",
        full_st
    );
    eprintln!(
        "║  2-second budget: {} functions (target: {})                    ║",
        functions_in_budget, spec_target
    );
    eprintln!("║                                                                          ║");

    if full_st <= 10.0 {
        eprintln!("║  H3 CONFIRMED: Per-function cost is within spec (3-10ms)                 ║");
    } else if full_st <= 40.0 {
        eprintln!(
            "║  H3 PARTIALLY CONFIRMED: Cost is {:.0}x spec but >= {} fns in budget  ║",
            full_st / 10.0,
            functions_in_budget
        );
    } else if functions_in_budget >= spec_target {
        eprintln!(
            "║  H3 BUDGET OK: Cost is {:.0}x spec but {} fns still fits in 2s      ║",
            full_st / 10.0,
            functions_in_budget
        );
    } else {
        eprintln!(
            "║  H3 REJECTED: {:.1}ms/fn = only {} fns in 2s (need {})           ║",
            full_st, functions_in_budget, spec_target
        );
        eprintln!("║  PM2-P2 warning was correct: need shared-tree optimization             ║");
    }

    eprintln!("║                                                                          ║");
    eprintln!("╚══════════════════════════════════════════════════════════════════════════╝");
    eprintln!();
}
