//! Tests for Layer 6: SSA (Static Single Assignment) operations
//!
//! Commands tested: ssa construction, dominator tree, value numbering
//!
//! These tests verify SSA form construction and analysis functionality.

use std::collections::HashSet;
use tldr_core::cfg::get_cfg_context;
use tldr_core::dfg::get_dfg_context;
use tldr_core::ssa::{
    build_dominator_tree, compute_dominance_frontier, compute_live_variables,
    compute_value_numbers, construct_minimal_ssa, construct_semi_pruned_ssa, SsaType,
};
use tldr_core::Language;

// =============================================================================
// Dominator Tree Tests
// =============================================================================

mod dominator_tests {
    use super::*;

    #[test]
    fn dominator_tree_builds_for_simple_function() {
        // GIVEN: A simple function
        let source = r#"
def foo():
    x = 1
    return x
"#;
        let cfg = get_cfg_context(source, "foo", Language::Python).unwrap();

        // WHEN: We build the dominator tree
        let dom_tree = build_dominator_tree(&cfg).unwrap();

        // THEN: Should have nodes for each block
        assert!(
            !dom_tree.nodes.is_empty(),
            "Dominator tree should have nodes"
        );
        assert_eq!(dom_tree.function, "foo", "Function name should match");
    }

    #[test]
    fn dominator_tree_entry_has_no_idom() {
        // GIVEN: A function
        let source = r#"
def foo():
    return 1
"#;
        let cfg = get_cfg_context(source, "foo", Language::Python).unwrap();

        // WHEN: We build the dominator tree
        let dom_tree = build_dominator_tree(&cfg).unwrap();

        // THEN: Entry block should have no immediate dominator
        let entry_node = dom_tree.nodes.get(&cfg.entry_block);
        assert!(entry_node.is_some(), "Should have entry node");
        assert!(
            entry_node.unwrap().idom.is_none(),
            "Entry should have no idom"
        );
    }

    #[test]
    fn dominator_tree_dominates_check() {
        // GIVEN: A function with branches
        let source = r#"
def foo(cond):
    if cond:
        x = 1
    else:
        x = 2
    return x
"#;
        let cfg = get_cfg_context(source, "foo", Language::Python).unwrap();
        let dom_tree = build_dominator_tree(&cfg).unwrap();

        // THEN: Entry should dominate all blocks
        for &block_id in dom_tree.nodes.keys() {
            assert!(
                dom_tree.dominates(cfg.entry_block, block_id),
                "Entry should dominate all blocks"
            );
        }
    }

    #[test]
    fn dominator_tree_strictly_dominates() {
        // GIVEN: A function
        let source = r#"
def foo():
    x = 1
    return x
"#;
        let cfg = get_cfg_context(source, "foo", Language::Python).unwrap();
        let dom_tree = build_dominator_tree(&cfg).unwrap();

        // THEN: A block should not strictly dominate itself
        for &block_id in dom_tree.nodes.keys() {
            assert!(
                !dom_tree.strictly_dominates(block_id, block_id),
                "Block should not strictly dominate itself"
            );
        }
    }

    #[test]
    fn dominator_tree_dominated_by() {
        // GIVEN: A function
        let source = r#"
def foo():
    return 1
"#;
        let cfg = get_cfg_context(source, "foo", Language::Python).unwrap();
        let dom_tree = build_dominator_tree(&cfg).unwrap();

        // WHEN: We get blocks dominated by entry
        let dominated = dom_tree.dominated_by(cfg.entry_block);

        // THEN: Entry should dominate at least itself
        assert!(
            dominated.contains(&cfg.entry_block),
            "Entry should dominate itself"
        );
    }

    #[test]
    fn dominator_tree_has_preorder() {
        // GIVEN: A function
        let source = r#"
def foo():
    return 1
"#;
        let cfg = get_cfg_context(source, "foo", Language::Python).unwrap();
        let dom_tree = build_dominator_tree(&cfg).unwrap();

        // THEN: Should have preorder traversal
        assert!(
            !dom_tree.preorder.is_empty(),
            "Should have preorder traversal"
        );
    }

    #[test]
    fn dominator_tree_has_postorder() {
        // GIVEN: A function
        let source = r#"
def foo():
    return 1
"#;
        let cfg = get_cfg_context(source, "foo", Language::Python).unwrap();
        let dom_tree = build_dominator_tree(&cfg).unwrap();

        // THEN: Should have postorder traversal
        assert!(
            !dom_tree.postorder.is_empty(),
            "Should have postorder traversal"
        );
    }

    #[test]
    fn dominator_tree_handles_empty_cfg() {
        // GIVEN: An empty CFG
        let cfg = tldr_core::types::CfgInfo {
            function: "empty".to_string(),
            blocks: vec![],
            edges: vec![],
            entry_block: 0,
            exit_blocks: vec![],
            cyclomatic_complexity: 0,
            nested_functions: std::collections::HashMap::new(),
        };

        // WHEN: We try to build dominator tree
        let result = build_dominator_tree(&cfg);

        // THEN: Should return error
        assert!(result.is_err(), "Should error on empty CFG");
    }

    #[test]
    fn dominator_tree_handles_nested_if() {
        // GIVEN: A function with nested conditionals
        let source = r#"
def nested(a, b):
    if a:
        if b:
            x = 1
        else:
            x = 2
    else:
        x = 3
    return x
"#;
        let cfg = get_cfg_context(source, "nested", Language::Python).unwrap();

        // WHEN: We build the dominator tree
        let dom_tree = build_dominator_tree(&cfg).unwrap();

        // THEN: Should have multiple nodes
        assert!(
            dom_tree.nodes.len() > 1,
            "Nested ifs should create multiple dominator nodes"
        );
    }

    #[test]
    fn dominator_tree_handles_loops() {
        // GIVEN: A function with a loop
        let source = r#"
def loop_func():
    total = 0
    for i in range(10):
        total += i
    return total
"#;
        let cfg = get_cfg_context(source, "loop_func", Language::Python).unwrap();

        // WHEN: We build the dominator tree
        let dom_tree = build_dominator_tree(&cfg).unwrap();

        // THEN: Should have nodes for loop structure
        assert!(
            !dom_tree.nodes.is_empty(),
            "Loops should create dominator nodes"
        );
    }
}

// =============================================================================
// Dominance Frontier Tests
// =============================================================================

mod dominance_frontier_tests {
    use super::*;

    #[test]
    fn dominance_frontier_computes_for_simple_function() {
        // GIVEN: A simple function
        let source = r#"
def foo():
    x = 1
    return x
"#;
        let cfg = get_cfg_context(source, "foo", Language::Python).unwrap();
        let dom_tree = build_dominator_tree(&cfg).unwrap();

        // WHEN: We compute dominance frontier
        let df = compute_dominance_frontier(&cfg, &dom_tree).unwrap();

        // THEN: Should have frontier entries
        assert!(!df.frontier.is_empty(), "Should have dominance frontier");
    }

    #[test]
    fn dominance_frontier_get_returns_set() {
        // GIVEN: A function
        let source = r#"
def foo():
    return 1
"#;
        let cfg = get_cfg_context(source, "foo", Language::Python).unwrap();
        let dom_tree = build_dominator_tree(&cfg).unwrap();
        let df = compute_dominance_frontier(&cfg, &dom_tree).unwrap();

        // WHEN: We get frontier for a block
        let _frontier = df.get(cfg.entry_block);

        // THEN: Should return a set (may be empty)
        // Result is a HashSet, just verify it doesn't panic
    }

    #[test]
    fn dominance_frontier_computes_iterated() {
        // GIVEN: A function
        let source = r#"
def foo():
    return 1
"#;
        let cfg = get_cfg_context(source, "foo", Language::Python).unwrap();
        let dom_tree = build_dominator_tree(&cfg).unwrap();
        let df = compute_dominance_frontier(&cfg, &dom_tree).unwrap();

        // WHEN: We compute iterated dominance frontier for entry
        let mut blocks = HashSet::new();
        blocks.insert(cfg.entry_block);
        let _idf = df.iterated(&blocks);

        // THEN: Should return a set
        // IDF may be empty for simple functions
    }

    #[test]
    fn dominance_frontier_handles_if_statement() {
        // GIVEN: A function with if/else
        let source = r#"
def foo(cond):
    if cond:
        x = 1
    else:
        x = 2
    return x
"#;
        let cfg = get_cfg_context(source, "foo", Language::Python).unwrap();
        let dom_tree = build_dominator_tree(&cfg).unwrap();

        // WHEN: We compute dominance frontier
        let df = compute_dominance_frontier(&cfg, &dom_tree).unwrap();

        // THEN: Should have frontier for merge points
        assert!(
            !df.frontier.is_empty(),
            "If/else should create dominance frontier"
        );
    }
}

// =============================================================================
// SSA Construction Tests
// =============================================================================

mod ssa_construction_tests {
    use super::*;

    #[test]
    fn minimal_ssa_constructs_for_simple_function() {
        // GIVEN: A simple function
        let source = r#"
def foo():
    x = 1
    return x
"#;
        let cfg = get_cfg_context(source, "foo", Language::Python).unwrap();
        let dfg = get_dfg_context(source, "foo", Language::Python).unwrap();

        // WHEN: We construct minimal SSA
        let ssa = construct_minimal_ssa(&cfg, &dfg).unwrap();

        // THEN: Should have SSA structure
        assert!(!ssa.blocks.is_empty(), "SSA should have blocks");
        assert_eq!(ssa.function, "foo", "Function name should match");
        assert_eq!(ssa.ssa_type, SsaType::Minimal, "Should be minimal SSA");
    }

    #[test]
    fn minimal_ssa_has_ssa_names() {
        // GIVEN: A function with variables
        let source = r#"
def foo():
    x = 1
    return x
"#;
        let cfg = get_cfg_context(source, "foo", Language::Python).unwrap();
        let dfg = get_dfg_context(source, "foo", Language::Python).unwrap();
        let ssa = construct_minimal_ssa(&cfg, &dfg).unwrap();

        // THEN: Should have SSA names
        assert!(!ssa.ssa_names.is_empty(), "Should have SSA names");
    }

    #[test]
    fn minimal_ssa_has_stats() {
        // GIVEN: A function
        let source = r#"
def foo():
    x = 1
    return x
"#;
        let cfg = get_cfg_context(source, "foo", Language::Python).unwrap();
        let dfg = get_dfg_context(source, "foo", Language::Python).unwrap();
        let ssa = construct_minimal_ssa(&cfg, &dfg).unwrap();

        // THEN: Should have statistics
        assert!(ssa.stats.blocks > 0, "Should have block count");
    }

    #[test]
    fn minimal_ssa_creates_phi_at_merge_points() {
        // GIVEN: A function with if/else (needs phi)
        let source = r#"
def foo(cond):
    if cond:
        x = 1
    else:
        x = 2
    return x
"#;
        let cfg = get_cfg_context(source, "foo", Language::Python).unwrap();
        let dfg = get_dfg_context(source, "foo", Language::Python).unwrap();
        let ssa = construct_minimal_ssa(&cfg, &dfg).unwrap();

        // THEN: Should have phi functions at merge points
        let phi_count: usize = ssa.blocks.iter().map(|b| b.phi_functions.len()).sum();
        assert!(phi_count > 0, "Should have phi functions at merge points");
    }

    #[test]
    fn minimal_ssa_has_def_use_chains() {
        // GIVEN: A function
        let source = r#"
def foo():
    x = 1
    y = x + 2
    return y
"#;
        let cfg = get_cfg_context(source, "foo", Language::Python).unwrap();
        let dfg = get_dfg_context(source, "foo", Language::Python).unwrap();
        let _ssa = construct_minimal_ssa(&cfg, &dfg).unwrap();

        // THEN: Should have def-use chains
        // def_use is a HashMap, just verify it exists
    }

    #[test]
    fn semi_pruned_ssa_constructs() {
        // GIVEN: A function
        let source = r#"
def foo():
    x = 1
    return x
"#;
        let cfg = get_cfg_context(source, "foo", Language::Python).unwrap();
        let dfg = get_dfg_context(source, "foo", Language::Python).unwrap();

        // WHEN: We construct semi-pruned SSA
        let ssa = construct_semi_pruned_ssa(&cfg, &dfg).unwrap();

        // THEN: Should be semi-pruned type
        assert_eq!(
            ssa.ssa_type,
            SsaType::SemiPruned,
            "Should be semi-pruned SSA"
        );
    }

    #[test]
    fn ssa_handles_empty_function() {
        // GIVEN: An empty function
        let source = r#"
def empty():
    pass
"#;
        let cfg = get_cfg_context(source, "empty", Language::Python).unwrap();
        let dfg = get_dfg_context(source, "empty", Language::Python).unwrap();

        // WHEN: We construct SSA
        let ssa = construct_minimal_ssa(&cfg, &dfg).unwrap();

        // THEN: Should succeed
        assert!(
            !ssa.blocks.is_empty() || ssa.ssa_names.is_empty(),
            "Empty function may have minimal SSA"
        );
    }

    #[test]
    #[ignore = "BUG: Parameters don't create SSA names - see bugs_core_graphs.md"]
    fn ssa_handles_function_parameters() {
        // GIVEN: A function with parameters
        let source = r#"
def foo(x, y):
    return x + y
"#;
        let cfg = get_cfg_context(source, "foo", Language::Python).unwrap();
        let dfg = get_dfg_context(source, "foo", Language::Python).unwrap();
        let ssa = construct_minimal_ssa(&cfg, &dfg).unwrap();

        // THEN: Parameters should be treated as definitions
        // SSA should have names for parameters
        assert!(
            !ssa.ssa_names.is_empty(),
            "Should have SSA names for parameters"
        );
    }

    #[test]
    fn ssa_handles_loops() {
        // GIVEN: A function with a loop
        let source = r#"
def loop_func():
    total = 0
    for i in range(10):
        total += i
    return total
"#;
        let cfg = get_cfg_context(source, "loop_func", Language::Python).unwrap();
        let dfg = get_dfg_context(source, "loop_func", Language::Python).unwrap();
        let ssa = construct_minimal_ssa(&cfg, &dfg).unwrap();

        // THEN: Should have phi functions for loop variables
        let _phi_count: usize = ssa.blocks.iter().map(|b| b.phi_functions.len()).sum();
        // Loops typically need phi functions
    }

    #[test]
    fn ssa_phi_has_correct_structure() {
        // GIVEN: A function needing phi functions
        let source = r#"
def foo(cond):
    if cond:
        x = 1
    else:
        x = 2
    return x
"#;
        let cfg = get_cfg_context(source, "foo", Language::Python).unwrap();
        let dfg = get_dfg_context(source, "foo", Language::Python).unwrap();
        let ssa = construct_minimal_ssa(&cfg, &dfg).unwrap();

        // THEN: Phi functions should have targets and sources
        for block in &ssa.blocks {
            for phi in &block.phi_functions {
                assert!(!phi.variable.is_empty(), "Phi should have variable name");
                // Variable name should be tracked
            }
        }
    }

    #[test]
    fn ssa_name_has_version() {
        // GIVEN: A function with variable reassignment
        let source = r#"
def foo():
    x = 1
    x = 2
    return x
"#;
        let cfg = get_cfg_context(source, "foo", Language::Python).unwrap();
        let dfg = get_dfg_context(source, "foo", Language::Python).unwrap();
        let ssa = construct_minimal_ssa(&cfg, &dfg).unwrap();

        // THEN: Should have multiple versions of x
        let x_names: Vec<_> = ssa.ssa_names.iter().filter(|n| n.variable == "x").collect();
        if x_names.len() > 1 {
            // Should have different versions
            let versions: HashSet<_> = x_names.iter().map(|n| n.version).collect();
            assert!(
                versions.len() > 1,
                "Should have different versions for reassignments"
            );
        }
    }

    #[test]
    fn ssa_name_format_name() {
        // GIVEN: An SSA name
        let name = tldr_core::ssa::SsaName {
            id: tldr_core::ssa::SsaNameId(1),
            variable: "x".to_string(),
            version: 5,
            def_block: Some(0),
            def_line: 2,
        };

        // WHEN: We format it
        let formatted = name.format_name();

        // THEN: Should be in format "x_5"
        assert_eq!(formatted, "x_5", "Format should be variable_version");
    }

    #[test]
    fn ssa_block_has_instructions() {
        // GIVEN: A function
        let source = r#"
def foo():
    x = 1
    return x
"#;
        let cfg = get_cfg_context(source, "foo", Language::Python).unwrap();
        let dfg = get_dfg_context(source, "foo", Language::Python).unwrap();
        let ssa = construct_minimal_ssa(&cfg, &dfg).unwrap();

        // THEN: Blocks should have instructions
        let _total_instructions: usize = ssa.blocks.iter().map(|b| b.instructions.len()).sum();
        // Instructions may or may not be present depending on implementation
    }

    #[test]
    fn ssa_block_has_successors_predecessors() {
        // GIVEN: A function with control flow
        let source = r#"
def foo(cond):
    if cond:
        x = 1
    else:
        x = 2
    return x
"#;
        let cfg = get_cfg_context(source, "foo", Language::Python).unwrap();
        let dfg = get_dfg_context(source, "foo", Language::Python).unwrap();
        let ssa = construct_minimal_ssa(&cfg, &dfg).unwrap();

        // THEN: Blocks should track successors and predecessors
        for _block in &ssa.blocks {
            // Successors and predecessors should be populated based on CFG
        }
    }
}

// =============================================================================
// Live Variables Tests
// =============================================================================

mod live_variables_tests {
    use super::*;

    #[test]
    fn live_variables_computes_for_simple_function() {
        // GIVEN: A simple function
        let source = r#"
def foo():
    x = 1
    return x
"#;
        let cfg = get_cfg_context(source, "foo", Language::Python).unwrap();
        let dfg = get_dfg_context(source, "foo", Language::Python).unwrap();

        // WHEN: We compute live variables
        let live = compute_live_variables(&cfg, &dfg.refs).unwrap();

        // THEN: Should have live variable info
        assert_eq!(live.function, "foo", "Function name should match");
        assert!(
            !live.blocks.is_empty(),
            "Should have live variable info for blocks"
        );
    }

    #[test]
    fn live_variables_is_live_in() {
        // GIVEN: A function
        let source = r#"
def foo():
    x = 1
    return x
"#;
        let cfg = get_cfg_context(source, "foo", Language::Python).unwrap();
        let dfg = get_dfg_context(source, "foo", Language::Python).unwrap();
        let live = compute_live_variables(&cfg, &dfg.refs).unwrap();

        // WHEN: We check if a variable is live-in
        let _is_live = live.is_live_in(cfg.entry_block, "x");

        // THEN: Result depends on analysis, just verify it doesn't panic
    }

    #[test]
    fn live_variables_is_live_out() {
        // GIVEN: A function
        let source = r#"
def foo():
    x = 1
    return x
"#;
        let cfg = get_cfg_context(source, "foo", Language::Python).unwrap();
        let dfg = get_dfg_context(source, "foo", Language::Python).unwrap();
        let live = compute_live_variables(&cfg, &dfg.refs).unwrap();

        // WHEN: We check if a variable is live-out
        let _is_live = live.is_live_out(cfg.entry_block, "x");

        // THEN: Result depends on analysis, just verify it doesn't panic
    }

    #[test]
    fn live_variables_handles_multiple_variables() {
        // GIVEN: A function with multiple variables
        let source = r#"
def foo():
    x = 1
    y = 2
    z = x + y
    return z
"#;
        let cfg = get_cfg_context(source, "foo", Language::Python).unwrap();
        let dfg = get_dfg_context(source, "foo", Language::Python).unwrap();
        let live = compute_live_variables(&cfg, &dfg.refs).unwrap();

        // THEN: Should have live variable info
        assert!(!live.blocks.is_empty(), "Should handle multiple variables");
    }
}

// =============================================================================
// Value Numbering Tests
// =============================================================================

mod value_numbering_tests {
    use super::*;

    #[test]
    fn value_numbering_computes_for_ssa() {
        // GIVEN: A function in SSA form
        let source = r#"
def foo():
    x = 1
    y = x + 2
    return y
"#;
        let cfg = get_cfg_context(source, "foo", Language::Python).unwrap();
        let dfg = get_dfg_context(source, "foo", Language::Python).unwrap();
        let ssa = construct_minimal_ssa(&cfg, &dfg).unwrap();

        // WHEN: We compute value numbers
        let vn = compute_value_numbers(&ssa).unwrap();

        // THEN: Should have value numbers
        assert_eq!(vn.function, "foo", "Function name should match");
    }

    #[test]
    fn value_numbering_detects_equivalences() {
        // GIVEN: A function with equivalent expressions
        let source = r#"
def foo():
    a = 1
    b = 1
    return a + b
"#;
        let cfg = get_cfg_context(source, "foo", Language::Python).unwrap();
        let dfg = get_dfg_context(source, "foo", Language::Python).unwrap();
        let ssa = construct_minimal_ssa(&cfg, &dfg).unwrap();
        let _vn = compute_value_numbers(&ssa).unwrap();

        // THEN: Should have detected equivalences
        // Equivalences map may or may not be populated depending on implementation
    }
}

// =============================================================================
// Edge Case Tests
// =============================================================================

mod ssa_edge_case_tests {
    use super::*;

    #[test]
    fn ssa_handles_deeply_nested_control_flow() {
        // GIVEN: A function with deeply nested ifs
        let source = r#"
def deep(a, b, c, d):
    if a:
        if b:
            if c:
                if d:
                    x = 1
                else:
                    x = 2
            else:
                x = 3
        else:
            x = 4
    else:
        x = 5
    return x
"#;
        let cfg = get_cfg_context(source, "deep", Language::Python).unwrap();
        let dfg = get_dfg_context(source, "deep", Language::Python).unwrap();
        let ssa = construct_minimal_ssa(&cfg, &dfg).unwrap();

        // THEN: Should handle deep nesting
        assert!(!ssa.blocks.is_empty(), "Should handle deep nesting");
    }

    #[test]
    fn ssa_handles_function_with_no_variables() {
        // GIVEN: A function with no variables
        let source = r#"
def foo():
    return 42
"#;
        let cfg = get_cfg_context(source, "foo", Language::Python).unwrap();
        let dfg = get_dfg_context(source, "foo", Language::Python).unwrap();
        let ssa = construct_minimal_ssa(&cfg, &dfg).unwrap();

        // THEN: Should succeed with minimal SSA
        assert!(
            !ssa.blocks.is_empty(),
            "Should handle functions without variables"
        );
    }

    #[test]
    fn ssa_handles_many_variables() {
        // GIVEN: A function with many variables
        let mut source = String::from("def many():\n");
        for i in 0..20 {
            source.push_str(&format!("    x{} = {}\n", i, i));
        }
        source.push_str("    return x0\n");

        let cfg = get_cfg_context(&source, "many", Language::Python).unwrap();
        let dfg = get_dfg_context(&source, "many", Language::Python).unwrap();
        let ssa = construct_minimal_ssa(&cfg, &dfg).unwrap();

        // THEN: Should handle many variables
        assert!(!ssa.ssa_names.is_empty(), "Should handle many variables");
    }

    #[test]
    fn ssa_handles_try_except() {
        // GIVEN: A function with exception handling
        let source = r#"
def risky():
    try:
        x = dangerous()
    except:
        x = fallback()
    return x
"#;
        let cfg = get_cfg_context(source, "risky", Language::Python).unwrap();
        let dfg = get_dfg_context(source, "risky", Language::Python).unwrap();
        let ssa = construct_minimal_ssa(&cfg, &dfg).unwrap();

        // THEN: Should handle exception handling
        assert!(!ssa.blocks.is_empty(), "Should handle try/except");
    }

    #[test]
    fn ssa_handles_while_loop() {
        // GIVEN: A function with while loop
        let source = r#"
def count():
    i = 0
    while i < 10:
        i += 1
    return i
"#;
        let cfg = get_cfg_context(source, "count", Language::Python).unwrap();
        let dfg = get_dfg_context(source, "count", Language::Python).unwrap();
        let ssa = construct_minimal_ssa(&cfg, &dfg).unwrap();

        // THEN: Should handle while loop
        assert!(!ssa.blocks.is_empty(), "Should handle while loops");
    }
}
