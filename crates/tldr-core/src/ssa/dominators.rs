//! Dominator Tree and Dominance Frontier
//!
//! Implements the Lengauer-Tarjan algorithm for computing dominators
//! and the standard algorithm for dominance frontiers.
//!
//! # Algorithm
//!
//! The Lengauer-Tarjan algorithm computes immediate dominators in
//! O(E * alpha(E, V)) time where alpha is the inverse Ackermann function.
//!
//! # References
//!
//! - Lengauer & Tarjan (1979) - "A Fast Algorithm for Finding Dominators"

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use crate::error::TldrError;
use crate::types::CfgInfo;
use crate::TldrResult;

/// Node in the dominator tree
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DominatorNode {
    /// Block ID this node represents
    pub block_id: usize,
    /// Immediate dominator (None for entry block)
    pub idom: Option<usize>,
    /// Blocks immediately dominated by this node
    pub children: Vec<usize>,
    /// Depth in dominator tree (entry = 0)
    pub depth: u32,
}

/// Complete dominator tree for a function
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DominatorTree {
    /// Function name
    pub function: String,
    /// Nodes indexed by block ID
    pub nodes: HashMap<usize, DominatorNode>,
    /// Entry block ID
    pub entry: usize,
    /// Preorder numbering for fast dominance queries
    pub preorder: Vec<usize>,
    /// Postorder numbering for fast dominance queries
    pub postorder: Vec<usize>,
}

impl DominatorTree {
    /// Check if block a dominates block b
    pub fn dominates(&self, a: usize, b: usize) -> bool {
        if a == b {
            return true;
        }
        // Walk up from b to entry, checking if we hit a
        let mut current = b;
        while let Some(node) = self.nodes.get(&current) {
            if let Some(idom) = node.idom {
                if idom == a {
                    return true;
                }
                current = idom;
            } else {
                break;
            }
        }
        false
    }

    /// Check if block a strictly dominates block b (a dominates b and a != b)
    pub fn strictly_dominates(&self, a: usize, b: usize) -> bool {
        a != b && self.dominates(a, b)
    }

    /// Get all blocks dominated by a given block
    pub fn dominated_by(&self, block: usize) -> Vec<usize> {
        let mut result = Vec::new();
        let mut stack = vec![block];
        while let Some(current) = stack.pop() {
            result.push(current);
            if let Some(node) = self.nodes.get(&current) {
                stack.extend(node.children.iter());
            }
        }
        result
    }
}

/// Dominance frontier for each block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DominanceFrontier {
    /// For each block ID, the set of blocks in its dominance frontier
    pub frontier: HashMap<usize, HashSet<usize>>,
}

impl DominanceFrontier {
    /// Get the dominance frontier for a block
    pub fn get(&self, block: usize) -> HashSet<usize> {
        self.frontier.get(&block).cloned().unwrap_or_default()
    }

    /// Compute iterated dominance frontier for a set of blocks
    pub fn iterated(&self, blocks: &HashSet<usize>) -> HashSet<usize> {
        compute_iterated_df(self, blocks)
    }
}

/// Build dominator tree using Lengauer-Tarjan algorithm
///
/// # Arguments
/// * `cfg` - Control flow graph for the function
///
/// # Returns
/// * `DominatorTree` - The computed dominator tree
///
/// # Algorithm
/// Uses the Lengauer-Tarjan algorithm with path compression.
/// Complexity: O(E * alpha(E, V)) where alpha is inverse Ackermann.
///
/// # Edge Cases (from premortem)
/// - S10-P1-R1: Unreachable blocks are excluded from the tree (no panic)
/// - S10-P1-R2: Path compression uses iterative approach to avoid corruption
/// - S10-P1-R3: Irreducible CFGs are handled correctly
/// - S10-P1-R5: Self-loops are handled (visited check before recursion)
/// - S10-P3-R1: Uses cfg.entry_block, not hardcoded 0
pub fn build_dominator_tree(cfg: &CfgInfo) -> TldrResult<DominatorTree> {
    // Handle empty CFG
    if cfg.blocks.is_empty() {
        return Err(TldrError::InvalidArgs {
            arg: "cfg".to_string(),
            message: "Empty CFG has no blocks".to_string(),
            suggestion: None,
        });
    }

    let entry = cfg.entry_block;

    // Build adjacency lists for successors and predecessors
    let mut successors: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut predecessors: HashMap<usize, Vec<usize>> = HashMap::new();

    // Initialize all blocks
    for block in &cfg.blocks {
        successors.entry(block.id).or_default();
        predecessors.entry(block.id).or_default();
    }

    // Populate from edges
    for edge in &cfg.edges {
        successors.entry(edge.from).or_default().push(edge.to);
        predecessors.entry(edge.to).or_default().push(edge.from);
    }

    // Run Lengauer-Tarjan algorithm
    let mut lt = LengauerTarjan::new(entry, &successors, &predecessors);
    lt.compute();

    // Build the DominatorTree from computed idoms
    let mut nodes: HashMap<usize, DominatorNode> = HashMap::new();

    // First pass: create nodes with idom
    for &block in &lt.vertex {
        let idom = if block == entry {
            None
        } else {
            lt.idom.get(&block).copied()
        };

        nodes.insert(
            block,
            DominatorNode {
                block_id: block,
                idom,
                children: Vec::new(),
                depth: 0,
            },
        );
    }

    // Second pass: populate children
    for (&block, node) in &nodes.clone() {
        if let Some(idom) = node.idom {
            if let Some(parent) = nodes.get_mut(&idom) {
                parent.children.push(block);
            }
        }
    }

    // Sort children for deterministic traversal order
    for node in nodes.values_mut() {
        node.children.sort();
    }

    // Third pass: compute depths via BFS from entry
    compute_depths(&mut nodes, entry);

    // Compute preorder and postorder traversals
    let (preorder, postorder) = compute_traversals(&nodes, entry);

    Ok(DominatorTree {
        function: cfg.function.clone(),
        nodes,
        entry,
        preorder,
        postorder,
    })
}

/// Compute depths in the dominator tree using BFS
fn compute_depths(nodes: &mut HashMap<usize, DominatorNode>, entry: usize) {
    let mut queue = std::collections::VecDeque::new();
    queue.push_back((entry, 0u32));

    while let Some((block, depth)) = queue.pop_front() {
        if let Some(node) = nodes.get_mut(&block) {
            node.depth = depth;
            for &child in &node.children.clone() {
                queue.push_back((child, depth + 1));
            }
        }
    }
}

/// Compute preorder and postorder traversals of the dominator tree
fn compute_traversals(
    nodes: &HashMap<usize, DominatorNode>,
    entry: usize,
) -> (Vec<usize>, Vec<usize>) {
    let mut preorder = Vec::new();
    let mut postorder = Vec::new();
    let mut stack = vec![(entry, false)];

    while let Some((block, visited)) = stack.pop() {
        if visited {
            postorder.push(block);
        } else {
            preorder.push(block);
            stack.push((block, true));
            if let Some(node) = nodes.get(&block) {
                // Push children in reverse order for correct preorder
                for &child in node.children.iter().rev() {
                    stack.push((child, false));
                }
            }
        }
    }

    (preorder, postorder)
}

/// Lengauer-Tarjan algorithm state
struct LengauerTarjan<'a> {
    /// Entry block ID
    entry: usize,
    /// Successors adjacency list
    successors: &'a HashMap<usize, Vec<usize>>,
    /// Predecessors adjacency list
    predecessors: &'a HashMap<usize, Vec<usize>>,

    // DFS tree data
    /// DFS number for each block
    dfnum: HashMap<usize, usize>,
    /// Blocks in DFS order (dfnum -> block)
    vertex: Vec<usize>,
    /// Parent in DFS tree
    parent: HashMap<usize, usize>,

    // Semi-dominator computation
    /// Semi-dominator (stored as DFS number)
    semi: HashMap<usize, usize>,
    /// Ancestor for path compression
    ancestor: HashMap<usize, Option<usize>>,
    /// Label for EVAL operation
    label: HashMap<usize, usize>,

    // Result
    /// Immediate dominator
    idom: HashMap<usize, usize>,
    /// Bucket for processing
    bucket: HashMap<usize, Vec<usize>>,
}

impl<'a> LengauerTarjan<'a> {
    fn new(
        entry: usize,
        successors: &'a HashMap<usize, Vec<usize>>,
        predecessors: &'a HashMap<usize, Vec<usize>>,
    ) -> Self {
        LengauerTarjan {
            entry,
            successors,
            predecessors,
            dfnum: HashMap::new(),
            vertex: Vec::new(),
            parent: HashMap::new(),
            semi: HashMap::new(),
            ancestor: HashMap::new(),
            label: HashMap::new(),
            idom: HashMap::new(),
            bucket: HashMap::new(),
        }
    }

    /// Run the Lengauer-Tarjan algorithm
    fn compute(&mut self) {
        // Phase 1: DFS numbering
        self.dfs(self.entry);

        if self.vertex.is_empty() {
            return;
        }

        // Initialize data structures
        for &v in &self.vertex {
            self.ancestor.insert(v, None);
            self.label.insert(v, v);
            self.bucket.entry(v).or_default();
        }

        // Phase 2: Compute semi-dominators and build idom
        // Process vertices in reverse DFS order (excluding entry)
        let n = self.vertex.len();
        for i in (1..n).rev() {
            let w = self.vertex[i];

            // Step 2: Compute semi-dominator
            if let Some(preds) = self.predecessors.get(&w) {
                for &v in preds {
                    if self.dfnum.contains_key(&v) {
                        let u = self.eval(v);
                        let semi_u = self.semi.get(&u).copied().unwrap_or(n);
                        let semi_w = self.semi.get(&w).copied().unwrap_or(n);
                        if semi_u < semi_w {
                            self.semi.insert(w, semi_u);
                        }
                    }
                }
            }

            // Add w to bucket of vertex[semi[w]]
            let semi_w = self.semi.get(&w).copied().unwrap_or(self.dfnum[&w]);
            let semi_vertex = self.vertex[semi_w];
            self.bucket.entry(semi_vertex).or_default().push(w);

            // Link w to its parent
            let parent_w = self.parent.get(&w).copied();
            if let Some(p) = parent_w {
                self.link(p, w);

                // Step 3: Process bucket of parent
                if let Some(bucket) = self.bucket.get_mut(&p) {
                    let to_process = std::mem::take(bucket);
                    for v in to_process {
                        let u = self.eval(v);
                        let semi_u = self.semi.get(&u).copied().unwrap_or(n);
                        let semi_v = self.semi.get(&v).copied().unwrap_or(n);
                        if semi_u < semi_v {
                            self.idom.insert(v, u);
                        } else {
                            self.idom.insert(v, p);
                        }
                    }
                }
            }
        }

        // Phase 3: Finalize immediate dominators
        for i in 1..n {
            let w = self.vertex[i];
            let semi_w = self.semi.get(&w).copied().unwrap_or(self.dfnum[&w]);
            let semi_vertex = self.vertex[semi_w];

            if let Some(&idom_w) = self.idom.get(&w) {
                if idom_w != semi_vertex {
                    if let Some(&idom_idom_w) = self.idom.get(&idom_w) {
                        self.idom.insert(w, idom_idom_w);
                    }
                }
            }
        }
    }

    /// DFS from entry to compute numbering
    fn dfs(&mut self, start: usize) {
        let mut stack = vec![(start, None::<usize>, false)];
        let mut visited = HashSet::new();

        while let Some((v, parent, processed)) = stack.pop() {
            if processed {
                continue;
            }

            if visited.contains(&v) {
                continue;
            }
            visited.insert(v);

            // Assign DFS number
            let num = self.vertex.len();
            self.dfnum.insert(v, num);
            self.vertex.push(v);
            self.semi.insert(v, num);

            if let Some(p) = parent {
                self.parent.insert(v, p);
            }

            // Push successors
            if let Some(succs) = self.successors.get(&v) {
                for &w in succs.iter().rev() {
                    if !visited.contains(&w) {
                        stack.push((w, Some(v), false));
                    }
                }
            }
        }
    }

    /// EVAL operation with path compression
    fn eval(&mut self, v: usize) -> usize {
        if self.ancestor.get(&v).copied().flatten().is_none() {
            return v;
        }

        self.compress(v);
        self.label.get(&v).copied().unwrap_or(v)
    }

    /// Path compression (iterative to avoid stack overflow - S10-P1-R2)
    fn compress(&mut self, v: usize) {
        // First, collect the path from v to the root
        let mut path = Vec::new();
        let mut current = v;

        while let Some(Some(anc)) = self.ancestor.get(&current) {
            if self.ancestor.get(anc).copied().flatten().is_some() {
                path.push(current);
                current = *anc;
            } else {
                break;
            }
        }

        // Now compress the path
        for node in path.into_iter().rev() {
            if let Some(Some(anc)) = self.ancestor.get(&node).copied() {
                let label_anc = self.label.get(&anc).copied().unwrap_or(anc);
                let semi_label_anc = self.semi.get(&label_anc).copied().unwrap_or(usize::MAX);

                let label_node = self.label.get(&node).copied().unwrap_or(node);
                let semi_label_node = self.semi.get(&label_node).copied().unwrap_or(usize::MAX);

                if semi_label_anc < semi_label_node {
                    self.label.insert(node, label_anc);
                }

                // Update ancestor to grandparent
                if let Some(Some(anc_anc)) = self.ancestor.get(&anc).copied() {
                    self.ancestor.insert(node, Some(anc_anc));
                }
            }
        }
    }

    /// LINK operation
    fn link(&mut self, v: usize, w: usize) {
        self.ancestor.insert(w, Some(v));
    }
}

/// Compute dominance frontier for all blocks
///
/// # Arguments
/// * `cfg` - Control flow graph
/// * `dom_tree` - Pre-computed dominator tree
///
/// # Returns
/// * `DominanceFrontier` - Dominance frontier for each block
///
/// # Definition
/// DF(b) = {y | b dominates a predecessor of y, but b does not strictly dominate y}
///
/// # Algorithm (Cooper, Harvey, Kennedy - "A Simple, Fast Dominance Algorithm")
/// For each join point (block with multiple predecessors), walk up the
/// dominator tree from each predecessor until we reach the immediate
/// dominator of the join point. Each block visited along the way has
/// the join point in its dominance frontier.
///
/// # Complexity
/// O(E + V) - linear in the size of the CFG
///
/// # Edge Cases (from premortem)
/// - S10-P1-R7: Iteration limit to prevent infinite loops if dom tree corrupted
/// - S10-P1-R8: Pre-allocate HashSet capacity based on block count
/// - S10-P1-R9: Validate block IDs exist in dom tree
/// - S10-P1-R11: Invalid block IDs detected and skipped
pub fn compute_dominance_frontier(
    cfg: &CfgInfo,
    dom_tree: &DominatorTree,
) -> TldrResult<DominanceFrontier> {
    // Initialize empty frontier for each block in the dominator tree
    let mut frontier: HashMap<usize, HashSet<usize>> = HashMap::with_capacity(dom_tree.nodes.len());
    for &block_id in dom_tree.nodes.keys() {
        frontier.insert(block_id, HashSet::new());
    }

    // Build predecessors map from CFG edges
    let mut predecessors: HashMap<usize, Vec<usize>> = HashMap::new();
    for block in &cfg.blocks {
        predecessors.entry(block.id).or_default();
    }
    for edge in &cfg.edges {
        predecessors.entry(edge.to).or_default().push(edge.from);
    }

    // Maximum iterations to prevent infinite loops (S10-P1-R7)
    // In a valid dominator tree, we should never need more than depth * nodes iterations
    let max_iterations = dom_tree.nodes.len() * dom_tree.nodes.len();
    let mut total_iterations = 0;

    // For each block that is a join point (has multiple predecessors)
    for block in &cfg.blocks {
        let preds = predecessors
            .get(&block.id)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);

        // Only process join points (blocks with 2+ predecessors)
        if preds.len() < 2 {
            continue;
        }

        let join_point = block.id;

        // Get the immediate dominator of this join point
        let join_idom = dom_tree.nodes.get(&join_point).and_then(|n| n.idom);

        // For each predecessor of the join point
        for &pred in preds {
            // Skip if predecessor is not in dominator tree (unreachable)
            if !dom_tree.nodes.contains_key(&pred) {
                continue;
            }

            // Walk up the dominator tree from predecessor until we reach
            // the immediate dominator of the join point
            let mut runner = pred;

            loop {
                // Safety check for iteration limit (S10-P1-R7)
                total_iterations += 1;
                if total_iterations > max_iterations {
                    return Err(TldrError::InvalidArgs {
                        arg: "dom_tree".to_string(),
                        message: "Iteration limit exceeded computing dominance frontier - possible corrupted dominator tree".to_string(),
                        suggestion: Some("Check that the dominator tree was computed correctly".to_string()),
                    });
                }

                // Stop if we've reached the idom of the join point
                if Some(runner) == join_idom {
                    break;
                }

                // Stop if runner IS the join point (handles self-loops)
                if runner == join_point {
                    break;
                }

                // Add join point to runner's dominance frontier
                if let Some(df_set) = frontier.get_mut(&runner) {
                    df_set.insert(join_point);
                }

                // Move up to runner's immediate dominator
                match dom_tree.nodes.get(&runner).and_then(|n| n.idom) {
                    Some(idom) => runner = idom,
                    None => break, // Reached entry block (no idom)
                }
            }
        }
    }

    Ok(DominanceFrontier { frontier })
}

/// Compute iterated dominance frontier for a set of blocks
///
/// # Arguments
/// * `df` - Pre-computed dominance frontiers
/// * `blocks` - Set of blocks to compute IDF for
///
/// # Returns
/// * `HashSet<usize>` - The iterated dominance frontier
///
/// # Definition
/// IDF(S) = DF(S) union IDF(DF(S))
/// Returns the closure of the dominance frontier operation.
///
/// # Algorithm (Worklist)
/// 1. Initialize IDF = {} and worklist with input blocks
/// 2. While worklist not empty:
///    - Pop block b
///    - For each y in DF[b]:
///      - If y not in IDF, add to IDF and worklist
/// 3. Return IDF
///
/// # Edge Cases (from premortem)
/// - S10-P1-R10: Termination guaranteed by tracking processed blocks
/// - S10-P3-R18: Track processed set to avoid revisiting blocks
pub fn compute_iterated_df(df: &DominanceFrontier, blocks: &HashSet<usize>) -> HashSet<usize> {
    let mut idf = HashSet::new();
    let mut worklist: Vec<usize> = blocks.iter().copied().collect();
    let mut processed: HashSet<usize> = HashSet::new();

    while let Some(block) = worklist.pop() {
        // Skip if already processed (S10-P3-R18)
        if !processed.insert(block) {
            continue;
        }

        if let Some(frontier) = df.frontier.get(&block) {
            for &y in frontier {
                // Only add to IDF and worklist if not already present (S10-P1-R10)
                if idf.insert(y) {
                    // Only add to worklist if not already processed
                    if !processed.contains(&y) {
                        worklist.push(y);
                    }
                }
            }
        }
    }

    idf
}
