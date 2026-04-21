//! Dataflow Analysis Foundation Types & CFG Helpers
//!
//! This module provides shared types and helper functions for dataflow analyses:
//!
//! - `BlockId`: Type alias matching existing CFG types (usize)
//! - `DataflowError`: Error types for dataflow analyses
//! - `build_predecessors`: Build predecessor map from CFG
//! - `find_back_edges`: Identify loop header blocks using dominance
//! - `reverse_postorder`: Compute efficient iteration order
//!
//! # Mitigations Addressed
//!
//! - TIGER-PASS1-8: Use usize for BlockId to match existing CFG types
//! - TIGER-PASS1-9: Centralize CFG helper functions
//! - TIGER-PASS3-4: Add MAX_BLOCKS constant for pathological CFG defense
//! - TIGER-PASS1-6: Implement find_back_edges using dominance

use std::collections::{HashMap, HashSet, VecDeque};

use crate::error::TldrError;
use crate::ssa::dominators::build_dominator_tree;
use crate::types::CfgInfo;

// =============================================================================
// Type Aliases
// =============================================================================

/// Block ID type alias matching existing CFG types.
///
/// Using `usize` to match CfgBlock.id and CfgEdge.from/to.
/// TIGER-PASS1-8: Use consistent types across CFG and dataflow modules.
pub type BlockId = usize;

// =============================================================================
// Constants
// =============================================================================

/// Maximum number of CFG blocks before analysis is refused.
///
/// TIGER-PASS3-4: Defense against pathological CFGs (e.g., generated code).
/// Chosen based on typical function sizes; functions with >10k blocks
/// are likely generated or should be split.
pub const MAX_BLOCKS: usize = 10_000;

/// Maximum fixpoint iterations before giving up.
///
/// This is a base limit that will be dynamically adjusted based on:
/// - Available Expressions: blocks * expressions * 2 + 10
/// - Abstract Interpretation: blocks * 10 + 100
///
/// The base limit provides a safety bound for edge cases.
pub const MAX_ITERATIONS: usize = 100;

// =============================================================================
// Error Types
// =============================================================================

/// Errors specific to dataflow analyses.
///
/// These errors are designed to be converted to TldrError for consistency
/// with the rest of the codebase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataflowError {
    /// CFG is required but not provided or empty
    NoCfg,

    /// DFG is required but not provided or empty
    NoDfg,

    /// CFG exceeds MAX_BLOCKS limit (TIGER-PASS3-4)
    TooManyBlocks {
        /// Number of blocks in the CFG
        count: usize,
    },

    /// Analysis did not converge within iteration limit
    IterationLimit {
        /// Number of iterations performed before giving up
        iterations: usize,
    },

    /// CFG pattern not supported (e.g., exception edges)
    UnsupportedCfgPattern {
        /// Description of the unsupported pattern
        pattern: String,
    },
}

impl std::fmt::Display for DataflowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DataflowError::NoCfg => write!(f, "CFG is required but not provided or empty"),
            DataflowError::NoDfg => write!(f, "DFG is required but not provided or empty"),
            DataflowError::TooManyBlocks { count } => {
                write!(
                    f,
                    "CFG has {} blocks, exceeds maximum of {} (TIGER-PASS3-4)",
                    count, MAX_BLOCKS
                )
            }
            DataflowError::IterationLimit { iterations } => {
                write!(
                    f,
                    "Dataflow analysis did not converge after {} iterations",
                    iterations
                )
            }
            DataflowError::UnsupportedCfgPattern { pattern } => {
                write!(f, "Unsupported CFG pattern: {}", pattern)
            }
        }
    }
}

impl std::error::Error for DataflowError {}

impl From<DataflowError> for TldrError {
    fn from(err: DataflowError) -> Self {
        TldrError::InvalidArgs {
            arg: "dataflow".to_string(),
            message: err.to_string(),
            suggestion: None,
        }
    }
}

// =============================================================================
// CFG Helper Functions
// =============================================================================

/// Build a predecessor map from a CFG.
///
/// For each block, returns the list of blocks that have edges pointing to it.
/// This is the inverse of the successor relationship in CFG edges.
///
/// # Arguments
///
/// * `cfg` - The control flow graph
///
/// # Returns
///
/// HashMap where keys are block IDs and values are vectors of predecessor block IDs.
///
/// # Example
///
/// ```rust,ignore
/// let preds = build_predecessors(&cfg);
/// // For block 2, get all blocks that can jump to it
/// let block_2_preds = preds.get(&2).unwrap_or(&vec![]);
/// ```
///
/// # TIGER Mitigation
///
/// - TIGER-PASS1-9: Centralized helper function
/// - TIGER-PASS1-8: Returns HashMap<usize, Vec<usize>> matching CFG types
pub fn build_predecessors(cfg: &CfgInfo) -> HashMap<BlockId, Vec<BlockId>> {
    let mut predecessors: HashMap<BlockId, Vec<BlockId>> = HashMap::new();

    // Initialize all blocks with empty predecessor lists
    for block in &cfg.blocks {
        predecessors.entry(block.id).or_default();
    }

    // Populate from edges
    for edge in &cfg.edges {
        predecessors.entry(edge.to).or_default().push(edge.from);
    }

    predecessors
}

/// Build a successor map from a CFG.
///
/// For each block, returns the list of blocks that it has edges to.
/// This mirrors the edge structure in the CFG.
///
/// # Arguments
///
/// * `cfg` - The control flow graph
///
/// # Returns
///
/// HashMap where keys are block IDs and values are vectors of successor block IDs.
pub fn build_successors(cfg: &CfgInfo) -> HashMap<BlockId, Vec<BlockId>> {
    let mut successors: HashMap<BlockId, Vec<BlockId>> = HashMap::new();

    // Initialize all blocks with empty successor lists
    for block in &cfg.blocks {
        successors.entry(block.id).or_default();
    }

    // Populate from edges
    for edge in &cfg.edges {
        successors.entry(edge.from).or_default().push(edge.to);
    }

    successors
}

/// Find back edges and return the set of loop header block IDs.
///
/// A back edge is an edge from a block to one of its dominators in the CFG.
/// The target of a back edge is a loop header.
///
/// # Algorithm
///
/// 1. Build dominator tree using Lengauer-Tarjan algorithm
/// 2. For each edge (u -> v), check if v dominates u
/// 3. If so, (u -> v) is a back edge and v is a loop header
///
/// # Arguments
///
/// * `cfg` - The control flow graph
///
/// # Returns
///
/// HashSet of block IDs that are loop headers (targets of back edges).
///
/// # Errors
///
/// Returns empty set if dominator tree cannot be built (e.g., empty CFG).
///
/// # TIGER Mitigation
///
/// - TIGER-PASS1-5: Identifies loop headers for widening application
/// - TIGER-PASS1-6: Uses dominance-based back edge detection
///
/// # Example
///
/// ```rust,ignore
/// let loop_headers = find_back_edges(&cfg);
/// if loop_headers.contains(&block_id) {
///     // Apply widening at this block
///     state = widen_state(&old_state, &new_state);
/// }
/// ```
pub fn find_back_edges(cfg: &CfgInfo) -> HashSet<BlockId> {
    let mut loop_headers = HashSet::new();

    // Handle empty or trivial CFG
    if cfg.blocks.is_empty() {
        return loop_headers;
    }

    // Build dominator tree
    let dom_tree = match build_dominator_tree(cfg) {
        Ok(tree) => tree,
        Err(_) => return loop_headers, // Return empty set on error
    };

    // Check each edge for back edge property
    for edge in &cfg.edges {
        // An edge (u -> v) is a back edge if v dominates u
        if dom_tree.dominates(edge.to, edge.from) {
            loop_headers.insert(edge.to);
        }
    }

    loop_headers
}

/// Compute reverse postorder traversal of the CFG.
///
/// Reverse postorder is an efficient iteration order for forward dataflow
/// analysis because it ensures we process predecessors before successors
/// (except for back edges).
///
/// # Algorithm
///
/// 1. Perform DFS from entry, recording postorder (visit order when leaving)
/// 2. Reverse the postorder to get reverse postorder
///
/// # Arguments
///
/// * `cfg` - The control flow graph
///
/// # Returns
///
/// Vector of block IDs in reverse postorder.
///
/// # Properties
///
/// - Entry block is first (unless unreachable from entry)
/// - For acyclic CFGs, all predecessors appear before successors
/// - Improves convergence speed for dataflow analysis
///
/// # Example
///
/// ```rust,ignore
/// let order = reverse_postorder(&cfg);
/// for block_id in &order {
///     // Process blocks in efficient order
///     process_block(block_id, &state);
/// }
/// ```
pub fn reverse_postorder(cfg: &CfgInfo) -> Vec<BlockId> {
    if cfg.blocks.is_empty() {
        return Vec::new();
    }

    let successors = build_successors(cfg);
    let entry = cfg.entry_block;

    // DFS to compute postorder
    let mut postorder = Vec::new();
    let mut visited = HashSet::new();
    let mut stack = vec![(entry, false)];

    while let Some((block, finished)) = stack.pop() {
        if finished {
            postorder.push(block);
            continue;
        }

        if visited.contains(&block) {
            continue;
        }
        visited.insert(block);

        // Push marker for when we finish this node
        stack.push((block, true));

        // Push successors (in reverse for consistent ordering)
        if let Some(succs) = successors.get(&block) {
            for &succ in succs.iter().rev() {
                if !visited.contains(&succ) {
                    stack.push((succ, false));
                }
            }
        }
    }

    // Reverse to get reverse postorder
    postorder.reverse();
    postorder
}

/// Validate that a CFG is suitable for dataflow analysis.
///
/// Checks:
/// - CFG is not empty
/// - Number of blocks does not exceed MAX_BLOCKS
///
/// # Arguments
///
/// * `cfg` - The control flow graph to validate
///
/// # Returns
///
/// Ok(()) if valid, Err(DataflowError) otherwise.
///
/// # TIGER Mitigation
///
/// - TIGER-PASS3-4: Reject pathological CFGs early
pub fn validate_cfg(cfg: &CfgInfo) -> Result<(), DataflowError> {
    if cfg.blocks.is_empty() {
        return Err(DataflowError::NoCfg);
    }

    if cfg.blocks.len() > MAX_BLOCKS {
        return Err(DataflowError::TooManyBlocks {
            count: cfg.blocks.len(),
        });
    }

    Ok(())
}

/// Compute the reachable blocks from entry.
///
/// Returns the set of block IDs that are reachable from the entry block.
/// Useful for filtering out unreachable code in analysis results.
///
/// # Arguments
///
/// * `cfg` - The control flow graph
///
/// # Returns
///
/// HashSet of block IDs reachable from entry.
pub fn reachable_blocks(cfg: &CfgInfo) -> HashSet<BlockId> {
    if cfg.blocks.is_empty() {
        return HashSet::new();
    }

    let successors = build_successors(cfg);
    let entry = cfg.entry_block;

    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();
    queue.push_back(entry);

    while let Some(block) = queue.pop_front() {
        if visited.contains(&block) {
            continue;
        }
        visited.insert(block);

        if let Some(succs) = successors.get(&block) {
            for &succ in succs {
                if !visited.contains(&succ) {
                    queue.push_back(succ);
                }
            }
        }
    }

    visited
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BlockType, CfgBlock, CfgEdge, EdgeType};

    /// Helper to create a minimal CFG for testing
    fn make_test_cfg(
        blocks: Vec<(usize, BlockType)>,
        edges: Vec<(usize, usize)>,
        entry: usize,
    ) -> CfgInfo {
        CfgInfo {
            function: "test".to_string(),
            blocks: blocks
                .into_iter()
                .map(|(id, block_type)| CfgBlock {
                    id,
                    block_type,
                    lines: (1, 10),
                    calls: vec![],
                })
                .collect(),
            edges: edges
                .into_iter()
                .map(|(from, to)| CfgEdge {
                    from,
                    to,
                    edge_type: EdgeType::Unconditional,
                    condition: None,
                })
                .collect(),
            entry_block: entry,
            exit_blocks: vec![],
            cyclomatic_complexity: 1,
            nested_functions: HashMap::new(),
        }
    }

    // =========================================================================
    // BlockId Type Tests
    // =========================================================================

    #[test]
    fn test_block_id_is_usize() {
        // TIGER-PASS1-8: BlockId must be usize to match CFG types
        let block_id: BlockId = 42;
        let as_usize: usize = block_id;
        assert_eq!(as_usize, 42);
    }

    // =========================================================================
    // Constants Tests
    // =========================================================================

    #[test]
    fn test_max_blocks_is_10000() {
        // Acceptance criteria: MAX_BLOCKS = 10000
        assert_eq!(MAX_BLOCKS, 10_000);
    }

    #[test]
    fn test_max_iterations_is_100() {
        // Base limit for iterations
        assert_eq!(MAX_ITERATIONS, 100);
    }

    // =========================================================================
    // DataflowError Tests
    // =========================================================================

    #[test]
    fn test_dataflow_error_no_cfg() {
        let err = DataflowError::NoCfg;
        assert!(err.to_string().contains("CFG"));
    }

    #[test]
    fn test_dataflow_error_no_dfg() {
        let err = DataflowError::NoDfg;
        assert!(err.to_string().contains("DFG"));
    }

    #[test]
    fn test_dataflow_error_too_many_blocks() {
        let err = DataflowError::TooManyBlocks { count: 15000 };
        assert!(err.to_string().contains("15000"));
        assert!(err.to_string().contains("10000"));
    }

    #[test]
    fn test_dataflow_error_iteration_limit() {
        let err = DataflowError::IterationLimit { iterations: 500 };
        assert!(err.to_string().contains("500"));
    }

    #[test]
    fn test_dataflow_error_unsupported_pattern() {
        let err = DataflowError::UnsupportedCfgPattern {
            pattern: "exception edges".to_string(),
        };
        assert!(err.to_string().contains("exception edges"));
    }

    #[test]
    fn test_dataflow_error_equality() {
        let err1 = DataflowError::TooManyBlocks { count: 100 };
        let err2 = DataflowError::TooManyBlocks { count: 100 };
        let err3 = DataflowError::TooManyBlocks { count: 200 };
        assert_eq!(err1, err2);
        assert_ne!(err1, err3);
    }

    #[test]
    fn test_dataflow_error_to_tldr_error() {
        let err = DataflowError::NoCfg;
        let tldr_err: TldrError = err.into();
        match tldr_err {
            TldrError::InvalidArgs { message, .. } => {
                assert!(message.contains("CFG"));
            }
            _ => panic!("Expected InvalidArgs error"),
        }
    }

    // =========================================================================
    // build_predecessors Tests
    // =========================================================================

    #[test]
    fn test_build_predecessors_empty_cfg() {
        let cfg = make_test_cfg(vec![], vec![], 0);
        let preds = build_predecessors(&cfg);
        assert!(preds.is_empty());
    }

    #[test]
    fn test_build_predecessors_single_block() {
        let cfg = make_test_cfg(vec![(0, BlockType::Entry)], vec![], 0);
        let preds = build_predecessors(&cfg);
        assert_eq!(preds.len(), 1);
        assert!(preds.get(&0).unwrap().is_empty());
    }

    #[test]
    fn test_build_predecessors_linear_cfg() {
        // 0 -> 1 -> 2
        let cfg = make_test_cfg(
            vec![
                (0, BlockType::Entry),
                (1, BlockType::Body),
                (2, BlockType::Exit),
            ],
            vec![(0, 1), (1, 2)],
            0,
        );
        let preds = build_predecessors(&cfg);

        assert!(preds.get(&0).unwrap().is_empty());
        assert_eq!(preds.get(&1).unwrap(), &vec![0]);
        assert_eq!(preds.get(&2).unwrap(), &vec![1]);
    }

    #[test]
    fn test_build_predecessors_diamond_cfg() {
        //     0
        //    / \
        //   1   2
        //    \ /
        //     3
        let cfg = make_test_cfg(
            vec![
                (0, BlockType::Entry),
                (1, BlockType::Body),
                (2, BlockType::Body),
                (3, BlockType::Exit),
            ],
            vec![(0, 1), (0, 2), (1, 3), (2, 3)],
            0,
        );
        let preds = build_predecessors(&cfg);

        assert!(preds.get(&0).unwrap().is_empty());
        assert_eq!(preds.get(&1).unwrap(), &vec![0]);
        assert_eq!(preds.get(&2).unwrap(), &vec![0]);

        let preds_3 = preds.get(&3).unwrap();
        assert_eq!(preds_3.len(), 2);
        assert!(preds_3.contains(&1));
        assert!(preds_3.contains(&2));
    }

    #[test]
    fn test_build_predecessors_returns_correct_type() {
        // Acceptance criteria: build_predecessors returns HashMap<usize, Vec<usize>>
        let cfg = make_test_cfg(vec![(0, BlockType::Entry)], vec![], 0);
        let preds: HashMap<usize, Vec<usize>> = build_predecessors(&cfg);
        assert!(preds.contains_key(&0));
    }

    // =========================================================================
    // build_successors Tests
    // =========================================================================

    #[test]
    fn test_build_successors_linear_cfg() {
        // 0 -> 1 -> 2
        let cfg = make_test_cfg(
            vec![
                (0, BlockType::Entry),
                (1, BlockType::Body),
                (2, BlockType::Exit),
            ],
            vec![(0, 1), (1, 2)],
            0,
        );
        let succs = build_successors(&cfg);

        assert_eq!(succs.get(&0).unwrap(), &vec![1]);
        assert_eq!(succs.get(&1).unwrap(), &vec![2]);
        assert!(succs.get(&2).unwrap().is_empty());
    }

    // =========================================================================
    // find_back_edges Tests
    // =========================================================================

    #[test]
    fn test_find_back_edges_empty_cfg() {
        let cfg = make_test_cfg(vec![], vec![], 0);
        let headers = find_back_edges(&cfg);
        assert!(headers.is_empty());
    }

    #[test]
    fn test_find_back_edges_no_loops() {
        // 0 -> 1 -> 2 (no back edges)
        let cfg = make_test_cfg(
            vec![
                (0, BlockType::Entry),
                (1, BlockType::Body),
                (2, BlockType::Exit),
            ],
            vec![(0, 1), (1, 2)],
            0,
        );
        let headers = find_back_edges(&cfg);
        assert!(headers.is_empty());
    }

    #[test]
    fn test_find_back_edges_simple_loop() {
        // 0 -> 1 -> 2
        //      ^    |
        //      +----+
        // Back edge: 2 -> 1, so 1 is loop header
        let cfg = make_test_cfg(
            vec![
                (0, BlockType::Entry),
                (1, BlockType::LoopHeader),
                (2, BlockType::LoopBody),
            ],
            vec![(0, 1), (1, 2), (2, 1)],
            0,
        );
        let headers = find_back_edges(&cfg);
        assert_eq!(headers.len(), 1);
        assert!(headers.contains(&1));
    }

    #[test]
    fn test_find_back_edges_self_loop() {
        // 0 -> 1
        //      ^|
        //      +
        // Self-loop: 1 -> 1, so 1 is loop header
        let cfg = make_test_cfg(
            vec![(0, BlockType::Entry), (1, BlockType::LoopHeader)],
            vec![(0, 1), (1, 1)],
            0,
        );
        let headers = find_back_edges(&cfg);
        assert_eq!(headers.len(), 1);
        assert!(headers.contains(&1));
    }

    #[test]
    fn test_find_back_edges_returns_hashset_usize() {
        // Acceptance criteria: find_back_edges returns HashSet<usize>
        let cfg = make_test_cfg(vec![(0, BlockType::Entry)], vec![], 0);
        let headers: HashSet<usize> = find_back_edges(&cfg);
        assert!(headers.is_empty());
    }

    // =========================================================================
    // reverse_postorder Tests
    // =========================================================================

    #[test]
    fn test_reverse_postorder_empty_cfg() {
        let cfg = make_test_cfg(vec![], vec![], 0);
        let order = reverse_postorder(&cfg);
        assert!(order.is_empty());
    }

    #[test]
    fn test_reverse_postorder_single_block() {
        let cfg = make_test_cfg(vec![(0, BlockType::Entry)], vec![], 0);
        let order = reverse_postorder(&cfg);
        assert_eq!(order, vec![0]);
    }

    #[test]
    fn test_reverse_postorder_linear_cfg() {
        // 0 -> 1 -> 2
        let cfg = make_test_cfg(
            vec![
                (0, BlockType::Entry),
                (1, BlockType::Body),
                (2, BlockType::Exit),
            ],
            vec![(0, 1), (1, 2)],
            0,
        );
        let order = reverse_postorder(&cfg);
        assert_eq!(order, vec![0, 1, 2]);
    }

    #[test]
    fn test_reverse_postorder_diamond_cfg() {
        //     0
        //    / \
        //   1   2
        //    \ /
        //     3
        let cfg = make_test_cfg(
            vec![
                (0, BlockType::Entry),
                (1, BlockType::Body),
                (2, BlockType::Body),
                (3, BlockType::Exit),
            ],
            vec![(0, 1), (0, 2), (1, 3), (2, 3)],
            0,
        );
        let order = reverse_postorder(&cfg);

        // Entry should be first
        assert_eq!(order[0], 0);
        // Exit should be last
        assert_eq!(order[3], 3);
        // 1 and 2 should be between entry and exit
        assert!(order[1] == 1 || order[1] == 2);
        assert!(order[2] == 1 || order[2] == 2);
    }

    #[test]
    fn test_reverse_postorder_entry_first() {
        // Entry block should always be first in reverse postorder
        let cfg = make_test_cfg(
            vec![
                (0, BlockType::Entry),
                (1, BlockType::Body),
                (2, BlockType::Exit),
            ],
            vec![(0, 1), (0, 2)],
            0,
        );
        let order = reverse_postorder(&cfg);
        assert_eq!(order[0], cfg.entry_block);
    }

    #[test]
    fn test_reverse_postorder_returns_vec_usize() {
        // Acceptance criteria: reverse_postorder returns Vec<usize>
        let cfg = make_test_cfg(vec![(0, BlockType::Entry)], vec![], 0);
        let order: Vec<usize> = reverse_postorder(&cfg);
        assert!(!order.is_empty());
    }

    // =========================================================================
    // validate_cfg Tests
    // =========================================================================

    #[test]
    fn test_validate_cfg_empty() {
        let cfg = make_test_cfg(vec![], vec![], 0);
        let result = validate_cfg(&cfg);
        assert_eq!(result, Err(DataflowError::NoCfg));
    }

    #[test]
    fn test_validate_cfg_valid() {
        let cfg = make_test_cfg(vec![(0, BlockType::Entry)], vec![], 0);
        let result = validate_cfg(&cfg);
        assert!(result.is_ok());
    }

    // Note: Testing TooManyBlocks would require creating a large CFG,
    // which is slow. The constant check above verifies the limit.

    // =========================================================================
    // reachable_blocks Tests
    // =========================================================================

    #[test]
    fn test_reachable_blocks_empty_cfg() {
        let cfg = make_test_cfg(vec![], vec![], 0);
        let reachable = reachable_blocks(&cfg);
        assert!(reachable.is_empty());
    }

    #[test]
    fn test_reachable_blocks_all_reachable() {
        let cfg = make_test_cfg(
            vec![
                (0, BlockType::Entry),
                (1, BlockType::Body),
                (2, BlockType::Exit),
            ],
            vec![(0, 1), (1, 2)],
            0,
        );
        let reachable = reachable_blocks(&cfg);
        assert_eq!(reachable.len(), 3);
        assert!(reachable.contains(&0));
        assert!(reachable.contains(&1));
        assert!(reachable.contains(&2));
    }

    #[test]
    fn test_reachable_blocks_with_unreachable() {
        // Block 2 is not reachable from entry
        let cfg = make_test_cfg(
            vec![
                (0, BlockType::Entry),
                (1, BlockType::Exit),
                (2, BlockType::Body), // Unreachable
            ],
            vec![(0, 1)],
            0,
        );
        let reachable = reachable_blocks(&cfg);
        assert_eq!(reachable.len(), 2);
        assert!(reachable.contains(&0));
        assert!(reachable.contains(&1));
        assert!(!reachable.contains(&2));
    }
}
