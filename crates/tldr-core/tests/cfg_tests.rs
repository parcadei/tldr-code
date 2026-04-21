//! Tests for Layer 3: CFG operations
//!
//! Commands tested: cfg, complexity
//!
//! These tests verify the CFG extraction and complexity calculation functionality.

use std::path::PathBuf;

use tldr_core::cfg::get_cfg_context;
use tldr_core::metrics::calculate_complexity;
use tldr_core::{BlockType, EdgeType, Language, TldrError};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

// =============================================================================
// cfg command tests
// =============================================================================

mod cfg_tests {
    use super::*;

    #[test]
    fn cfg_extracts_basic_blocks() {
        // GIVEN: A Python function
        let file = fixtures_dir().join("simple-project/main.py");

        // WHEN: We extract the CFG for 'main'
        let cfg = get_cfg_context(file.to_str().unwrap(), "main", Language::Python);

        // THEN: Basic blocks should be extracted
        assert!(cfg.is_ok());
        let cfg = cfg.unwrap();
        assert!(!cfg.blocks.is_empty());
    }

    #[test]
    fn cfg_identifies_entry_block() {
        // GIVEN: A function
        let source = "def foo():\n    return 1";

        // WHEN: We extract the CFG
        let cfg = get_cfg_context(source, "foo", Language::Python).unwrap();

        // THEN: entry_block should point to the first block (index 0)
        assert_eq!(cfg.entry_block, 0);
        assert!(!cfg.blocks.is_empty());
        assert_eq!(cfg.blocks[0].block_type, BlockType::Entry);
    }

    #[test]
    fn cfg_identifies_exit_blocks() {
        // GIVEN: A function with multiple return statements
        let source = r#"
def multi_return(x):
    if x > 0:
        return 1
    return -1
"#;

        // WHEN: We extract the CFG
        let cfg = get_cfg_context(source, "multi_return", Language::Python).unwrap();

        // THEN: exit_blocks should list all return points
        assert!(!cfg.exit_blocks.is_empty());
        // Should have at least one exit block
        assert!(cfg.exit_blocks.iter().any(|&id| {
            cfg.blocks
                .get(id)
                .map(|b| b.block_type == BlockType::Exit)
                .unwrap_or(false)
        }));
    }

    #[test]
    fn cfg_creates_branch_edges() {
        // GIVEN: A function with if/else
        let source = r#"
def branching(x):
    if x > 0:
        return 1
    else:
        return -1
"#;

        // WHEN: We extract the CFG
        let cfg = get_cfg_context(source, "branching", Language::Python).unwrap();

        // THEN: True and False edges should exist
        let has_true_edge = cfg.edges.iter().any(|e| e.edge_type == EdgeType::True);
        let has_false_edge = cfg.edges.iter().any(|e| e.edge_type == EdgeType::False);
        assert!(has_true_edge, "Should have True edge");
        assert!(has_false_edge, "Should have False edge");
    }

    #[test]
    fn cfg_handles_loops() {
        // GIVEN: A function with a for loop
        let file = fixtures_dir().join("simple-project/main.py");

        // WHEN: We extract the CFG for 'process_data'
        let cfg =
            get_cfg_context(file.to_str().unwrap(), "process_data", Language::Python).unwrap();

        // THEN: LoopHeader and BackEdge should be present
        let has_loop_header = cfg
            .blocks
            .iter()
            .any(|b| b.block_type == BlockType::LoopHeader);
        let has_back_edge = cfg.edges.iter().any(|e| e.edge_type == EdgeType::BackEdge);
        assert!(has_loop_header, "Should have LoopHeader block");
        assert!(has_back_edge, "Should have BackEdge");
    }

    #[test]
    fn cfg_handles_while_loops() {
        // GIVEN: A function with a while loop
        let source = r#"
def with_while(n):
    i = 0
    while i < n:
        i += 1
    return i
"#;

        // WHEN: We extract the CFG
        let cfg = get_cfg_context(source, "with_while", Language::Python).unwrap();

        // THEN: Loop structure should be correct
        let has_loop_header = cfg
            .blocks
            .iter()
            .any(|b| b.block_type == BlockType::LoopHeader);
        let has_back_edge = cfg.edges.iter().any(|e| e.edge_type == EdgeType::BackEdge);
        assert!(has_loop_header, "Should have LoopHeader block");
        assert!(has_back_edge, "Should have BackEdge");
    }

    #[test]
    fn cfg_tracks_function_calls() {
        // GIVEN: A function that calls other functions
        let file = fixtures_dir().join("simple-project/main.py");

        // WHEN: We extract the CFG for 'main'
        let cfg = get_cfg_context(file.to_str().unwrap(), "main", Language::Python).unwrap();

        // THEN: Blocks should list the calls they make
        let all_calls: Vec<&String> = cfg.blocks.iter().flat_map(|b| b.calls.iter()).collect();
        assert!(!all_calls.is_empty(), "Should have function calls tracked");
        // main calls process_data
        assert!(
            all_calls.iter().any(|c| c.contains("process_data")),
            "Should track call to process_data"
        );
    }

    #[test]
    fn cfg_calculates_cyclomatic_complexity() {
        // GIVEN: A function with known complexity
        let file = fixtures_dir().join("python-project/app.py");

        // WHEN: We extract the CFG for 'complex_function'
        let cfg =
            get_cfg_context(file.to_str().unwrap(), "complex_function", Language::Python).unwrap();

        // THEN: Cyclomatic complexity should be calculated (E - N + 2)
        // The function has many branches, so complexity should be > 1
        assert!(
            cfg.cyclomatic_complexity > 1,
            "Complex function should have cyclomatic > 1, got {}",
            cfg.cyclomatic_complexity
        );
    }

    #[test]
    fn cfg_handles_function_not_found() {
        // GIVEN: A file
        let file = fixtures_dir().join("simple-project/main.py");

        // WHEN: We search for a nonexistent function
        let result = get_cfg_context(file.to_str().unwrap(), "nonexistent", Language::Python);

        // THEN: It should return empty CFG with all zeros (per spec)
        let cfg = result.unwrap();
        assert!(cfg.blocks.is_empty());
        assert_eq!(cfg.cyclomatic_complexity, 0);
    }

    #[test]
    fn cfg_extracts_nested_functions() {
        // GIVEN: A function containing nested function definitions
        let source = r#"
def outer():
    def inner():
        return 1
    return inner()
"#;

        // WHEN: We extract the CFG
        let cfg = get_cfg_context(source, "outer", Language::Python).unwrap();

        // THEN: nested_functions should contain CFGs for inner functions
        assert!(
            cfg.nested_functions.contains_key("inner"),
            "Should have nested function 'inner'"
        );
    }

    #[test]
    fn cfg_handles_try_except() {
        // GIVEN: A function with try/except blocks
        let source = r#"
def with_try():
    try:
        return risky()
    except:
        return fallback()
"#;

        // WHEN: We extract the CFG
        let cfg = get_cfg_context(source, "with_try", Language::Python).unwrap();

        // THEN: Exception edges should be modeled
        assert!(!cfg.blocks.is_empty());
        // Should have multiple blocks for try/except structure
        assert!(
            cfg.blocks.len() >= 3,
            "Try/except should have multiple blocks, got {}",
            cfg.blocks.len()
        );
    }

    #[test]
    fn cfg_handles_async_await() {
        // GIVEN: An async function with await
        let source = r#"
async def async_func():
    result = await fetch()
    return result
"#;

        // WHEN: We extract the CFG
        let cfg = get_cfg_context(source, "async_func", Language::Python).unwrap();

        // THEN: Should successfully extract CFG (await points may create block boundaries)
        assert!(!cfg.blocks.is_empty());
    }

    #[test]
    fn cfg_captures_line_ranges() {
        // GIVEN: A function spanning multiple lines
        let source = r#"
def multiline():
    a = 1
    b = 2
    c = 3
    return a + b + c
"#;

        // WHEN: We extract the CFG
        let cfg = get_cfg_context(source, "multiline", Language::Python).unwrap();

        // THEN: Each block should have accurate line ranges
        assert!(!cfg.blocks.is_empty());
        let entry = &cfg.blocks[0];
        assert!(entry.lines.0 > 0, "Line numbers should be positive");
        assert!(
            entry.lines.1 >= entry.lines.0,
            "End line should be >= start line"
        );
    }

    #[test]
    fn cfg_handles_break_continue() {
        // GIVEN: A loop with break and continue statements
        let source = r#"
def with_break_continue():
    for i in range(10):
        if i == 5:
            break
        if i % 2 == 0:
            continue
        print(i)
"#;

        // WHEN: We extract the CFG
        let cfg = get_cfg_context(source, "with_break_continue", Language::Python).unwrap();

        // THEN: Break and Continue edges should be present
        let has_break = cfg.edges.iter().any(|e| e.edge_type == EdgeType::Break);
        let has_continue = cfg.edges.iter().any(|e| e.edge_type == EdgeType::Continue);
        assert!(has_break, "Should have Break edge");
        assert!(has_continue, "Should have Continue edge");
    }

    #[test]
    fn cfg_handles_conditions() {
        // GIVEN: Branches with conditions
        let source = r#"
def with_conditions(x):
    if x > 0:
        return "positive"
    return "non-positive"
"#;

        // WHEN: We extract the CFG
        let cfg = get_cfg_context(source, "with_conditions", Language::Python).unwrap();

        // THEN: Edge conditions should be captured
        let true_edge = cfg.edges.iter().find(|e| e.edge_type == EdgeType::True);
        assert!(true_edge.is_some());
        // Condition may be captured
        if let Some(edge) = true_edge {
            // Condition is optional based on implementation
            // Just verify the edge exists
            assert!(edge.from < cfg.blocks.len());
            assert!(edge.to < cfg.blocks.len());
        }
    }

    #[test]
    fn cfg_works_from_source_string() {
        // GIVEN: Source code as a string (not file path)
        let source = r#"
def foo():
    if True:
        return 1
    return 0
"#;

        // WHEN: We extract the CFG
        let cfg = get_cfg_context(source, "foo", Language::Python);

        // THEN: It should work the same as file-based extraction
        assert!(cfg.is_ok());
        let cfg = cfg.unwrap();
        assert!(!cfg.blocks.is_empty());
        assert!(cfg.cyclomatic_complexity >= 2); // if adds complexity
    }
}

// =============================================================================
// complexity command tests
// =============================================================================

mod complexity_tests {
    use super::*;

    #[test]
    fn complexity_calculates_cyclomatic() {
        // GIVEN: A function
        let file = fixtures_dir().join("python-project/app.py");

        // WHEN: We calculate complexity
        let metrics =
            calculate_complexity(file.to_str().unwrap(), "complex_function", Language::Python);

        // THEN: Cyclomatic complexity should be calculated
        assert!(metrics.is_ok());
        let metrics = metrics.unwrap();
        assert!(metrics.cyclomatic > 0);
    }

    #[test]
    fn complexity_calculates_cognitive() {
        // GIVEN: A function with nested structures
        let file = fixtures_dir().join("python-project/app.py");

        // WHEN: We calculate complexity
        let metrics =
            calculate_complexity(file.to_str().unwrap(), "complex_function", Language::Python)
                .unwrap();

        // THEN: Cognitive complexity should be > 0 due to nesting
        // Cognitive adds penalty for nesting, so it's usually >= cyclomatic for nested code
        assert!(
            metrics.cognitive > 0,
            "Complex function should have cognitive > 0, got {}",
            metrics.cognitive
        );
    }

    #[test]
    fn complexity_tracks_nesting_depth() {
        // GIVEN: A deeply nested function
        let file = fixtures_dir().join("python-project/app.py");

        // WHEN: We calculate complexity
        let metrics =
            calculate_complexity(file.to_str().unwrap(), "complex_function", Language::Python)
                .unwrap();

        // THEN: nesting_depth should reflect max nesting
        // The complex_function has nested if statements
        assert!(
            metrics.nesting_depth >= 2,
            "Should have nesting depth >= 2, got {}",
            metrics.nesting_depth
        );
    }

    #[test]
    fn complexity_counts_lines_of_code() {
        // GIVEN: A function
        let file = fixtures_dir().join("python-project/app.py");

        // WHEN: We calculate complexity
        let metrics =
            calculate_complexity(file.to_str().unwrap(), "long_function", Language::Python)
                .unwrap();

        // THEN: LOC should be counted
        // long_function has ~30 lines
        assert!(
            metrics.lines_of_code > 20,
            "Should have LOC > 20, got {}",
            metrics.lines_of_code
        );
    }

    #[test]
    fn complexity_counts_decision_points() {
        // GIVEN: A function with various decision points
        // if, elif, else, for, while, case, catch, &&, ||, ?:
        let source = r#"
def decisions(a, b):
    if a > 0:
        pass
    elif a < 0:
        pass
    for i in range(10):
        if b and a:
            pass
    return a or b
"#;

        // WHEN: We calculate cyclomatic complexity
        let metrics = calculate_complexity(source, "decisions", Language::Python).unwrap();

        // THEN: Each decision point should increment the count
        // Base 1 + if + elif + for + if + and + or = 7
        assert!(
            metrics.cyclomatic >= 5,
            "Should have cyclomatic >= 5, got {}",
            metrics.cyclomatic
        );
    }

    #[test]
    fn complexity_adds_cognitive_penalty_for_nesting() {
        // GIVEN: Nested control structures
        let source = r#"
def nested(a, b, c):
    if a:
        if b:
            if c:
                return 1
    return 0
"#;

        // WHEN: We calculate cognitive complexity
        let metrics = calculate_complexity(source, "nested", Language::Python).unwrap();

        // THEN: Additional increments for each nesting level
        // Per SonarSource cognitive complexity rules
        // First if: 1, second if: 1+1 (nesting), third if: 1+2 (nesting)
        assert!(
            metrics.cognitive >= 3,
            "Should have cognitive >= 3 for nested ifs, got {}",
            metrics.cognitive
        );
    }

    #[test]
    fn complexity_simple_function() {
        // GIVEN: A simple linear function
        let source = r#"
def add_to_total(current, value):
    return current + value
"#;

        // WHEN: We calculate complexity
        let metrics = calculate_complexity(source, "add_to_total", Language::Python).unwrap();

        // THEN: Complexity should be 1 (no branches)
        assert_eq!(
            metrics.cyclomatic, 1,
            "Simple function should have cyclomatic = 1"
        );
        assert_eq!(
            metrics.cognitive, 0,
            "Simple function should have cognitive = 0"
        );
    }

    #[test]
    fn complexity_handles_function_not_found() {
        // GIVEN: A file
        let file = fixtures_dir().join("simple-project/main.py");

        // WHEN: We calculate complexity for nonexistent function
        let result = calculate_complexity(file.to_str().unwrap(), "nonexistent", Language::Python);

        // THEN: It should return an error
        assert!(matches!(result, Err(TldrError::FunctionNotFound { .. })));
    }
}
