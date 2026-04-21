//! Graph utilities for call graph analysis
//!
//! This module provides shared graph infrastructure used by analysis commands:
//!
//! - `build_reverse_graph`: Build callee -> [callers] mapping
//! - `build_forward_graph`: Build caller -> [callees] mapping
//! - `collect_nodes`: Extract all unique function references
//!
//! # Example
//!
//! ```rust,ignore
//! use tldr_core::callgraph::graph_utils::{build_forward_graph, build_reverse_graph, collect_nodes};
//! use tldr_core::types::ProjectCallGraph;
//!
//! let graph = ProjectCallGraph::new();
//! // ... add edges ...
//!
//! let forward = build_forward_graph(&graph);
//! let reverse = build_reverse_graph(&graph);
//! let nodes = collect_nodes(&graph);
//! ```

use std::collections::{HashMap, HashSet};

use crate::types::{FunctionRef, ProjectCallGraph};

/// Build reverse graph: callee -> [callers]
///
/// For each function, returns the list of functions that call it.
/// This is useful for impact analysis ("who calls this function?").
///
/// # Arguments
/// * `call_graph` - The project call graph
///
/// # Returns
/// A HashMap where keys are callees (FunctionRef) and values are vectors of callers
pub fn build_reverse_graph(
    call_graph: &ProjectCallGraph,
) -> HashMap<FunctionRef, Vec<FunctionRef>> {
    let mut reverse: HashMap<FunctionRef, Vec<FunctionRef>> = HashMap::new();

    for edge in call_graph.edges() {
        let callee = FunctionRef::new(edge.dst_file.clone(), edge.dst_func.clone());
        let caller = FunctionRef::new(edge.src_file.clone(), edge.src_func.clone());

        reverse.entry(callee).or_default().push(caller);
    }

    reverse
}

/// Build forward graph: caller -> [callees]
///
/// For each function, returns the list of functions it calls.
/// This is useful for hubs analysis (out-degree) and PageRank computation.
///
/// # Arguments
/// * `call_graph` - The project call graph
///
/// # Returns
/// A HashMap where keys are callers (FunctionRef) and values are vectors of callees
pub fn build_forward_graph(
    call_graph: &ProjectCallGraph,
) -> HashMap<FunctionRef, Vec<FunctionRef>> {
    let mut forward: HashMap<FunctionRef, Vec<FunctionRef>> = HashMap::new();

    for edge in call_graph.edges() {
        let caller = FunctionRef::new(edge.src_file.clone(), edge.src_func.clone());
        let callee = FunctionRef::new(edge.dst_file.clone(), edge.dst_func.clone());

        forward.entry(caller).or_default().push(callee);
    }

    forward
}

/// Collect all unique nodes (function references) from the call graph
///
/// Returns a HashSet of all functions that appear as either caller or callee
/// in any edge of the call graph.
///
/// # Arguments
/// * `call_graph` - The project call graph
///
/// # Returns
/// A HashSet of unique FunctionRef values
pub fn collect_nodes(call_graph: &ProjectCallGraph) -> HashSet<FunctionRef> {
    let mut nodes = HashSet::new();

    for edge in call_graph.edges() {
        nodes.insert(FunctionRef::new(
            edge.src_file.clone(),
            edge.src_func.clone(),
        ));
        nodes.insert(FunctionRef::new(
            edge.dst_file.clone(),
            edge.dst_func.clone(),
        ));
    }

    nodes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::CallEdge;
    use std::path::PathBuf;

    /// Create a simple test graph: A -> B -> C, D -> C
    fn create_test_graph() -> ProjectCallGraph {
        let mut graph = ProjectCallGraph::new();

        // A calls B
        graph.add_edge(CallEdge {
            src_file: PathBuf::from("a.py"),
            src_func: "func_a".to_string(),
            dst_file: PathBuf::from("b.py"),
            dst_func: "func_b".to_string(),
        });

        // B calls C
        graph.add_edge(CallEdge {
            src_file: PathBuf::from("b.py"),
            src_func: "func_b".to_string(),
            dst_file: PathBuf::from("c.py"),
            dst_func: "func_c".to_string(),
        });

        // D calls C
        graph.add_edge(CallEdge {
            src_file: PathBuf::from("d.py"),
            src_func: "func_d".to_string(),
            dst_file: PathBuf::from("c.py"),
            dst_func: "func_c".to_string(),
        });

        graph
    }

    /// Create a diamond-shaped graph: A -> B, A -> C, B -> D, C -> D
    fn create_diamond_graph() -> ProjectCallGraph {
        let mut graph = ProjectCallGraph::new();

        // A -> B
        graph.add_edge(CallEdge {
            src_file: PathBuf::from("a.py"),
            src_func: "func_a".to_string(),
            dst_file: PathBuf::from("b.py"),
            dst_func: "func_b".to_string(),
        });

        // A -> C
        graph.add_edge(CallEdge {
            src_file: PathBuf::from("a.py"),
            src_func: "func_a".to_string(),
            dst_file: PathBuf::from("c.py"),
            dst_func: "func_c".to_string(),
        });

        // B -> D
        graph.add_edge(CallEdge {
            src_file: PathBuf::from("b.py"),
            src_func: "func_b".to_string(),
            dst_file: PathBuf::from("d.py"),
            dst_func: "func_d".to_string(),
        });

        // C -> D
        graph.add_edge(CallEdge {
            src_file: PathBuf::from("c.py"),
            src_func: "func_c".to_string(),
            dst_file: PathBuf::from("d.py"),
            dst_func: "func_d".to_string(),
        });

        graph
    }

    #[test]
    fn test_forward_reverse_consistency() {
        // Test that an edge in forward graph corresponds to reverse edge in reverse graph
        let graph = create_test_graph();

        let forward = build_forward_graph(&graph);
        let reverse = build_reverse_graph(&graph);

        // For each caller -> callee in forward, callee should have caller in reverse
        for (caller, callees) in &forward {
            for callee in callees {
                let callers_of_callee = reverse
                    .get(callee)
                    .expect("callee should be in reverse graph");
                assert!(
                    callers_of_callee.contains(caller),
                    "Forward edge {:?} -> {:?} should have reverse entry",
                    caller,
                    callee
                );
            }
        }

        // And vice versa: for each callee -> caller in reverse, caller should have callee in forward
        for (callee, callers) in &reverse {
            for caller in callers {
                let callees_of_caller = forward
                    .get(caller)
                    .expect("caller should be in forward graph");
                assert!(
                    callees_of_caller.contains(callee),
                    "Reverse edge {:?} -> {:?} should have forward entry",
                    callee,
                    caller
                );
            }
        }
    }

    #[test]
    fn test_collect_nodes_unique() {
        // Test that collect_nodes returns exactly the unique nodes
        let graph = create_test_graph();
        let nodes = collect_nodes(&graph);

        // Should have 4 unique functions: A, B, C, D
        assert_eq!(nodes.len(), 4);

        // Check each expected node is present
        assert!(nodes.contains(&FunctionRef::new(PathBuf::from("a.py"), "func_a")));
        assert!(nodes.contains(&FunctionRef::new(PathBuf::from("b.py"), "func_b")));
        assert!(nodes.contains(&FunctionRef::new(PathBuf::from("c.py"), "func_c")));
        assert!(nodes.contains(&FunctionRef::new(PathBuf::from("d.py"), "func_d")));
    }

    #[test]
    fn test_empty_graph_handling() {
        // Test that empty graph returns empty results
        let graph = ProjectCallGraph::new();

        let forward = build_forward_graph(&graph);
        let reverse = build_reverse_graph(&graph);
        let nodes = collect_nodes(&graph);

        assert!(
            forward.is_empty(),
            "Forward graph of empty graph should be empty"
        );
        assert!(
            reverse.is_empty(),
            "Reverse graph of empty graph should be empty"
        );
        assert!(nodes.is_empty(), "Nodes of empty graph should be empty");
    }

    #[test]
    fn test_forward_graph_structure() {
        // Test the specific structure of forward graph
        let graph = create_test_graph();
        let forward = build_forward_graph(&graph);

        // A calls B (1 callee)
        let a = FunctionRef::new(PathBuf::from("a.py"), "func_a");
        assert_eq!(forward.get(&a).map(|v| v.len()), Some(1));

        // B calls C (1 callee)
        let b = FunctionRef::new(PathBuf::from("b.py"), "func_b");
        assert_eq!(forward.get(&b).map(|v| v.len()), Some(1));

        // D calls C (1 callee)
        let d = FunctionRef::new(PathBuf::from("d.py"), "func_d");
        assert_eq!(forward.get(&d).map(|v| v.len()), Some(1));

        // C calls nothing (not in forward graph as caller)
        let c = FunctionRef::new(PathBuf::from("c.py"), "func_c");
        assert!(!forward.contains_key(&c));
    }

    #[test]
    fn test_reverse_graph_structure() {
        // Test the specific structure of reverse graph
        let graph = create_test_graph();
        let reverse = build_reverse_graph(&graph);

        // C is called by B and D (2 callers)
        let c = FunctionRef::new(PathBuf::from("c.py"), "func_c");
        assert_eq!(reverse.get(&c).map(|v| v.len()), Some(2));

        // B is called by A (1 caller)
        let b = FunctionRef::new(PathBuf::from("b.py"), "func_b");
        assert_eq!(reverse.get(&b).map(|v| v.len()), Some(1));

        // A is not called by anyone (entry point - not in reverse graph as callee)
        let a = FunctionRef::new(PathBuf::from("a.py"), "func_a");
        assert!(!reverse.contains_key(&a));

        // D is not called by anyone (entry point)
        let d = FunctionRef::new(PathBuf::from("d.py"), "func_d");
        assert!(!reverse.contains_key(&d));
    }

    #[test]
    fn test_diamond_graph_nodes() {
        // Test diamond graph has correct node count
        let graph = create_diamond_graph();
        let nodes = collect_nodes(&graph);

        // Should have 4 unique nodes: A, B, C, D
        assert_eq!(nodes.len(), 4);
    }

    #[test]
    fn test_diamond_forward_out_degrees() {
        // Test forward graph out-degrees in diamond
        let graph = create_diamond_graph();
        let forward = build_forward_graph(&graph);

        // A has out-degree 2 (calls B and C)
        let a = FunctionRef::new(PathBuf::from("a.py"), "func_a");
        assert_eq!(forward.get(&a).map(|v| v.len()), Some(2));

        // B has out-degree 1 (calls D)
        let b = FunctionRef::new(PathBuf::from("b.py"), "func_b");
        assert_eq!(forward.get(&b).map(|v| v.len()), Some(1));

        // C has out-degree 1 (calls D)
        let c = FunctionRef::new(PathBuf::from("c.py"), "func_c");
        assert_eq!(forward.get(&c).map(|v| v.len()), Some(1));

        // D has out-degree 0 (leaf node)
        let d = FunctionRef::new(PathBuf::from("d.py"), "func_d");
        assert!(!forward.contains_key(&d));
    }

    #[test]
    fn test_diamond_reverse_in_degrees() {
        // Test reverse graph in-degrees in diamond
        let graph = create_diamond_graph();
        let reverse = build_reverse_graph(&graph);

        // D has in-degree 2 (called by B and C)
        let d = FunctionRef::new(PathBuf::from("d.py"), "func_d");
        assert_eq!(reverse.get(&d).map(|v| v.len()), Some(2));

        // B has in-degree 1 (called by A)
        let b = FunctionRef::new(PathBuf::from("b.py"), "func_b");
        assert_eq!(reverse.get(&b).map(|v| v.len()), Some(1));

        // C has in-degree 1 (called by A)
        let c = FunctionRef::new(PathBuf::from("c.py"), "func_c");
        assert_eq!(reverse.get(&c).map(|v| v.len()), Some(1));

        // A has in-degree 0 (entry point)
        let a = FunctionRef::new(PathBuf::from("a.py"), "func_a");
        assert!(!reverse.contains_key(&a));
    }

    #[test]
    fn test_self_loop_handling() {
        // Test graph with self-loop
        let mut graph = ProjectCallGraph::new();

        // A calls itself
        graph.add_edge(CallEdge {
            src_file: PathBuf::from("a.py"),
            src_func: "func_a".to_string(),
            dst_file: PathBuf::from("a.py"),
            dst_func: "func_a".to_string(),
        });

        let forward = build_forward_graph(&graph);
        let reverse = build_reverse_graph(&graph);
        let nodes = collect_nodes(&graph);

        // Only 1 unique node
        assert_eq!(nodes.len(), 1);

        // A calls A in forward
        let a = FunctionRef::new(PathBuf::from("a.py"), "func_a");
        assert_eq!(forward.get(&a).map(|v| v.len()), Some(1));
        assert!(forward.get(&a).unwrap().contains(&a));

        // A is called by A in reverse
        assert_eq!(reverse.get(&a).map(|v| v.len()), Some(1));
        assert!(reverse.get(&a).unwrap().contains(&a));
    }

    #[test]
    fn test_multiple_edges_same_pair() {
        // Test when there are duplicate edges (same caller/callee pair)
        // Note: ProjectCallGraph uses HashSet<CallEdge>, so duplicates are deduplicated
        let mut graph = ProjectCallGraph::new();

        // Add same edge twice
        graph.add_edge(CallEdge {
            src_file: PathBuf::from("a.py"),
            src_func: "func_a".to_string(),
            dst_file: PathBuf::from("b.py"),
            dst_func: "func_b".to_string(),
        });
        graph.add_edge(CallEdge {
            src_file: PathBuf::from("a.py"),
            src_func: "func_a".to_string(),
            dst_file: PathBuf::from("b.py"),
            dst_func: "func_b".to_string(),
        });

        // Should only have 1 edge due to HashSet deduplication
        assert_eq!(graph.edge_count(), 1);

        let forward = build_forward_graph(&graph);
        let a = FunctionRef::new(PathBuf::from("a.py"), "func_a");

        // A should have 1 callee (not 2)
        assert_eq!(forward.get(&a).map(|v| v.len()), Some(1));
    }
}
