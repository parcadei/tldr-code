//! Inheritance pattern detection
//!
//! Detects patterns in inheritance hierarchies:
//! - ABC/Protocol/Interface detection
//! - Mixin class detection
//! - Diamond inheritance detection (A2 - optimized using BFS + set intersection)
//!
//! # Diamond Detection Algorithm (A2 mitigation)
//!
//! Instead of O(n^3) path enumeration, we use BFS + set intersection:
//! 1. For each class with 2+ parents, compute ancestor_set(P_i) for each parent
//! 2. Diamond common ancestors = intersection of all ancestor_sets
//! 3. Complexity: O(|nodes| * |edges|) for BFS per class

use std::collections::{HashMap, HashSet};

use crate::types::{DiamondPattern, InheritanceGraph};

/// Detect ABC/Protocol/Interface patterns and mark nodes
pub fn detect_abc_protocol(graph: &mut InheritanceGraph) {
    // For Python: ABC inheritors and @abstractmethod
    // For TypeScript: abstract class and interface
    // For Rust: trait definitions

    for (_name, node) in graph.nodes.iter_mut() {
        // Check if bases contain ABC
        if node.bases.iter().any(|b| b == "ABC" || b == "ABCMeta") {
            node.is_abstract = Some(true);
        }

        // Check if bases contain Protocol
        if node
            .bases
            .iter()
            .any(|b| b == "Protocol" || b.ends_with(".Protocol"))
        {
            node.protocol = Some(true);
        }
    }
}

/// Detect mixin classes using naming heuristics and usage patterns
///
/// Heuristics:
/// 1. Name ends with "Mixin" (case-insensitive) -> definite mixin
/// 2. Appears as secondary base (not first) in 2+ classes with no bases itself -> likely mixin (A6)
pub fn detect_mixins(graph: &mut InheritanceGraph) {
    // Pre-compute secondary_base_count in single pass (A6 optimization)
    let mut secondary_base_count: HashMap<String, usize> = HashMap::new();

    for node in graph.nodes.values() {
        if node.bases.len() > 1 {
            // Secondary bases are all bases except the first
            for base in &node.bases[1..] {
                *secondary_base_count.entry(base.clone()).or_insert(0) += 1;
            }
        }
    }

    // Mark mixins
    for (name, node) in graph.nodes.iter_mut() {
        // Heuristic 1: Name ends with "Mixin"
        if name.to_lowercase().ends_with("mixin") {
            node.mixin = Some(true);
            continue;
        }

        // Heuristic 2: Used as secondary base 2+ times and has no bases
        if node.bases.is_empty() {
            if let Some(&count) = secondary_base_count.get(name) {
                if count >= 2 {
                    node.mixin = Some(true);
                }
            }
        }
    }
}

/// Detect diamond inheritance patterns using BFS + set intersection (A2 optimization)
///
/// A diamond occurs when a class has multiple paths to the same ancestor
/// through different immediate parents.
///
/// ```text
///        A          <- common_ancestor
///       / \
///      B   C
///       \ /
///        D          <- class_name (diamond)
/// ```
pub fn detect_diamonds(graph: &InheritanceGraph) -> Vec<DiamondPattern> {
    let mut diamonds = Vec::new();

    // For each class with 2+ parents
    for (class_name, parents) in graph.multi_parent_classes() {
        if parents.len() < 2 {
            continue;
        }

        // Compute ancestor sets for each parent using BFS
        let ancestor_sets: Vec<HashSet<String>> = parents
            .iter()
            .map(|parent| graph.ancestors_bfs(parent))
            .collect();

        // Find common ancestors (intersection of all ancestor sets)
        if ancestor_sets.is_empty() {
            continue;
        }

        let common: HashSet<String> = if ancestor_sets.len() == 1 {
            ancestor_sets[0].clone()
        } else {
            ancestor_sets[1..]
                .iter()
                .fold(ancestor_sets[0].clone(), |acc, s| {
                    acc.intersection(s).cloned().collect()
                })
        };

        // For each common ancestor, create a diamond pattern
        for ancestor in common {
            let paths = compute_paths_to_ancestor(graph, class_name, &ancestor, parents);
            if paths.len() >= 2 {
                diamonds.push(DiamondPattern {
                    class_name: class_name.clone(),
                    common_ancestor: ancestor,
                    paths,
                });
            }
        }
    }

    diamonds
}

/// Compute paths from class to ancestor through each parent
fn compute_paths_to_ancestor(
    graph: &InheritanceGraph,
    class_name: &str,
    ancestor: &str,
    parents: &[String],
) -> Vec<Vec<String>> {
    let mut paths = Vec::new();

    for parent in parents {
        // Check if this parent has a path to the ancestor
        if let Some(path) = find_path_to_ancestor(graph, parent, ancestor) {
            let mut full_path = vec![class_name.to_string()];
            full_path.extend(path);
            paths.push(full_path);
        }
    }

    paths
}

/// Find a path from start to ancestor using BFS
fn find_path_to_ancestor(
    graph: &InheritanceGraph,
    start: &str,
    ancestor: &str,
) -> Option<Vec<String>> {
    use std::collections::VecDeque;

    if start == ancestor {
        return Some(vec![start.to_string()]);
    }

    let mut queue = VecDeque::new();
    let mut visited = HashSet::new();
    let mut parent_map: HashMap<String, String> = HashMap::new();

    queue.push_back(start.to_string());
    visited.insert(start.to_string());

    while let Some(current) = queue.pop_front() {
        if let Some(parents) = graph.parents.get(&current) {
            for parent in parents {
                if !visited.contains(parent) {
                    visited.insert(parent.clone());
                    parent_map.insert(parent.clone(), current.clone());
                    queue.push_back(parent.clone());

                    if parent == ancestor {
                        // Reconstruct path
                        let mut path = vec![ancestor.to_string()];
                        let mut curr = ancestor.to_string();
                        while let Some(child) = parent_map.get(&curr) {
                            path.push(child.clone());
                            curr = child.clone();
                        }
                        path.reverse();
                        return Some(path);
                    }
                }
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{InheritanceNode, Language};
    use std::path::PathBuf;

    fn create_node(name: &str, bases: Vec<&str>) -> InheritanceNode {
        let mut node = InheritanceNode::new(name, PathBuf::from("test.py"), 1, Language::Python);
        node.bases = bases.into_iter().map(|s| s.to_string()).collect();
        node
    }

    #[test]
    fn test_abc_detection() {
        let mut graph = InheritanceGraph::new();
        graph.add_node(create_node("Animal", vec!["ABC"]));
        graph.add_node(create_node("Dog", vec!["Animal"]));
        graph.add_edge("Animal", "ABC");
        graph.add_edge("Dog", "Animal");

        detect_abc_protocol(&mut graph);

        let animal = graph.nodes.get("Animal").unwrap();
        assert_eq!(animal.is_abstract, Some(true));
    }

    #[test]
    fn test_protocol_detection() {
        let mut graph = InheritanceGraph::new();
        graph.add_node(create_node("Serializable", vec!["Protocol"]));

        detect_abc_protocol(&mut graph);

        let serializable = graph.nodes.get("Serializable").unwrap();
        assert_eq!(serializable.protocol, Some(true));
    }

    #[test]
    fn test_mixin_detection_by_name() {
        let mut graph = InheritanceGraph::new();
        graph.add_node(create_node("TimestampMixin", vec![]));
        graph.add_node(create_node("User", vec!["Base", "TimestampMixin"]));

        detect_mixins(&mut graph);

        let mixin = graph.nodes.get("TimestampMixin").unwrap();
        assert_eq!(mixin.mixin, Some(true));
    }

    #[test]
    fn test_mixin_detection_by_usage() {
        let mut graph = InheritanceGraph::new();
        graph.add_node(create_node("Auditable", vec![]));
        graph.add_node(create_node("User", vec!["Base", "Auditable"]));
        graph.add_node(create_node("Post", vec!["Base", "Auditable"]));
        graph.add_node(create_node("Comment", vec!["Base", "Auditable"]));

        detect_mixins(&mut graph);

        let auditable = graph.nodes.get("Auditable").unwrap();
        assert_eq!(auditable.mixin, Some(true));
    }

    #[test]
    fn test_diamond_detection() {
        let mut graph = InheritanceGraph::new();

        // Create diamond: D -> B -> A, D -> C -> A
        graph.add_node(create_node("A", vec![]));
        graph.add_node(create_node("B", vec!["A"]));
        graph.add_node(create_node("C", vec!["A"]));
        graph.add_node(create_node("D", vec!["B", "C"]));

        graph.add_edge("B", "A");
        graph.add_edge("C", "A");
        graph.add_edge("D", "B");
        graph.add_edge("D", "C");

        let diamonds = detect_diamonds(&graph);

        assert_eq!(diamonds.len(), 1);
        assert_eq!(diamonds[0].class_name, "D");
        assert_eq!(diamonds[0].common_ancestor, "A");
        assert_eq!(diamonds[0].paths.len(), 2);
    }

    #[test]
    fn test_no_diamond_single_inheritance() {
        let mut graph = InheritanceGraph::new();

        // Linear: C -> B -> A
        graph.add_node(create_node("A", vec![]));
        graph.add_node(create_node("B", vec!["A"]));
        graph.add_node(create_node("C", vec!["B"]));

        graph.add_edge("B", "A");
        graph.add_edge("C", "B");

        let diamonds = detect_diamonds(&graph);
        assert!(diamonds.is_empty());
    }

    #[test]
    fn test_no_diamond_disjoint_parents() {
        let mut graph = InheritanceGraph::new();

        // D has two parents with no common ancestor
        graph.add_node(create_node("A", vec![]));
        graph.add_node(create_node("B", vec![]));
        graph.add_node(create_node("D", vec!["A", "B"]));

        graph.add_edge("D", "A");
        graph.add_edge("D", "B");

        let diamonds = detect_diamonds(&graph);
        assert!(diamonds.is_empty());
    }
}
