//! Class filtering for focused inheritance views
//!
//! Supports:
//! - Exact class name matching
//! - Fuzzy suggestions for typos (A17)
//! - Depth-limited traversal
//! - Ancestor + descendant filtering

use std::collections::{HashSet, VecDeque};

use crate::error::TldrError;
use crate::types::InheritanceGraph;
use crate::TldrResult;

/// Filter the graph to show only the specified class and its relatives
///
/// Returns a new graph containing:
/// - The target class
/// - All ancestors (up to depth, if specified)
/// - All descendants (up to depth, if specified)
pub fn filter_by_class(
    graph: &InheritanceGraph,
    class_name: &str,
    depth: Option<usize>,
) -> TldrResult<InheritanceGraph> {
    // Check if class exists
    if !graph.nodes.contains_key(class_name) {
        // Try fuzzy matching for suggestions
        let suggestions = get_fuzzy_suggestions(class_name, graph);

        let suggestion = if suggestions.is_empty() {
            None
        } else {
            Some(format!("Did you mean: {}?", suggestions.join(", ")))
        };

        return Err(TldrError::NotFound {
            entity: "class".to_string(),
            name: class_name.to_string(),
            suggestion,
        });
    }

    let max_depth = depth.unwrap_or(usize::MAX);

    // Collect nodes to include using BFS
    let mut included = HashSet::new();
    included.insert(class_name.to_string());

    // BFS for ancestors
    collect_ancestors(graph, class_name, max_depth, &mut included);

    // BFS for descendants
    collect_descendants(graph, class_name, max_depth, &mut included);

    // Build filtered graph
    let mut filtered = InheritanceGraph::new();

    for name in &included {
        if let Some(node) = graph.nodes.get(name) {
            filtered.add_node(node.clone());
        }
    }

    // Add edges between included nodes
    for (child, parents) in &graph.parents {
        if included.contains(child) {
            for parent in parents {
                if included.contains(parent) {
                    filtered.add_edge(child, parent);
                }
            }
        }
    }

    Ok(filtered)
}

/// Collect ancestors up to max_depth using BFS
fn collect_ancestors(
    graph: &InheritanceGraph,
    start: &str,
    max_depth: usize,
    included: &mut HashSet<String>,
) {
    let mut queue: VecDeque<(String, usize)> = VecDeque::new();

    if let Some(parents) = graph.parents.get(start) {
        for parent in parents {
            queue.push_back((parent.clone(), 1));
        }
    }

    while let Some((current, depth)) = queue.pop_front() {
        if depth > max_depth {
            continue;
        }

        if included.insert(current.clone()) {
            if let Some(parents) = graph.parents.get(&current) {
                for parent in parents {
                    queue.push_back((parent.clone(), depth + 1));
                }
            }
        }
    }
}

/// Collect descendants up to max_depth using BFS
fn collect_descendants(
    graph: &InheritanceGraph,
    start: &str,
    max_depth: usize,
    included: &mut HashSet<String>,
) {
    let mut queue: VecDeque<(String, usize)> = VecDeque::new();

    if let Some(children) = graph.children.get(start) {
        for child in children {
            queue.push_back((child.clone(), 1));
        }
    }

    while let Some((current, depth)) = queue.pop_front() {
        if depth > max_depth {
            continue;
        }

        if included.insert(current.clone()) {
            if let Some(children) = graph.children.get(&current) {
                for child in children {
                    queue.push_back((child.clone(), depth + 1));
                }
            }
        }
    }
}

/// Get fuzzy match suggestions for a class name typo
///
/// Uses Levenshtein distance to find similar class names
pub fn get_fuzzy_suggestions(query: &str, graph: &InheritanceGraph) -> Vec<String> {
    let query_lower = query.to_lowercase();
    let mut candidates: Vec<(String, usize)> = Vec::new();

    for name in graph.nodes.keys() {
        let name_lower = name.to_lowercase();

        // Exact prefix match
        if name_lower.starts_with(&query_lower) || query_lower.starts_with(&name_lower) {
            candidates.push((name.clone(), 0));
            continue;
        }

        // Levenshtein distance for small queries
        if query.len() <= 20 && name.len() <= 30 {
            let distance = levenshtein(&query_lower, &name_lower);
            // Only suggest if distance is reasonable (< 40% of query length)
            let threshold = (query.len() as f64 * 0.4).ceil() as usize + 2;
            if distance <= threshold {
                candidates.push((name.clone(), distance));
            }
        }
    }

    // Sort by distance and take top 3
    candidates.sort_by_key(|(_, d)| *d);
    candidates.truncate(3);

    candidates.into_iter().map(|(name, _)| name).collect()
}

/// Simple Levenshtein distance implementation
fn levenshtein(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let m = a_chars.len();
    let n = b_chars.len();

    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }

    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr: Vec<usize> = vec![0; n + 1];

    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[n]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{InheritanceNode, Language};
    use std::path::PathBuf;

    fn create_test_graph() -> InheritanceGraph {
        let mut graph = InheritanceGraph::new();

        // A -> B -> C -> D chain
        // A -> E
        graph.add_node(InheritanceNode::new(
            "A",
            PathBuf::from("test.py"),
            1,
            Language::Python,
        ));
        graph.add_node(InheritanceNode::new(
            "B",
            PathBuf::from("test.py"),
            2,
            Language::Python,
        ));
        graph.add_node(InheritanceNode::new(
            "C",
            PathBuf::from("test.py"),
            3,
            Language::Python,
        ));
        graph.add_node(InheritanceNode::new(
            "D",
            PathBuf::from("test.py"),
            4,
            Language::Python,
        ));
        graph.add_node(InheritanceNode::new(
            "E",
            PathBuf::from("test.py"),
            5,
            Language::Python,
        ));

        graph.add_edge("B", "A");
        graph.add_edge("C", "B");
        graph.add_edge("D", "C");
        graph.add_edge("E", "A");

        graph
    }

    #[test]
    fn test_filter_exact_match() {
        let graph = create_test_graph();
        let filtered = filter_by_class(&graph, "B", None).unwrap();

        // B should include A (ancestor), C and D (descendants)
        assert!(filtered.nodes.contains_key("A"));
        assert!(filtered.nodes.contains_key("B"));
        assert!(filtered.nodes.contains_key("C"));
        assert!(filtered.nodes.contains_key("D"));
        // E is not related to B
        assert!(!filtered.nodes.contains_key("E"));
    }

    #[test]
    fn test_filter_with_depth_limit() {
        let graph = create_test_graph();
        let filtered = filter_by_class(&graph, "B", Some(1)).unwrap();

        // Depth 1: A (parent), C (child)
        assert!(filtered.nodes.contains_key("A"));
        assert!(filtered.nodes.contains_key("B"));
        assert!(filtered.nodes.contains_key("C"));
        // D is depth 2 from B
        assert!(!filtered.nodes.contains_key("D"));
    }

    #[test]
    fn test_filter_not_found() {
        let graph = create_test_graph();
        let result = filter_by_class(&graph, "NotExists", None);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("NotExists"));
    }

    #[test]
    fn test_fuzzy_suggestions() {
        let mut graph = InheritanceGraph::new();
        graph.add_node(InheritanceNode::new(
            "Animal",
            PathBuf::from("test.py"),
            1,
            Language::Python,
        ));
        graph.add_node(InheritanceNode::new(
            "Mammal",
            PathBuf::from("test.py"),
            2,
            Language::Python,
        ));
        graph.add_node(InheritanceNode::new(
            "Dog",
            PathBuf::from("test.py"),
            3,
            Language::Python,
        ));

        let suggestions = get_fuzzy_suggestions("Anmal", &graph); // Typo
        assert!(!suggestions.is_empty());
        assert!(suggestions.contains(&"Animal".to_string()));
    }

    #[test]
    fn test_levenshtein() {
        assert_eq!(levenshtein("", ""), 0);
        assert_eq!(levenshtein("abc", "abc"), 0);
        assert_eq!(levenshtein("abc", "ab"), 1);
        assert_eq!(levenshtein("abc", "abcd"), 1);
        assert_eq!(levenshtein("abc", "abd"), 1);
        assert_eq!(levenshtein("kitten", "sitting"), 3);
    }
}
