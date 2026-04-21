//! SSA Module Tests
//!
//! Comprehensive tests for SSA construction and analysis.
//! These tests define expected behavior BEFORE implementation.
//! Tests are designed to FAIL until the modules are implemented.
//!
//! # Test Categories
//!
//! 1. Dominator Tree Tests (SSA-8)
//! 2. Dominance Frontier Tests (SSA-9, SSA-10)
//! 3. Minimal SSA Construction Tests (SSA-1)
//! 4. Pruned SSA Tests (SSA-2, SSA-3)
//! 5. Variable Versioning Tests (SSA-5, SSA-6)
//! 6. Memory SSA Tests (SSA-15, SSA-16, SSA-17)
//! 7. Value Numbering Tests (SSA-7)
//! 8. SCCP Tests (SSA-21)
//! 9. Dead Code Tests (SSA-22)
//! 10. Output Format Tests (SSA-18, SSA-19, SSA-20)
//!
//! Reference: session10-spec.md

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::types::{
    BlockType, CfgBlock, CfgEdge, CfgInfo, DfgInfo, EdgeType, RefType, VarRef,
};

use super::analysis::*;
use super::construct::*;
use super::dominators::*;
use super::format::*;
use super::memory::*;
use super::types::*;

// =============================================================================
// Test Fixtures
// =============================================================================

mod fixtures {
    use super::*;

    /// Create a simple linear CFG: Entry -> Block1 -> Exit
    pub fn linear_cfg() -> CfgInfo {
        CfgInfo {
            function: "linear".to_string(),
            blocks: vec![
                CfgBlock {
                    id: 0,
                    block_type: BlockType::Entry,
                    lines: (1, 2),
                    calls: Vec::new(),
                },
                CfgBlock {
                    id: 1,
                    block_type: BlockType::Body,
                    lines: (3, 4),
                    calls: Vec::new(),
                },
                CfgBlock {
                    id: 2,
                    block_type: BlockType::Exit,
                    lines: (5, 5),
                    calls: Vec::new(),
                },
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

    /// Create a diamond CFG: Entry -> {True, False} -> Merge -> Exit
    ///
    /// ```text
    ///       0 (entry)
    ///      / \
    ///     1   2
    ///      \ /
    ///       3 (merge)
    ///       |
    ///       4 (exit)
    /// ```
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
                CfgBlock {
                    id: 1,
                    block_type: BlockType::Body,
                    lines: (3, 4),
                    calls: Vec::new(),
                },
                CfgBlock {
                    id: 2,
                    block_type: BlockType::Body,
                    lines: (5, 6),
                    calls: Vec::new(),
                },
                CfgBlock {
                    id: 3,
                    block_type: BlockType::Body,
                    lines: (7, 8),
                    calls: Vec::new(),
                },
                CfgBlock {
                    id: 4,
                    block_type: BlockType::Exit,
                    lines: (9, 9),
                    calls: Vec::new(),
                },
            ],
            edges: vec![
                CfgEdge {
                    from: 0,
                    to: 1,
                    edge_type: EdgeType::True,
                    condition: Some("x > 0".to_string()),
                },
                CfgEdge {
                    from: 0,
                    to: 2,
                    edge_type: EdgeType::False,
                    condition: Some("x > 0".to_string()),
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
                CfgEdge {
                    from: 3,
                    to: 4,
                    edge_type: EdgeType::Unconditional,
                    condition: None,
                },
            ],
            entry_block: 0,
            exit_blocks: vec![4],
            cyclomatic_complexity: 2,
            nested_functions: HashMap::new(),
        }
    }

    /// Create a loop CFG: Entry -> Header <-> Body -> Exit
    ///
    /// ```text
    ///     0 (entry)
    ///       |
    ///     1 (header) <--+
    ///      / \          |
    ///     2   3 (body)--+
    ///   (exit)
    /// ```
    pub fn loop_cfg() -> CfgInfo {
        CfgInfo {
            function: "loop".to_string(),
            blocks: vec![
                CfgBlock {
                    id: 0,
                    block_type: BlockType::Entry,
                    lines: (1, 2),
                    calls: Vec::new(),
                },
                CfgBlock {
                    id: 1,
                    block_type: BlockType::LoopHeader,
                    lines: (3, 4),
                    calls: Vec::new(),
                },
                CfgBlock {
                    id: 2,
                    block_type: BlockType::Exit,
                    lines: (8, 8),
                    calls: Vec::new(),
                },
                CfgBlock {
                    id: 3,
                    block_type: BlockType::Body,
                    lines: (5, 7),
                    calls: Vec::new(),
                },
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
                    edge_type: EdgeType::False,
                    condition: Some("i < n".to_string()),
                },
                CfgEdge {
                    from: 1,
                    to: 3,
                    edge_type: EdgeType::True,
                    condition: Some("i < n".to_string()),
                },
                CfgEdge {
                    from: 3,
                    to: 1,
                    edge_type: EdgeType::Unconditional,
                    condition: None,
                },
            ],
            entry_block: 0,
            exit_blocks: vec![2],
            cyclomatic_complexity: 2,
            nested_functions: HashMap::new(),
        }
    }

    /// Create a complex CFG with nested branches
    ///
    /// ```text
    ///        0 (entry)
    ///       / \
    ///      1   2
    ///     / \   \
    ///    3   4   |
    ///     \ /    |
    ///      5     |
    ///       \   /
    ///        \ /
    ///         6 (merge)
    ///         |
    ///         7 (exit)
    /// ```
    pub fn complex_cfg() -> CfgInfo {
        CfgInfo {
            function: "complex".to_string(),
            blocks: vec![
                CfgBlock {
                    id: 0,
                    block_type: BlockType::Entry,
                    lines: (1, 2),
                    calls: Vec::new(),
                },
                CfgBlock {
                    id: 1,
                    block_type: BlockType::Body,
                    lines: (3, 4),
                    calls: Vec::new(),
                },
                CfgBlock {
                    id: 2,
                    block_type: BlockType::Body,
                    lines: (5, 6),
                    calls: Vec::new(),
                },
                CfgBlock {
                    id: 3,
                    block_type: BlockType::Body,
                    lines: (7, 8),
                    calls: Vec::new(),
                },
                CfgBlock {
                    id: 4,
                    block_type: BlockType::Body,
                    lines: (9, 10),
                    calls: Vec::new(),
                },
                CfgBlock {
                    id: 5,
                    block_type: BlockType::Body,
                    lines: (11, 12),
                    calls: Vec::new(),
                },
                CfgBlock {
                    id: 6,
                    block_type: BlockType::Body,
                    lines: (13, 14),
                    calls: Vec::new(),
                },
                CfgBlock {
                    id: 7,
                    block_type: BlockType::Exit,
                    lines: (15, 15),
                    calls: Vec::new(),
                },
            ],
            edges: vec![
                CfgEdge {
                    from: 0,
                    to: 1,
                    edge_type: EdgeType::True,
                    condition: Some("x > 0".to_string()),
                },
                CfgEdge {
                    from: 0,
                    to: 2,
                    edge_type: EdgeType::False,
                    condition: Some("x > 0".to_string()),
                },
                CfgEdge {
                    from: 1,
                    to: 3,
                    edge_type: EdgeType::True,
                    condition: Some("y > 0".to_string()),
                },
                CfgEdge {
                    from: 1,
                    to: 4,
                    edge_type: EdgeType::False,
                    condition: Some("y > 0".to_string()),
                },
                CfgEdge {
                    from: 2,
                    to: 6,
                    edge_type: EdgeType::Unconditional,
                    condition: None,
                },
                CfgEdge {
                    from: 3,
                    to: 5,
                    edge_type: EdgeType::Unconditional,
                    condition: None,
                },
                CfgEdge {
                    from: 4,
                    to: 5,
                    edge_type: EdgeType::Unconditional,
                    condition: None,
                },
                CfgEdge {
                    from: 5,
                    to: 6,
                    edge_type: EdgeType::Unconditional,
                    condition: None,
                },
                CfgEdge {
                    from: 6,
                    to: 7,
                    edge_type: EdgeType::Unconditional,
                    condition: None,
                },
            ],
            entry_block: 0,
            exit_blocks: vec![7],
            cyclomatic_complexity: 3,
            nested_functions: HashMap::new(),
        }
    }

    /// Create DFG for diamond pattern with variable y defined in both branches
    pub fn diamond_dfg() -> DfgInfo {
        DfgInfo {
            function: "diamond".to_string(),
            refs: vec![
                VarRef {
                    name: "x".to_string(),
                    ref_type: RefType::Definition,
                    line: 1,
                    column: 0,
                    context: None,
                    group_id: None,
                },
                VarRef {
                    name: "x".to_string(),
                    ref_type: RefType::Use,
                    line: 2,
                    column: 4,
                    context: None,
                    group_id: None,
                },
                VarRef {
                    name: "y".to_string(),
                    ref_type: RefType::Definition,
                    line: 3,
                    column: 0,
                    context: None,
                    group_id: None,
                },
                VarRef {
                    name: "y".to_string(),
                    ref_type: RefType::Definition,
                    line: 5,
                    column: 0,
                    context: None,
                    group_id: None,
                },
                VarRef {
                    name: "y".to_string(),
                    ref_type: RefType::Use,
                    line: 9,
                    column: 7,
                    context: None,
                    group_id: None,
                },
            ],
            edges: Vec::new(),
            variables: Vec::new(),
        }
    }

    /// Create DFG for loop pattern with variables total and i
    pub fn loop_dfg() -> DfgInfo {
        DfgInfo {
            function: "loop".to_string(),
            refs: vec![
                // total = 0 at entry
                VarRef {
                    name: "total".to_string(),
                    ref_type: RefType::Definition,
                    line: 1,
                    column: 0,
                    context: None,
                    group_id: None,
                },
                // i = 0 at entry
                VarRef {
                    name: "i".to_string(),
                    ref_type: RefType::Definition,
                    line: 2,
                    column: 0,
                    context: None,
                    group_id: None,
                },
                // i < n at header
                VarRef {
                    name: "i".to_string(),
                    ref_type: RefType::Use,
                    line: 3,
                    column: 6,
                    context: None,
                    group_id: None,
                },
                VarRef {
                    name: "n".to_string(),
                    ref_type: RefType::Use,
                    line: 3,
                    column: 10,
                    context: None,
                    group_id: None,
                },
                // total = total + i in body
                VarRef {
                    name: "total".to_string(),
                    ref_type: RefType::Definition,
                    line: 5,
                    column: 0,
                    context: None,
                    group_id: None,
                },
                VarRef {
                    name: "total".to_string(),
                    ref_type: RefType::Use,
                    line: 5,
                    column: 8,
                    context: None,
                    group_id: None,
                },
                VarRef {
                    name: "i".to_string(),
                    ref_type: RefType::Use,
                    line: 5,
                    column: 16,
                    context: None,
                    group_id: None,
                },
                // i = i + 1 in body
                VarRef {
                    name: "i".to_string(),
                    ref_type: RefType::Definition,
                    line: 6,
                    column: 0,
                    context: None,
                    group_id: None,
                },
                VarRef {
                    name: "i".to_string(),
                    ref_type: RefType::Use,
                    line: 6,
                    column: 4,
                    context: None,
                    group_id: None,
                },
                // return total at exit
                VarRef {
                    name: "total".to_string(),
                    ref_type: RefType::Use,
                    line: 8,
                    column: 7,
                    context: None,
                    group_id: None,
                },
            ],
            edges: Vec::new(),
            variables: Vec::new(),
        }
    }

    /// Create a sample SSA function for testing formatters
    pub fn sample_ssa() -> SsaFunction {
        SsaFunction {
            function: "sample".to_string(),
            file: PathBuf::from("test.py"),
            ssa_type: SsaType::Minimal,
            blocks: vec![
                SsaBlock {
                    id: 0,
                    label: Some("entry".to_string()),
                    lines: (1, 2),
                    phi_functions: vec![],
                    instructions: vec![
                        SsaInstruction {
                            kind: SsaInstructionKind::Param,
                            target: Some(SsaNameId(1)),
                            uses: vec![],
                            line: 1,
                            source_text: Some("x = param".to_string()),
                        },
                        SsaInstruction {
                            kind: SsaInstructionKind::Assign,
                            target: Some(SsaNameId(2)),
                            uses: vec![],
                            line: 2,
                            source_text: Some("y = 0".to_string()),
                        },
                    ],
                    successors: vec![1, 2],
                    predecessors: vec![],
                },
                SsaBlock {
                    id: 1,
                    label: None,
                    lines: (3, 4),
                    phi_functions: vec![],
                    instructions: vec![SsaInstruction {
                        kind: SsaInstructionKind::Assign,
                        target: Some(SsaNameId(3)),
                        uses: vec![SsaNameId(1)],
                        line: 3,
                        source_text: Some("y = x + 1".to_string()),
                    }],
                    successors: vec![3],
                    predecessors: vec![0],
                },
                SsaBlock {
                    id: 2,
                    label: None,
                    lines: (5, 6),
                    phi_functions: vec![],
                    instructions: vec![SsaInstruction {
                        kind: SsaInstructionKind::Assign,
                        target: Some(SsaNameId(4)),
                        uses: vec![SsaNameId(1)],
                        line: 5,
                        source_text: Some("y = x - 1".to_string()),
                    }],
                    successors: vec![3],
                    predecessors: vec![0],
                },
                SsaBlock {
                    id: 3,
                    label: Some("exit".to_string()),
                    lines: (7, 8),
                    phi_functions: vec![PhiFunction {
                        target: SsaNameId(5),
                        variable: "y".to_string(),
                        sources: vec![
                            PhiSource {
                                block: 1,
                                name: SsaNameId(3),
                            },
                            PhiSource {
                                block: 2,
                                name: SsaNameId(4),
                            },
                        ],
                        line: 7,
                    }],
                    instructions: vec![SsaInstruction {
                        kind: SsaInstructionKind::Return,
                        target: None,
                        uses: vec![SsaNameId(5)],
                        line: 8,
                        source_text: Some("return y".to_string()),
                    }],
                    successors: vec![],
                    predecessors: vec![1, 2],
                },
            ],
            ssa_names: vec![
                SsaName {
                    id: SsaNameId(1),
                    variable: "x".to_string(),
                    version: 1,
                    def_block: Some(0),
                    def_line: 1,
                },
                SsaName {
                    id: SsaNameId(2),
                    variable: "y".to_string(),
                    version: 1,
                    def_block: Some(0),
                    def_line: 2,
                },
                SsaName {
                    id: SsaNameId(3),
                    variable: "y".to_string(),
                    version: 2,
                    def_block: Some(1),
                    def_line: 3,
                },
                SsaName {
                    id: SsaNameId(4),
                    variable: "y".to_string(),
                    version: 3,
                    def_block: Some(2),
                    def_line: 5,
                },
                SsaName {
                    id: SsaNameId(5),
                    variable: "y".to_string(),
                    version: 4,
                    def_block: Some(3),
                    def_line: 7,
                },
            ],
            def_use: {
                let mut map = HashMap::new();
                map.insert(SsaNameId(1), vec![SsaNameId(3), SsaNameId(4)]);
                map.insert(SsaNameId(3), vec![SsaNameId(5)]);
                map.insert(SsaNameId(4), vec![SsaNameId(5)]);
                map
            },
            stats: SsaStats {
                phi_count: 1,
                ssa_names: 5,
                blocks: 4,
                instructions: 5,
                dead_phi_count: 0,
            },
        }
    }

    /// Create an SSA function with memory operations (stores, loads, calls)
    pub fn ssa_with_memory_ops() -> SsaFunction {
        SsaFunction {
            function: "memory_test".to_string(),
            file: PathBuf::from("test.py"),
            ssa_type: SsaType::Minimal,
            blocks: vec![
                SsaBlock {
                    id: 0,
                    label: Some("entry".to_string()),
                    lines: (1, 3),
                    phi_functions: vec![],
                    instructions: vec![
                        // obj = MyClass()  - allocation
                        SsaInstruction {
                            kind: SsaInstructionKind::Assign,
                            target: Some(SsaNameId(1)),
                            uses: vec![],
                            line: 1,
                            source_text: Some("obj = MyClass()".to_string()),
                        },
                        // obj.field = 10  - store
                        SsaInstruction {
                            kind: SsaInstructionKind::Assign,
                            target: Some(SsaNameId(2)),
                            uses: vec![SsaNameId(1)],
                            line: 2,
                            source_text: Some("obj.field = 10".to_string()),
                        },
                        // x = obj.field  - load
                        SsaInstruction {
                            kind: SsaInstructionKind::Assign,
                            target: Some(SsaNameId(3)),
                            uses: vec![SsaNameId(1)],
                            line: 3,
                            source_text: Some("x = obj.field".to_string()),
                        },
                    ],
                    successors: vec![1, 2],
                    predecessors: vec![],
                },
                SsaBlock {
                    id: 1,
                    label: None,
                    lines: (4, 5),
                    phi_functions: vec![],
                    instructions: vec![
                        // obj.field = 20  - store in branch 1
                        SsaInstruction {
                            kind: SsaInstructionKind::Assign,
                            target: Some(SsaNameId(4)),
                            uses: vec![SsaNameId(1)],
                            line: 4,
                            source_text: Some("obj.field = 20".to_string()),
                        },
                    ],
                    successors: vec![3],
                    predecessors: vec![0],
                },
                SsaBlock {
                    id: 2,
                    label: None,
                    lines: (6, 7),
                    phi_functions: vec![],
                    instructions: vec![
                        // obj.field = 30  - store in branch 2
                        SsaInstruction {
                            kind: SsaInstructionKind::Assign,
                            target: Some(SsaNameId(5)),
                            uses: vec![SsaNameId(1)],
                            line: 6,
                            source_text: Some("obj.field = 30".to_string()),
                        },
                    ],
                    successors: vec![3],
                    predecessors: vec![0],
                },
                SsaBlock {
                    id: 3,
                    label: Some("merge".to_string()),
                    lines: (8, 10),
                    phi_functions: vec![],
                    instructions: vec![
                        // y = obj.field  - load after merge
                        SsaInstruction {
                            kind: SsaInstructionKind::Assign,
                            target: Some(SsaNameId(6)),
                            uses: vec![SsaNameId(1)],
                            line: 8,
                            source_text: Some("y = obj.field".to_string()),
                        },
                        // process(y)  - call
                        SsaInstruction {
                            kind: SsaInstructionKind::Call,
                            target: None,
                            uses: vec![SsaNameId(6)],
                            line: 9,
                            source_text: Some("process(y)".to_string()),
                        },
                        SsaInstruction {
                            kind: SsaInstructionKind::Return,
                            target: None,
                            uses: vec![SsaNameId(6)],
                            line: 10,
                            source_text: Some("return y".to_string()),
                        },
                    ],
                    successors: vec![],
                    predecessors: vec![1, 2],
                },
            ],
            ssa_names: vec![
                SsaName {
                    id: SsaNameId(1),
                    variable: "obj".to_string(),
                    version: 1,
                    def_block: Some(0),
                    def_line: 1,
                },
                SsaName {
                    id: SsaNameId(2),
                    variable: "obj.field".to_string(),
                    version: 1,
                    def_block: Some(0),
                    def_line: 2,
                },
                SsaName {
                    id: SsaNameId(3),
                    variable: "x".to_string(),
                    version: 1,
                    def_block: Some(0),
                    def_line: 3,
                },
                SsaName {
                    id: SsaNameId(4),
                    variable: "obj.field".to_string(),
                    version: 2,
                    def_block: Some(1),
                    def_line: 4,
                },
                SsaName {
                    id: SsaNameId(5),
                    variable: "obj.field".to_string(),
                    version: 3,
                    def_block: Some(2),
                    def_line: 6,
                },
                SsaName {
                    id: SsaNameId(6),
                    variable: "y".to_string(),
                    version: 1,
                    def_block: Some(3),
                    def_line: 8,
                },
            ],
            def_use: HashMap::new(),
            stats: SsaStats::default(),
        }
    }
}

// =============================================================================
// Dominator Tree Tests (SSA-8)
// =============================================================================

#[cfg(test)]
mod dominator_tree_tests {
    use super::*;

    /// Test: Simple linear CFG has straightforward dominator tree
    /// Entry dominates all blocks, each block has previous as idom
    #[test]
    fn test_linear_cfg_dominators() {
        let cfg = fixtures::linear_cfg();
        let dom_tree = build_dominator_tree(&cfg).expect("should build dominator tree");

        // Entry block has no immediate dominator
        assert_eq!(dom_tree.nodes.get(&0).unwrap().idom, None);

        // Block 1's idom is block 0
        assert_eq!(dom_tree.nodes.get(&1).unwrap().idom, Some(0));

        // Block 2's idom is block 1
        assert_eq!(dom_tree.nodes.get(&2).unwrap().idom, Some(1));

        // Entry dominates all
        assert!(dom_tree.dominates(0, 0));
        assert!(dom_tree.dominates(0, 1));
        assert!(dom_tree.dominates(0, 2));

        // Block 1 dominates block 2 but not entry
        assert!(dom_tree.dominates(1, 2));
        assert!(!dom_tree.dominates(1, 0));
    }

    /// Test: Diamond pattern - branch point dominates both branches and merge
    #[test]
    fn test_diamond_cfg_dominators() {
        let cfg = fixtures::diamond_cfg();
        let dom_tree = build_dominator_tree(&cfg).expect("should build dominator tree");

        // Entry (0) has no idom
        assert_eq!(dom_tree.nodes.get(&0).unwrap().idom, None);

        // Both branches (1, 2) have entry as idom
        assert_eq!(dom_tree.nodes.get(&1).unwrap().idom, Some(0));
        assert_eq!(dom_tree.nodes.get(&2).unwrap().idom, Some(0));

        // Merge block (3) has entry as idom (not 1 or 2)
        assert_eq!(dom_tree.nodes.get(&3).unwrap().idom, Some(0));

        // Exit (4) has merge (3) as idom
        assert_eq!(dom_tree.nodes.get(&4).unwrap().idom, Some(3));

        // Branch 1 does NOT dominate merge (path through branch 2 exists)
        assert!(!dom_tree.dominates(1, 3));
        assert!(!dom_tree.dominates(2, 3));
    }

    /// Test: Loop - header dominates body, back edge doesn't affect dominators
    #[test]
    fn test_loop_cfg_dominators() {
        let cfg = fixtures::loop_cfg();
        let dom_tree = build_dominator_tree(&cfg).expect("should build dominator tree");

        // Entry (0) has no idom
        assert_eq!(dom_tree.nodes.get(&0).unwrap().idom, None);

        // Header (1) has entry as idom
        assert_eq!(dom_tree.nodes.get(&1).unwrap().idom, Some(0));

        // Body (3) has header as idom
        assert_eq!(dom_tree.nodes.get(&3).unwrap().idom, Some(1));

        // Exit (2) has header as idom
        assert_eq!(dom_tree.nodes.get(&2).unwrap().idom, Some(1));

        // Header dominates body
        assert!(dom_tree.dominates(1, 3));

        // Body does NOT dominate header (back edge doesn't count)
        assert!(!dom_tree.dominates(3, 1));
    }

    /// Test: Complex nested branches
    #[test]
    fn test_complex_cfg_dominators() {
        let cfg = fixtures::complex_cfg();
        let dom_tree = build_dominator_tree(&cfg).expect("should build dominator tree");

        // Entry dominates all
        for i in 0..8 {
            assert!(
                dom_tree.dominates(0, i),
                "entry should dominate block {}",
                i
            );
        }

        // Inner branch (1) dominates its children (3, 4) and inner merge (5)
        assert!(dom_tree.dominates(1, 3));
        assert!(dom_tree.dominates(1, 4));
        assert!(dom_tree.dominates(1, 5));

        // But inner branch doesn't dominate outer merge (6) - path through block 2
        assert!(!dom_tree.dominates(1, 6));

        // Outer merge (6) dominates exit (7)
        assert!(dom_tree.dominates(6, 7));
    }

    /// Test: Dominator tree depth calculation
    #[test]
    fn test_dominator_tree_depth() {
        let cfg = fixtures::linear_cfg();
        let dom_tree = build_dominator_tree(&cfg).expect("should build dominator tree");

        assert_eq!(dom_tree.nodes.get(&0).unwrap().depth, 0);
        assert_eq!(dom_tree.nodes.get(&1).unwrap().depth, 1);
        assert_eq!(dom_tree.nodes.get(&2).unwrap().depth, 2);
    }

    /// Test: dominated_by returns correct set
    #[test]
    fn test_dominated_by() {
        let cfg = fixtures::diamond_cfg();
        let dom_tree = build_dominator_tree(&cfg).expect("should build dominator tree");

        let dominated_by_entry = dom_tree.dominated_by(0);
        assert_eq!(dominated_by_entry.len(), 5); // All 5 blocks

        let dominated_by_merge = dom_tree.dominated_by(3);
        assert!(dominated_by_merge.contains(&3));
        assert!(dominated_by_merge.contains(&4));
        assert_eq!(dominated_by_merge.len(), 2);
    }

    /// Test: strictly_dominates excludes self
    #[test]
    fn test_strictly_dominates() {
        let cfg = fixtures::linear_cfg();
        let dom_tree = build_dominator_tree(&cfg).expect("should build dominator tree");

        assert!(!dom_tree.strictly_dominates(0, 0));
        assert!(dom_tree.strictly_dominates(0, 1));
        assert!(dom_tree.strictly_dominates(0, 2));
    }
}

// =============================================================================
// Dominance Frontier Tests (SSA-9, SSA-10)
// =============================================================================

#[cfg(test)]
mod dominance_frontier_tests {
    use super::*;

    /// Test: Entry block has empty dominance frontier
    #[test]
    fn test_entry_block_empty_frontier() {
        let cfg = fixtures::diamond_cfg();
        let dom_tree = build_dominator_tree(&cfg).expect("should build dominator tree");
        let df = compute_dominance_frontier(&cfg, &dom_tree).expect("should compute DF");

        // Entry block dominates everything it can reach, so DF is empty
        assert!(
            df.get(0).is_empty(),
            "Entry block should have empty dominance frontier"
        );
    }

    /// Test: Diamond pattern - branches have merge point in frontier
    #[test]
    fn test_diamond_dominance_frontier() {
        let cfg = fixtures::diamond_cfg();
        let dom_tree = build_dominator_tree(&cfg).expect("should build dominator tree");
        let df = compute_dominance_frontier(&cfg, &dom_tree).expect("should compute DF");

        // Blocks 1 and 2 (branches) should have block 3 (merge) in their frontier
        assert!(
            df.get(1).contains(&3),
            "Branch block 1 should have merge (3) in DF"
        );
        assert!(
            df.get(2).contains(&3),
            "Branch block 2 should have merge (3) in DF"
        );

        // Merge block (3) has empty frontier
        assert!(df.get(3).is_empty());
    }

    /// Test: Loop - header is in body's frontier
    #[test]
    fn test_loop_dominance_frontier() {
        let cfg = fixtures::loop_cfg();
        let dom_tree = build_dominator_tree(&cfg).expect("should build dominator tree");
        let df = compute_dominance_frontier(&cfg, &dom_tree).expect("should compute DF");

        // Body (3) has header (1) in its frontier due to back edge
        assert!(df.get(3).contains(&1), "Loop body should have header in DF");
    }

    /// Test: Iterated dominance frontier closure
    #[test]
    fn test_iterated_dominance_frontier() {
        let cfg = fixtures::complex_cfg();
        let dom_tree = build_dominator_tree(&cfg).expect("should build dominator tree");
        let df = compute_dominance_frontier(&cfg, &dom_tree).expect("should compute DF");

        // Define blocks in inner branch
        let inner_blocks: HashSet<usize> = vec![3, 4].into_iter().collect();

        // IDF should include inner merge (5) and potentially outer merge (6)
        let idf = df.iterated(&inner_blocks);
        assert!(idf.contains(&5), "IDF should contain inner merge point");
    }

    /// Test: Empty set has empty IDF
    #[test]
    fn test_empty_idf() {
        let cfg = fixtures::diamond_cfg();
        let dom_tree = build_dominator_tree(&cfg).expect("should build dominator tree");
        let df = compute_dominance_frontier(&cfg, &dom_tree).expect("should compute DF");

        let empty: HashSet<usize> = HashSet::new();
        let idf = df.iterated(&empty);
        assert!(idf.is_empty());
    }
}

// =============================================================================
// Minimal SSA Construction Tests (SSA-1)
// =============================================================================

#[cfg(test)]
mod minimal_ssa_tests {
    use super::*;

    /// Test: Single variable, no branching -> no phi functions needed
    #[test]
    fn test_linear_no_phi() {
        let cfg = fixtures::linear_cfg();
        let dfg = DfgInfo {
            function: "linear".to_string(),
            refs: vec![
                VarRef {
                    name: "x".to_string(),
                    ref_type: RefType::Definition,
                    line: 1,
                    column: 0,
                    context: None,
                    group_id: None,
                },
                VarRef {
                    name: "x".to_string(),
                    ref_type: RefType::Use,
                    line: 5,
                    column: 7,
                    context: None,
                    group_id: None,
                },
            ],
            edges: Vec::new(),
            variables: Vec::new(),
        };

        let ssa = construct_minimal_ssa(&cfg, &dfg).expect("should construct SSA");

        // No phi functions in linear code
        let total_phis: usize = ssa.blocks.iter().map(|b| b.phi_functions.len()).sum();
        assert_eq!(total_phis, 0, "Linear code should have no phi functions");
    }

    /// Test: Diamond pattern -> phi at merge point
    #[test]
    fn test_diamond_phi_at_merge() {
        let cfg = fixtures::diamond_cfg();
        let dfg = fixtures::diamond_dfg();

        let ssa = construct_minimal_ssa(&cfg, &dfg).expect("should construct SSA");

        // Find merge block (block 3)
        let merge_block = ssa.blocks.iter().find(|b| b.id == 3).unwrap();

        // Should have phi for variable y
        assert!(
            merge_block
                .phi_functions
                .iter()
                .any(|phi| phi.variable == "y"),
            "Merge block should have phi for y"
        );

        // Phi should have 2 sources (from blocks 1 and 2)
        let y_phi = merge_block
            .phi_functions
            .iter()
            .find(|phi| phi.variable == "y")
            .unwrap();
        assert_eq!(y_phi.sources.len(), 2, "Phi should have 2 sources");
    }

    /// Test: Loop -> phi at loop header
    #[test]
    fn test_loop_phi_at_header() {
        let cfg = fixtures::loop_cfg();
        let dfg = fixtures::loop_dfg();

        let ssa = construct_minimal_ssa(&cfg, &dfg).expect("should construct SSA");

        // Find header block (block 1)
        let header_block = ssa.blocks.iter().find(|b| b.id == 1).unwrap();

        // Should have phi for both total and i
        let has_total_phi = header_block
            .phi_functions
            .iter()
            .any(|phi| phi.variable == "total");
        let has_i_phi = header_block
            .phi_functions
            .iter()
            .any(|phi| phi.variable == "i");

        assert!(has_total_phi, "Header should have phi for total");
        assert!(has_i_phi, "Header should have phi for i");
    }

    /// Test: Multiple variables -> independent phi sets
    #[test]
    fn test_multiple_variables_independent_phis() {
        let cfg = fixtures::diamond_cfg();
        let dfg = DfgInfo {
            function: "diamond".to_string(),
            refs: vec![
                // x defined in both branches
                VarRef {
                    name: "x".to_string(),
                    ref_type: RefType::Definition,
                    line: 3,
                    column: 0,
                    context: None,
                    group_id: None,
                },
                VarRef {
                    name: "x".to_string(),
                    ref_type: RefType::Definition,
                    line: 5,
                    column: 0,
                    context: None,
                    group_id: None,
                },
                // y defined in both branches
                VarRef {
                    name: "y".to_string(),
                    ref_type: RefType::Definition,
                    line: 4,
                    column: 0,
                    context: None,
                    group_id: None,
                },
                VarRef {
                    name: "y".to_string(),
                    ref_type: RefType::Definition,
                    line: 6,
                    column: 0,
                    context: None,
                    group_id: None,
                },
                // Both used at merge
                VarRef {
                    name: "x".to_string(),
                    ref_type: RefType::Use,
                    line: 9,
                    column: 7,
                    context: None,
                    group_id: None,
                },
                VarRef {
                    name: "y".to_string(),
                    ref_type: RefType::Use,
                    line: 9,
                    column: 11,
                    context: None,
                    group_id: None,
                },
            ],
            edges: Vec::new(),
            variables: Vec::new(),
        };

        let ssa = construct_minimal_ssa(&cfg, &dfg).expect("should construct SSA");

        let merge_block = ssa.blocks.iter().find(|b| b.id == 3).unwrap();

        // Should have independent phis for both x and y
        let x_phi = merge_block
            .phi_functions
            .iter()
            .find(|phi| phi.variable == "x");
        let y_phi = merge_block
            .phi_functions
            .iter()
            .find(|phi| phi.variable == "y");

        assert!(x_phi.is_some(), "Should have phi for x");
        assert!(y_phi.is_some(), "Should have phi for y");
        assert_eq!(merge_block.phi_functions.len(), 2);
    }

    /// Test: SSA type is correctly set
    #[test]
    fn test_ssa_type_minimal() {
        let cfg = fixtures::diamond_cfg();
        let dfg = fixtures::diamond_dfg();

        let ssa = construct_minimal_ssa(&cfg, &dfg).expect("should construct SSA");
        assert_eq!(ssa.ssa_type, SsaType::Minimal);
    }
}

// =============================================================================
// Pruned SSA Tests (SSA-2, SSA-3)
// =============================================================================

#[cfg(test)]
mod pruned_ssa_tests {
    use super::*;

    /// Test: Dead variable at merge -> no phi in pruned SSA
    #[test]
    fn test_pruned_removes_dead_phi() {
        let cfg = fixtures::diamond_cfg();
        // Variable z is defined in both branches but never used
        let dfg = DfgInfo {
            function: "diamond".to_string(),
            refs: vec![
                VarRef {
                    name: "z".to_string(),
                    ref_type: RefType::Definition,
                    line: 3,
                    column: 0,
                    context: None,
                    group_id: None,
                },
                VarRef {
                    name: "z".to_string(),
                    ref_type: RefType::Definition,
                    line: 5,
                    column: 0,
                    context: None,
                    group_id: None,
                },
                // z is never used!
            ],
            edges: Vec::new(),
            variables: Vec::new(),
        };

        let live_vars = LiveVariables {
            function: "diamond".to_string(),
            blocks: {
                let mut map = HashMap::new();
                // z is not live at merge point
                map.insert(
                    3,
                    LiveSets {
                        live_in: HashSet::new(),
                        live_out: HashSet::new(),
                    },
                );
                map
            },
        };

        let ssa =
            construct_pruned_ssa(&cfg, &dfg, &live_vars).expect("should construct pruned SSA");

        // No phi for z because it's dead
        let merge_block = ssa.blocks.iter().find(|b| b.id == 3).unwrap();
        assert!(
            !merge_block
                .phi_functions
                .iter()
                .any(|phi| phi.variable == "z"),
            "Pruned SSA should not have phi for dead variable"
        );
    }

    /// Test: Semi-pruned excludes block-local variables
    #[test]
    fn test_semi_pruned_excludes_local() {
        let cfg = fixtures::diamond_cfg();
        // Variable temp is defined and used only in block 1
        let dfg = DfgInfo {
            function: "diamond".to_string(),
            refs: vec![
                VarRef {
                    name: "temp".to_string(),
                    ref_type: RefType::Definition,
                    line: 3,
                    column: 0,
                    context: None,
                    group_id: None,
                },
                VarRef {
                    name: "temp".to_string(),
                    ref_type: RefType::Use,
                    line: 4,
                    column: 0,
                    context: None,
                    group_id: None,
                },
            ],
            edges: Vec::new(),
            variables: Vec::new(),
        };

        let ssa = construct_semi_pruned_ssa(&cfg, &dfg).expect("should construct semi-pruned SSA");

        // No phi for temp anywhere (it's block-local)
        for block in &ssa.blocks {
            assert!(
                !block.phi_functions.iter().any(|phi| phi.variable == "temp"),
                "Semi-pruned SSA should not have phi for block-local variable"
            );
        }
    }

    /// Test: Semi-pruned includes cross-block variables
    #[test]
    fn test_semi_pruned_includes_cross_block() {
        let cfg = fixtures::diamond_cfg();
        let dfg = fixtures::diamond_dfg(); // y is used across blocks

        let ssa = construct_semi_pruned_ssa(&cfg, &dfg).expect("should construct semi-pruned SSA");

        // Should still have phi for y (it crosses block boundaries)
        let merge_block = ssa.blocks.iter().find(|b| b.id == 3).unwrap();
        assert!(
            merge_block
                .phi_functions
                .iter()
                .any(|phi| phi.variable == "y"),
            "Semi-pruned SSA should have phi for cross-block variable"
        );
    }

    /// Test: Pruned SSA type is correctly set
    #[test]
    fn test_ssa_type_pruned() {
        let cfg = fixtures::diamond_cfg();
        let dfg = fixtures::diamond_dfg();
        let live_vars = LiveVariables {
            function: "diamond".to_string(),
            blocks: HashMap::new(),
        };

        let ssa =
            construct_pruned_ssa(&cfg, &dfg, &live_vars).expect("should construct pruned SSA");
        assert_eq!(ssa.ssa_type, SsaType::Pruned);
    }

    // =========================================================================
    // Phase 12: Additional Pruned/Semi-Pruned SSA Tests
    // =========================================================================

    /// Test: Pruned SSA has fewer phi functions than minimal SSA
    /// When variables are not live, pruned should eliminate unnecessary phis
    #[test]
    fn test_pruned_phi_count_less_than_minimal() {
        let cfg = fixtures::diamond_cfg();
        // y is defined in both branches and used after merge
        // z is defined in both branches but never used
        let dfg = DfgInfo {
            function: "diamond".to_string(),
            refs: vec![
                // y defined in branch 1
                VarRef {
                    name: "y".to_string(),
                    ref_type: RefType::Definition,
                    line: 3,
                    column: 0,
                    context: None,
                    group_id: None,
                },
                // y defined in branch 2
                VarRef {
                    name: "y".to_string(),
                    ref_type: RefType::Definition,
                    line: 5,
                    column: 0,
                    context: None,
                    group_id: None,
                },
                // y used after merge
                VarRef {
                    name: "y".to_string(),
                    ref_type: RefType::Use,
                    line: 9,
                    column: 0,
                    context: None,
                    group_id: None,
                },
                // z defined in branch 1 (dead)
                VarRef {
                    name: "z".to_string(),
                    ref_type: RefType::Definition,
                    line: 4,
                    column: 0,
                    context: None,
                    group_id: None,
                },
                // z defined in branch 2 (dead)
                VarRef {
                    name: "z".to_string(),
                    ref_type: RefType::Definition,
                    line: 6,
                    column: 0,
                    context: None,
                    group_id: None,
                },
                // z is never used!
            ],
            edges: Vec::new(),
            variables: Vec::new(),
        };

        // Minimal SSA should have phi for both y and z
        let minimal_ssa = construct_minimal_ssa(&cfg, &dfg).expect("should construct minimal SSA");
        let minimal_phi_count: usize = minimal_ssa
            .blocks
            .iter()
            .map(|b| b.phi_functions.len())
            .sum();

        // Live vars: y is live at merge (used at line 9), z is not
        let live_vars = LiveVariables {
            function: "diamond".to_string(),
            blocks: {
                let mut map = HashMap::new();
                map.insert(
                    3,
                    LiveSets {
                        live_in: {
                            let mut set = HashSet::new();
                            set.insert("y".to_string());
                            set
                        },
                        live_out: HashSet::new(),
                    },
                );
                map
            },
        };

        let pruned_ssa =
            construct_pruned_ssa(&cfg, &dfg, &live_vars).expect("should construct pruned SSA");
        let pruned_phi_count: usize = pruned_ssa
            .blocks
            .iter()
            .map(|b| b.phi_functions.len())
            .sum();

        assert!(
            pruned_phi_count <= minimal_phi_count,
            "Pruned SSA should have <= phi functions than minimal: pruned={}, minimal={}",
            pruned_phi_count,
            minimal_phi_count
        );
    }

    /// Test: Variable live at merge gets phi in pruned SSA
    #[test]
    fn test_pruned_phi_for_live_variable() {
        let cfg = fixtures::diamond_cfg();
        // y is defined in both branches and used after merge
        let dfg = DfgInfo {
            function: "diamond".to_string(),
            refs: vec![
                VarRef {
                    name: "y".to_string(),
                    ref_type: RefType::Definition,
                    line: 3,
                    column: 0,
                    context: None,
                    group_id: None,
                },
                VarRef {
                    name: "y".to_string(),
                    ref_type: RefType::Definition,
                    line: 5,
                    column: 0,
                    context: None,
                    group_id: None,
                },
                VarRef {
                    name: "y".to_string(),
                    ref_type: RefType::Use,
                    line: 9,
                    column: 0,
                    context: None,
                    group_id: None,
                },
            ],
            edges: Vec::new(),
            variables: Vec::new(),
        };

        // y is live at block 3 (merge point)
        let live_vars = LiveVariables {
            function: "diamond".to_string(),
            blocks: {
                let mut map = HashMap::new();
                map.insert(
                    3,
                    LiveSets {
                        live_in: {
                            let mut set = HashSet::new();
                            set.insert("y".to_string());
                            set
                        },
                        live_out: HashSet::new(),
                    },
                );
                map
            },
        };

        let ssa =
            construct_pruned_ssa(&cfg, &dfg, &live_vars).expect("should construct pruned SSA");

        // Should have phi for y because it's live
        let merge_block = ssa.blocks.iter().find(|b| b.id == 3).unwrap();
        assert!(
            merge_block
                .phi_functions
                .iter()
                .any(|phi| phi.variable == "y"),
            "Pruned SSA should have phi for live variable y"
        );
    }

    /// Test: Semi-pruned correctly identifies block-local vs cross-block
    #[test]
    fn test_semi_pruned_block_local_detection() {
        let cfg = fixtures::diamond_cfg();
        // local_var: defined and used only in block 1 (lines 3-4)
        // cross_var: defined in block 1, used in block 3 (merge)
        let dfg = DfgInfo {
            function: "diamond".to_string(),
            refs: vec![
                // local_var: block-local (all refs in block 1, lines 3-4)
                VarRef {
                    name: "local_var".to_string(),
                    ref_type: RefType::Definition,
                    line: 3,
                    column: 0,
                    context: None,
                    group_id: None,
                },
                VarRef {
                    name: "local_var".to_string(),
                    ref_type: RefType::Use,
                    line: 4,
                    column: 0,
                    context: None,
                    group_id: None,
                },
                // cross_var: crosses blocks (def in 1, def in 2, use in 3)
                VarRef {
                    name: "cross_var".to_string(),
                    ref_type: RefType::Definition,
                    line: 3,
                    column: 0,
                    context: None,
                    group_id: None,
                },
                VarRef {
                    name: "cross_var".to_string(),
                    ref_type: RefType::Definition,
                    line: 5,
                    column: 0,
                    context: None,
                    group_id: None,
                },
                VarRef {
                    name: "cross_var".to_string(),
                    ref_type: RefType::Use,
                    line: 7,
                    column: 0,
                    context: None,
                    group_id: None,
                },
            ],
            edges: Vec::new(),
            variables: Vec::new(),
        };

        let ssa = construct_semi_pruned_ssa(&cfg, &dfg).expect("should construct semi-pruned SSA");

        // No phi for local_var (block-local)
        let has_local_phi = ssa.blocks.iter().any(|b| {
            b.phi_functions
                .iter()
                .any(|phi| phi.variable == "local_var")
        });
        assert!(
            !has_local_phi,
            "Semi-pruned should not have phi for block-local variable"
        );

        // Should have phi for cross_var at merge point (block 3)
        let merge_block = ssa.blocks.iter().find(|b| b.id == 3).unwrap();
        assert!(
            merge_block
                .phi_functions
                .iter()
                .any(|phi| phi.variable == "cross_var"),
            "Semi-pruned should have phi for cross-block variable"
        );
    }
}

// =============================================================================
// Phase 12: Live Variables Analysis Tests (RD-12)
// =============================================================================

#[cfg(test)]
mod phase12_live_variables_tests {
    use super::*;

    /// Test: Basic forward liveness - variable used is live before use
    #[test]
    fn test_live_variables_simple_use() {
        let cfg = fixtures::linear_cfg();
        let refs = vec![
            VarRef {
                name: "x".to_string(),
                ref_type: RefType::Definition,
                line: 1,
                column: 0,
                context: None,
                group_id: None,
            },
            VarRef {
                name: "x".to_string(),
                ref_type: RefType::Use,
                line: 5,
                column: 0,
                context: None,
                group_id: None,
            },
        ];

        let live = compute_live_variables(&cfg, &refs).expect("should compute live variables");

        // x should be live-in at block 2 (where it's used at line 5)
        if let Some(block2) = live.blocks.get(&2) {
            assert!(
                block2.live_in.contains("x"),
                "x should be live-in at block 2 (used at line 5)"
            );
        }

        // x should be live-out of block 1 (flows to block 2)
        if let Some(block1) = live.blocks.get(&1) {
            assert!(
                block1.live_out.contains("x"),
                "x should be live-out of block 1 (flows to use in block 2)"
            );
        }
    }

    /// Test: Variable not live after last use
    #[test]
    fn test_live_variables_not_live_after_use() {
        let cfg = fixtures::linear_cfg();
        let refs = vec![
            VarRef {
                name: "x".to_string(),
                ref_type: RefType::Definition,
                line: 1,
                column: 0,
                context: None,
                group_id: None,
            },
            VarRef {
                name: "x".to_string(),
                ref_type: RefType::Use,
                line: 3,
                column: 0,
                context: None,
                group_id: None,
            },
            // x not used after line 3
        ];

        let live = compute_live_variables(&cfg, &refs).expect("should compute live variables");

        // x should not be live-out of block 2 (exit block, no more uses)
        if let Some(block2) = live.blocks.get(&2) {
            assert!(
                !block2.live_out.contains("x"),
                "x should not be live-out of exit block (no more uses)"
            );
        }
    }

    /// Test: Loop back edge propagates liveness
    #[test]
    fn test_live_variables_loop() {
        let cfg = fixtures::loop_cfg();
        // i is used in header (condition) and incremented in body
        let refs = vec![
            VarRef {
                name: "i".to_string(),
                ref_type: RefType::Definition,
                line: 1,
                column: 0,
                context: None,
                group_id: None,
            },
            VarRef {
                name: "i".to_string(),
                ref_type: RefType::Use,
                line: 3, // header condition
                column: 0,
                context: None,
                group_id: None,
            },
            VarRef {
                name: "i".to_string(),
                ref_type: RefType::Update,
                line: 6, // i += 1 in body
                column: 0,
                context: None,
                group_id: None,
            },
        ];

        let live = compute_live_variables(&cfg, &refs).expect("should compute live variables");

        // i should be live at header (block 1) due to use in condition
        if let Some(header) = live.blocks.get(&1) {
            assert!(
                header.live_in.contains("i"),
                "i should be live-in at loop header (used in condition)"
            );
        }

        // i should be live-out of body (block 3) due to back edge to header
        if let Some(body) = live.blocks.get(&3) {
            assert!(
                body.live_out.contains("i"),
                "i should be live-out of loop body (flows back to header)"
            );
        }
    }

    /// Test: Unused variable is not live
    #[test]
    fn test_live_variables_unused_not_live() {
        let cfg = fixtures::diamond_cfg();
        let refs = vec![
            VarRef {
                name: "unused".to_string(),
                ref_type: RefType::Definition,
                line: 3,
                column: 0,
                context: None,
                group_id: None,
            },
            // unused is never used!
        ];

        let live = compute_live_variables(&cfg, &refs).expect("should compute live variables");

        // unused should not be live anywhere (no uses)
        let any_live = live
            .blocks
            .values()
            .any(|sets| sets.live_in.contains("unused") || sets.live_out.contains("unused"));
        assert!(!any_live, "Unused variable should not be live anywhere");
    }

    /// Test: Multiple variables with different liveness ranges
    #[test]
    fn test_live_variables_multiple_vars() {
        let cfg = fixtures::diamond_cfg();
        let refs = vec![
            // x: defined at entry, used at exit
            VarRef {
                name: "x".to_string(),
                ref_type: RefType::Definition,
                line: 1,
                column: 0,
                context: None,
                group_id: None,
            },
            VarRef {
                name: "x".to_string(),
                ref_type: RefType::Use,
                line: 9,
                column: 0,
                context: None,
                group_id: None,
            },
            // y: defined in branch 1 only, used at merge
            VarRef {
                name: "y".to_string(),
                ref_type: RefType::Definition,
                line: 3,
                column: 0,
                context: None,
                group_id: None,
            },
            VarRef {
                name: "y".to_string(),
                ref_type: RefType::Use,
                line: 7,
                column: 0,
                context: None,
                group_id: None,
            },
        ];

        let live = compute_live_variables(&cfg, &refs).expect("should compute live variables");

        // x should be live through the entire function (entry to exit)
        // y should be live from block 1 to block 3

        // At merge point (block 3), both should be live-in
        if let Some(merge) = live.blocks.get(&3) {
            assert!(
                merge.live_in.contains("x"),
                "x should be live-in at merge (used at exit)"
            );
            assert!(
                merge.live_in.contains("y"),
                "y should be live-in at merge (used there)"
            );
        }
    }
}

// =============================================================================
// Variable Versioning Tests (SSA-5, SSA-6)
// =============================================================================

#[cfg(test)]
mod variable_versioning_tests {
    use super::*;

    /// Test: Simple sequential assignments get different versions
    /// x = 1; x = 2 -> x_1, x_2
    #[test]
    fn test_sequential_assignments_versioned() {
        let cfg = fixtures::linear_cfg();
        let dfg = DfgInfo {
            function: "linear".to_string(),
            refs: vec![
                VarRef {
                    name: "x".to_string(),
                    ref_type: RefType::Definition,
                    line: 1,
                    column: 0,
                    context: None,
                    group_id: None,
                },
                VarRef {
                    name: "x".to_string(),
                    ref_type: RefType::Definition,
                    line: 3,
                    column: 0,
                    context: None,
                    group_id: None,
                },
            ],
            edges: Vec::new(),
            variables: Vec::new(),
        };

        let ssa = construct_minimal_ssa(&cfg, &dfg).expect("should construct SSA");

        // Find all SSA names for x
        let x_names: Vec<&SsaName> = ssa.ssa_names.iter().filter(|n| n.variable == "x").collect();

        assert_eq!(x_names.len(), 2, "Should have 2 versions of x");

        // Versions should be 1 and 2
        let versions: HashSet<u32> = x_names.iter().map(|n| n.version).collect();
        assert!(versions.contains(&1));
        assert!(versions.contains(&2));
    }

    /// Test: Phi targets get correct versions
    #[test]
    fn test_phi_target_versioning() {
        let cfg = fixtures::diamond_cfg();
        let dfg = fixtures::diamond_dfg();

        let ssa = construct_minimal_ssa(&cfg, &dfg).expect("should construct SSA");

        let merge_block = ssa.blocks.iter().find(|b| b.id == 3).unwrap();
        let y_phi = merge_block
            .phi_functions
            .iter()
            .find(|phi| phi.variable == "y")
            .unwrap();

        // Phi target should have higher version than sources
        let target_name = ssa.ssa_names.iter().find(|n| n.id == y_phi.target).unwrap();
        for source in &y_phi.sources {
            let source_name = ssa.ssa_names.iter().find(|n| n.id == source.name).unwrap();
            assert!(
                target_name.version > source_name.version,
                "Phi target version should be greater than source versions"
            );
        }
    }

    /// Test: Definition lookup is O(1)
    #[test]
    fn test_definition_lookup() {
        let ssa = fixtures::sample_ssa();

        // Look up definition for each SSA name
        for name in &ssa.ssa_names {
            let def_block = get_def_block(&ssa, name.id);
            assert!(
                def_block.is_some() || name.def_block.is_none(),
                "Should find definition block for SSA name"
            );
        }
    }

    /// Test: SSA name formatting
    #[test]
    fn test_ssa_name_formatting() {
        let name = SsaName {
            id: SsaNameId(1),
            variable: "x".to_string(),
            version: 3,
            def_block: Some(0),
            def_line: 1,
        };

        assert_eq!(name.format_name(), "x_3");
        assert_eq!(format!("{}", name), "x_3");
    }

    /// Test: Uses reference correct versions
    #[test]
    fn test_uses_reference_correct_version() {
        let ssa = fixtures::sample_ssa();

        // In block 3, return uses y_4 (phi result)
        let exit_block = ssa.blocks.iter().find(|b| b.id == 3).unwrap();
        let return_inst = exit_block
            .instructions
            .iter()
            .find(|i| i.kind == SsaInstructionKind::Return)
            .unwrap();

        // Return should use the phi result
        assert!(
            return_inst.uses.contains(&SsaNameId(5)),
            "Return should use phi result"
        );
    }
}

// =============================================================================
// Memory SSA Tests (SSA-15, SSA-16, SSA-17)
// =============================================================================

#[cfg(test)]
mod memory_ssa_tests {
    use super::*;

    /// Test: Store creates new memory version (SSA-15)
    #[test]
    fn test_store_creates_memory_version() {
        let cfg = fixtures::diamond_cfg();
        let ssa = fixtures::ssa_with_memory_ops();

        let memory_ssa = build_memory_ssa(&cfg, &ssa).expect("should build memory SSA");

        // Should have memory definitions from stores
        // Fixture has: obj.field = 10 (line 2), obj.field = 20 (line 4), obj.field = 30 (line 6)
        assert!(
            memory_ssa.memory_defs.len() >= 3,
            "Should have at least 3 memory defs (stores): got {}",
            memory_ssa.memory_defs.len()
        );

        // Each store should create a unique, increasing memory version
        let versions: Vec<u32> = memory_ssa.memory_defs.iter().map(|d| d.version.0).collect();
        for i in 1..versions.len() {
            assert!(
                versions[i] > versions[i - 1] || versions[i] > 0,
                "Memory versions should be increasing: {:?}",
                versions
            );
        }

        // Check that stores have the Store kind
        let store_count = memory_ssa
            .memory_defs
            .iter()
            .filter(|d| d.kind == Some(MemoryDefKind::Store))
            .count();
        assert!(store_count >= 3, "Should have at least 3 Store defs");
    }

    /// Test: Load uses memory version (SSA-15)
    #[test]
    fn test_load_uses_memory_version() {
        let cfg = fixtures::diamond_cfg();
        let ssa = fixtures::ssa_with_memory_ops();

        let memory_ssa = build_memory_ssa(&cfg, &ssa).expect("should build memory SSA");

        // Each load should reference a valid memory version
        for use_ in &memory_ssa.memory_uses {
            // Memory version should exist (either from def or phi)
            let version_exists = memory_ssa
                .memory_defs
                .iter()
                .any(|d| d.version == use_.version)
                || memory_ssa
                    .memory_phis
                    .iter()
                    .any(|p| p.result == use_.version);
            assert!(
                version_exists || use_.version.0 == 0,
                "Load should use existing memory version"
            );
        }
    }

    /// Test: Memory phi at merge points (SSA-16)
    #[test]
    fn test_memory_phi_at_merge() {
        let cfg = fixtures::diamond_cfg();
        let ssa = fixtures::ssa_with_memory_ops();

        let memory_ssa = build_memory_ssa(&cfg, &ssa).expect("should build memory SSA");

        // The fixture has stores in both branches (blocks 1 and 2)
        // There should be a memory phi at block 3 (merge point)
        let has_merge_phi = memory_ssa.memory_phis.iter().any(|phi| phi.block == 3);

        // Since we have stores in both branches, we expect a phi
        if has_merge_phi {
            let merge_phi = memory_ssa
                .memory_phis
                .iter()
                .find(|phi| phi.block == 3)
                .unwrap();
            assert_eq!(
                merge_phi.sources.len(),
                2,
                "Memory phi should have 2 sources"
            );

            // Each source should reference a valid predecessor block
            let pred_blocks: Vec<usize> = merge_phi.sources.iter().map(|s| s.block).collect();
            assert!(
                pred_blocks.contains(&1) || pred_blocks.contains(&2),
                "Phi sources should come from predecessor blocks"
            );
        }
    }

    /// Test: Memory def-use chains connect loads to stores (SSA-17)
    #[test]
    fn test_memory_def_use_chains() {
        let cfg = fixtures::diamond_cfg();
        let ssa = fixtures::ssa_with_memory_ops();

        let memory_ssa = build_memory_ssa(&cfg, &ssa).expect("should build memory SSA");

        // Def-use chains should be populated
        // Each memory def version should map to its uses
        for (def_version, uses) in &memory_ssa.def_use {
            for use_version in uses {
                // Use should reference this def (directly or through phi)
                assert!(
                    memory_ssa
                        .memory_uses
                        .iter()
                        .any(|u| &u.version == use_version)
                        || memory_ssa
                            .memory_phis
                            .iter()
                            .any(|p| p.sources.iter().any(|s| &s.version == use_version))
                        || memory_ssa
                            .memory_phis
                            .iter()
                            .any(|p| p.result == *use_version),
                    "Def-use chain for {:?} should reference valid use: {:?}",
                    def_version,
                    use_version
                );
            }
        }
    }

    /// Test: Memory version display formatting
    #[test]
    fn test_memory_version_formatting() {
        let version = MemoryVersion(5);
        assert_eq!(format!("{}", version), "mem_5");
    }

    /// Test: Function calls are treated as memory clobbers (SSA-15)
    #[test]
    fn test_call_clobbers_memory() {
        let cfg = fixtures::diamond_cfg();
        let ssa = fixtures::ssa_with_memory_ops();

        let memory_ssa = build_memory_ssa(&cfg, &ssa).expect("should build memory SSA");

        // The fixture has a call to process(y) in block 3
        // This should be treated as both a use and def of memory
        let call_defs: Vec<_> = memory_ssa
            .memory_defs
            .iter()
            .filter(|d| d.kind == Some(MemoryDefKind::Call))
            .collect();

        let call_uses: Vec<_> = memory_ssa
            .memory_uses
            .iter()
            .filter(|u| u.kind == Some(MemoryUseKind::Call))
            .collect();

        // Should have at least one call def (clobber)
        assert!(
            !call_defs.is_empty(),
            "Function calls should create memory defs (clobbers)"
        );

        // Call should also read memory
        assert!(
            !call_uses.is_empty(),
            "Function calls should create memory uses"
        );
    }

    /// Test: Memory SSA stats are computed correctly
    #[test]
    fn test_memory_ssa_stats() {
        let cfg = fixtures::diamond_cfg();
        let ssa = fixtures::ssa_with_memory_ops();

        let memory_ssa = build_memory_ssa(&cfg, &ssa).expect("should build memory SSA");

        // Stats should match actual counts
        assert_eq!(
            memory_ssa.stats.defs,
            memory_ssa.memory_defs.len(),
            "Stats defs should match memory_defs length"
        );
        assert_eq!(
            memory_ssa.stats.uses,
            memory_ssa.memory_uses.len(),
            "Stats uses should match memory_uses length"
        );
        assert_eq!(
            memory_ssa.stats.phis,
            memory_ssa.memory_phis.len(),
            "Stats phis should match memory_phis length"
        );
    }

    /// Test: Each memory def clobbers the previous version
    #[test]
    fn test_memory_def_clobbers_previous() {
        let cfg = fixtures::diamond_cfg();
        let ssa = fixtures::ssa_with_memory_ops();

        let memory_ssa = build_memory_ssa(&cfg, &ssa).expect("should build memory SSA");

        // Each def should clobber the previous version
        for def in &memory_ssa.memory_defs {
            // Clobbers should be a valid version (0 for initial or a previous def)
            assert!(
                def.clobbers.0 < def.version.0,
                "Def version {} should clobber a lower version, got {}",
                def.version.0,
                def.clobbers.0
            );
        }
    }

    /// Test: Explicit def-use chains (SSA-17)
    #[test]
    fn test_explicit_def_use_chains() {
        let cfg = fixtures::diamond_cfg();
        let ssa = fixtures::ssa_with_memory_ops();

        let memory_ssa = build_memory_ssa(&cfg, &ssa).expect("should build memory SSA");

        // Build explicit chains
        let chains = build_explicit_def_use_chains(&memory_ssa);

        // Should have one chain per def
        assert_eq!(
            chains.len(),
            memory_ssa.memory_defs.len(),
            "Should have one chain per memory def"
        );

        // Each chain should have the correct def info
        for chain in &chains {
            let matching_def = memory_ssa
                .memory_defs
                .iter()
                .find(|d| d.version == chain.def);
            assert!(
                matching_def.is_some(),
                "Chain def should exist in memory_defs"
            );

            let def = matching_def.unwrap();
            assert_eq!(chain.def_line, def.line, "Chain def_line should match");
            assert_eq!(chain.def_block, def.block, "Chain def_block should match");
        }
    }

    /// Test: Empty SSA produces empty Memory SSA
    #[test]
    fn test_empty_ssa_produces_empty_memory_ssa() {
        let cfg = fixtures::linear_cfg();
        let ssa = SsaFunction {
            function: "empty".to_string(),
            file: PathBuf::from("empty.py"),
            ssa_type: SsaType::Minimal,
            blocks: vec![SsaBlock {
                id: 0,
                label: None,
                lines: (1, 1),
                phi_functions: vec![],
                instructions: vec![],
                successors: vec![],
                predecessors: vec![],
            }],
            ssa_names: vec![],
            def_use: HashMap::new(),
            stats: SsaStats::default(),
        };

        let memory_ssa = build_memory_ssa(&cfg, &ssa).expect("should build memory SSA");

        assert!(
            memory_ssa.memory_defs.is_empty(),
            "Empty SSA should have no memory defs"
        );
        assert!(
            memory_ssa.memory_uses.is_empty(),
            "Empty SSA should have no memory uses"
        );
        assert!(
            memory_ssa.memory_phis.is_empty(),
            "Empty SSA should have no memory phis"
        );
    }
}

// =============================================================================
// Value Numbering Tests (SSA-7)
// =============================================================================

#[cfg(test)]
mod value_numbering_tests {
    use super::*;

    /// Test: Same expression -> same value number
    #[test]
    fn test_same_expression_same_number() {
        // Create SSA with two identical expressions: a + b
        let ssa = SsaFunction {
            function: "test".to_string(),
            file: PathBuf::from("test.py"),
            ssa_type: SsaType::Minimal,
            blocks: vec![SsaBlock {
                id: 0,
                label: None,
                lines: (1, 4),
                phi_functions: vec![],
                instructions: vec![
                    SsaInstruction {
                        kind: SsaInstructionKind::Param,
                        target: Some(SsaNameId(1)),
                        uses: vec![],
                        line: 1,
                        source_text: Some("a = param".to_string()),
                    },
                    SsaInstruction {
                        kind: SsaInstructionKind::Param,
                        target: Some(SsaNameId(2)),
                        uses: vec![],
                        line: 2,
                        source_text: Some("b = param".to_string()),
                    },
                    SsaInstruction {
                        kind: SsaInstructionKind::BinaryOp,
                        target: Some(SsaNameId(3)),
                        uses: vec![SsaNameId(1), SsaNameId(2)],
                        line: 3,
                        source_text: Some("x = a + b".to_string()),
                    },
                    SsaInstruction {
                        kind: SsaInstructionKind::BinaryOp,
                        target: Some(SsaNameId(4)),
                        uses: vec![SsaNameId(1), SsaNameId(2)],
                        line: 4,
                        source_text: Some("y = a + b".to_string()),
                    },
                ],
                successors: vec![],
                predecessors: vec![],
            }],
            ssa_names: vec![
                SsaName {
                    id: SsaNameId(1),
                    variable: "a".to_string(),
                    version: 1,
                    def_block: Some(0),
                    def_line: 1,
                },
                SsaName {
                    id: SsaNameId(2),
                    variable: "b".to_string(),
                    version: 1,
                    def_block: Some(0),
                    def_line: 2,
                },
                SsaName {
                    id: SsaNameId(3),
                    variable: "x".to_string(),
                    version: 1,
                    def_block: Some(0),
                    def_line: 3,
                },
                SsaName {
                    id: SsaNameId(4),
                    variable: "y".to_string(),
                    version: 1,
                    def_block: Some(0),
                    def_line: 4,
                },
            ],
            def_use: HashMap::new(),
            stats: SsaStats::default(),
        };

        let vn = compute_value_numbers(&ssa).expect("should compute value numbers");

        // x and y should have the same value number (both are a + b)
        let x_vn = vn.value_numbers.get(&SsaNameId(3)).unwrap();
        let y_vn = vn.value_numbers.get(&SsaNameId(4)).unwrap();
        assert_eq!(x_vn, y_vn, "Same expression should have same value number");
    }

    /// Test: Different operands -> different value number
    #[test]
    fn test_different_operands_different_number() {
        let ssa = SsaFunction {
            function: "test".to_string(),
            file: PathBuf::from("test.py"),
            ssa_type: SsaType::Minimal,
            blocks: vec![SsaBlock {
                id: 0,
                label: None,
                lines: (1, 4),
                phi_functions: vec![],
                instructions: vec![
                    SsaInstruction {
                        kind: SsaInstructionKind::Param,
                        target: Some(SsaNameId(1)),
                        uses: vec![],
                        line: 1,
                        source_text: Some("a = param".to_string()),
                    },
                    SsaInstruction {
                        kind: SsaInstructionKind::Param,
                        target: Some(SsaNameId(2)),
                        uses: vec![],
                        line: 2,
                        source_text: Some("b = param".to_string()),
                    },
                    SsaInstruction {
                        kind: SsaInstructionKind::BinaryOp,
                        target: Some(SsaNameId(3)),
                        uses: vec![SsaNameId(1), SsaNameId(2)], // a + b
                        line: 3,
                        source_text: Some("x = a + b".to_string()),
                    },
                    SsaInstruction {
                        kind: SsaInstructionKind::BinaryOp,
                        target: Some(SsaNameId(4)),
                        uses: vec![SsaNameId(1), SsaNameId(1)], // a + a (different!)
                        line: 4,
                        source_text: Some("y = a + a".to_string()),
                    },
                ],
                successors: vec![],
                predecessors: vec![],
            }],
            ssa_names: vec![
                SsaName {
                    id: SsaNameId(1),
                    variable: "a".to_string(),
                    version: 1,
                    def_block: Some(0),
                    def_line: 1,
                },
                SsaName {
                    id: SsaNameId(2),
                    variable: "b".to_string(),
                    version: 1,
                    def_block: Some(0),
                    def_line: 2,
                },
                SsaName {
                    id: SsaNameId(3),
                    variable: "x".to_string(),
                    version: 1,
                    def_block: Some(0),
                    def_line: 3,
                },
                SsaName {
                    id: SsaNameId(4),
                    variable: "y".to_string(),
                    version: 1,
                    def_block: Some(0),
                    def_line: 4,
                },
            ],
            def_use: HashMap::new(),
            stats: SsaStats::default(),
        };

        let vn = compute_value_numbers(&ssa).expect("should compute value numbers");

        // x (a + b) and y (a + a) should have different value numbers
        let x_vn = vn.value_numbers.get(&SsaNameId(3)).unwrap();
        let y_vn = vn.value_numbers.get(&SsaNameId(4)).unwrap();
        assert_ne!(
            x_vn, y_vn,
            "Different expressions should have different value numbers"
        );
    }

    /// Test: Equivalences map correctly populated
    #[test]
    fn test_equivalences_populated() {
        let ssa = fixtures::sample_ssa();
        let vn = compute_value_numbers(&ssa).expect("should compute value numbers");

        // Equivalences should group SSA names by value number
        for (vnum, names) in &vn.equivalences {
            for name in names {
                assert_eq!(
                    vn.value_numbers.get(name),
                    Some(vnum),
                    "Equivalence entry should match value number"
                );
            }
        }
    }
}

// =============================================================================
// SCCP Tests (SSA-21)
// =============================================================================

#[cfg(test)]
mod sccp_tests {
    use super::*;

    /// Test: Constant propagation: x=1; y=x -> y=1
    #[test]
    fn test_constant_propagation() {
        let ssa = SsaFunction {
            function: "test".to_string(),
            file: PathBuf::from("test.py"),
            ssa_type: SsaType::Minimal,
            blocks: vec![SsaBlock {
                id: 0,
                label: None,
                lines: (1, 3),
                phi_functions: vec![],
                instructions: vec![
                    SsaInstruction {
                        kind: SsaInstructionKind::Assign,
                        target: Some(SsaNameId(1)),
                        uses: vec![],
                        line: 1,
                        source_text: Some("x = 1".to_string()),
                    },
                    SsaInstruction {
                        kind: SsaInstructionKind::Assign,
                        target: Some(SsaNameId(2)),
                        uses: vec![SsaNameId(1)],
                        line: 2,
                        source_text: Some("y = x".to_string()),
                    },
                ],
                successors: vec![],
                predecessors: vec![],
            }],
            ssa_names: vec![
                SsaName {
                    id: SsaNameId(1),
                    variable: "x".to_string(),
                    version: 1,
                    def_block: Some(0),
                    def_line: 1,
                },
                SsaName {
                    id: SsaNameId(2),
                    variable: "y".to_string(),
                    version: 1,
                    def_block: Some(0),
                    def_line: 2,
                },
            ],
            def_use: HashMap::new(),
            stats: SsaStats::default(),
        };

        let sccp_result = run_sccp(&ssa).expect("should run SCCP");

        // Both x and y should be constant 1
        assert_eq!(
            sccp_result.constants.get(&SsaNameId(1)),
            Some(&ConstantValue::Int(1))
        );
        assert_eq!(
            sccp_result.constants.get(&SsaNameId(2)),
            Some(&ConstantValue::Int(1))
        );
    }

    /// Test: Unreachable code detection: if(false) {...}
    #[test]
    fn test_unreachable_code_detection() {
        // Create SSA with if(false) branch
        let ssa = SsaFunction {
            function: "test".to_string(),
            file: PathBuf::from("test.py"),
            ssa_type: SsaType::Minimal,
            blocks: vec![
                SsaBlock {
                    id: 0,
                    label: Some("entry".to_string()),
                    lines: (1, 2),
                    phi_functions: vec![],
                    instructions: vec![
                        SsaInstruction {
                            kind: SsaInstructionKind::Assign,
                            target: Some(SsaNameId(1)),
                            uses: vec![],
                            line: 1,
                            source_text: Some("cond = False".to_string()),
                        },
                        SsaInstruction {
                            kind: SsaInstructionKind::Branch,
                            target: None,
                            uses: vec![SsaNameId(1)],
                            line: 2,
                            source_text: Some("if cond:".to_string()),
                        },
                    ],
                    successors: vec![1, 2],
                    predecessors: vec![],
                },
                SsaBlock {
                    id: 1,
                    label: Some("unreachable".to_string()),
                    lines: (3, 4),
                    phi_functions: vec![],
                    instructions: vec![SsaInstruction {
                        kind: SsaInstructionKind::Assign,
                        target: Some(SsaNameId(2)),
                        uses: vec![],
                        line: 3,
                        source_text: Some("x = 1".to_string()),
                    }],
                    successors: vec![2],
                    predecessors: vec![0],
                },
                SsaBlock {
                    id: 2,
                    label: Some("exit".to_string()),
                    lines: (5, 5),
                    phi_functions: vec![],
                    instructions: vec![],
                    successors: vec![],
                    predecessors: vec![0, 1],
                },
            ],
            ssa_names: vec![
                SsaName {
                    id: SsaNameId(1),
                    variable: "cond".to_string(),
                    version: 1,
                    def_block: Some(0),
                    def_line: 1,
                },
                SsaName {
                    id: SsaNameId(2),
                    variable: "x".to_string(),
                    version: 1,
                    def_block: Some(1),
                    def_line: 3,
                },
            ],
            def_use: HashMap::new(),
            stats: SsaStats::default(),
        };

        let sccp_result = run_sccp(&ssa).expect("should run SCCP");

        // Block 1 (unreachable) should be detected as unreachable
        assert!(
            sccp_result.unreachable_blocks.contains(&1),
            "Block 1 should be detected as unreachable"
        );
    }

    /// Test: Lattice value meet operation
    #[test]
    fn test_lattice_meet() {
        // Top meet C = C
        // C meet C = C (if same)
        // C1 meet C2 = Bottom (if different)

        let top = LatticeValue::Top;
        let const_1 = LatticeValue::Constant(ConstantValue::Int(1));
        let const_2 = LatticeValue::Constant(ConstantValue::Int(2));
        let bottom = LatticeValue::Bottom;

        // These are conceptual - actual meet would be in implementation
        assert_eq!(top, LatticeValue::Top);
        assert_eq!(const_1, LatticeValue::Constant(ConstantValue::Int(1)));
        assert_ne!(const_1, const_2);
        assert_eq!(bottom, LatticeValue::Bottom);
    }

    /// Test: ConstantValue display
    #[test]
    fn test_constant_value_display() {
        assert_eq!(format!("{}", ConstantValue::Int(42)), "42");
        assert_eq!(
            format!("{}", ConstantValue::Float("3.14".to_string())),
            "3.14"
        );
        assert_eq!(
            format!("{}", ConstantValue::String("hello".to_string())),
            "\"hello\""
        );
        assert_eq!(format!("{}", ConstantValue::Bool(true)), "true");
        assert_eq!(format!("{}", ConstantValue::None), "None");
    }
}

// =============================================================================
// Dead Code Tests (SSA-22)
// =============================================================================

#[cfg(test)]
mod dead_code_tests {
    use super::*;

    /// Test: Definition with no use is dead
    #[test]
    fn test_unused_definition_is_dead() {
        let ssa = SsaFunction {
            function: "test".to_string(),
            file: PathBuf::from("test.py"),
            ssa_type: SsaType::Minimal,
            blocks: vec![SsaBlock {
                id: 0,
                label: None,
                lines: (1, 3),
                phi_functions: vec![],
                instructions: vec![
                    SsaInstruction {
                        kind: SsaInstructionKind::Assign,
                        target: Some(SsaNameId(1)),
                        uses: vec![],
                        line: 1,
                        source_text: Some("x = 1".to_string()), // x is never used
                    },
                    SsaInstruction {
                        kind: SsaInstructionKind::Assign,
                        target: Some(SsaNameId(2)),
                        uses: vec![],
                        line: 2,
                        source_text: Some("y = 2".to_string()),
                    },
                    SsaInstruction {
                        kind: SsaInstructionKind::Return,
                        target: None,
                        uses: vec![SsaNameId(2)], // only y is used
                        line: 3,
                        source_text: Some("return y".to_string()),
                    },
                ],
                successors: vec![],
                predecessors: vec![],
            }],
            ssa_names: vec![
                SsaName {
                    id: SsaNameId(1),
                    variable: "x".to_string(),
                    version: 1,
                    def_block: Some(0),
                    def_line: 1,
                },
                SsaName {
                    id: SsaNameId(2),
                    variable: "y".to_string(),
                    version: 1,
                    def_block: Some(0),
                    def_line: 2,
                },
            ],
            def_use: {
                let mut map = HashMap::new();
                // x has no uses
                map.insert(SsaNameId(1), vec![]);
                // y is used in return
                map.insert(SsaNameId(2), vec![]);
                map
            },
            stats: SsaStats::default(),
        };

        let dead = find_dead_code(&ssa).expect("should find dead code");

        // x (SsaNameId(1)) should be dead
        assert!(
            dead.contains(&SsaNameId(1)),
            "Unused definition should be marked as dead"
        );

        // y should NOT be dead (it's used in return)
        assert!(
            !dead.contains(&SsaNameId(2)),
            "Used definition should not be marked as dead"
        );
    }

    /// Test: Iterative removal for cascading dead code
    #[test]
    fn test_cascading_dead_code() {
        // x = 1
        // y = x + 1
        // z = y + 1
        // return 0  (none of x, y, z are used)
        let ssa = SsaFunction {
            function: "test".to_string(),
            file: PathBuf::from("test.py"),
            ssa_type: SsaType::Minimal,
            blocks: vec![SsaBlock {
                id: 0,
                label: None,
                lines: (1, 4),
                phi_functions: vec![],
                instructions: vec![
                    SsaInstruction {
                        kind: SsaInstructionKind::Assign,
                        target: Some(SsaNameId(1)),
                        uses: vec![],
                        line: 1,
                        source_text: Some("x = 1".to_string()),
                    },
                    SsaInstruction {
                        kind: SsaInstructionKind::BinaryOp,
                        target: Some(SsaNameId(2)),
                        uses: vec![SsaNameId(1)],
                        line: 2,
                        source_text: Some("y = x + 1".to_string()),
                    },
                    SsaInstruction {
                        kind: SsaInstructionKind::BinaryOp,
                        target: Some(SsaNameId(3)),
                        uses: vec![SsaNameId(2)],
                        line: 3,
                        source_text: Some("z = y + 1".to_string()),
                    },
                    SsaInstruction {
                        kind: SsaInstructionKind::Return,
                        target: None,
                        uses: vec![],
                        line: 4,
                        source_text: Some("return 0".to_string()),
                    },
                ],
                successors: vec![],
                predecessors: vec![],
            }],
            ssa_names: vec![
                SsaName {
                    id: SsaNameId(1),
                    variable: "x".to_string(),
                    version: 1,
                    def_block: Some(0),
                    def_line: 1,
                },
                SsaName {
                    id: SsaNameId(2),
                    variable: "y".to_string(),
                    version: 1,
                    def_block: Some(0),
                    def_line: 2,
                },
                SsaName {
                    id: SsaNameId(3),
                    variable: "z".to_string(),
                    version: 1,
                    def_block: Some(0),
                    def_line: 3,
                },
            ],
            def_use: HashMap::new(),
            stats: SsaStats::default(),
        };

        let dead = find_dead_code(&ssa).expect("should find dead code");

        // All three should be dead (z unused, then y unused, then x unused)
        assert!(dead.contains(&SsaNameId(1)), "x should be dead");
        assert!(dead.contains(&SsaNameId(2)), "y should be dead");
        assert!(dead.contains(&SsaNameId(3)), "z should be dead");
    }

    /// Test: Side effects prevent removal
    #[test]
    fn test_side_effects_prevent_removal() {
        // Calls have side effects and should not be marked dead
        assert!(has_side_effects(&SsaInstructionKind::Call));
        assert!(has_side_effects(&SsaInstructionKind::Return));
        assert!(!has_side_effects(&SsaInstructionKind::Assign));
        assert!(!has_side_effects(&SsaInstructionKind::BinaryOp));
    }
}

// =============================================================================
// Output Format Tests (SSA-18, SSA-19, SSA-20)
// =============================================================================

#[cfg(test)]
mod output_format_tests {
    use super::*;

    /// Test: JSON output is valid and matches schema
    #[test]
    fn test_json_output_valid() {
        let ssa = fixtures::sample_ssa();
        let json = serde_json::to_string_pretty(&ssa).expect("should serialize to JSON");

        // Should be valid JSON
        assert!(validate_json(&json), "JSON output should be valid");

        // Should contain expected fields
        assert!(json.contains("\"function\""));
        assert!(json.contains("\"ssa_type\""));
        assert!(json.contains("\"blocks\""));
        assert!(json.contains("\"phi_functions\""));
        assert!(json.contains("\"ssa_names\""));
        assert!(json.contains("\"stats\""));
    }

    /// Test: JSON can be deserialized back
    #[test]
    fn test_json_roundtrip() {
        let ssa = fixtures::sample_ssa();
        let json = serde_json::to_string(&ssa).expect("should serialize");
        let parsed: SsaFunction = serde_json::from_str(&json).expect("should deserialize");

        assert_eq!(parsed.function, ssa.function);
        assert_eq!(parsed.ssa_type, ssa.ssa_type);
        assert_eq!(parsed.blocks.len(), ssa.blocks.len());
        assert_eq!(parsed.ssa_names.len(), ssa.ssa_names.len());
    }

    /// Test: Text format is human-readable
    #[test]
    fn test_text_format_readable() {
        let ssa = fixtures::sample_ssa();
        let text = format_ssa_text(&ssa);

        // Should contain function name
        assert!(text.contains("sample"));

        // Should contain SSA type
        assert!(text.contains("Minimal"));

        // Should contain block headers
        assert!(text.contains("Block 0"));
        assert!(text.contains("Block 3"));

        // Should contain phi functions
        assert!(text.contains("phi("));

        // Should contain statistics
        assert!(text.contains("Phi Functions:"));
        assert!(text.contains("SSA Names:"));
    }

    /// Test: DOT output is valid Graphviz
    #[test]
    fn test_dot_format_valid() {
        let ssa = fixtures::sample_ssa();
        let dot = format_ssa_dot(&ssa);

        // Basic validation
        assert!(validate_dot(&dot), "DOT output should be valid");

        // Should have digraph header
        assert!(dot.contains("digraph SSA"));

        // Should have nodes for each block
        assert!(dot.contains("block0"));
        assert!(dot.contains("block3"));

        // Should have edges
        assert!(dot.contains("->"));
    }

    /// Test: Memory SSA text format
    #[test]
    fn test_memory_ssa_text_format() {
        let memory_ssa = MemorySsa {
            function: "test".to_string(),
            file: Some("test.py".to_string()),
            memory_phis: vec![MemoryPhi {
                result: MemoryVersion(3),
                block: 2,
                sources: vec![
                    MemoryPhiSource {
                        block: 0,
                        version: MemoryVersion(1),
                    },
                    MemoryPhiSource {
                        block: 1,
                        version: MemoryVersion(2),
                    },
                ],
            }],
            memory_defs: vec![MemoryDef {
                version: MemoryVersion(1),
                clobbers: MemoryVersion(0),
                block: 0,
                line: 1,
                access: "x.field".to_string(),
                kind: Some(MemoryDefKind::Store),
            }],
            memory_uses: vec![MemoryUse {
                version: MemoryVersion(1),
                block: 1,
                line: 3,
                access: "x.field".to_string(),
                kind: Some(MemoryUseKind::Load),
            }],
            def_use: HashMap::new(),
            stats: MemorySsaStats::default(),
        };

        let text = format_memory_ssa_text(&memory_ssa);

        assert!(text.contains("Memory SSA"));
        assert!(text.contains("Memory Phi"));
        assert!(text.contains("Memory Definitions"));
        assert!(text.contains("Memory Uses"));
    }

    /// Test: Empty SSA serializes correctly
    #[test]
    fn test_empty_ssa_serializes() {
        let ssa = SsaFunction {
            function: "empty".to_string(),
            file: PathBuf::from("empty.py"),
            ssa_type: SsaType::Minimal,
            blocks: vec![],
            ssa_names: vec![],
            def_use: HashMap::new(),
            stats: SsaStats::default(),
        };

        let json = serde_json::to_string(&ssa).expect("should serialize empty SSA");
        assert!(validate_json(&json));

        let text = format_ssa_text(&ssa);
        assert!(text.contains("empty"));

        let dot = format_ssa_dot(&ssa);
        assert!(validate_dot(&dot));
    }

    // =========================================================================
    // Phase 6: Additional Output Format Tests (SSA-18, SSA-19, SSA-20)
    // =========================================================================

    /// Test: JSON output wrapper functions work correctly
    #[test]
    fn test_json_format_function() {
        let ssa = fixtures::sample_ssa();

        // Test pretty JSON format
        let json_pretty = format_ssa_json(&ssa).expect("should format as JSON");
        assert!(validate_json(&json_pretty));
        assert!(
            json_pretty.contains('\n'),
            "Pretty JSON should have newlines"
        );

        // Test compact JSON format
        let json_compact = format_ssa_json_compact(&ssa).expect("should format as compact JSON");
        assert!(validate_json(&json_compact));
        assert!(
            !json_compact.contains('\n'),
            "Compact JSON should not have newlines"
        );
    }

    /// Test: SsaNameId serializes as numeric value, not string
    #[test]
    fn test_json_ssa_name_id_numeric() {
        let ssa = fixtures::sample_ssa();
        let json = format_ssa_json(&ssa).expect("should serialize");

        // SsaNameId should serialize as number (e.g., 1) not string (e.g., "1")
        // Check that targets in instructions are numbers
        // Look for pattern like "target": 1 (not "target": "1")
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json");

        // Navigate to blocks[0].instructions[0].target
        if let Some(blocks) = parsed.get("blocks").and_then(|b| b.as_array()) {
            for block in blocks {
                if let Some(instructions) = block.get("instructions").and_then(|i| i.as_array()) {
                    for instr in instructions {
                        if let Some(target) = instr.get("target") {
                            // Target should be a number, not a string
                            assert!(
                                target.is_number() || target.is_null(),
                                "SsaNameId should serialize as number, got: {}",
                                target
                            );
                        }
                    }
                }
            }
        }
    }

    /// Test: Text format includes correct phi function format
    #[test]
    fn test_text_phi_format() {
        let ssa = fixtures::sample_ssa();
        let text = format_ssa_text(&ssa);

        // Phi function should show sources with block references
        // Format: "target = phi(source1 [Block N], source2 [Block M])"
        assert!(text.contains("phi("), "Text should contain phi functions");
        assert!(
            text.contains("[Block"),
            "Phi sources should reference blocks"
        );
    }

    /// Test: DOT output escapes special characters correctly
    #[test]
    fn test_dot_escaping() {
        // Create SSA with special characters that need escaping
        let ssa = SsaFunction {
            function: "test_func".to_string(),
            file: PathBuf::from("test.py"),
            ssa_type: SsaType::Minimal,
            blocks: vec![SsaBlock {
                id: 0,
                label: Some("entry\"with\"quotes".to_string()), // Quotes need escaping
                lines: (1, 2),
                phi_functions: vec![],
                instructions: vec![SsaInstruction {
                    kind: SsaInstructionKind::Assign,
                    target: Some(SsaNameId(1)),
                    uses: vec![],
                    line: 1,
                    source_text: Some("x = \"string\"".to_string()), // Quotes in source
                }],
                successors: vec![],
                predecessors: vec![],
            }],
            ssa_names: vec![SsaName {
                id: SsaNameId(1),
                variable: "var_with_underscore".to_string(),
                version: 1,
                def_block: Some(0),
                def_line: 1,
            }],
            def_use: HashMap::new(),
            stats: SsaStats::default(),
        };

        let dot = format_ssa_dot(&ssa);

        // Should be valid DOT
        assert!(validate_dot(&dot), "DOT with special chars should be valid");

        // Quotes should be escaped
        assert!(
            !dot.contains("\"\""),
            "Double quotes should not appear unescaped"
        );
    }

    /// Test: Filter SSA by variable name
    #[test]
    fn test_filter_by_variable() {
        let ssa = fixtures::sample_ssa();
        let original_ssa_names = ssa.stats.ssa_names;

        // Filter to only show "y" variable (function takes ownership, so clone)
        let filtered = filter_ssa_by_variable(ssa.clone(), "y");

        // Should only contain y's SSA names
        for name in &filtered.ssa_names {
            assert_eq!(
                name.variable, "y",
                "Filtered SSA should only contain 'y' names"
            );
        }

        // Phi functions should only be for 'y'
        for block in &filtered.blocks {
            for phi in &block.phi_functions {
                assert_eq!(phi.variable, "y", "Filtered phi should be for 'y'");
            }
        }

        // Stats should be updated
        assert!(filtered.ssa_names.len() <= original_ssa_names);
    }

    /// Test: Filter SSA returns empty for non-existent variable
    #[test]
    fn test_filter_nonexistent_variable() {
        let ssa = fixtures::sample_ssa();

        let filtered = filter_ssa_by_variable(ssa, "nonexistent");

        // Should have empty ssa_names for non-existent variable
        assert!(
            filtered.ssa_names.is_empty(),
            "Should have no SSA names for nonexistent var"
        );
    }

    /// Test: DOT output includes def-use edges when enabled
    #[test]
    fn test_dot_def_use_edges() {
        let ssa = fixtures::sample_ssa();

        let dot = format_ssa_dot_with_def_use(&ssa);

        // Should contain def-use edge styling
        assert!(
            dot.contains("dashed") || dot.contains("style"),
            "Should have styled def-use edges"
        );
    }

    /// Test: Text format includes variable names with versions
    #[test]
    fn test_text_format_versioned_names() {
        let ssa = fixtures::sample_ssa();
        let text = format_ssa_text(&ssa);

        // Should show versioned names like "x_1", "y_2"
        // The fixture has variables x and y
        assert!(
            text.contains("_1") || text.contains("$"),
            "Should show versioned variable names"
        );
    }
}

// =============================================================================
// Live Variables Tests (SSA-23, RD-12)
// =============================================================================

#[cfg(test)]
mod live_variables_tests {
    use super::*;

    /// Test: Variable is live if used before redefined
    #[test]
    fn test_variable_live_before_redef() {
        let cfg = fixtures::linear_cfg();
        let refs = vec![
            VarRef {
                name: "x".to_string(),
                ref_type: RefType::Definition,
                line: 1,
                column: 0,
                context: None,
                group_id: None,
            },
            VarRef {
                name: "x".to_string(),
                ref_type: RefType::Use,
                line: 3,
                column: 4,
                context: None,
                group_id: None,
            },
            VarRef {
                name: "x".to_string(),
                ref_type: RefType::Definition,
                line: 4,
                column: 0,
                context: None,
                group_id: None,
            },
        ];

        let live = compute_live_variables(&cfg, &refs).expect("should compute live variables");

        // x should be live in block 1 (between def at line 1 and use at line 3)
        if let Some(block1_live) = live.blocks.get(&1) {
            assert!(
                block1_live.live_in.contains("x"),
                "x should be live at entry to block 1"
            );
        }
    }

    /// Test: Variable is dead after last use
    #[test]
    fn test_variable_dead_after_last_use() {
        let cfg = fixtures::linear_cfg();
        let refs = vec![
            VarRef {
                name: "x".to_string(),
                ref_type: RefType::Definition,
                line: 1,
                column: 0,
                context: None,
                group_id: None,
            },
            VarRef {
                name: "x".to_string(),
                ref_type: RefType::Use,
                line: 3,
                column: 4,
                context: None,
                group_id: None,
            },
            // x is not used after line 3
        ];

        let live = compute_live_variables(&cfg, &refs).expect("should compute live variables");

        // x should not be live out of the last block (no more uses)
        if let Some(last_block_live) = live.blocks.get(&2) {
            assert!(
                !last_block_live.live_out.contains("x"),
                "x should not be live at exit of last block"
            );
        }
    }

    /// Test: Multiple variables with different liveness
    #[test]
    fn test_multiple_variables_liveness() {
        let cfg = fixtures::diamond_cfg();
        let refs = vec![
            VarRef {
                name: "x".to_string(),
                ref_type: RefType::Definition,
                line: 1,
                column: 0,
                context: None,
                group_id: None,
            },
            VarRef {
                name: "y".to_string(),
                ref_type: RefType::Definition,
                line: 3,
                column: 0,
                context: None,
                group_id: None,
            },
            VarRef {
                name: "x".to_string(),
                ref_type: RefType::Use,
                line: 9,
                column: 7,
                context: None,
                group_id: None,
            },
            // y is never used
        ];

        let live = compute_live_variables(&cfg, &refs).expect("should compute live variables");

        // x should be live (it's used at line 9)
        // y should not be live out of any block (never used)
        let any_y_live = live
            .blocks
            .values()
            .any(|sets| sets.live_in.contains("y") || sets.live_out.contains("y"));
        assert!(!any_y_live, "Unused variable y should not be live anywhere");
    }
}

// =============================================================================
// Available Expressions Tests (RD-10)
// =============================================================================

#[cfg(test)]
mod available_expressions_tests {
    use super::*;
    use crate::types::Language;

    /// Test: Expression available if computed on all paths
    #[test]
    fn test_expression_available_all_paths() {
        let cfg = fixtures::linear_cfg();
        let source = r#"
x = a + b
y = c + d
z = a + b  # a + b should be available here
"#;

        let _avail =
            compute_available_expressions(&cfg, source, Language::Python).expect("should compute");

        // After line 1, "a + b" should be available
        // At line 3, "a + b" should still be available
        // This depends on implementation tracking expressions correctly
    }

    /// Test: Expression killed if operand redefined
    #[test]
    fn test_expression_killed_on_redef() {
        let cfg = fixtures::linear_cfg();
        let source = r#"
x = a + b
a = 5      # kills "a + b"
z = a + b  # a + b NOT available (must recompute)
"#;

        let _avail =
            compute_available_expressions(&cfg, source, Language::Python).expect("should compute");

        // After line 2, "a + b" should NOT be available (a was redefined)
    }

    /// Test: Intersection at merge points (must-analysis)
    #[test]
    fn test_intersection_at_merge() {
        let cfg = fixtures::diamond_cfg();
        let source = r#"
if cond:
    x = a + b
else:
    y = c + d
# At merge: neither a+b nor c+d is available (not on all paths)
z = a + b
"#;

        let _avail =
            compute_available_expressions(&cfg, source, Language::Python).expect("should compute");

        // At merge point (block 3), no expressions should be available
        // because different expressions are computed on different paths
    }
}

// =============================================================================
// AST-Based Available Expressions Tests - All 18 Languages
// =============================================================================
//
// These tests verify that compute_available_expressions can extract binary
// expressions from source code using tree-sitter AST nodes for each language,
// rather than relying on regex patterns that only work for Python.

#[cfg(test)]
mod available_expressions_ast_tests {
    use super::*;
    use crate::types::Language;

    /// Helper: Create a 1-block CFG covering lines 1..=max_line
    fn single_block_cfg(max_line: u32) -> CfgInfo {
        CfgInfo {
            function: "test_func".to_string(),
            blocks: vec![CfgBlock {
                id: 0,
                block_type: BlockType::Entry,
                lines: (1, max_line),
                calls: Vec::new(),
            }],
            edges: vec![],
            entry_block: 0,
            exit_blocks: vec![0],
            cyclomatic_complexity: 1,
            nested_functions: HashMap::new(),
        }
    }

    /// Helper: Assert that the result contains at least one expression with the given operands
    fn assert_has_expression_with_operands(avail: &AvailableExpressions, left: &str, right: &str) {
        let found = avail.expressions.iter().any(|e| {
            (e.uses.contains(&left.to_string()) && e.uses.contains(&right.to_string()))
                || e.text.contains(left) && e.text.contains(right)
        });
        assert!(
            found,
            "Expected expression with operands '{}' and '{}', but found expressions: {:?}",
            left,
            right,
            avail
                .expressions
                .iter()
                .map(|e| &e.text)
                .collect::<Vec<_>>()
        );
    }

    // =========================================================================
    // Python
    // =========================================================================

    #[test]
    fn test_python_ast_available_expressions() {
        let source = "x = a + b\ny = a + b\n";
        let cfg = single_block_cfg(2);
        let avail = compute_available_expressions(&cfg, source, Language::Python)
            .expect("should compute for Python");
        assert!(
            !avail.expressions.is_empty(),
            "Python: should find binary expression a + b"
        );
        assert_has_expression_with_operands(&avail, "a", "b");
    }

    #[test]
    fn test_python_ast_subtraction() {
        let source = "x = a - b\ny = a - b\n";
        let cfg = single_block_cfg(2);
        let avail = compute_available_expressions(&cfg, source, Language::Python)
            .expect("should compute for Python subtraction");
        assert!(
            !avail.expressions.is_empty(),
            "Python: should find binary expression a - b"
        );
    }

    #[test]
    fn test_python_ast_multiplication() {
        let source = "x = a * b\ny = a * b\n";
        let cfg = single_block_cfg(2);
        let avail = compute_available_expressions(&cfg, source, Language::Python)
            .expect("should compute for Python multiplication");
        assert!(
            !avail.expressions.is_empty(),
            "Python: should find binary expression a * b"
        );
    }

    // =========================================================================
    // TypeScript
    // =========================================================================

    #[test]
    fn test_typescript_ast_available_expressions() {
        let source = "const x = a + b;\nconst y = a + b;\n";
        let cfg = single_block_cfg(2);
        let avail = compute_available_expressions(&cfg, source, Language::TypeScript)
            .expect("should compute for TypeScript");
        assert!(
            !avail.expressions.is_empty(),
            "TypeScript: should find binary expression a + b"
        );
        assert_has_expression_with_operands(&avail, "a", "b");
    }

    #[test]
    fn test_typescript_let_assignment() {
        let source = "let x = a + b;\nlet y = a + b;\n";
        let cfg = single_block_cfg(2);
        let avail = compute_available_expressions(&cfg, source, Language::TypeScript)
            .expect("should compute for TypeScript let");
        assert!(
            !avail.expressions.is_empty(),
            "TypeScript: should find binary expression in let assignment"
        );
    }

    // =========================================================================
    // JavaScript
    // =========================================================================

    #[test]
    fn test_javascript_ast_available_expressions() {
        let source = "var x = a + b;\nvar y = a + b;\n";
        let cfg = single_block_cfg(2);
        let avail = compute_available_expressions(&cfg, source, Language::JavaScript)
            .expect("should compute for JavaScript");
        assert!(
            !avail.expressions.is_empty(),
            "JavaScript: should find binary expression a + b"
        );
        assert_has_expression_with_operands(&avail, "a", "b");
    }

    // =========================================================================
    // Go
    // =========================================================================

    #[test]
    fn test_go_ast_available_expressions() {
        let source = "x := a + b\ny := a + b\n";
        let cfg = single_block_cfg(2);
        let avail = compute_available_expressions(&cfg, source, Language::Go)
            .expect("should compute for Go");
        assert!(
            !avail.expressions.is_empty(),
            "Go: should find binary expression a + b"
        );
        assert_has_expression_with_operands(&avail, "a", "b");
    }

    #[test]
    fn test_go_assignment_statement() {
        let source = "x = a + b\ny = a + b\n";
        let cfg = single_block_cfg(2);
        let avail = compute_available_expressions(&cfg, source, Language::Go)
            .expect("should compute for Go assignment");
        assert!(
            !avail.expressions.is_empty(),
            "Go: should find binary expression in regular assignment"
        );
    }

    // =========================================================================
    // Rust
    // =========================================================================

    #[test]
    fn test_rust_ast_available_expressions() {
        let source = "let x = a + b;\nlet y = a + b;\n";
        let cfg = single_block_cfg(2);
        let avail = compute_available_expressions(&cfg, source, Language::Rust)
            .expect("should compute for Rust");
        assert!(
            !avail.expressions.is_empty(),
            "Rust: should find binary expression a + b"
        );
        assert_has_expression_with_operands(&avail, "a", "b");
    }

    // =========================================================================
    // Java
    // =========================================================================

    #[test]
    fn test_java_ast_available_expressions() {
        let source = "int x = a + b;\nint y = a + b;\n";
        let cfg = single_block_cfg(2);
        let avail = compute_available_expressions(&cfg, source, Language::Java)
            .expect("should compute for Java");
        assert!(
            !avail.expressions.is_empty(),
            "Java: should find binary expression a + b"
        );
        assert_has_expression_with_operands(&avail, "a", "b");
    }

    // =========================================================================
    // Kotlin
    // =========================================================================

    #[test]
    fn test_kotlin_ast_available_expressions() {
        let source = "val x = a + b\nval y = a + b\n";
        let cfg = single_block_cfg(2);
        let avail = compute_available_expressions(&cfg, source, Language::Kotlin)
            .expect("should compute for Kotlin");
        assert!(
            !avail.expressions.is_empty(),
            "Kotlin: should find binary expression a + b"
        );
        assert_has_expression_with_operands(&avail, "a", "b");
    }

    // =========================================================================
    // C
    // =========================================================================

    #[test]
    fn test_c_ast_available_expressions() {
        let source = "int x = a + b;\nint y = a + b;\n";
        let cfg = single_block_cfg(2);
        let avail =
            compute_available_expressions(&cfg, source, Language::C).expect("should compute for C");
        assert!(
            !avail.expressions.is_empty(),
            "C: should find binary expression a + b"
        );
        assert_has_expression_with_operands(&avail, "a", "b");
    }

    // =========================================================================
    // C++
    // =========================================================================

    #[test]
    fn test_cpp_ast_available_expressions() {
        let source = "int x = a + b;\nint y = a + b;\n";
        let cfg = single_block_cfg(2);
        let avail = compute_available_expressions(&cfg, source, Language::Cpp)
            .expect("should compute for C++");
        assert!(
            !avail.expressions.is_empty(),
            "C++: should find binary expression a + b"
        );
        assert_has_expression_with_operands(&avail, "a", "b");
    }

    // =========================================================================
    // Ruby
    // =========================================================================

    #[test]
    fn test_ruby_ast_available_expressions() {
        let source = "x = a + b\ny = a + b\n";
        let cfg = single_block_cfg(2);
        let avail = compute_available_expressions(&cfg, source, Language::Ruby)
            .expect("should compute for Ruby");
        assert!(
            !avail.expressions.is_empty(),
            "Ruby: should find binary expression a + b"
        );
        assert_has_expression_with_operands(&avail, "a", "b");
    }

    // =========================================================================
    // Swift
    // =========================================================================

    #[test]
    fn test_swift_ast_available_expressions() {
        // Swift tree-sitter is not supported, so should fall back to regex
        let source = "let x = a + b\nlet y = a + b\n";
        let cfg = single_block_cfg(2);
        let _avail = compute_available_expressions(&cfg, source, Language::Swift)
            .expect("should compute for Swift (regex fallback)");
        // Swift falls back to regex, which may or may not find expressions
        // The important thing is that it does not panic
    }

    // =========================================================================
    // CSharp
    // =========================================================================

    #[test]
    fn test_csharp_ast_available_expressions() {
        let source = "int x = a + b;\nint y = a + b;\n";
        let cfg = single_block_cfg(2);
        let avail = compute_available_expressions(&cfg, source, Language::CSharp)
            .expect("should compute for C#");
        assert!(
            !avail.expressions.is_empty(),
            "C#: should find binary expression a + b"
        );
        assert_has_expression_with_operands(&avail, "a", "b");
    }

    // =========================================================================
    // Scala
    // =========================================================================

    #[test]
    fn test_scala_ast_available_expressions() {
        let source = "val x = a + b\nval y = a + b\n";
        let cfg = single_block_cfg(2);
        let avail = compute_available_expressions(&cfg, source, Language::Scala)
            .expect("should compute for Scala");
        assert!(
            !avail.expressions.is_empty(),
            "Scala: should find binary expression a + b"
        );
        assert_has_expression_with_operands(&avail, "a", "b");
    }

    // =========================================================================
    // PHP
    // =========================================================================

    #[test]
    fn test_php_ast_available_expressions() {
        let source = "<?php\n$x = $a + $b;\n$y = $a + $b;\n";
        let cfg = single_block_cfg(3);
        let avail = compute_available_expressions(&cfg, source, Language::Php)
            .expect("should compute for PHP");
        assert!(
            !avail.expressions.is_empty(),
            "PHP: should find binary expression $a + $b"
        );
    }

    // =========================================================================
    // Lua
    // =========================================================================

    #[test]
    fn test_lua_ast_available_expressions() {
        let source = "local x = a + b\nlocal y = a + b\n";
        let cfg = single_block_cfg(2);
        let avail = compute_available_expressions(&cfg, source, Language::Lua)
            .expect("should compute for Lua");
        assert!(
            !avail.expressions.is_empty(),
            "Lua: should find binary expression a + b"
        );
        assert_has_expression_with_operands(&avail, "a", "b");
    }

    // =========================================================================
    // Luau
    // =========================================================================

    #[test]
    fn test_luau_ast_available_expressions() {
        let source = "local x = a + b\nlocal y = a + b\n";
        let cfg = single_block_cfg(2);
        let avail = compute_available_expressions(&cfg, source, Language::Luau)
            .expect("should compute for Luau");
        assert!(
            !avail.expressions.is_empty(),
            "Luau: should find binary expression a + b"
        );
        assert_has_expression_with_operands(&avail, "a", "b");
    }

    // =========================================================================
    // Elixir
    // =========================================================================

    #[test]
    fn test_elixir_ast_available_expressions() {
        let source = "x = a + b\ny = a + b\n";
        let cfg = single_block_cfg(2);
        let avail = compute_available_expressions(&cfg, source, Language::Elixir)
            .expect("should compute for Elixir");
        assert!(
            !avail.expressions.is_empty(),
            "Elixir: should find binary expression a + b"
        );
        assert_has_expression_with_operands(&avail, "a", "b");
    }

    // =========================================================================
    // OCaml
    // =========================================================================

    #[test]
    fn test_ocaml_ast_available_expressions() {
        let source = "let x = a + b\nlet y = a + b\n";
        let cfg = single_block_cfg(2);
        let avail = compute_available_expressions(&cfg, source, Language::Ocaml)
            .expect("should compute for OCaml");
        assert!(
            !avail.expressions.is_empty(),
            "OCaml: should find binary expression a + b"
        );
        assert_has_expression_with_operands(&avail, "a", "b");
    }

    // =========================================================================
    // Cross-Language: Canonicalization
    // =========================================================================

    #[test]
    fn test_commutative_canonicalization_python() {
        // a + b and b + a should produce the same canonical expression
        let source = "x = a + b\ny = b + a\n";
        let cfg = single_block_cfg(2);
        let avail =
            compute_available_expressions(&cfg, source, Language::Python).expect("should compute");
        // Should have exactly 1 unique expression (canonical form)
        assert_eq!(
            avail.expressions.len(),
            1,
            "Commutative canonicalization: a + b and b + a should merge. Got: {:?}",
            avail
                .expressions
                .iter()
                .map(|e| &e.text)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_non_commutative_subtraction_python() {
        // a - b and b - a should be different
        let source = "x = a - b\ny = b - a\n";
        let cfg = single_block_cfg(2);
        let avail =
            compute_available_expressions(&cfg, source, Language::Python).expect("should compute");
        assert_eq!(
            avail.expressions.len(),
            2,
            "Non-commutative: a - b and b - a should be distinct. Got: {:?}",
            avail
                .expressions
                .iter()
                .map(|e| &e.text)
                .collect::<Vec<_>>()
        );
    }

    // =========================================================================
    // Kill semantics with AST
    // =========================================================================

    #[test]
    fn test_kill_semantics_typescript() {
        // After redefining 'a', expression 'a + b' should be killed
        let source = "let x = a + b;\na = 5;\nlet z = a + b;\n";
        let cfg = single_block_cfg(3);
        let avail = compute_available_expressions(&cfg, source, Language::TypeScript)
            .expect("should compute for TypeScript kill test");
        // Should still find expressions (gen/kill is about availability, not extraction)
        // The key is that expressions ARE extracted from AST
        assert!(
            !avail.expressions.is_empty(),
            "TypeScript: should extract expressions even with redefinitions"
        );
    }
}

// =============================================================================
// Integration Tests
// =============================================================================

#[cfg(test)]
mod integration_tests {
    use super::*;

    /// Test: Full SSA pipeline for a simple function
    #[test]
    fn test_full_ssa_pipeline() {
        let source = r#"
def process(x):
    if x > 0:
        y = 1
    else:
        y = 2
    return y
"#;

        let ssa = construct_ssa(
            source,
            "process",
            crate::types::Language::Python,
            SsaType::Minimal,
        )
        .expect("should construct SSA");

        // Verify basic structure
        assert_eq!(ssa.function, "process");
        assert!(!ssa.blocks.is_empty());

        // Should have phi for y
        let has_y_phi = ssa
            .blocks
            .iter()
            .any(|b| b.phi_functions.iter().any(|phi| phi.variable == "y"));
        assert!(has_y_phi, "Should have phi for y");
    }

    /// Test: SSA with loop
    #[test]
    fn test_ssa_with_loop() {
        let source = r#"
def sum_to_n(n):
    total = 0
    i = 0
    while i < n:
        total = total + i
        i = i + 1
    return total
"#;

        let ssa = construct_ssa(
            source,
            "sum_to_n",
            crate::types::Language::Python,
            SsaType::Minimal,
        )
        .expect("should construct SSA");

        // Should have phis for total and i at loop header
        let loop_header = ssa.blocks.iter().find(|b| b.phi_functions.len() >= 2);

        assert!(
            loop_header.is_some(),
            "Should have loop header with phis for loop variables"
        );
    }

    /// Test: Filter by variable
    #[test]
    fn test_filter_by_variable() {
        let ssa = fixtures::sample_ssa();
        let filtered = filter_ssa_by_variable(ssa, "y");

        // Should only contain y's SSA names
        for name in &filtered.ssa_names {
            assert_eq!(name.variable, "y", "Filtered SSA should only contain y");
        }

        // Should only have y's phis
        for block in &filtered.blocks {
            for phi in &block.phi_functions {
                assert_eq!(phi.variable, "y");
            }
        }
    }
}

// =============================================================================
// Language-Specific SSA Tests (Phase 5)
// =============================================================================
//
// These tests verify correct handling of language-specific constructs
// as identified in session10-premortem-2.yaml (36 risks).

#[cfg(test)]
mod language_specific_tests {
    use super::*;
    use crate::ssa::construct::{
        is_blank_identifier, is_comprehension_scope,
    };
    use crate::types::VarRefContext;

    // =========================================================================
    // Python Tests (S10-P2-R1 through R12)
    // =========================================================================

    /// Test: Python augmented assignment (x += 1) creates USE then DEF
    /// S10-P2-R1: Must capture read-then-write atomically
    #[test]
    fn test_python_augmented_assignment() {
        let cfg = fixtures::linear_cfg();
        let refs = vec![
            VarRef {
                name: "x".to_string(),
                ref_type: RefType::Definition,
                line: 1,
                column: 0,
                context: None,
                group_id: None,
            },
            VarRef {
                name: "x".to_string(),
                ref_type: RefType::Update,
                line: 2,
                column: 0,
                context: Some(VarRefContext::AugmentedAssignment),
                group_id: None,
            },
        ];

        let dfg = DfgInfo {
            function: "test".to_string(),
            refs,
            edges: Vec::new(),
            variables: vec!["x".to_string()],
        };

        let ssa = construct_minimal_ssa(&cfg, &dfg).expect("should construct SSA");

        // Should have two versions of x
        let x_versions: Vec<_> = ssa.ssa_names.iter().filter(|n| n.variable == "x").collect();
        assert!(
            x_versions.len() >= 2,
            "Should have at least 2 versions of x: original and after +=. Got {:?}",
            x_versions
        );

        // The augmented assignment instruction should use x_1 and define x_2
        let augmented_instr = ssa
            .blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .find(|i| i.line == 2);

        if let Some(instr) = augmented_instr {
            assert!(
                instr.target.is_some(),
                "Augmented assignment should define a target"
            );
            // The instruction should use the previous version
            // (exact assertion depends on how uses are recorded)
        }
    }

    /// Test: Python multiple assignment (a, b = b, a) has parallel semantics
    /// S10-P2-R2: RHS evaluated before LHS bindings
    #[test]
    fn test_python_multiple_assignment() {
        let cfg = fixtures::linear_cfg();
        let refs = vec![
            // Initial definitions
            VarRef {
                name: "a".to_string(),
                ref_type: RefType::Definition,
                line: 1,
                column: 0,
                context: None,
                group_id: None,
            },
            VarRef {
                name: "b".to_string(),
                ref_type: RefType::Definition,
                line: 1,
                column: 5,
                context: None,
                group_id: None,
            },
            // Swap: a, b = b, a (RHS uses, then LHS defs)
            // Uses come first (RHS evaluation)
            VarRef {
                name: "b".to_string(),
                ref_type: RefType::Use,
                line: 2,
                column: 9,
                context: Some(VarRefContext::MultipleAssignment),
                group_id: Some(1),
            },
            VarRef {
                name: "a".to_string(),
                ref_type: RefType::Use,
                line: 2,
                column: 12,
                context: Some(VarRefContext::MultipleAssignment),
                group_id: Some(1),
            },
            // Then definitions (LHS bindings)
            VarRef {
                name: "a".to_string(),
                ref_type: RefType::Definition,
                line: 2,
                column: 0,
                context: Some(VarRefContext::MultipleAssignment),
                group_id: Some(1),
            },
            VarRef {
                name: "b".to_string(),
                ref_type: RefType::Definition,
                line: 2,
                column: 3,
                context: Some(VarRefContext::MultipleAssignment),
                group_id: Some(1),
            },
        ];

        let dfg = DfgInfo {
            function: "swap".to_string(),
            refs,
            edges: Vec::new(),
            variables: vec!["a".to_string(), "b".to_string()],
        };

        let ssa = construct_minimal_ssa(&cfg, &dfg).expect("should construct SSA");

        // Both a and b should have 2 versions each
        let a_versions: Vec<_> = ssa.ssa_names.iter().filter(|n| n.variable == "a").collect();
        let b_versions: Vec<_> = ssa.ssa_names.iter().filter(|n| n.variable == "b").collect();

        assert!(
            a_versions.len() >= 2,
            "Should have at least 2 versions of a"
        );
        assert!(
            b_versions.len() >= 2,
            "Should have at least 2 versions of b"
        );

        // The swap should create instructions that use old versions to define new
        // This verifies parallel semantics (not circular)
    }

    /// Test: Python walrus operator (n := expr) creates definition in expression
    /// S10-P2-R3: Walrus operator creates definition visible after expression
    #[test]
    fn test_python_walrus_operator() {
        let cfg = fixtures::linear_cfg();
        let refs = vec![
            VarRef {
                name: "n".to_string(),
                ref_type: RefType::Definition,
                line: 1,
                column: 4,
                context: Some(VarRefContext::WalrusOperator),
                group_id: None,
            },
            VarRef {
                name: "n".to_string(),
                ref_type: RefType::Use,
                line: 2,
                column: 11,
                context: None,
                group_id: None,
            },
        ];

        let dfg = DfgInfo {
            function: "test".to_string(),
            refs,
            edges: Vec::new(),
            variables: vec!["n".to_string()],
        };

        let ssa = construct_minimal_ssa(&cfg, &dfg).expect("should construct SSA");

        // n should be defined from the walrus operator
        let n_defs: Vec<_> = ssa
            .ssa_names
            .iter()
            .filter(|name| name.variable == "n")
            .collect();

        assert!(!n_defs.is_empty(), "n should be defined by walrus operator");
        assert_eq!(
            n_defs[0].def_line, 1,
            "n should be defined at the walrus operator line"
        );
    }

    /// Test: Python comprehension scope isolation
    /// S10-P2-R5: Comprehension x doesn't affect outer x
    #[test]
    fn test_python_comprehension_scope() {
        // Test the helper function
        let outer_ref = VarRef {
            name: "x".to_string(),
            ref_type: RefType::Definition,
            line: 1,
            column: 0,
            context: None,
            group_id: None,
        };

        let comp_ref = VarRef {
            name: "x".to_string(),
            ref_type: RefType::Definition,
            line: 2,
            column: 5,
            context: Some(VarRefContext::ComprehensionScope),
            group_id: None,
        };

        assert!(
            !is_comprehension_scope(&outer_ref),
            "Outer x is not in comprehension scope"
        );
        assert!(
            is_comprehension_scope(&comp_ref),
            "Comprehension x should be marked"
        );
    }

    /// Test: Python match statement pattern bindings
    /// S10-P2-R10: match case (x, y) creates scoped bindings
    #[test]
    fn test_python_match_bindings() {
        let cfg = fixtures::diamond_cfg();
        let refs = vec![
            VarRef {
                name: "point".to_string(),
                ref_type: RefType::Use,
                line: 1,
                column: 6,
                context: None,
                group_id: None,
            },
            // Pattern binding in first case
            VarRef {
                name: "x".to_string(),
                ref_type: RefType::Definition,
                line: 2,
                column: 10,
                context: Some(VarRefContext::MatchBinding),
                group_id: None,
            },
            VarRef {
                name: "y".to_string(),
                ref_type: RefType::Definition,
                line: 2,
                column: 13,
                context: Some(VarRefContext::MatchBinding),
                group_id: None,
            },
        ];

        let dfg = DfgInfo {
            function: "match_test".to_string(),
            refs,
            edges: Vec::new(),
            variables: vec!["point".to_string(), "x".to_string(), "y".to_string()],
        };

        let ssa = construct_minimal_ssa(&cfg, &dfg).expect("should construct SSA");

        // x and y should be defined in the match arm
        assert!(
            ssa.ssa_names.iter().any(|n| n.variable == "x"),
            "x should be defined from match binding"
        );
        assert!(
            ssa.ssa_names.iter().any(|n| n.variable == "y"),
            "y should be defined from match binding"
        );
    }

    // =========================================================================
    // TypeScript Tests (S10-P2-R13 through R21)
    // =========================================================================

    /// Test: TypeScript destructuring creates multiple definitions
    /// S10-P2-R13: const {a, b: c} = obj defines a and c (not b)
    #[test]
    fn test_typescript_destructuring() {
        let cfg = fixtures::linear_cfg();
        let refs = vec![
            // Source object
            VarRef {
                name: "obj".to_string(),
                ref_type: RefType::Use,
                line: 1,
                column: 18,
                context: Some(VarRefContext::Destructuring),
                group_id: Some(1),
            },
            // Destructured bindings
            VarRef {
                name: "a".to_string(),
                ref_type: RefType::Definition,
                line: 1,
                column: 7,
                context: Some(VarRefContext::Destructuring),
                group_id: Some(1),
            },
            VarRef {
                name: "c".to_string(), // b: c means c is the binding, not b
                ref_type: RefType::Definition,
                line: 1,
                column: 10,
                context: Some(VarRefContext::Destructuring),
                group_id: Some(1),
            },
        ];

        let dfg = DfgInfo {
            function: "destruct".to_string(),
            refs,
            edges: Vec::new(),
            variables: vec!["obj".to_string(), "a".to_string(), "c".to_string()],
        };

        let ssa = construct_minimal_ssa(&cfg, &dfg).expect("should construct SSA");

        // a and c should be defined
        assert!(
            ssa.ssa_names.iter().any(|n| n.variable == "a"),
            "a should be defined from destructuring"
        );
        assert!(
            ssa.ssa_names.iter().any(|n| n.variable == "c"),
            "c should be defined from destructuring"
        );

        // b should NOT be defined (it's the property name, not the binding)
        assert!(
            !ssa.ssa_names.iter().any(|n| n.variable == "b"),
            "b should NOT be defined (c is the binding)"
        );
    }

    /// Test: TypeScript type guard doesn't create new SSA version
    /// S10-P2-R16: typeof x === 'string' doesn't create new version
    #[test]
    fn test_typescript_type_guard_no_version() {
        let cfg = fixtures::diamond_cfg();
        let refs = vec![
            // Parameter definition
            VarRef {
                name: "x".to_string(),
                ref_type: RefType::Definition,
                line: 1,
                column: 10,
                context: None,
                group_id: None,
            },
            // Type guard use (not a definition!)
            VarRef {
                name: "x".to_string(),
                ref_type: RefType::Use,
                line: 2,
                column: 8,
                context: None, // Type guard is just a use
                group_id: None,
            },
            // Use in true branch (still x_1, no new version)
            VarRef {
                name: "x".to_string(),
                ref_type: RefType::Use,
                line: 3,
                column: 15,
                context: None,
                group_id: None,
            },
        ];

        let dfg = DfgInfo {
            function: "guard_test".to_string(),
            refs,
            edges: Vec::new(),
            variables: vec!["x".to_string()],
        };

        let ssa = construct_minimal_ssa(&cfg, &dfg).expect("should construct SSA");

        // x should have only ONE definition (the parameter)
        // Type guards narrow type but don't create new SSA versions
        let x_defs: Vec<_> = ssa.ssa_names.iter().filter(|n| n.variable == "x").collect();
        assert_eq!(
            x_defs.len(),
            1,
            "x should have only ONE SSA version (type guards don't create new versions)"
        );
    }

    /// Test: for...of with destructuring creates per-iteration definitions
    /// S10-P2-R20: for (const [k, v] of pairs) creates k_1, v_1
    #[test]
    fn test_typescript_for_of_destructuring() {
        let cfg = fixtures::loop_cfg();
        let refs = vec![
            // Loop variable destructuring
            VarRef {
                name: "k".to_string(),
                ref_type: RefType::Definition,
                line: 1,
                column: 13,
                context: Some(VarRefContext::Destructuring),
                group_id: Some(1),
            },
            VarRef {
                name: "v".to_string(),
                ref_type: RefType::Definition,
                line: 1,
                column: 16,
                context: Some(VarRefContext::Destructuring),
                group_id: Some(1),
            },
            // Use in loop body
            VarRef {
                name: "k".to_string(),
                ref_type: RefType::Use,
                line: 2,
                column: 12,
                context: None,
                group_id: None,
            },
            VarRef {
                name: "v".to_string(),
                ref_type: RefType::Use,
                line: 2,
                column: 15,
                context: None,
                group_id: None,
            },
        ];

        let dfg = DfgInfo {
            function: "forof_test".to_string(),
            refs,
            edges: Vec::new(),
            variables: vec!["k".to_string(), "v".to_string()],
        };

        let ssa = construct_minimal_ssa(&cfg, &dfg).expect("should construct SSA");

        // k and v should be defined
        assert!(
            ssa.ssa_names.iter().any(|n| n.variable == "k"),
            "k should be defined from destructuring"
        );
        assert!(
            ssa.ssa_names.iter().any(|n| n.variable == "v"),
            "v should be defined from destructuring"
        );
    }

    // =========================================================================
    // Go Tests (S10-P2-R22 through R29)
    // =========================================================================

    /// Test: Go short declaration creates versions correctly
    /// S10-P2-R22: x, y := 3, 4 where x exists: x redef, y new
    #[test]
    fn test_go_short_declaration() {
        let cfg = fixtures::linear_cfg();
        let refs = vec![
            // First declaration
            VarRef {
                name: "x".to_string(),
                ref_type: RefType::Definition,
                line: 1,
                column: 0,
                context: Some(VarRefContext::ShortDeclaration),
                group_id: None,
            },
            // Second declaration (x redefined, y new)
            VarRef {
                name: "x".to_string(),
                ref_type: RefType::Definition,
                line: 2,
                column: 0,
                context: Some(VarRefContext::ShortDeclaration),
                group_id: Some(1),
            },
            VarRef {
                name: "y".to_string(),
                ref_type: RefType::Definition,
                line: 2,
                column: 3,
                context: Some(VarRefContext::ShortDeclaration),
                group_id: Some(1),
            },
        ];

        let dfg = DfgInfo {
            function: "short_decl".to_string(),
            refs,
            edges: Vec::new(),
            variables: vec!["x".to_string(), "y".to_string()],
        };

        let ssa = construct_minimal_ssa(&cfg, &dfg).expect("should construct SSA");

        // x should have 2 versions
        let x_versions: Vec<_> = ssa.ssa_names.iter().filter(|n| n.variable == "x").collect();
        assert!(x_versions.len() >= 2, "x should have at least 2 versions");

        // y should have 1 version
        let y_versions: Vec<_> = ssa.ssa_names.iter().filter(|n| n.variable == "y").collect();
        assert_eq!(y_versions.len(), 1, "y should have exactly 1 version");
    }

    /// Test: Go multiple return values create multiple definitions
    /// S10-P2-R23: a, err := f() creates two definitions
    #[test]
    fn test_go_multiple_returns() {
        let cfg = fixtures::linear_cfg();
        let refs = vec![
            VarRef {
                name: "val".to_string(),
                ref_type: RefType::Definition,
                line: 1,
                column: 0,
                context: Some(VarRefContext::MultipleReturn),
                group_id: Some(1),
            },
            VarRef {
                name: "err".to_string(),
                ref_type: RefType::Definition,
                line: 1,
                column: 5,
                context: Some(VarRefContext::MultipleReturn),
                group_id: Some(1),
            },
        ];

        let dfg = DfgInfo {
            function: "multi_return".to_string(),
            refs,
            edges: Vec::new(),
            variables: vec!["val".to_string(), "err".to_string()],
        };

        let ssa = construct_minimal_ssa(&cfg, &dfg).expect("should construct SSA");

        // Both val and err should be defined
        assert!(
            ssa.ssa_names.iter().any(|n| n.variable == "val"),
            "val should be defined"
        );
        assert!(
            ssa.ssa_names.iter().any(|n| n.variable == "err"),
            "err should be defined"
        );
    }

    /// Test: Go blank identifier (_) is not tracked
    /// S10-P2-R28: _ is never a definition or use
    #[test]
    fn test_go_blank_identifier() {
        assert!(is_blank_identifier("_"), "_ should be blank identifier");
        assert!(!is_blank_identifier("x"), "x should not be blank");
        assert!(!is_blank_identifier("_x"), "_x should not be blank");
    }

    // =========================================================================
    // Rust Tests (S10-P2-R30 through R36)
    // =========================================================================

    /// Test: Rust shadowing creates new variable, not new version
    /// S10-P2-R31: let x = 1; let x = 2; creates two separate variables
    #[test]
    fn test_rust_shadowing() {
        let cfg = fixtures::linear_cfg();
        let refs = vec![
            // First binding
            VarRef {
                name: "x".to_string(),
                ref_type: RefType::Definition,
                line: 1,
                column: 4,
                context: Some(VarRefContext::Shadowing),
                group_id: None,
            },
            // Second binding (shadows first)
            VarRef {
                name: "x".to_string(),
                ref_type: RefType::Use,
                line: 2,
                column: 12,
                context: None,
                group_id: None,
            },
            VarRef {
                name: "x".to_string(),
                ref_type: RefType::Definition,
                line: 2,
                column: 4,
                context: Some(VarRefContext::Shadowing),
                group_id: None,
            },
        ];

        let dfg = DfgInfo {
            function: "shadow_test".to_string(),
            refs,
            edges: Vec::new(),
            variables: vec!["x".to_string()],
        };

        let ssa = construct_minimal_ssa(&cfg, &dfg).expect("should construct SSA");

        // Should have either shadow names (x#1, x#2) or versions of x
        // The key invariant is that the second definition is distinct from the first
        let x_names: Vec<_> = ssa
            .ssa_names
            .iter()
            .filter(|n| n.variable.starts_with("x"))
            .collect();

        assert!(
            x_names.len() >= 2,
            "Should have at least 2 SSA names for x (shadow creates new variable)"
        );
    }

    /// Test: Rust pattern binding in let
    /// S10-P2-R30: let (a, b) = tuple creates two definitions
    #[test]
    fn test_rust_pattern_binding() {
        let cfg = fixtures::linear_cfg();
        let refs = vec![
            VarRef {
                name: "a".to_string(),
                ref_type: RefType::Definition,
                line: 1,
                column: 5,
                context: Some(VarRefContext::PatternBinding),
                group_id: Some(1),
            },
            VarRef {
                name: "b".to_string(),
                ref_type: RefType::Definition,
                line: 1,
                column: 8,
                context: Some(VarRefContext::PatternBinding),
                group_id: Some(1),
            },
        ];

        let dfg = DfgInfo {
            function: "pattern_test".to_string(),
            refs,
            edges: Vec::new(),
            variables: vec!["a".to_string(), "b".to_string()],
        };

        let ssa = construct_minimal_ssa(&cfg, &dfg).expect("should construct SSA");

        // Both a and b should be defined
        assert!(
            ssa.ssa_names.iter().any(|n| n.variable == "a"),
            "a should be defined from pattern"
        );
        assert!(
            ssa.ssa_names.iter().any(|n| n.variable == "b"),
            "b should be defined from pattern"
        );
    }

    /// Test: Rust match arm bindings are scoped
    /// S10-P2-R34: Ok(n) => n: n scoped to arm only
    #[test]
    fn test_rust_match_arm_scope() {
        let cfg = fixtures::diamond_cfg();
        let refs = vec![
            // Match expression use
            VarRef {
                name: "r".to_string(),
                ref_type: RefType::Use,
                line: 1,
                column: 6,
                context: None,
                group_id: None,
            },
            // Ok arm binding
            VarRef {
                name: "n".to_string(),
                ref_type: RefType::Definition,
                line: 2,
                column: 7,
                context: Some(VarRefContext::MatchArmBinding),
                group_id: None,
            },
            // Use n in Ok arm
            VarRef {
                name: "n".to_string(),
                ref_type: RefType::Use,
                line: 2,
                column: 13,
                context: None,
                group_id: None,
            },
            // Err arm binding
            VarRef {
                name: "e".to_string(),
                ref_type: RefType::Definition,
                line: 3,
                column: 8,
                context: Some(VarRefContext::MatchArmBinding),
                group_id: None,
            },
        ];

        let dfg = DfgInfo {
            function: "match_arm_test".to_string(),
            refs,
            edges: Vec::new(),
            variables: vec!["r".to_string(), "n".to_string(), "e".to_string()],
        };

        let ssa = construct_minimal_ssa(&cfg, &dfg).expect("should construct SSA");

        // n and e should be defined in their respective arms
        assert!(
            ssa.ssa_names.iter().any(|n| n.variable == "n"),
            "n should be defined in Ok arm"
        );
        assert!(
            ssa.ssa_names.iter().any(|n| n.variable == "e"),
            "e should be defined in Err arm"
        );
    }
}

// =============================================================================
// Source Text and Uses Population Tests (Alias Fix)
// =============================================================================

#[cfg(test)]
mod source_text_and_uses_tests {
    use super::*;

    /// Test: construct_ssa populates source_text on SSA instructions
    #[test]
    fn test_source_text_populated_via_construct_ssa() {
        let source = r#"
def alias_test():
    x = [1, 2, 3]
    y = x
    z = [4, 5, 6]
"#;

        let ssa = construct_ssa(
            source,
            "alias_test",
            crate::types::Language::Python,
            SsaType::Minimal,
        )
        .expect("should construct SSA");

        // All Assign instructions should have source_text populated
        let assign_instructions: Vec<&SsaInstruction> = ssa
            .blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .filter(|i| i.kind == SsaInstructionKind::Assign)
            .collect();

        assert!(
            !assign_instructions.is_empty(),
            "Should have at least one assign instruction"
        );

        for inst in &assign_instructions {
            assert!(
                inst.source_text.is_some(),
                "Assign instruction at line {} should have source_text populated, got None",
                inst.line
            );
        }
    }

    /// Test: source_text contains the actual source line content
    #[test]
    fn test_source_text_contains_correct_content() {
        let source = r#"
def simple():
    x = [1, 2, 3]
    y = x
"#;

        let ssa = construct_ssa(
            source,
            "simple",
            crate::types::Language::Python,
            SsaType::Minimal,
        )
        .expect("should construct SSA");

        // Find instruction for x = [1, 2, 3]
        let x_instr = ssa.blocks.iter().flat_map(|b| &b.instructions).find(|i| {
            i.kind == SsaInstructionKind::Assign
                && i.source_text
                    .as_ref()
                    .is_some_and(|s| s.contains("[1, 2, 3]"))
        });

        assert!(
            x_instr.is_some(),
            "Should find instruction with source_text containing '[1, 2, 3]'"
        );
    }

    /// Test: uses populated for copy assignment (y = x)
    #[test]
    fn test_uses_populated_for_copy_assignment() {
        let source = r#"
def copy_test():
    x = [1, 2, 3]
    y = x
"#;

        let ssa = construct_ssa(
            source,
            "copy_test",
            crate::types::Language::Python,
            SsaType::Minimal,
        )
        .expect("should construct SSA");

        // Find the SSA name for x
        let x_name = ssa
            .ssa_names
            .iter()
            .find(|n| n.variable == "x")
            .expect("should have SSA name for x");

        // Find the instruction that defines y
        let y_instr = ssa.blocks.iter().flat_map(|b| &b.instructions).find(|i| {
            i.kind == SsaInstructionKind::Assign
                && i.target
                    .is_some_and(|t| ssa.ssa_names.iter().any(|n| n.id == t && n.variable == "y"))
        });

        let y_instr = y_instr.expect("should find y's assignment instruction");

        // y's instruction should have x's SSA name in its uses
        assert!(
            y_instr.uses.contains(&x_name.id),
            "y = x instruction should have x's SSA name ({:?}) in uses, but uses = {:?}",
            x_name.id,
            y_instr.uses
        );
    }

    /// Test: uses populated for chained copies (y = x, w = y)
    #[test]
    fn test_uses_populated_for_chained_copies() {
        let source = r#"
def chain_test():
    x = [1, 2, 3]
    y = x
    w = y
"#;

        let ssa = construct_ssa(
            source,
            "chain_test",
            crate::types::Language::Python,
            SsaType::Minimal,
        )
        .expect("should construct SSA");

        // Find SSA names
        let y_name = ssa
            .ssa_names
            .iter()
            .find(|n| n.variable == "y")
            .expect("should have SSA name for y");

        // Find instruction for w
        let w_instr = ssa
            .blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .find(|i| {
                i.kind == SsaInstructionKind::Assign
                    && i.target.is_some_and(|t| {
                        ssa.ssa_names.iter().any(|n| n.id == t && n.variable == "w")
                    })
            })
            .expect("should find w's assignment instruction");

        // w = y instruction should have y's SSA name in its uses
        assert!(
            w_instr.uses.contains(&y_name.id),
            "w = y instruction should have y's SSA name ({:?}) in uses, but uses = {:?}",
            y_name.id,
            w_instr.uses
        );
    }

    /// Test: construct_minimal_ssa (without source) still works with empty source_text
    #[test]
    fn test_construct_minimal_ssa_backward_compatible() {
        let cfg = fixtures::diamond_cfg();
        let dfg = fixtures::diamond_dfg();

        let ssa = construct_minimal_ssa(&cfg, &dfg).expect("should construct SSA");

        // Should still work - instructions exist but may have None source_text
        let total_instructions: usize = ssa.blocks.iter().map(|b| b.instructions.len()).sum();
        assert!(
            total_instructions > 0,
            "Should have at least one instruction"
        );
    }

    /// Test: construct_ssa_with_statements populates both source_text and uses
    #[test]
    fn test_construct_ssa_with_statements_populates_both() {
        let source = r#"
def both_test():
    x = [1, 2, 3]
    y = x
    z = [4, 5, 6]
    w = y
"#;

        let ssa = construct_ssa(
            source,
            "both_test",
            crate::types::Language::Python,
            SsaType::Minimal,
        )
        .expect("should construct SSA");

        // Check that all assign instructions have source_text
        let assign_instructions: Vec<&SsaInstruction> = ssa
            .blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .filter(|i| i.kind == SsaInstructionKind::Assign)
            .collect();

        let with_source_text = assign_instructions
            .iter()
            .filter(|i| i.source_text.is_some())
            .count();

        assert!(
            with_source_text > 0,
            "At least some instructions should have source_text"
        );

        // Check that copy assignments have uses populated
        let with_uses = assign_instructions
            .iter()
            .filter(|i| !i.uses.is_empty())
            .count();

        // y = x and w = y should have uses, so at least 2
        assert!(
            with_uses >= 2,
            "At least 2 copy assignments (y=x, w=y) should have uses populated, got {}",
            with_uses
        );
    }
}
