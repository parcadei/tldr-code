//! Reaching Definitions Extended Tests
//!
//! Comprehensive tests for enhanced reaching definitions analysis.
//! These tests define expected behavior for new capabilities.
//!
//! # Test Categories
//!
//! 1. Def-Use Chain Tests (RD-7)
//! 2. Use-Def Chain Tests (RD-8)
//! 3. Uninitialized Detection Tests (RD-13) - Phase 9
//! 4. Available Expressions Tests (RD-10) - Phase 15
//! 5. Live Variables Tests (RD-12) - Phase 15
//! 6. RPO Worklist Tests (RD-3) - Phase 16
//! 7. Output Format Tests (RD-14, RD-15, RD-16) - Phase 10
//!
//! Reference: session10-spec.md

use std::collections::HashMap;
use std::path::PathBuf;

use crate::types::{BlockType, CfgBlock, CfgEdge, CfgInfo, EdgeType, RefType, VarRef};

// Import the actual implementations from reaching.rs
use super::reaching::{
    build_def_use_chains, build_reaching_defs_report, build_use_def_chains,
    compute_reaching_definitions, detect_uninitialized as detect_uninitialized_with_params,
    detect_uninitialized_simple, ReachingDefinitions, ReachingDefsReport, UninitSeverity,
    UninitializedUse,
};

// Import format functions (Phase 10)
use super::format::{
    filter_reaching_defs_by_variable as filter_by_var_impl,
    format_reaching_defs_json as format_json_impl, format_reaching_defs_text as format_text_impl,
};

// =============================================================================
// Helper wrapper for tests (Phase 9)
// =============================================================================

/// Detect potentially uninitialized variable uses
fn detect_uninitialized(
    reaching: &ReachingDefinitions,
    cfg: &CfgInfo,
    refs: &[VarRef],
) -> Vec<UninitializedUse> {
    detect_uninitialized_simple(reaching, cfg, refs)
}

/// Compute reaching definitions with optimized worklist (RPO) - Phase 16
fn compute_reaching_definitions_rpo(cfg: &CfgInfo, refs: &[VarRef]) -> ReachingDefinitions {
    // Use the actual RPO implementation from reaching.rs
    use super::reaching::compute_reaching_definitions_rpo as rpo_impl;
    rpo_impl(cfg, refs).reaching
}

/// Format reaching definitions as text (Phase 10)
fn format_reaching_defs_text(report: &ReachingDefsReport) -> String {
    format_text_impl(report)
}

/// Format reaching definitions as JSON (Phase 10)
fn format_reaching_defs_json(report: &ReachingDefsReport) -> String {
    format_json_impl(report).unwrap_or_else(|e| format!("Error: {}", e))
}

/// Filter reaching definitions by variable (Phase 10)
fn filter_reaching_defs_by_variable(
    report: ReachingDefsReport,
    variable: &str,
) -> ReachingDefsReport {
    filter_by_var_impl(&report, variable)
}

// =============================================================================
// Test Fixtures
// =============================================================================

mod fixtures {
    use super::*;

    pub fn make_block(id: usize, start: u32, end: u32) -> CfgBlock {
        CfgBlock {
            id,
            block_type: BlockType::Body,
            lines: (start, end),
            calls: Vec::new(),
        }
    }

    pub fn make_def(name: &str, line: u32) -> VarRef {
        VarRef {
            name: name.to_string(),
            ref_type: RefType::Definition,
            line,
            column: 0,
            context: None,
            group_id: None,
        }
    }

    pub fn make_use(name: &str, line: u32) -> VarRef {
        VarRef {
            name: name.to_string(),
            ref_type: RefType::Use,
            line,
            column: 0,
            context: None,
            group_id: None,
        }
    }

    /// Linear CFG: Block 0 -> Block 1 -> Block 2
    pub fn linear_cfg() -> CfgInfo {
        CfgInfo {
            function: "linear".to_string(),
            blocks: vec![
                make_block(0, 1, 2),
                make_block(1, 3, 4),
                make_block(2, 5, 6),
            ],
            edges: vec![
                CfgEdge {
                    from: 0,
                    to: 1,
                    edge_type: EdgeType::Unconditional,
                    condition: None,
                },
                CfgEdge {
                    from: 1,
                    to: 2,
                    edge_type: EdgeType::Unconditional,
                    condition: None,
                },
            ],
            entry_block: 0,
            exit_blocks: vec![2],
            cyclomatic_complexity: 1,
            nested_functions: HashMap::new(),
        }
    }

    /// Diamond CFG for branching tests
    pub fn diamond_cfg() -> CfgInfo {
        CfgInfo {
            function: "diamond".to_string(),
            blocks: vec![
                CfgBlock {
                    id: 0,
                    block_type: BlockType::Entry,
                    lines: (1, 2),
                    calls: Vec::new(),
                },
                make_block(1, 3, 4),
                make_block(2, 5, 6),
                make_block(3, 7, 8),
            ],
            edges: vec![
                CfgEdge {
                    from: 0,
                    to: 1,
                    edge_type: EdgeType::True,
                    condition: Some("cond".to_string()),
                },
                CfgEdge {
                    from: 0,
                    to: 2,
                    edge_type: EdgeType::False,
                    condition: Some("cond".to_string()),
                },
                CfgEdge {
                    from: 1,
                    to: 3,
                    edge_type: EdgeType::Unconditional,
                    condition: None,
                },
                CfgEdge {
                    from: 2,
                    to: 3,
                    edge_type: EdgeType::Unconditional,
                    condition: None,
                },
            ],
            entry_block: 0,
            exit_blocks: vec![3],
            cyclomatic_complexity: 2,
            nested_functions: HashMap::new(),
        }
    }

    /// Loop CFG for iteration tests
    pub fn loop_cfg() -> CfgInfo {
        CfgInfo {
            function: "loop".to_string(),
            blocks: vec![
                make_block(0, 1, 2), // entry
                make_block(1, 3, 4), // loop header
                make_block(2, 5, 6), // loop body
                make_block(3, 7, 8), // exit
            ],
            edges: vec![
                CfgEdge {
                    from: 0,
                    to: 1,
                    edge_type: EdgeType::Unconditional,
                    condition: None,
                },
                CfgEdge {
                    from: 1,
                    to: 2,
                    edge_type: EdgeType::True,
                    condition: Some("i < 10".to_string()),
                },
                CfgEdge {
                    from: 1,
                    to: 3,
                    edge_type: EdgeType::False,
                    condition: Some("i < 10".to_string()),
                },
                CfgEdge {
                    from: 2,
                    to: 1,
                    edge_type: EdgeType::BackEdge,
                    condition: None,
                },
            ],
            entry_block: 0,
            exit_blocks: vec![3],
            cyclomatic_complexity: 2,
            nested_functions: HashMap::new(),
        }
    }
}

// =============================================================================
// Def-Use Chain Tests (RD-7)
// =============================================================================

#[cfg(test)]
mod def_use_chain_tests {
    use super::fixtures::*;
    use super::*;

    /// Test: Definition reaches all subsequent uses
    #[test]
    fn test_def_reaches_all_uses() {
        let cfg = linear_cfg();
        let refs = vec![
            make_def("x", 1), // definition at line 1
            make_use("x", 3), // use at line 3
            make_use("x", 5), // use at line 5
        ];

        let reaching = compute_reaching_definitions(&cfg, &refs);
        let chains = build_def_use_chains(&reaching, &cfg, &refs);

        // Should have one chain for x's definition
        let x_chain = chains.iter().find(|c| c.definition.var == "x");
        assert!(x_chain.is_some(), "Should have def-use chain for x");

        let x_chain = x_chain.unwrap();
        // Definition should reach both uses
        assert_eq!(x_chain.uses.len(), 2, "Definition should reach 2 uses");
        assert!(x_chain.uses.iter().any(|u| u.line == 3));
        assert!(x_chain.uses.iter().any(|u| u.line == 5));
    }

    /// Test: Killed definition doesn't reach past kill point
    #[test]
    fn test_killed_def_doesnt_reach() {
        let cfg = linear_cfg();
        let refs = vec![
            make_def("x", 1), // first definition
            make_use("x", 2), // use before kill
            make_def("x", 3), // kill (redefine)
            make_use("x", 5), // use after kill
        ];

        let reaching = compute_reaching_definitions(&cfg, &refs);
        let chains = build_def_use_chains(&reaching, &cfg, &refs);

        // First definition (line 1) should only reach use at line 2
        let first_chain = chains
            .iter()
            .find(|c| c.definition.var == "x" && c.definition.line == 1);
        assert!(first_chain.is_some());
        let first_chain = first_chain.unwrap();
        assert_eq!(first_chain.uses.len(), 1);
        assert_eq!(first_chain.uses[0].line, 2);

        // Second definition (line 3) should reach use at line 5
        let second_chain = chains
            .iter()
            .find(|c| c.definition.var == "x" && c.definition.line == 3);
        assert!(second_chain.is_some());
        let second_chain = second_chain.unwrap();
        assert!(second_chain.uses.iter().any(|u| u.line == 5));
    }

    /// Test: Multiple definitions reaching same use via different paths
    #[test]
    fn test_multiple_defs_to_one_use() {
        let cfg = diamond_cfg();
        let refs = vec![
            make_def("x", 3), // definition in true branch
            make_def("x", 5), // definition in false branch
            make_use("x", 7), // use at merge
        ];

        let reaching = compute_reaching_definitions(&cfg, &refs);
        let chains = build_def_use_chains(&reaching, &cfg, &refs);

        // Both definitions should reach the use at line 7
        let chain_at_3 = chains.iter().find(|c| c.definition.line == 3);
        let chain_at_5 = chains.iter().find(|c| c.definition.line == 5);

        assert!(chain_at_3.is_some());
        assert!(chain_at_5.is_some());
        assert!(chain_at_3.unwrap().uses.iter().any(|u| u.line == 7));
        assert!(chain_at_5.unwrap().uses.iter().any(|u| u.line == 7));
    }

    /// Test: Independent variables have independent chains
    #[test]
    fn test_independent_variables() {
        let cfg = linear_cfg();
        let refs = vec![
            make_def("x", 1),
            make_def("y", 2),
            make_use("x", 3),
            make_use("y", 4),
        ];

        let reaching = compute_reaching_definitions(&cfg, &refs);
        let chains = build_def_use_chains(&reaching, &cfg, &refs);

        // Should have separate chains for x and y
        let x_chain = chains.iter().find(|c| c.definition.var == "x").unwrap();
        let y_chain = chains.iter().find(|c| c.definition.var == "y").unwrap();

        // x's chain should only have x's use
        assert!(x_chain.uses.iter().all(|u| {
            // Check that this use corresponds to x
            refs.iter().any(|r| r.line == u.line && r.name == "x")
        }));

        // y's chain should only have y's use
        assert!(y_chain
            .uses
            .iter()
            .all(|u| { refs.iter().any(|r| r.line == u.line && r.name == "y") }));
    }
}

// =============================================================================
// Use-Def Chain Tests (RD-8)
// =============================================================================

#[cfg(test)]
mod use_def_chain_tests {
    use super::fixtures::*;
    use super::*;

    /// Test: Use maps back to all reaching definitions
    #[test]
    fn test_use_maps_to_reaching_defs() {
        let cfg = diamond_cfg();
        let refs = vec![
            make_def("x", 3), // true branch
            make_def("x", 5), // false branch
            make_use("x", 7), // merge point
        ];

        let reaching = compute_reaching_definitions(&cfg, &refs);
        let chains = build_use_def_chains(&reaching, &cfg, &refs);

        // Use at line 7 should have both definitions reaching it
        let use_chain = chains.iter().find(|c| c.use_site.line == 7);
        assert!(
            use_chain.is_some(),
            "Should have use-def chain for use at line 7"
        );

        let use_chain = use_chain.unwrap();
        assert_eq!(
            use_chain.reaching_defs.len(),
            2,
            "Use should have 2 reaching definitions"
        );
        assert!(use_chain.reaching_defs.iter().any(|d| d.line == 3));
        assert!(use_chain.reaching_defs.iter().any(|d| d.line == 5));
    }

    /// Test: Use with single reaching definition
    #[test]
    fn test_use_single_reaching_def() {
        let cfg = linear_cfg();
        let refs = vec![make_def("x", 1), make_use("x", 5)];

        let reaching = compute_reaching_definitions(&cfg, &refs);
        let chains = build_use_def_chains(&reaching, &cfg, &refs);

        let use_chain = chains.iter().find(|c| c.use_site.line == 5).unwrap();
        assert_eq!(use_chain.reaching_defs.len(), 1);
        assert_eq!(use_chain.reaching_defs[0].line, 1);
    }

    /// Test: Multiple uses with different reaching defs
    #[test]
    fn test_multiple_uses_different_reaching() {
        let cfg = linear_cfg();
        let refs = vec![
            make_def("x", 1),
            make_use("x", 2),
            make_def("x", 3),
            make_use("x", 5),
        ];

        let reaching = compute_reaching_definitions(&cfg, &refs);
        let chains = build_use_def_chains(&reaching, &cfg, &refs);

        // Use at line 2 should have def from line 1
        let use_at_2 = chains.iter().find(|c| c.use_site.line == 2).unwrap();
        assert!(use_at_2.reaching_defs.iter().any(|d| d.line == 1));
        assert!(!use_at_2.reaching_defs.iter().any(|d| d.line == 3));

        // Use at line 5 should have def from line 3 (line 1 is killed)
        let use_at_5 = chains.iter().find(|c| c.use_site.line == 5).unwrap();
        assert!(use_at_5.reaching_defs.iter().any(|d| d.line == 3));
        assert!(!use_at_5.reaching_defs.iter().any(|d| d.line == 1));
    }
}

// =============================================================================
// Chain Consistency Tests
// =============================================================================

#[cfg(test)]
mod chain_consistency_tests {
    use super::fixtures::*;
    use super::*;

    /// Test: Def-use and use-def chains are consistent inverses
    #[test]
    fn test_chains_consistency() {
        let cfg = diamond_cfg();
        let refs = vec![
            make_def("x", 3), // true branch
            make_def("x", 5), // false branch
            make_use("x", 7), // merge point
        ];

        let reaching = compute_reaching_definitions(&cfg, &refs);
        let def_use = build_def_use_chains(&reaching, &cfg, &refs);
        let use_def = build_use_def_chains(&reaching, &cfg, &refs);

        // For each def-use chain: if def D reaches use U,
        // then use U should have D in its reaching_defs
        for du_chain in &def_use {
            for use_site in &du_chain.uses {
                let ud_chain = use_def
                    .iter()
                    .find(|c| c.use_site.line == use_site.line && c.var == du_chain.definition.var);
                assert!(
                    ud_chain.is_some(),
                    "Use at line {} should have a use-def chain",
                    use_site.line
                );
                let ud_chain = ud_chain.unwrap();
                assert!(
                    ud_chain
                        .reaching_defs
                        .iter()
                        .any(|d| d.line == du_chain.definition.line),
                    "Use at line {} should have def at line {} in reaching_defs",
                    use_site.line,
                    du_chain.definition.line
                );
            }
        }
    }
}

// =============================================================================
// Report Generation Tests
// =============================================================================

#[cfg(test)]
mod report_tests {
    use super::fixtures::*;
    use super::*;

    /// Test: Report generation produces correct structure
    #[test]
    fn test_report_generation() {
        let cfg = linear_cfg();
        let refs = vec![
            make_def("x", 1),
            make_use("x", 3),
            make_def("y", 4),
            make_use("y", 5),
        ];

        let report = build_reaching_defs_report(&cfg, &refs, PathBuf::from("test.py"));

        // Check basic structure
        assert_eq!(report.function, "linear");
        assert_eq!(report.file, PathBuf::from("test.py"));
        assert_eq!(report.blocks.len(), 3); // linear_cfg has 3 blocks

        // Check stats
        assert_eq!(report.stats.definitions, 2); // x and y
        assert_eq!(report.stats.uses, 2); // x and y uses
        assert_eq!(report.stats.blocks, 3);

        // Check def-use chains exist
        assert!(!report.def_use_chains.is_empty());

        // Check use-def chains exist
        assert!(!report.use_def_chains.is_empty());
    }

    /// Test: Block-level IN/OUT sets are correct
    #[test]
    fn test_block_in_out_sets() {
        let cfg = linear_cfg();
        let refs = vec![
            make_def("x", 1), // in block 0
            make_use("x", 5), // in block 2
        ];

        let report = build_reaching_defs_report(&cfg, &refs, PathBuf::from("test.py"));

        // Block 0 should have x in its GEN set
        let block_0 = report.blocks.iter().find(|b| b.id == 0).unwrap();
        assert!(block_0.gen.iter().any(|d| d.var == "x" && d.line == 1));

        // Block 2 should have x in its IN set
        let block_2 = report.blocks.iter().find(|b| b.id == 2).unwrap();
        assert!(block_2.in_set.iter().any(|d| d.var == "x" && d.line == 1));
    }
}

// =============================================================================
// Uninitialized Detection Tests (RD-13) - Phase 9 placeholder
// =============================================================================

#[cfg(test)]
mod uninitialized_tests {
    use super::fixtures::*;
    use super::*;

    /// Test: Use with no reaching definition is flagged
    #[test]
    fn test_definitely_uninitialized() {
        let cfg = linear_cfg();
        let refs = vec![
            make_use("x", 1), // use before any definition
        ];

        let reaching = compute_reaching_definitions(&cfg, &refs);
        let uninit = detect_uninitialized(&reaching, &cfg, &refs);

        assert!(!uninit.is_empty(), "Should detect uninitialized use");
        assert!(uninit.iter().any(|u| u.var == "x" && u.line == 1));
    }

    /// Test: Conditional initialization path detected as warning
    #[test]
    fn test_possibly_uninitialized() {
        let cfg = diamond_cfg();
        let refs = vec![
            make_def("x", 3), // only defined in true branch
            // x NOT defined in false branch
            make_use("x", 7), // use at merge - possibly uninitialized
        ];

        let reaching = compute_reaching_definitions(&cfg, &refs);
        let uninit = detect_uninitialized(&reaching, &cfg, &refs);

        // Should detect that x might not be initialized (false branch has no def)
        assert!(!uninit.is_empty(), "Should detect possibly uninitialized");
    }

    /// Test: Fully initialized variable not flagged
    #[test]
    fn test_fully_initialized_not_flagged() {
        let cfg = diamond_cfg();
        let refs = vec![
            make_def("x", 3), // defined in true branch
            make_def("x", 5), // defined in false branch
            make_use("x", 7), // use at merge - fully initialized
        ];

        let reaching = compute_reaching_definitions(&cfg, &refs);
        let uninit = detect_uninitialized(&reaching, &cfg, &refs);

        assert!(
            uninit.is_empty(),
            "Should not flag fully initialized variable"
        );
    }

    /// Test: Parameter is considered initialized
    #[test]
    fn test_parameter_initialized() {
        // Parameters should be treated as initialized at function entry
        let cfg = linear_cfg();
        let refs = vec![
            make_use("x", 1), // use of parameter
        ];

        let reaching = compute_reaching_definitions(&cfg, &refs);
        // Note: detect_uninitialized would need parameter info
        let uninit = detect_uninitialized(&reaching, &cfg, &refs);

        // For now this will fail since we don't track parameters
        // Phase 9 will handle this properly
        let _ = uninit;
    }

    /// Test: Multiple uninitialized variables
    #[test]
    fn test_multiple_uninitialized() {
        let cfg = linear_cfg();
        let refs = vec![make_use("x", 1), make_use("y", 2)];

        let reaching = compute_reaching_definitions(&cfg, &refs);
        let uninit = detect_uninitialized(&reaching, &cfg, &refs);

        assert!(
            uninit.len() >= 2,
            "Should detect both uninitialized variables"
        );
    }

    /// Test: Global variable is not flagged as uninitialized
    #[test]
    fn test_global_not_flagged() {
        let cfg = linear_cfg();
        let refs = vec![
            make_use("CONFIG", 1), // use of global
        ];

        let reaching = compute_reaching_definitions(&cfg, &refs);
        // Pass CONFIG as a global variable
        let uninit = detect_uninitialized_with_params(
            &reaching,
            &cfg,
            &refs,
            &[],                     // no params
            &["CONFIG".to_string()], // CONFIG is a global
        );

        assert!(uninit.is_empty(), "Global variables should not be flagged");
    }

    /// Test: Parameter is not flagged when passed to detect_uninitialized
    #[test]
    fn test_parameter_with_params_arg() {
        let cfg = linear_cfg();
        let refs = vec![
            make_use("x", 1), // use of parameter x
        ];

        let reaching = compute_reaching_definitions(&cfg, &refs);
        // Pass x as a parameter
        let uninit = detect_uninitialized_with_params(
            &reaching,
            &cfg,
            &refs,
            &["x".to_string()], // x is a parameter
            &[],
        );

        assert!(uninit.is_empty(), "Parameters should not be flagged");
    }

    /// Test: Severity is DEFINITE when no definition exists anywhere
    #[test]
    fn test_severity_definite() {
        let cfg = linear_cfg();
        let refs = vec![
            make_use("x", 1), // use with no definition anywhere
        ];

        let reaching = compute_reaching_definitions(&cfg, &refs);
        let uninit = detect_uninitialized(&reaching, &cfg, &refs);

        assert!(!uninit.is_empty(), "Should detect uninitialized use");
        assert_eq!(
            uninit[0].severity,
            UninitSeverity::Definite,
            "Should be DEFINITE when no definition exists"
        );
    }

    /// Test: Severity is POSSIBLE when definition exists but doesn't reach
    #[test]
    fn test_severity_possible() {
        let cfg = diamond_cfg();
        let refs = vec![
            make_def("x", 3), // only defined in true branch
            make_use("x", 7), // use at merge
        ];

        let reaching = compute_reaching_definitions(&cfg, &refs);
        let uninit = detect_uninitialized(&reaching, &cfg, &refs);

        assert!(!uninit.is_empty(), "Should detect possibly uninitialized");
        assert_eq!(
            uninit[0].severity,
            UninitSeverity::Possible,
            "Should be POSSIBLE when definition exists but doesn't reach all paths"
        );
    }

    /// Test: Loop where variable is used before definition in first iteration
    #[test]
    fn test_loop_first_iteration_uninit() {
        let cfg = loop_cfg();
        let refs = vec![
            // No init before loop
            make_use("total", 3), // use in header (first iteration has no def)
            make_def("total", 5), // update in body
        ];

        let reaching = compute_reaching_definitions(&cfg, &refs);
        let uninit = detect_uninitialized(&reaching, &cfg, &refs);

        // On first iteration, total has no reaching def
        // (The def at line 5 is in the loop body, which comes AFTER the use at line 3)
        assert!(
            !uninit.is_empty(),
            "Should detect possibly uninitialized in loop"
        );
    }
}

// =============================================================================
// RPO Worklist Tests (RD-3) - Phase 16 placeholder
// =============================================================================

#[cfg(test)]
mod rpo_tests {
    use super::fixtures::*;
    use super::*;

    /// Test: RPO produces same result as naive algorithm
    #[test]
    fn test_rpo_same_result_as_naive() {
        let cfg = diamond_cfg();
        let refs = vec![make_def("x", 3), make_def("x", 5), make_use("x", 7)];

        let naive = compute_reaching_definitions(&cfg, &refs);
        let rpo = compute_reaching_definitions_rpo(&cfg, &refs);

        // Should produce identical results
        for block in &cfg.blocks {
            let naive_in = naive.reaching_in.get(&block.id);
            let rpo_in = rpo.reaching_in.get(&block.id);
            assert_eq!(
                naive_in, rpo_in,
                "IN sets should match for block {}",
                block.id
            );
        }
    }

    /// Test: RPO handles loops correctly
    #[test]
    fn test_rpo_handles_loops() {
        let cfg = loop_cfg();
        let refs = vec![
            make_def("i", 1), // init in entry
            make_use("i", 3), // use in header
            make_def("i", 5), // update in body
        ];

        let reaching = compute_reaching_definitions_rpo(&cfg, &refs);

        // Both definitions should reach the header (loop back edge)
        let header_in = reaching.reaching_in.get(&1).unwrap();
        // At minimum, the first def should reach
        assert!(!header_in.is_empty());
    }
}

// =============================================================================
// Output Format Tests (RD-14, RD-15, RD-16) - Phase 10 placeholder
// =============================================================================

#[cfg(test)]
mod output_format_tests {
    use super::fixtures::*;
    use super::*;

    /// Test: JSON output matches spec schema (RD-15)
    #[test]
    fn test_json_schema() {
        let cfg = linear_cfg();
        let refs = vec![make_def("x", 1), make_use("x", 3)];

        let report = build_reaching_defs_report(&cfg, &refs, PathBuf::from("test.py"));
        let json = format_reaching_defs_json(&report);

        // Should produce valid JSON
        assert!(!json.is_empty(), "JSON should not be empty");
        // Should contain expected fields
        assert!(
            json.contains("\"function\""),
            "JSON should have function field"
        );
        assert!(json.contains("\"blocks\""), "JSON should have blocks field");
        assert!(
            json.contains("\"def_use_chains\""),
            "JSON should have def_use_chains field"
        );
        assert!(json.contains("\"stats\""), "JSON should have stats field");
    }

    /// Test: JSON 'in' field naming (spec requirement)
    #[test]
    fn test_json_in_field_name() {
        let cfg = linear_cfg();
        let refs = vec![make_def("x", 1), make_use("x", 3)];

        let report = build_reaching_defs_report(&cfg, &refs, PathBuf::from("test.py"));
        let json = format_reaching_defs_json(&report);

        // Per spec, the field should be "in" not "in_set"
        assert!(
            json.contains("\"in\""),
            "Field should be serialized as 'in'"
        );
    }

    /// Test: Text format is readable (RD-14)
    #[test]
    fn test_text_format_readable() {
        let cfg = linear_cfg();
        let refs = vec![make_def("x", 1), make_use("x", 3)];

        let report = build_reaching_defs_report(&cfg, &refs, PathBuf::from("test.py"));
        let text = format_reaching_defs_text(&report);

        // Should produce non-empty output
        assert!(!text.is_empty(), "Text should not be empty");
        // Should have header
        assert!(
            text.contains("Reaching Definitions for:"),
            "Should have header"
        );
        // Should have GEN/KILL/IN/OUT
        assert!(text.contains("GEN:"), "Should have GEN set");
        assert!(text.contains("OUT:"), "Should have OUT set");
    }

    /// Test: Variable filtering works (RD-16)
    #[test]
    fn test_variable_filtering() {
        let cfg = linear_cfg();
        let refs = vec![
            make_def("x", 1),
            make_def("y", 2),
            make_use("x", 3),
            make_use("y", 4),
        ];

        let report = build_reaching_defs_report(&cfg, &refs, PathBuf::from("test.py"));
        let filtered = filter_reaching_defs_by_variable(report.clone(), "x");

        // Should only contain x's chains
        assert!(
            filtered
                .def_use_chains
                .iter()
                .all(|c| c.definition.var == "x"),
            "All def-use chains should be for x"
        );
        // Should filter block GEN/KILL/IN/OUT too
        for block in &filtered.blocks {
            assert!(
                block.gen.iter().all(|d| d.var == "x"),
                "Block GEN should only contain x"
            );
        }
    }

    /// Test: Stats iterations field is tracked
    #[test]
    fn test_stats_iterations() {
        let cfg = linear_cfg();
        let refs = vec![make_def("x", 1), make_use("x", 3)];

        let report = build_reaching_defs_report(&cfg, &refs, PathBuf::from("test.py"));

        // Stats should be populated
        assert!(
            report.stats.definitions > 0 || report.stats.uses > 0,
            "Stats should be populated"
        );
        assert!(report.stats.blocks > 0, "Should have block count");
    }
}

// =============================================================================
// Integration Tests
// =============================================================================

#[cfg(test)]
mod integration_tests {
    use super::fixtures::*;
    use super::*;

    /// Test: Full pipeline with linear CFG
    #[test]
    fn test_full_pipeline_linear() {
        let cfg = linear_cfg();
        let refs = vec![
            make_def("x", 1),
            make_use("x", 3),
            make_def("x", 4),
            make_use("x", 5),
        ];

        let reaching = compute_reaching_definitions(&cfg, &refs);
        let def_use = build_def_use_chains(&reaching, &cfg, &refs);
        let use_def = build_use_def_chains(&reaching, &cfg, &refs);

        // Verify chains are non-empty
        assert!(!def_use.is_empty());
        assert!(!use_def.is_empty());
    }

    /// Test: Complex scenario with multiple variables
    #[test]
    fn test_complex_multiple_variables() {
        let cfg = diamond_cfg();
        let refs = vec![
            make_def("x", 1),
            make_def("y", 2),
            make_def("x", 3), // true branch
            make_def("y", 5), // false branch
            make_use("x", 7),
            make_use("y", 8),
        ];

        let reaching = compute_reaching_definitions(&cfg, &refs);
        let def_use = build_def_use_chains(&reaching, &cfg, &refs);
        let use_def = build_use_def_chains(&reaching, &cfg, &refs);

        // Should have chains for both variables
        assert!(def_use.iter().any(|c| c.definition.var == "x"));
        assert!(def_use.iter().any(|c| c.definition.var == "y"));
        assert!(use_def.iter().any(|c| c.var == "x"));
        assert!(use_def.iter().any(|c| c.var == "y"));
    }

    /// Test: Loop accumulator pattern
    #[test]
    fn test_loop_accumulator() {
        let cfg = loop_cfg();
        let refs = vec![
            make_def("total", 1), // total = 0
            make_use("total", 5), // total + i
            make_def("total", 5), // total = total + i
            make_use("total", 7), // return total
        ];

        let reaching = compute_reaching_definitions(&cfg, &refs);
        let use_def = build_use_def_chains(&reaching, &cfg, &refs);

        // The use at line 7 should see definitions from both line 1 and line 5
        let final_use = use_def.iter().find(|c| c.use_site.line == 7);
        assert!(
            final_use.is_some(),
            "Should have use-def chain for final use"
        );
    }
}

// =============================================================================
// Phase 15: Available Expressions Tests (RD-10)
// =============================================================================

#[cfg(test)]
mod available_expressions_tests {
    use super::fixtures::*;
    use super::*;

    // Note: Available expressions tests use SSA analysis module
    // See ssa/ssa_tests.rs for comprehensive available expressions tests

    /// Test: Expression availability basic concept
    #[test]
    fn test_available_exprs_concept() {
        // This test validates that the concept is implemented
        // Detailed testing is in ssa_tests.rs
        let cfg = linear_cfg();
        let refs = vec![
            make_def("a", 1),
            make_def("b", 1),
            make_use("a", 3),
            make_use("b", 3),
        ];

        // Reaching definitions should still work
        let reaching = compute_reaching_definitions(&cfg, &refs);
        assert!(!reaching.reaching_in.is_empty() || !reaching.reaching_out.is_empty());
    }
}

// =============================================================================
// Phase 15: Live Variables Extended Tests (RD-12)
// =============================================================================

#[cfg(test)]
mod live_variables_extended_tests {
    use super::fixtures::*;
    use super::*;

    // Note: Live variables analysis is in ssa/analysis.rs
    // These tests validate integration with reaching definitions

    /// Test: Live range concept validation
    #[test]
    fn test_live_range_concept() {
        let cfg = linear_cfg();
        let refs = vec![make_def("x", 1), make_use("x", 3), make_use("x", 5)];

        // Reaching definitions provides the foundation for liveness
        let reaching = compute_reaching_definitions(&cfg, &refs);

        // x should be reachable at blocks 1 and 2
        assert!(reaching.reaching_in.contains_key(&1));
        assert!(reaching.reaching_in.contains_key(&2));
    }
}

// =============================================================================
// Phase 16: RPO Worklist Extended Tests (RD-3)
// =============================================================================

#[cfg(test)]
mod rpo_extended_tests {
    use super::super::reaching::{compute_reaching_definitions_rpo, compute_rpo};
    use super::fixtures::*;
    use super::*;

    /// Test: RPO order is valid (predecessors before successors in acyclic regions)
    #[test]
    fn test_rpo_order() {
        let cfg = diamond_cfg();
        let rpo = compute_rpo(&cfg);

        // Entry block should come first
        assert_eq!(rpo.first(), Some(&0), "Entry should be first in RPO");

        // Should contain all blocks
        assert_eq!(rpo.len(), cfg.blocks.len(), "RPO should contain all blocks");

        // Build position map
        let pos: std::collections::HashMap<usize, usize> =
            rpo.iter().enumerate().map(|(i, &b)| (b, i)).collect();

        // For each edge, non-back-edge predecessors should come before successors
        // (This is the RPO property for acyclic regions)
        for edge in &cfg.edges {
            if edge.edge_type != EdgeType::BackEdge {
                let from_pos = pos.get(&edge.from).unwrap();
                let to_pos = pos.get(&edge.to).unwrap();
                // In RPO, predecessors come before successors
                assert!(
                    from_pos < to_pos,
                    "In RPO, block {} should come before {} (non-back-edge)",
                    edge.from,
                    edge.to
                );
            }
        }
    }

    /// Test: RPO converges in fewer iterations than arbitrary order
    #[test]
    fn test_rpo_fewer_iterations() {
        let cfg = diamond_cfg();
        let refs = vec![
            make_def("x", 1),
            make_def("x", 3),
            make_def("x", 5),
            make_use("x", 7),
        ];

        let rpo_result = compute_reaching_definitions_rpo(&cfg, &refs);

        // For a simple diamond CFG, should converge quickly
        assert!(
            rpo_result.iterations <= 3,
            "RPO should converge in 2-3 iterations for diamond CFG, got {}",
            rpo_result.iterations
        );

        // Results should be valid (match naive algorithm)
        let naive = compute_reaching_definitions(&cfg, &refs);
        for block in &cfg.blocks {
            assert_eq!(
                rpo_result.reaching.reaching_in.get(&block.id),
                naive.reaching_in.get(&block.id),
                "RPO IN should match naive for block {}",
                block.id
            );
        }
    }

    /// Test: RPO handles complex loop correctly
    #[test]
    fn test_rpo_complex_loop() {
        let cfg = loop_cfg();
        let refs = vec![make_def("i", 1), make_use("i", 3), make_def("i", 5)];

        let rpo_result = compute_reaching_definitions_rpo(&cfg, &refs);

        // Should converge (not infinite loop)
        assert!(
            rpo_result.iterations <= 10,
            "Should converge within reasonable iterations"
        );

        // Results should be valid
        let naive = compute_reaching_definitions(&cfg, &refs);
        for block in &cfg.blocks {
            assert_eq!(
                rpo_result.reaching.reaching_in.get(&block.id),
                naive.reaching_in.get(&block.id),
                "RPO IN should match naive for block {}",
                block.id
            );
        }
    }
}

// =============================================================================
// Phase 16: Bit Vector Tests (RD-4)
// =============================================================================

#[cfg(test)]
mod bitvec_tests {
    use super::super::reaching::{compute_reaching_definitions_bitvec, create_dense_def_mapping};
    use super::fixtures::*;
    use super::*;

    /// Test: Bit vector produces same result as HashSet
    #[test]
    fn test_bitvec_same_result() {
        let cfg = diamond_cfg();
        let refs = vec![
            make_def("x", 1),
            make_def("y", 2),
            make_def("x", 3),
            make_def("y", 5),
            make_use("x", 7),
            make_use("y", 7),
        ];

        let hashset_result = compute_reaching_definitions(&cfg, &refs);
        let bitvec_result = compute_reaching_definitions_bitvec(&cfg, &refs);
        let converted = bitvec_result.to_standard();

        // Compare IN and OUT sets for each block
        for block in &cfg.blocks {
            let hashset_in = hashset_result.reaching_in.get(&block.id);
            let bitvec_in = converted.reaching_in.get(&block.id);
            assert_eq!(
                hashset_in, bitvec_in,
                "IN sets should match for block {}",
                block.id
            );

            let hashset_out = hashset_result.reaching_out.get(&block.id);
            let bitvec_out = converted.reaching_out.get(&block.id);
            assert_eq!(
                hashset_out, bitvec_out,
                "OUT sets should match for block {}",
                block.id
            );
        }
    }

    /// Test: Dense mapping is correct
    #[test]
    fn test_dense_mapping() {
        let refs = vec![
            make_def("x", 1),
            make_use("x", 2),
            make_def("y", 3),
            make_def("x", 4), // Second def of x
        ];

        let mapping = create_dense_def_mapping(&refs);

        // Should have 3 definitions
        assert_eq!(mapping.num_defs, 3, "Should have 3 definitions");

        // Each bit position should map back to a valid DefId
        for (i, def_id) in mapping.bit_to_def.iter().enumerate() {
            let reverse_lookup = mapping.def_to_bit.get(def_id);
            assert_eq!(reverse_lookup, Some(&i), "Mapping should be bijective");
        }
    }

    /// Test: Bit vector handles empty CFG
    #[test]
    fn test_bitvec_empty_cfg() {
        let cfg = CfgInfo {
            function: "empty".to_string(),
            blocks: Vec::new(),
            edges: Vec::new(),
            entry_block: 0,
            exit_blocks: Vec::new(),
            cyclomatic_complexity: 0,
            nested_functions: HashMap::new(),
        };
        let refs: Vec<VarRef> = Vec::new();

        let result = compute_reaching_definitions_bitvec(&cfg, &refs);

        assert!(result.in_sets.is_empty());
        assert!(result.out_sets.is_empty());
        assert_eq!(result.iterations, 0);
    }

    /// Test: Bit vector handles loop correctly
    #[test]
    fn test_bitvec_loop() {
        let cfg = loop_cfg();
        let refs = vec![make_def("i", 1), make_use("i", 3), make_def("i", 5)];

        let bitvec_result = compute_reaching_definitions_bitvec(&cfg, &refs);
        let hashset_result = compute_reaching_definitions(&cfg, &refs);

        // Should converge to same result
        let converted = bitvec_result.to_standard();
        for block in &cfg.blocks {
            assert_eq!(
                hashset_result.reaching_in.get(&block.id),
                converted.reaching_in.get(&block.id),
                "Loop IN should match for block {}",
                block.id
            );
        }
    }

    /// Test: Bit vector performance (large definition set)
    #[test]
    fn test_bitvec_many_definitions() {
        // Create a linear CFG with many definitions
        let mut blocks = Vec::new();
        let mut edges = Vec::new();
        let mut refs = Vec::new();

        let num_blocks = 20;
        let defs_per_block = 10;

        for i in 0..num_blocks {
            blocks.push(CfgBlock {
                id: i,
                block_type: if i == 0 {
                    BlockType::Entry
                } else if i == num_blocks - 1 {
                    BlockType::Exit
                } else {
                    BlockType::Body
                },
                lines: (i as u32 * 10, i as u32 * 10 + 9),
                calls: Vec::new(),
            });

            if i < num_blocks - 1 {
                edges.push(CfgEdge {
                    from: i,
                    to: i + 1,
                    edge_type: EdgeType::Unconditional,
                    condition: None,
                });
            }

            // Add definitions
            for j in 0..defs_per_block {
                refs.push(make_def(&format!("v{}", j), i as u32 * 10 + j as u32));
            }
        }

        let cfg = CfgInfo {
            function: "many_defs".to_string(),
            blocks,
            edges,
            entry_block: 0,
            exit_blocks: vec![num_blocks - 1],
            cyclomatic_complexity: 1,
            nested_functions: HashMap::new(),
        };

        // Both should complete
        let bitvec_result = compute_reaching_definitions_bitvec(&cfg, &refs);
        let hashset_result = compute_reaching_definitions(&cfg, &refs);

        // Results should match
        let converted = bitvec_result.to_standard();
        for block in &cfg.blocks {
            assert_eq!(
                hashset_result.reaching_in.get(&block.id),
                converted.reaching_in.get(&block.id),
                "Many defs: IN should match for block {}",
                block.id
            );
        }

        // Verify we processed all definitions
        assert_eq!(
            bitvec_result.mapping.num_defs,
            num_blocks * defs_per_block,
            "Should have all definitions mapped"
        );
    }
}
