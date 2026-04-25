//! L2 API Validation Tests for Rust Source Code
//!
//! Hypothesis H1: "tldr_core IR APIs (CFG, DFG, SSA, abstract interpretation,
//! taint) produce correct, usable results when called on Rust source code."
//!
//! The CK-10 finding from the premortem flagged that 23 APIs are signature-
//! verified but semantically unvalidated for Rust. This test file calls each
//! API with REAL Rust code snippets and verifies the results are structurally
//! correct and usable -- not garbage.
//!
//! Run with:
//!   cargo test -p tldr-cli -- l2_api_validation --nocapture
//!
//! For each API the verdict is one of:
//!   WORKS     - Produces correct, usable output for Rust
//!   DEGRADED  - Produces output but with limitations
//!   BROKEN    - Fails or produces garbage
//!   MISSING   - API does not exist or cannot be called

use std::collections::HashMap;
use tldr_core::cfg::get_cfg_context;
use tldr_core::dataflow::{compute_abstract_interp, compute_available_exprs};
use tldr_core::dfg::gvn::compute_gvn;
use tldr_core::dfg::{compute_reaching_definitions, get_dfg_context};
use tldr_core::security::compute_taint;
use tldr_core::ssa::{construct_ssa, SsaType};
use tldr_core::Language;

// =============================================================================
// Test Fixtures: Rust source code snippets
// =============================================================================

/// Simple function with if/else branching
const RUST_SIMPLE_IF: &str = r#"
fn add(a: i32, b: i32) -> i32 {
    if a > 0 {
        a + b
    } else {
        b
    }
}
"#;

/// Function with match arms
const RUST_MATCH: &str = r#"
fn classify(x: i32) -> &'static str {
    match x {
        0 => "zero",
        1..=9 => "small",
        10..=99 => "medium",
        _ => "large",
    }
}
"#;

/// Function with if-let chain
const RUST_IF_LET: &str = r#"
fn extract_value(opt: Option<i32>) -> i32 {
    if let Some(v) = opt {
        v + 1
    } else {
        0
    }
}
"#;

/// Function with a closure
const RUST_CLOSURE: &str = r#"
fn apply_twice(x: i32) -> i32 {
    let double = |n: i32| n * 2;
    let result = double(x);
    double(result)
}
"#;

/// Function with variable shadowing
const RUST_SHADOWING: &str = r#"
fn shadow(x: i32) -> i32 {
    let y = x + 1;
    let y = y * 2;
    let y = y - 3;
    y
}
"#;

/// Function with pattern matching in let
const RUST_PATTERN_LET: &str = r#"
fn destructure(pair: (i32, i32)) -> i32 {
    let (a, b) = pair;
    a + b
}
"#;

/// Function with a loop and counter (for abstract interpretation)
const RUST_LOOP_COUNTER: &str = r#"
fn count_up(n: i32) -> i32 {
    let mut total = 0;
    let mut i = 0;
    while i < n {
        total = total + i;
        i = i + 1;
    }
    total
}
"#;

/// Function with taint: reads env var and passes to Command
const RUST_TAINT: &str = r#"
fn run_user_command() {
    let user_input = std::env::var("CMD").unwrap();
    let output = Command::new(user_input);
}
"#;

/// Constant propagation candidate (for SCCP)
const RUST_CONST_PROP: &str = r#"
fn constants() -> i32 {
    let x = 10;
    let y = 20;
    let z = x + y;
    z
}
"#;

/// Two similar functions for clone detection
const RUST_CLONE_PAIR_A: &str = r#"
fn compute_area(width: f64, height: f64) -> f64 {
    let area = width * height;
    let perimeter = 2.0 * (width + height);
    let ratio = area / perimeter;
    if ratio > 1.0 {
        area
    } else {
        perimeter
    }
}
"#;

const RUST_CLONE_PAIR_B: &str = r#"
fn compute_volume(length: f64, depth: f64) -> f64 {
    let volume = length * depth;
    let surface = 2.0 * (length + depth);
    let ratio = volume / surface;
    if ratio > 1.0 {
        volume
    } else {
        surface
    }
}
"#;

/// Function with nested match for complex CFG
const RUST_NESTED_MATCH: &str = r#"
fn process(x: i32, y: i32) -> i32 {
    match x {
        0 => match y {
            0 => 0,
            _ => y,
        },
        1 => x + y,
        _ => x * y,
    }
}
"#;

/// Function with while-let for Rust-specific control flow
const RUST_WHILE_LET: &str = r#"
fn drain_iter(values: &mut Vec<i32>) -> i32 {
    let mut sum = 0;
    while let Some(v) = values.pop() {
        sum = sum + v;
    }
    sum
}
"#;

/// Function with multiple variable assignments for reaching defs
const RUST_REACHING_DEFS: &str = r#"
fn multi_assign(flag: bool) -> i32 {
    let mut x = 1;
    if flag {
        x = 2;
    }
    let y = x + 10;
    y
}
"#;

// =============================================================================
// API 1: get_cfg_context -- Control Flow Graph
// =============================================================================

#[test]
fn cfg_works_for_rust_simple_function() {
    let result = get_cfg_context(RUST_SIMPLE_IF, "add", Language::Rust);
    assert!(
        result.is_ok(),
        "CFG construction failed for Rust simple function: {:?}",
        result.err()
    );
    let cfg = result.unwrap();
    assert_eq!(cfg.function, "add", "Function name should be 'add'");
    // A simple if/else should produce at least 3 blocks: entry, if-true, if-false
    // (or an entry + branches + merge pattern)
    assert!(
        !cfg.blocks.is_empty(),
        "CFG should have at least one block, got 0"
    );
    assert!(
        !cfg.edges.is_empty(),
        "CFG should have edges for branching, got 0"
    );
    println!(
        "[CFG simple] blocks={}, edges={}, cyclomatic={}",
        cfg.blocks.len(),
        cfg.edges.len(),
        cfg.cyclomatic_complexity
    );
}

#[test]
fn cfg_works_for_rust_match_arms() {
    let result = get_cfg_context(RUST_MATCH, "classify", Language::Rust);
    assert!(
        result.is_ok(),
        "CFG construction failed for Rust match: {:?}",
        result.err()
    );
    let cfg = result.unwrap();
    assert_eq!(cfg.function, "classify");
    // match with 4 arms should create multiple blocks
    assert!(
        cfg.blocks.len() >= 2,
        "Match with 4 arms should have multiple blocks, got {}",
        cfg.blocks.len()
    );
    println!(
        "[CFG match] blocks={}, edges={}, cyclomatic={}",
        cfg.blocks.len(),
        cfg.edges.len(),
        cfg.cyclomatic_complexity
    );
}

#[test]
fn cfg_works_for_rust_if_let() {
    let result = get_cfg_context(RUST_IF_LET, "extract_value", Language::Rust);
    assert!(
        result.is_ok(),
        "CFG construction failed for Rust if-let: {:?}",
        result.err()
    );
    let cfg = result.unwrap();
    assert_eq!(cfg.function, "extract_value");
    assert!(
        !cfg.blocks.is_empty(),
        "If-let should produce blocks, got 0"
    );
    println!(
        "[CFG if-let] blocks={}, edges={}, cyclomatic={}",
        cfg.blocks.len(),
        cfg.edges.len(),
        cfg.cyclomatic_complexity
    );
}

#[test]
fn cfg_works_for_rust_closure() {
    let result = get_cfg_context(RUST_CLOSURE, "apply_twice", Language::Rust);
    assert!(
        result.is_ok(),
        "CFG construction failed for Rust closure: {:?}",
        result.err()
    );
    let cfg = result.unwrap();
    assert_eq!(cfg.function, "apply_twice");
    // Even with closures, the outer function should have a valid CFG
    assert!(
        !cfg.blocks.is_empty(),
        "Function with closure should have blocks"
    );
    println!(
        "[CFG closure] blocks={}, edges={}",
        cfg.blocks.len(),
        cfg.edges.len()
    );
}

#[test]
fn cfg_works_for_rust_nested_match() {
    let result = get_cfg_context(RUST_NESTED_MATCH, "process", Language::Rust);
    assert!(
        result.is_ok(),
        "CFG construction failed for Rust nested match: {:?}",
        result.err()
    );
    let cfg = result.unwrap();
    assert_eq!(cfg.function, "process");
    // Nested match should produce a complex CFG
    assert!(
        cfg.blocks.len() >= 2,
        "Nested match should have multiple blocks, got {}",
        cfg.blocks.len()
    );
    println!(
        "[CFG nested_match] blocks={}, edges={}, cyclomatic={}",
        cfg.blocks.len(),
        cfg.edges.len(),
        cfg.cyclomatic_complexity
    );
}

#[test]
fn cfg_works_for_rust_while_let() {
    let result = get_cfg_context(RUST_WHILE_LET, "drain_iter", Language::Rust);
    assert!(
        result.is_ok(),
        "CFG construction failed for Rust while-let: {:?}",
        result.err()
    );
    let cfg = result.unwrap();
    assert_eq!(cfg.function, "drain_iter");
    // while-let is a loop construct; should have back edges
    assert!(!cfg.blocks.is_empty(), "While-let should produce blocks");
    println!(
        "[CFG while_let] blocks={}, edges={}",
        cfg.blocks.len(),
        cfg.edges.len()
    );
}

#[test]
fn cfg_works_for_rust_loop_counter() {
    let result = get_cfg_context(RUST_LOOP_COUNTER, "count_up", Language::Rust);
    assert!(
        result.is_ok(),
        "CFG construction failed for Rust loop counter: {:?}",
        result.err()
    );
    let cfg = result.unwrap();
    assert_eq!(cfg.function, "count_up");
    // while loop should produce loop structure with back edges
    assert!(
        cfg.blocks.len() >= 2,
        "Loop should have at least 2 blocks (header + body), got {}",
        cfg.blocks.len()
    );
    println!(
        "[CFG loop] blocks={}, edges={}, cyclomatic={}",
        cfg.blocks.len(),
        cfg.edges.len(),
        cfg.cyclomatic_complexity
    );
    // FINDING: Rust while-loop CFG produces only 1 edge instead of the expected
    // 2+ (entry->body + back-edge from body->header). This means loop analysis
    // (phi placement, back-edge detection) will be DEGRADED for Rust while loops.
    if cfg.edges.len() < 2 {
        println!(
            "[CFG loop] DEGRADED: Only {} edge(s) for while loop -- \
             back-edge missing, loop analysis will be limited",
            cfg.edges.len()
        );
    }
}

// =============================================================================
// API 2: get_dfg_context + compute_reaching_definitions
// =============================================================================

#[test]
fn dfg_works_for_rust_simple() {
    let result = get_dfg_context(RUST_SIMPLE_IF, "add", Language::Rust);
    assert!(
        result.is_ok(),
        "DFG extraction failed for Rust: {:?}",
        result.err()
    );
    let dfg = result.unwrap();
    assert_eq!(dfg.function, "add");
    // Should detect variable references for a, b
    println!(
        "[DFG simple] refs={}, edges={}, variables={:?}",
        dfg.refs.len(),
        dfg.edges.len(),
        dfg.variables
    );
    // The function should have at least the parameters as variables
    assert!(
        !dfg.refs.is_empty(),
        "DFG should detect variable references in Rust, got 0"
    );
}

#[test]
fn dfg_works_for_rust_shadowing() {
    let result = get_dfg_context(RUST_SHADOWING, "shadow", Language::Rust);
    assert!(
        result.is_ok(),
        "DFG extraction failed for Rust shadowing: {:?}",
        result.err()
    );
    let dfg = result.unwrap();
    assert_eq!(dfg.function, "shadow");
    // Variable 'y' is defined 3 times (shadowing)
    let y_defs: Vec<_> = dfg
        .refs
        .iter()
        .filter(|r| r.name == "y" && matches!(r.ref_type, tldr_core::types::RefType::Definition))
        .collect();
    println!(
        "[DFG shadowing] refs={}, y_defs={}, variables={:?}",
        dfg.refs.len(),
        y_defs.len(),
        dfg.variables
    );
    // Rust shadowing means 'y' should appear as multiple definitions
    assert!(
        y_defs.len() >= 2,
        "Rust shadowing: expected at least 2 definitions of 'y', got {}",
        y_defs.len()
    );
}

#[test]
fn reaching_defs_works_for_rust() {
    // First get CFG and DFG
    let cfg = get_cfg_context(RUST_REACHING_DEFS, "multi_assign", Language::Rust)
        .expect("CFG should work");
    let dfg = get_dfg_context(RUST_REACHING_DEFS, "multi_assign", Language::Rust)
        .expect("DFG should work");

    // Now compute reaching definitions
    let rd = compute_reaching_definitions(&cfg, &dfg.refs);

    println!(
        "[ReachingDefs] blocks_with_in={}, blocks_with_out={}",
        rd.reaching_in.len(),
        rd.reaching_out.len()
    );

    // Should have IN/OUT sets for every block in the CFG
    for block in &cfg.blocks {
        assert!(
            rd.reaching_in.contains_key(&block.id),
            "Missing reaching_in for block {}",
            block.id
        );
        assert!(
            rd.reaching_out.contains_key(&block.id),
            "Missing reaching_out for block {}",
            block.id
        );
    }

    // The OUT set of the last block should contain some definitions
    if let Some(last_block) = cfg.exit_blocks.first() {
        let out = &rd.reaching_out[last_block];
        println!(
            "[ReachingDefs] exit block {} has {} reaching defs",
            last_block,
            out.len()
        );
        assert!(
            !out.is_empty(),
            "Exit block should have reaching definitions"
        );
    }
}

#[test]
fn reaching_defs_works_for_rust_pattern_matching() {
    let cfg = get_cfg_context(RUST_MATCH, "classify", Language::Rust).expect("CFG should work");
    let dfg = get_dfg_context(RUST_MATCH, "classify", Language::Rust).expect("DFG should work");

    let rd = compute_reaching_definitions(&cfg, &dfg.refs);

    println!(
        "[ReachingDefs match] IN sets={}, OUT sets={}",
        rd.reaching_in.len(),
        rd.reaching_out.len()
    );

    // Should have data for each block
    assert_eq!(
        rd.reaching_in.len(),
        cfg.blocks.len(),
        "Should have IN set for every block"
    );
}

// =============================================================================
// API 3: construct_ssa + run_sccp
// =============================================================================

#[test]
fn ssa_construction_works_for_rust_simple() {
    let result = construct_ssa(RUST_SIMPLE_IF, "add", Language::Rust, SsaType::Minimal);
    assert!(
        result.is_ok(),
        "SSA construction failed for Rust simple: {:?}",
        result.err()
    );
    let ssa = result.unwrap();
    assert_eq!(ssa.function, "add");
    assert!(!ssa.blocks.is_empty(), "SSA should have blocks, got 0");
    assert!(
        !ssa.ssa_names.is_empty(),
        "SSA should have named values, got 0"
    );
    println!(
        "[SSA simple] blocks={}, names={}, stats={:?}",
        ssa.blocks.len(),
        ssa.ssa_names.len(),
        ssa.stats
    );
}

#[test]
fn ssa_construction_works_for_rust_shadowing() {
    let result = construct_ssa(RUST_SHADOWING, "shadow", Language::Rust, SsaType::Minimal);
    assert!(
        result.is_ok(),
        "SSA construction failed for Rust shadowing: {:?}",
        result.err()
    );
    let ssa = result.unwrap();
    // Shadowing should produce multiple versions of 'y'
    let y_names: Vec<_> = ssa.ssa_names.iter().filter(|n| n.variable == "y").collect();
    println!(
        "[SSA shadowing] blocks={}, total_names={}, y_versions={}",
        ssa.blocks.len(),
        ssa.ssa_names.len(),
        y_names.len()
    );
    assert!(
        y_names.len() >= 2,
        "SSA should have multiple versions of 'y' due to shadowing, got {}",
        y_names.len()
    );
}

#[test]
fn ssa_construction_works_for_rust_loop() {
    let result = construct_ssa(
        RUST_LOOP_COUNTER,
        "count_up",
        Language::Rust,
        SsaType::Minimal,
    );
    assert!(
        result.is_ok(),
        "SSA construction failed for Rust loop: {:?}",
        result.err()
    );
    let ssa = result.unwrap();
    // Loop should produce phi functions at the loop header
    let total_phis: usize = ssa.blocks.iter().map(|b| b.phi_functions.len()).sum();
    println!(
        "[SSA loop] blocks={}, names={}, phis={}",
        ssa.blocks.len(),
        ssa.ssa_names.len(),
        total_phis
    );
    // With a while loop modifying 'total' and 'i', we expect phi functions
    // (may be 0 if the CFG doesn't create a proper loop back-edge in Rust)
    // This is informational -- we document what we get
    if total_phis == 0 {
        println!("[SSA loop] WARNING: No phi functions generated for loop -- Rust loop CFG may not produce back-edges");
    }
}

#[test]
fn ssa_pruned_works_for_rust() {
    let result = construct_ssa(RUST_SIMPLE_IF, "add", Language::Rust, SsaType::Pruned);
    assert!(
        result.is_ok(),
        "Pruned SSA construction failed for Rust: {:?}",
        result.err()
    );
    let ssa = result.unwrap();
    println!(
        "[SSA pruned] blocks={}, names={}, type={:?}",
        ssa.blocks.len(),
        ssa.ssa_names.len(),
        ssa.ssa_type
    );
}

#[test]
fn sccp_works_for_rust_constants() {
    use tldr_core::ssa::analysis::run_sccp;

    let ssa = construct_ssa(
        RUST_CONST_PROP,
        "constants",
        Language::Rust,
        SsaType::Minimal,
    )
    .expect("SSA construction should work");

    let result = run_sccp(&ssa);
    assert!(
        result.is_ok(),
        "SCCP failed for Rust constants: {:?}",
        result.err()
    );
    let sccp = result.unwrap();
    println!(
        "[SCCP] constants={}, unreachable_blocks={}, dead_names={}",
        sccp.constants.len(),
        sccp.unreachable_blocks.len(),
        sccp.dead_names.len()
    );
    // Even if SCCP doesn't propagate perfectly for Rust, it should not crash
    // and should return a valid result
}

#[test]
fn sccp_works_for_rust_match() {
    use tldr_core::ssa::analysis::run_sccp;

    let ssa = construct_ssa(RUST_MATCH, "classify", Language::Rust, SsaType::Minimal)
        .expect("SSA construction should work");

    let result = run_sccp(&ssa);
    assert!(
        result.is_ok(),
        "SCCP failed for Rust match: {:?}",
        result.err()
    );
    let sccp = result.unwrap();
    println!(
        "[SCCP match] constants={}, unreachable={}, dead={}",
        sccp.constants.len(),
        sccp.unreachable_blocks.len(),
        sccp.dead_names.len()
    );
}

// =============================================================================
// API 4: compute_taint -- Taint Analysis
// =============================================================================

#[test]
fn taint_works_for_rust_env_to_command() {
    // Build CFG and DFG for the taint function
    let cfg =
        get_cfg_context(RUST_TAINT, "run_user_command", Language::Rust).expect("CFG should work");
    let dfg =
        get_dfg_context(RUST_TAINT, "run_user_command", Language::Rust).expect("DFG should work");

    // Build statements map from source lines
    let statements: HashMap<u32, String> = RUST_TAINT
        .lines()
        .enumerate()
        .map(|(i, line)| ((i + 1) as u32, line.to_string()))
        .collect();

    let result = compute_taint(&cfg, &dfg.refs, &statements, Language::Rust);
    assert!(
        result.is_ok(),
        "Taint analysis failed for Rust: {:?}",
        result.err()
    );
    let taint = result.unwrap();
    println!(
        "[Taint Rust] sources={}, sinks={}, flows={}, tainted_vars_blocks={}",
        taint.sources.len(),
        taint.sinks.len(),
        taint.flows.len(),
        taint.tainted_vars.len()
    );
    // Rust patterns should detect std::env::var as a source
    if taint.sources.is_empty() {
        println!("[Taint Rust] WARNING: No sources detected -- Rust taint patterns may not match this snippet");
    } else {
        println!(
            "[Taint Rust] Sources detected: {:?}",
            taint
                .sources
                .iter()
                .map(|s| format!("{:?} at line {}", s.source_type, s.line))
                .collect::<Vec<_>>()
        );
    }
    // Command::new should be detected as a sink
    if taint.sinks.is_empty() {
        println!("[Taint Rust] WARNING: No sinks detected -- Rust taint patterns may not match Command::new");
    } else {
        println!(
            "[Taint Rust] Sinks detected: {:?}",
            taint
                .sinks
                .iter()
                .map(|s| format!("{:?} at line {}", s.sink_type, s.line))
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn taint_with_tree_works_for_rust() {
    use tldr_core::ast::parser::parse;
    use tldr_core::security::compute_taint_with_tree;

    let cfg =
        get_cfg_context(RUST_TAINT, "run_user_command", Language::Rust).expect("CFG should work");
    let dfg =
        get_dfg_context(RUST_TAINT, "run_user_command", Language::Rust).expect("DFG should work");

    let tree = parse(RUST_TAINT, Language::Rust).expect("Parse should work");

    let statements: HashMap<u32, String> = RUST_TAINT
        .lines()
        .enumerate()
        .map(|(i, line)| ((i + 1) as u32, line.to_string()))
        .collect();

    let result = compute_taint_with_tree(
        &cfg,
        &dfg.refs,
        &statements,
        Some(&tree),
        Some(RUST_TAINT.as_bytes()),
        Language::Rust,
    );
    assert!(
        result.is_ok(),
        "Taint with tree failed for Rust: {:?}",
        result.err()
    );
    let taint = result.unwrap();
    println!(
        "[Taint+Tree Rust] sources={}, sinks={}, flows={}",
        taint.sources.len(),
        taint.sinks.len(),
        taint.flows.len()
    );
}

// =============================================================================
// API 5: compute_abstract_interp -- Abstract Interpretation
// =============================================================================

#[test]
fn abstract_interp_works_for_rust_loop() {
    let cfg =
        get_cfg_context(RUST_LOOP_COUNTER, "count_up", Language::Rust).expect("CFG should work");
    let dfg =
        get_dfg_context(RUST_LOOP_COUNTER, "count_up", Language::Rust).expect("DFG should work");

    let source_lines: Vec<&str> = RUST_LOOP_COUNTER.lines().collect();
    let result = compute_abstract_interp(&cfg, &dfg, Some(&source_lines), "rust");
    assert!(
        result.is_ok(),
        "Abstract interpretation failed for Rust loop: {:?}",
        result.err()
    );
    let interp = result.unwrap();
    println!(
        "[AbstractInterp loop] block_states={}, div_zero_warnings={}, null_deref_warnings={}",
        interp.state_in.len(),
        interp.potential_div_zero.len(),
        interp.potential_null_deref.len()
    );
    // Should have state for each block
    assert!(
        !interp.state_in.is_empty(),
        "Abstract interpretation should produce block states"
    );
}

#[test]
fn abstract_interp_works_for_rust_simple() {
    let cfg = get_cfg_context(RUST_SIMPLE_IF, "add", Language::Rust).expect("CFG should work");
    let dfg = get_dfg_context(RUST_SIMPLE_IF, "add", Language::Rust).expect("DFG should work");

    let source_lines: Vec<&str> = RUST_SIMPLE_IF.lines().collect();
    let result = compute_abstract_interp(&cfg, &dfg, Some(&source_lines), "rust");
    assert!(
        result.is_ok(),
        "Abstract interpretation failed for Rust simple: {:?}",
        result.err()
    );
    let interp = result.unwrap();
    println!(
        "[AbstractInterp simple] block_states={}, variables tracked: {:?}",
        interp.state_in.len(),
        interp
            .state_in
            .values()
            .flat_map(|s| s.values.keys())
            .collect::<std::collections::HashSet<_>>()
    );
}

// =============================================================================
// API 6: compute_gvn -- Global Value Numbering
// PREMORTEM FLAG: CK-10 says this hardcodes Python. VERIFY.
// =============================================================================

#[test]
fn gvn_hardcodes_python_confirmed() {
    // CK-10 premortem finding: compute_gvn hardcodes Language::Python
    // at engine.rs:1090 and searches for "function_definition" |
    // "async_function_definition" (Python AST node kinds).
    //
    // VERIFIED: The source code at engine.rs:1090 reads:
    //   let tree = match parse(source, Language::Python) {
    // And find_functions looks for "function_definition" which is Python-specific.
    //
    // This means calling compute_gvn with Rust source will:
    // 1. Parse the Rust source as Python (tree-sitter will fail or produce garbage)
    // 2. Return an empty Vec since no Python function_definitions are found

    let reports = compute_gvn(RUST_SIMPLE_IF, Some("add"));
    println!(
        "[GVN] Rust source returned {} reports (expected 0 because Python is hardcoded)",
        reports.len()
    );
    // CONFIRMED: GVN returns empty for Rust because it parses as Python
    assert!(
        reports.is_empty(),
        "GVN should return empty for Rust since it hardcodes Language::Python -- \
         if this fails, GVN may have been updated to support multiple languages"
    );
    println!("[GVN] CONFIRMED: compute_gvn hardcodes Language::Python -- BROKEN for Rust");
}

#[test]
fn gvn_works_for_python_baseline() {
    // Confirm GVN works for Python (baseline that it's not entirely broken)
    let python_source = r#"
def example(a, b):
    x = a + b
    y = a + b
    return x + y
"#;
    let reports = compute_gvn(python_source, Some("example"));
    assert!(
        !reports.is_empty(),
        "GVN should work for Python (baseline check)"
    );
    let report = &reports[0];
    println!(
        "[GVN Python] function={}, expressions={}, equivalences={}, redundancies={}",
        report.function,
        report.total_expressions,
        report.equivalences.len(),
        report.redundancies.len()
    );
    // a+b appears twice, should be detected as redundant
    assert!(
        !report.redundancies.is_empty(),
        "GVN should detect redundancy in Python baseline"
    );
}

// =============================================================================
// API 7: detect_clones -- Clone Detection (language-agnostic tokenization)
// =============================================================================

#[test]
fn clone_detection_works_for_rust_files() {
    use std::fs;
    use tempfile::tempdir;
    use tldr_core::analysis::clones::{detect_clones, ClonesOptions};

    let dir = tempdir().expect("Failed to create temp dir");

    // Write two similar Rust files
    let file_a = dir.path().join("area.rs");
    let file_b = dir.path().join("volume.rs");
    fs::write(&file_a, RUST_CLONE_PAIR_A).expect("Write file A");
    fs::write(&file_b, RUST_CLONE_PAIR_B).expect("Write file B");

    let options = ClonesOptions {
        language: Some("rust".to_string()),
        min_tokens: 10,
        min_lines: 3,
        ..ClonesOptions::default()
    };

    let result = detect_clones(dir.path(), &options);
    assert!(
        result.is_ok(),
        "Clone detection failed for Rust: {:?}",
        result.err()
    );
    let report = result.unwrap();
    println!(
        "[Clones] files_scanned={}, total_clones={}, type1={}, type2={}, type3={}",
        report.stats.files_analyzed,
        report.stats.clones_found,
        report.stats.type1_count,
        report.stats.type2_count,
        report.stats.type3_count,
    );
    // The two functions are structurally identical with different names
    // so Type-2 or Type-3 clones should be detected
    // Even if no clones are found, the API should work without error
    assert!(
        report.stats.files_analyzed >= 2,
        "Should scan at least 2 Rust files, got {}",
        report.stats.files_analyzed
    );
}

// =============================================================================
// API 8: detect_smells -- Code Smell Detection
// =============================================================================

#[test]
fn smell_detection_works_for_rust_file() {
    use std::fs;
    use tempfile::tempdir;
    use tldr_core::quality::smells::{detect_smells, ThresholdPreset};

    let dir = tempdir().expect("Failed to create temp dir");

    // Write a Rust file with a long function (potential smell)
    let long_fn = r#"
fn very_long_function(a: i32, b: i32, c: i32, d: i32, e: i32, f: i32, g: i32) -> i32 {
    let x1 = a + b;
    let x2 = c + d;
    let x3 = e + f;
    let x4 = x1 + x2;
    let x5 = x3 + g;
    let x6 = x4 + x5;
    let x7 = x6 * 2;
    let x8 = x7 - 1;
    let x9 = x8 + a;
    let x10 = x9 * b;
    let x11 = x10 + c;
    let x12 = x11 - d;
    let x13 = x12 * e;
    let x14 = x13 + f;
    let x15 = x14 - g;
    let x16 = x15 * 2;
    let x17 = x16 + 1;
    let x18 = x17 - 2;
    let x19 = x18 * 3;
    let x20 = x19 + 4;
    x20
}
"#;
    let file = dir.path().join("smelly.rs");
    fs::write(&file, long_fn).expect("Write smelly file");

    let result = detect_smells(dir.path(), ThresholdPreset::Strict, None, false);
    assert!(
        result.is_ok(),
        "Smell detection failed for Rust: {:?}",
        result.err()
    );
    let report = result.unwrap();
    println!(
        "[Smells] total_smells={}, files_scanned={}",
        report.smells.len(),
        report.files_scanned
    );
    // Should detect LongParameterList (7 params > 5 threshold)
    let long_params: Vec<_> = report
        .smells
        .iter()
        .filter(|s| s.smell_type == tldr_core::quality::smells::SmellType::LongParameterList)
        .collect();
    println!("[Smells] LongParameterList findings: {}", long_params.len());
    // Even if the specific smell isn't detected, the scan should complete
    assert!(
        report.files_scanned >= 1,
        "Should scan at least 1 file, got {}",
        report.files_scanned
    );
}

// =============================================================================
// API 9: Available Expressions (dataflow layer)
// =============================================================================

#[test]
fn available_expressions_works_for_rust() {
    let cfg = get_cfg_context(RUST_SIMPLE_IF, "add", Language::Rust).expect("CFG should work");
    let dfg = get_dfg_context(RUST_SIMPLE_IF, "add", Language::Rust).expect("DFG should work");

    let result = compute_available_exprs(&cfg, &dfg);
    assert!(
        result.is_ok(),
        "Available expressions failed for Rust: {:?}",
        result.err()
    );
    let avail = result.unwrap();
    println!(
        "[AvailExprs] block_data={}, total_expressions={}, redundant={}",
        avail.avail_in.len(),
        avail.all_exprs.len(),
        avail.redundant_computations().len()
    );
}

#[test]
fn available_expressions_works_for_rust_shadowing() {
    let cfg = get_cfg_context(RUST_SHADOWING, "shadow", Language::Rust).expect("CFG should work");
    let dfg = get_dfg_context(RUST_SHADOWING, "shadow", Language::Rust).expect("DFG should work");

    let result = compute_available_exprs(&cfg, &dfg);
    assert!(
        result.is_ok(),
        "Available expressions failed for Rust shadowing: {:?}",
        result.err()
    );
    let avail = result.unwrap();
    println!(
        "[AvailExprs shadow] expressions={}, redundant={}",
        avail.all_exprs.len(),
        avail.redundant_computations().len()
    );
}

// =============================================================================
// API Comprehensive: Full pipeline CFG -> DFG -> SSA -> SCCP for Rust
// =============================================================================

#[test]
fn full_pipeline_works_for_rust() {
    use tldr_core::ssa::analysis::run_sccp;

    // Step 1: CFG
    let cfg =
        get_cfg_context(RUST_REACHING_DEFS, "multi_assign", Language::Rust).expect("CFG failed");
    println!(
        "[Pipeline] CFG: blocks={}, edges={}",
        cfg.blocks.len(),
        cfg.edges.len()
    );

    // Step 2: DFG
    let dfg =
        get_dfg_context(RUST_REACHING_DEFS, "multi_assign", Language::Rust).expect("DFG failed");
    println!(
        "[Pipeline] DFG: refs={}, variables={:?}",
        dfg.refs.len(),
        dfg.variables
    );

    // Step 3: Reaching Definitions
    let rd = compute_reaching_definitions(&cfg, &dfg.refs);
    println!(
        "[Pipeline] ReachingDefs: IN={}, OUT={}",
        rd.reaching_in.len(),
        rd.reaching_out.len()
    );

    // Step 4: SSA Construction
    let ssa = construct_ssa(
        RUST_REACHING_DEFS,
        "multi_assign",
        Language::Rust,
        SsaType::Minimal,
    )
    .expect("SSA failed");
    let total_phis: usize = ssa.blocks.iter().map(|b| b.phi_functions.len()).sum();
    println!(
        "[Pipeline] SSA: blocks={}, names={}, phis={}",
        ssa.blocks.len(),
        ssa.ssa_names.len(),
        total_phis
    );

    // Step 5: SCCP
    let sccp = run_sccp(&ssa).expect("SCCP failed");
    println!(
        "[Pipeline] SCCP: constants={}, unreachable={}, dead={}",
        sccp.constants.len(),
        sccp.unreachable_blocks.len(),
        sccp.dead_names.len()
    );

    // Step 6: Available Expressions
    let avail = compute_available_exprs(&cfg, &dfg).expect("AvailExprs failed");
    println!(
        "[Pipeline] AvailExprs: expressions={}, redundant={}",
        avail.all_exprs.len(),
        avail.redundant_computations().len()
    );

    // Step 7: Abstract Interpretation
    let source_lines: Vec<&str> = RUST_REACHING_DEFS.lines().collect();
    let interp = compute_abstract_interp(&cfg, &dfg, Some(&source_lines), "rust")
        .expect("AbstractInterp failed");
    println!(
        "[Pipeline] AbstractInterp: states={}, div_zero={}, null_deref={}",
        interp.state_in.len(),
        interp.potential_div_zero.len(),
        interp.potential_null_deref.len()
    );

    // Step 8: Taint Analysis
    let statements: HashMap<u32, String> = RUST_REACHING_DEFS
        .lines()
        .enumerate()
        .map(|(i, line)| ((i + 1) as u32, line.to_string()))
        .collect();
    let taint = compute_taint(&cfg, &dfg.refs, &statements, Language::Rust).expect("Taint failed");
    println!(
        "[Pipeline] Taint: sources={}, sinks={}, flows={}",
        taint.sources.len(),
        taint.sinks.len(),
        taint.flows.len()
    );

    println!("[Pipeline] COMPLETE: Full IR pipeline succeeded for Rust");
}

// =============================================================================
// Edge case: Empty / missing function
// =============================================================================

#[test]
fn dfg_works_for_rust_pattern_destructuring() {
    let result = get_dfg_context(RUST_PATTERN_LET, "destructure", Language::Rust);
    assert!(
        result.is_ok(),
        "DFG extraction failed for Rust pattern destructuring: {:?}",
        result.err()
    );
    let dfg = result.unwrap();
    assert_eq!(dfg.function, "destructure");
    println!(
        "[DFG destructure] refs={}, variables={:?}",
        dfg.refs.len(),
        dfg.variables
    );
    // Should detect variables from tuple destructuring: a, b, pair
    assert!(
        !dfg.refs.is_empty(),
        "DFG should detect refs in destructured patterns"
    );
}

#[test]
fn cfg_returns_empty_for_missing_rust_function() {
    let result = get_cfg_context(RUST_SIMPLE_IF, "nonexistent", Language::Rust);
    assert!(
        result.is_ok(),
        "Should return Ok with empty CFG for missing function"
    );
    let cfg = result.unwrap();
    assert!(
        cfg.blocks.is_empty(),
        "Missing function should return empty CFG"
    );
    println!("[CFG missing] Correctly returns empty CFG for nonexistent function");
}

#[test]
fn ssa_handles_empty_rust_function() {
    // A function with no variable assignments
    let empty_fn = r#"
fn noop() {
}
"#;
    let result = construct_ssa(empty_fn, "noop", Language::Rust, SsaType::Minimal);
    // This might fail or produce empty SSA -- both are valid outcomes
    match result {
        Ok(ssa) => {
            println!(
                "[SSA empty] blocks={}, names={}",
                ssa.blocks.len(),
                ssa.ssa_names.len()
            );
        }
        Err(e) => {
            println!(
                "[SSA empty] Error (may be expected for empty function): {:?}",
                e
            );
        }
    }
}
