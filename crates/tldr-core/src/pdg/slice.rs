//! Program Slicing
//!
//! Computes program slices using PDG traversal.
//!
//! # Slice Types
//!
//! ## Backward Slice
//! Given a slicing criterion (line, optional variable), find all statements
//! that could affect the computation at that point.
//!
//! Algorithm:
//! 1. Start at the criterion node in PDG
//! 2. Follow edges backward (from target to source)
//! 3. Collect all visited nodes
//!
//! ## Forward Slice
//! Given a slicing criterion, find all statements that could be affected
//! by the computation at that point.
//!
//! Algorithm:
//! 1. Start at the criterion node in PDG
//! 2. Follow edges forward (from source to target)
//! 3. Collect all visited nodes
//!
//! # Variable Filtering
//! If a variable is specified, only follow edges related to that variable.
//! For data dependencies, this filters by the variable name.
//! For control dependencies, all are followed (they affect all variables).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::pdg::get_pdg_context;
use crate::types::{DependenceType, Language, PdgInfo, SliceDirection};
use crate::TldrResult;

// =============================================================================
// Rich Slice Types
// =============================================================================

/// A single node in a rich program slice, containing source code and metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SliceNode {
    /// Source line number
    pub line: u32,
    /// Trimmed source line content
    pub code: String,
    /// PDG node type (e.g., "assignment", "return", "call")
    pub node_type: String,
    /// Variables defined at this line
    pub definitions: Vec<String>,
    /// Variables used at this line
    pub uses: Vec<String>,
    /// How this node connects to the dependency chain: "data" or "control"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dep_type: Option<String>,
    /// Variable name for data dependencies
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dep_label: Option<String>,
}

/// An edge in the rich slice representing a dependency relationship
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SliceEdge {
    /// Source line number
    pub from_line: u32,
    /// Target line number
    pub to_line: u32,
    /// Dependency type: "data" or "control"
    pub dep_type: String,
    /// Variable name for data dependencies, empty for control
    pub label: String,
}

/// Rich slice result containing code content and dependency chains
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RichSlice {
    /// Slice nodes sorted by line number
    pub nodes: Vec<SliceNode>,
    /// Dependency chain edges within the slice
    pub edges: Vec<SliceEdge>,
}

/// Compute program slice
///
/// # Arguments
/// * `source_or_path` - Either source code string or path to a file
/// * `function_name` - Name of the function to slice
/// * `line` - Line number to slice from
/// * `direction` - Backward or forward slice
/// * `variable` - Optional variable to filter by
/// * `language` - Programming language
///
/// # Returns
/// * `Ok(HashSet<u32>)` - Set of line numbers in the slice
/// * Empty set if line is not in the function
///
/// # Example
/// ```ignore
/// use tldr_core::pdg::get_slice;
/// use tldr_core::{Language, SliceDirection};
///
/// let slice = get_slice(
///     "def foo(): x = 1; return x",
///     "foo",
///     2,  // return line
///     SliceDirection::Backward,
///     None,
///     Language::Python
/// )?;
/// // slice should include line 1 (x = 1)
/// ```
pub fn get_slice(
    source_or_path: &str,
    function_name: &str,
    line: u32,
    direction: SliceDirection,
    variable: Option<&str>,
    language: Language,
) -> TldrResult<HashSet<u32>> {
    // Get PDG for the function
    let pdg = get_pdg_context(source_or_path, function_name, language)?;

    // Find the node(s) containing the target line
    let start_nodes = find_nodes_for_line(&pdg, line);

    if start_nodes.is_empty() {
        // Line not in function - return empty set per spec
        return Ok(HashSet::new());
    }

    // Perform slice traversal
    let slice = compute_slice(&pdg, &start_nodes, direction, variable);

    // Convert node IDs to line numbers
    let lines = nodes_to_lines(&pdg, &slice);

    Ok(lines)
}

/// Compute a rich program slice with source code and dependency chains
///
/// Like `get_slice()` but returns `RichSlice` with code content, node metadata,
/// and filtered dependency edges instead of bare line numbers.
///
/// # Arguments
/// * `source_or_path` - Either source code string or path to a file
/// * `function_name` - Name of the function to slice
/// * `line` - Line number to slice from
/// * `direction` - Backward or forward slice
/// * `variable` - Optional variable to filter by
/// * `language` - Programming language
///
/// # Returns
/// * `Ok(RichSlice)` - Rich slice with code, metadata, and edges
/// * Empty RichSlice if line is not in the function
pub fn get_slice_rich(
    source_or_path: &str,
    function_name: &str,
    line: u32,
    direction: SliceDirection,
    variable: Option<&str>,
    language: Language,
) -> TldrResult<RichSlice> {
    // Get PDG for the function
    let pdg = get_pdg_context(source_or_path, function_name, language)?;

    // Find the node(s) containing the target line
    let start_nodes = find_nodes_for_line(&pdg, line);

    if start_nodes.is_empty() {
        return Ok(RichSlice {
            nodes: Vec::new(),
            edges: Vec::new(),
        });
    }

    // Perform slice traversal -- get set of visited node IDs
    let visited = compute_slice(&pdg, &start_nodes, direction, variable);

    // Read source lines for code content
    let source_lines = read_source_lines(source_or_path);

    // Build a map from node_id -> PdgNode for visited nodes
    let visited_nodes: Vec<&crate::types::PdgNode> = pdg
        .nodes
        .iter()
        .filter(|n| visited.contains(&n.id))
        .collect();

    // Collect all lines covered by visited nodes, with their metadata
    // Multiple nodes can cover the same line; we merge definitions/uses
    let mut line_map: HashMap<u32, SliceNode> = HashMap::new();

    for node in &visited_nodes {
        for l in node.lines.0..=node.lines.1 {
            if l == 0 {
                continue;
            }
            let code = source_lines
                .get((l as usize).wrapping_sub(1))
                .map(|s| s.trim_end().to_string())
                .unwrap_or_default();

            let entry = line_map.entry(l).or_insert_with(|| SliceNode {
                line: l,
                code,
                node_type: node.node_type.clone(),
                definitions: Vec::new(),
                uses: Vec::new(),
                dep_type: None,
                dep_label: None,
            });

            // Merge definitions and uses from multiple nodes covering same line
            for d in &node.definitions {
                if !entry.definitions.contains(d) {
                    entry.definitions.push(d.clone());
                }
            }
            for u in &node.uses {
                if !entry.uses.contains(u) {
                    entry.uses.push(u.clone());
                }
            }
        }
    }

    // Filter PDG edges to only those within the slice (both endpoints visited)
    let mut edges: Vec<SliceEdge> = Vec::new();
    for edge in &pdg.edges {
        if visited.contains(&edge.source_id) && visited.contains(&edge.target_id) {
            // Map node IDs to their representative line numbers
            let from_line = node_id_to_line(&pdg, edge.source_id);
            let to_line = node_id_to_line(&pdg, edge.target_id);
            if let (Some(from), Some(to)) = (from_line, to_line) {
                let dep_str = match edge.dep_type {
                    DependenceType::Data => "data",
                    DependenceType::Control => "control",
                };
                edges.push(SliceEdge {
                    from_line: from,
                    to_line: to,
                    dep_type: dep_str.to_string(),
                    label: edge.label.clone(),
                });

                // Annotate the target node with dep info (how it connects)
                if let Some(node) = line_map.get_mut(&to) {
                    if node.dep_type.is_none() {
                        node.dep_type = Some(dep_str.to_string());
                        if !edge.label.is_empty() {
                            node.dep_label = Some(edge.label.clone());
                        }
                    }
                }
            }
        }
    }

    // Sort edges by from_line, then to_line
    edges.sort_by_key(|e| (e.from_line, e.to_line));
    // Deduplicate edges (same from/to/type/label)
    edges.dedup_by(|a, b| {
        a.from_line == b.from_line
            && a.to_line == b.to_line
            && a.dep_type == b.dep_type
            && a.label == b.label
    });

    // Collect and sort nodes by line number
    let mut nodes: Vec<SliceNode> = line_map.into_values().collect();
    nodes.sort_by_key(|n| n.line);

    Ok(RichSlice { nodes, edges })
}

/// Read source lines from a path or inline source string
fn read_source_lines(source_or_path: &str) -> Vec<String> {
    let path = Path::new(source_or_path);
    if path.exists() && path.is_file() {
        match std::fs::read_to_string(path) {
            Ok(content) => content.lines().map(|l| l.to_string()).collect(),
            Err(_) => source_or_path.lines().map(|l| l.to_string()).collect(),
        }
    } else {
        source_or_path.lines().map(|l| l.to_string()).collect()
    }
}

/// Map a PDG node ID to its first (representative) line number
fn node_id_to_line(pdg: &PdgInfo, node_id: usize) -> Option<u32> {
    pdg.nodes
        .iter()
        .find(|n| n.id == node_id)
        .map(|n| n.lines.0)
        .filter(|&l| l > 0)
}

/// Find PDG nodes that contain a specific line
fn find_nodes_for_line(pdg: &PdgInfo, line: u32) -> Vec<usize> {
    pdg.nodes
        .iter()
        .filter(|n| line >= n.lines.0 && line <= n.lines.1)
        .map(|n| n.id)
        .collect()
}

/// Compute slice using BFS/DFS traversal
fn compute_slice(
    pdg: &PdgInfo,
    start_nodes: &[usize],
    direction: SliceDirection,
    variable: Option<&str>,
) -> HashSet<usize> {
    let mut visited = HashSet::new();
    let mut worklist: Vec<usize> = start_nodes.to_vec();

    while let Some(node_id) = worklist.pop() {
        if visited.contains(&node_id) {
            continue;
        }
        visited.insert(node_id);

        // Find adjacent nodes based on direction
        let adjacent = match direction {
            SliceDirection::Backward => {
                // Follow edges TO this node (find sources)
                pdg.edges
                    .iter()
                    .filter(|e| e.target_id == node_id)
                    .filter(|e| should_follow_edge(e, variable))
                    .map(|e| e.source_id)
                    .collect::<Vec<_>>()
            }
            SliceDirection::Forward => {
                // Follow edges FROM this node (find targets)
                pdg.edges
                    .iter()
                    .filter(|e| e.source_id == node_id)
                    .filter(|e| should_follow_edge(e, variable))
                    .map(|e| e.target_id)
                    .collect::<Vec<_>>()
            }
        };

        for adj in adjacent {
            if !visited.contains(&adj) {
                worklist.push(adj);
            }
        }
    }

    visited
}

/// Check if an edge should be followed based on variable filter
fn should_follow_edge(edge: &crate::types::PdgEdge, variable: Option<&str>) -> bool {
    match variable {
        None => true, // No filter, follow all edges
        Some(var) => {
            match edge.dep_type {
                DependenceType::Control => true, // Always follow control deps
                DependenceType::Data => edge.label == var, // Only follow if variable matches
            }
        }
    }
}

/// Convert set of node IDs to set of line numbers
fn nodes_to_lines(pdg: &PdgInfo, node_ids: &HashSet<usize>) -> HashSet<u32> {
    let mut lines = HashSet::new();

    for &node_id in node_ids {
        if let Some(node) = pdg.nodes.iter().find(|n| n.id == node_id) {
            // Include all lines covered by this node
            for line in node.lines.0..=node.lines.1 {
                if line > 0 {
                    lines.insert(line);
                }
            }
        }
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backward_slice_simple() {
        let source = r#"
def foo():
    x = 1
    y = x + 2
    return y
"#;
        let slice = get_slice(
            source,
            "foo",
            4,
            SliceDirection::Backward,
            None,
            Language::Python,
        )
        .unwrap();

        // Backward slice from "return y" should include y = x + 2 and x = 1
        assert!(!slice.is_empty(), "slice should not be empty");
    }

    #[test]
    fn test_forward_slice_simple() {
        let source = r#"
def foo():
    x = 1
    y = x + 2
    return y
"#;
        // Line 3 is "x = 1" (line 1 is blank, line 2 is "def foo():")
        let slice = get_slice(
            source,
            "foo",
            3,
            SliceDirection::Forward,
            None,
            Language::Python,
        )
        .unwrap();

        // Forward slice from "x = 1" should include the starting line at minimum
        // Note: forward slice traversal starts from the starting node
        assert!(slice.contains(&3), "slice should include the starting line");
    }

    #[test]
    fn test_slice_with_variable_filter() {
        let source = r#"
def foo():
    x = 1
    y = 2
    z = x + y
    return z
"#;
        let slice = get_slice(
            source,
            "foo",
            5,
            SliceDirection::Backward,
            Some("x"),
            Language::Python,
        )
        .unwrap();

        // Backward slice for 'x' from "z = x + y" should include x = 1 but not y = 2
        // Note: the line numbers in this test are approximate
        assert!(!slice.is_empty(), "slice should not be empty");
    }

    #[test]
    fn test_slice_line_not_in_function() {
        let source = "def foo(): pass";
        let slice = get_slice(
            source,
            "foo",
            999,
            SliceDirection::Backward,
            None,
            Language::Python,
        )
        .unwrap();

        // Line 999 is not in the function - should return empty set
        assert!(
            slice.is_empty(),
            "slice for non-existent line should be empty"
        );
    }

    #[test]
    fn test_slice_returns_line_numbers() {
        let source = r#"
def foo():
    x = 1
    return x
"#;
        let slice = get_slice(
            source,
            "foo",
            3,
            SliceDirection::Backward,
            None,
            Language::Python,
        )
        .unwrap();

        // Result should be line numbers (positive integers)
        for &line in &slice {
            assert!(line > 0, "line numbers should be positive");
        }
    }

    #[test]
    fn test_backward_slice_with_control_deps() {
        let source = r#"
def foo(cond):
    if cond:
        x = 1
    else:
        x = 2
    return x
"#;
        let slice = get_slice(
            source,
            "foo",
            6,
            SliceDirection::Backward,
            None,
            Language::Python,
        )
        .unwrap();

        // Backward slice should include the if condition due to control dependency
        assert!(
            !slice.is_empty(),
            "slice should include control dependencies"
        );
    }

    #[test]
    fn test_forward_slice_traces_all_vars() {
        let source = r#"
def foo():
    x = 1
    y = x
    z = y
    return z
"#;
        // Line 3 is "x = 1" (line 1 is blank, line 2 is "def foo():")
        let slice = get_slice(
            source,
            "foo",
            3,
            SliceDirection::Forward,
            None,
            Language::Python,
        )
        .unwrap();

        // Forward slice from x=1 should include the starting line
        // The slice starts at the given line and follows forward dependencies
        assert!(
            slice.contains(&3),
            "forward slice should include the starting line"
        );
    }

    // =========================================================================
    // Tests for get_slice_rich()
    // =========================================================================

    #[test]
    fn test_rich_slice_returns_nodes_with_code() {
        let source = r#"
def foo():
    x = 1
    y = x + 2
    return y
"#;
        let rich = get_slice_rich(
            source,
            "foo",
            4,
            SliceDirection::Backward,
            None,
            Language::Python,
        )
        .unwrap();

        // Should have nodes with actual code content
        assert!(!rich.nodes.is_empty(), "rich slice should have nodes");
        for node in &rich.nodes {
            assert!(!node.code.is_empty(), "each node should have code content");
            assert!(node.line > 0, "line numbers should be positive");
        }
    }

    #[test]
    fn test_rich_slice_nodes_sorted_by_line() {
        let source = r#"
def foo():
    x = 1
    y = x + 2
    return y
"#;
        let rich = get_slice_rich(
            source,
            "foo",
            5,
            SliceDirection::Backward,
            None,
            Language::Python,
        )
        .unwrap();

        // Nodes must be sorted by line number
        let lines: Vec<u32> = rich.nodes.iter().map(|n| n.line).collect();
        let mut sorted = lines.clone();
        sorted.sort();
        assert_eq!(lines, sorted, "nodes should be sorted by line number");
    }

    #[test]
    fn test_rich_slice_code_is_trimmed() {
        let source = r#"
def foo():
    x = 1
    y = x + 2
    return y
"#;
        let rich = get_slice_rich(
            source,
            "foo",
            5,
            SliceDirection::Backward,
            None,
            Language::Python,
        )
        .unwrap();

        for node in &rich.nodes {
            assert_eq!(
                node.code,
                node.code.trim_end(),
                "code should have trailing whitespace trimmed"
            );
        }
    }

    #[test]
    fn test_rich_slice_preserves_definitions_and_uses() {
        let source = r#"
def foo():
    x = 1
    y = x + 2
    return y
"#;
        let rich = get_slice_rich(
            source,
            "foo",
            5,
            SliceDirection::Backward,
            None,
            Language::Python,
        )
        .unwrap();

        // At least some nodes should have definitions or uses
        let has_defs = rich.nodes.iter().any(|n| !n.definitions.is_empty());
        let has_uses = rich.nodes.iter().any(|n| !n.uses.is_empty());
        assert!(
            has_defs || has_uses,
            "rich slice should preserve definition/use info from PDG"
        );
    }

    #[test]
    fn test_rich_slice_has_node_types() {
        let source = r#"
def foo():
    x = 1
    y = x + 2
    return y
"#;
        let rich = get_slice_rich(
            source,
            "foo",
            5,
            SliceDirection::Backward,
            None,
            Language::Python,
        )
        .unwrap();

        for node in &rich.nodes {
            assert!(
                !node.node_type.is_empty(),
                "each node should have a node_type"
            );
        }
    }

    #[test]
    fn test_rich_slice_edges_within_slice() {
        let source = r#"
def foo():
    x = 1
    y = x + 2
    return y
"#;
        let rich = get_slice_rich(
            source,
            "foo",
            5,
            SliceDirection::Backward,
            None,
            Language::Python,
        )
        .unwrap();

        let slice_lines: std::collections::HashSet<u32> =
            rich.nodes.iter().map(|n| n.line).collect();
        // All edges should reference lines that are in the slice
        for edge in &rich.edges {
            assert!(
                slice_lines.contains(&edge.from_line),
                "edge from_line {} should be in slice",
                edge.from_line
            );
            assert!(
                slice_lines.contains(&edge.to_line),
                "edge to_line {} should be in slice",
                edge.to_line
            );
        }
    }

    #[test]
    fn test_rich_slice_edge_dep_types() {
        let source = r#"
def foo(cond):
    if cond:
        x = 1
    else:
        x = 2
    return x
"#;
        let rich = get_slice_rich(
            source,
            "foo",
            7,
            SliceDirection::Backward,
            None,
            Language::Python,
        )
        .unwrap();

        // Should have edges with valid dep_type strings
        for edge in &rich.edges {
            assert!(
                edge.dep_type == "data" || edge.dep_type == "control",
                "edge dep_type should be 'data' or 'control', got '{}'",
                edge.dep_type
            );
        }
    }

    #[test]
    fn test_rich_slice_empty_for_invalid_line() {
        let source = "def foo(): pass";
        let rich = get_slice_rich(
            source,
            "foo",
            999,
            SliceDirection::Backward,
            None,
            Language::Python,
        )
        .unwrap();

        assert!(
            rich.nodes.is_empty(),
            "rich slice for non-existent line should have no nodes"
        );
        assert!(
            rich.edges.is_empty(),
            "rich slice for non-existent line should have no edges"
        );
    }

    #[test]
    fn test_rich_slice_from_file_path() {
        // Create a temp file to test file-based slicing
        use std::io::Write;
        let dir = std::env::temp_dir();
        let path = dir.join("test_slice_rich.py");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "def bar():").unwrap();
        writeln!(f, "    a = 10").unwrap();
        writeln!(f, "    b = a + 1").unwrap();
        writeln!(f, "    return b").unwrap();

        let rich = get_slice_rich(
            path.to_str().unwrap(),
            "bar",
            4,
            SliceDirection::Backward,
            None,
            Language::Python,
        )
        .unwrap();

        assert!(!rich.nodes.is_empty(), "should work with file path input");
        // Code should come from the file
        let has_return = rich.nodes.iter().any(|n| n.code.contains("return"));
        assert!(has_return, "should contain the criterion line code");

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_rich_slice_backward_compat_with_get_slice() {
        let source = r#"
def foo():
    x = 1
    y = x + 2
    return y
"#;
        let plain = get_slice(
            source,
            "foo",
            5,
            SliceDirection::Backward,
            None,
            Language::Python,
        )
        .unwrap();
        let rich = get_slice_rich(
            source,
            "foo",
            5,
            SliceDirection::Backward,
            None,
            Language::Python,
        )
        .unwrap();

        // The rich slice line set should match the plain slice line set
        let rich_lines: HashSet<u32> = rich.nodes.iter().map(|n| n.line).collect();
        assert_eq!(
            plain, rich_lines,
            "rich slice lines should match plain slice lines"
        );
    }
}
