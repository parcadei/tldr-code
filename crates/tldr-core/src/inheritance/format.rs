//! Output formatting for inheritance analysis
//!
//! Supports:
//! - DOT format for Graphviz visualization (A19 - proper escaping)
//! - Text format for human-readable output
//! - JSON is handled by serde serialization

use crate::types::{BaseResolution, InheritanceNode, InheritanceReport};

/// Escape a string for DOT format (A19 mitigation)
///
/// DOT requires escaping:
/// - Backslashes
/// - Double quotes
/// - Angle brackets in HTML-like labels
/// - Newlines
pub fn escape_dot_string(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 8);

    for c in s.chars() {
        match c {
            '\\' => result.push_str("\\\\"),
            '"' => result.push_str("\\\""),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '<' => result.push_str("\\<"),
            '>' => result.push_str("\\>"),
            '{' => result.push_str("\\{"),
            '}' => result.push_str("\\}"),
            '|' => result.push_str("\\|"),
            _ => result.push(c),
        }
    }

    result
}

/// Format the inheritance report as DOT (Graphviz) output
pub fn format_dot(report: &InheritanceReport) -> String {
    let mut out = String::new();

    out.push_str("digraph inheritance {\n");
    out.push_str("    rankdir=BT;\n");
    out.push_str("    node [shape=box, fontname=\"Helvetica\"];\n");
    out.push_str("    edge [arrowhead=empty];\n");
    out.push('\n');

    // Output nodes
    for node in &report.nodes {
        let name_escaped = escape_dot_string(&node.name);

        // Build label
        let mut label_parts = Vec::new();

        // Add stereotypes for abstract/protocol/interface/mixin
        if node.is_abstract == Some(true) {
            label_parts.push("<<abstract>>".to_string());
        }
        if node.protocol == Some(true) {
            label_parts.push("<<protocol>>".to_string());
        }
        if node.interface == Some(true) {
            label_parts.push("<<interface>>".to_string());
        }
        if node.mixin == Some(true) {
            label_parts.push("<<mixin>>".to_string());
        }

        // Class name
        label_parts.push(name_escaped.clone());

        // File location
        let file_name = node
            .file
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| "?".to_string());
        label_parts.push(format!("({}:{})", escape_dot_string(&file_name), node.line));

        let label = label_parts.join("\\n");

        // Node styling
        let mut attrs = vec![format!("label=\"{}\"", label)];

        // Color based on type
        if node.is_abstract == Some(true) || node.protocol == Some(true) {
            attrs.push("style=filled".to_string());
            attrs.push("fillcolor=lightyellow".to_string());
        } else if node.interface == Some(true) {
            attrs.push("style=filled".to_string());
            attrs.push("fillcolor=lightblue".to_string());
        } else if node.mixin == Some(true) {
            attrs.push("style=filled".to_string());
            attrs.push("fillcolor=lightgreen".to_string());
        }

        out.push_str(&format!(
            "    \"{}\" [{}];\n",
            name_escaped,
            attrs.join(", ")
        ));
    }

    out.push('\n');

    // Output edges
    for edge in &report.edges {
        let child_escaped = escape_dot_string(&edge.child);
        let parent_escaped = escape_dot_string(&edge.parent);

        // Different edge styles based on kind and resolution
        let mut edge_attrs = Vec::new();

        // External edges are dashed
        if edge.external {
            edge_attrs.push("style=dashed".to_string());

            // Different colors for stdlib vs unresolved
            match edge.resolution {
                BaseResolution::Stdlib => {
                    edge_attrs.push("color=blue".to_string());
                }
                BaseResolution::Unresolved => {
                    edge_attrs.push("color=gray".to_string());
                }
                _ => {}
            }
        }

        let edge_str = if edge_attrs.is_empty() {
            format!("    \"{}\" -> \"{}\";\n", child_escaped, parent_escaped)
        } else {
            format!(
                "    \"{}\" -> \"{}\" [{}];\n",
                child_escaped,
                parent_escaped,
                edge_attrs.join(", ")
            )
        };

        out.push_str(&edge_str);
    }

    // Add external nodes (bases not in project)
    let node_names: std::collections::HashSet<_> = report.nodes.iter().map(|n| &n.name).collect();

    for edge in &report.edges {
        if edge.external && !node_names.contains(&edge.parent) {
            let parent_escaped = escape_dot_string(&edge.parent);

            let color = match edge.resolution {
                BaseResolution::Stdlib => "lightblue",
                BaseResolution::Unresolved => "lightgray",
                _ => "white",
            };

            out.push_str(&format!(
                "    \"{}\" [label=\"{}\\n(external)\", style=filled, fillcolor={}, shape=ellipse];\n",
                parent_escaped, parent_escaped, color
            ));
        }
    }

    out.push_str("}\n");

    out
}

/// Check if the report is Rust-only with no meaningful hierarchy
///
/// Returns true when all languages are Rust and every node is a root
/// (meaning no trait impl edges exist -- just a flat list of structs/enums).
fn is_rust_all_roots(report: &InheritanceReport) -> bool {
    if report.languages.is_empty() {
        return false;
    }
    let all_rust = report
        .languages
        .iter()
        .all(|l| matches!(l, crate::types::Language::Rust));
    if !all_rust {
        return false;
    }
    // All nodes are roots means no non-external edges exist
    let has_internal_edges = report.edges.iter().any(|e| !e.external);
    !has_internal_edges && report.count > 0
}

/// Format a Rust-specific notice when all types are independent (no hierarchy)
fn format_rust_notice(report: &InheritanceReport) -> String {
    let mut out = String::new();

    out.push_str("=== Inheritance Graph ===\n\n");

    out.push_str(&format!("Project: {}\n", report.project_path.display()));
    out.push_str(&format!("Types found: {}\n", report.count));
    out.push_str(&format!(
        "Languages: {}\n",
        format_languages(&report.languages)
    ));
    out.push_str(&format!("Scan time: {}ms\n", report.scan_time_ms));
    out.push('\n');

    out.push_str("Rust Language Notice\n");
    out.push_str("====================\n");
    out.push_str(&format!(
        "Rust does not use class-based inheritance. All {} types are independent structs/enums.\n",
        report.count
    ));
    out.push_str(
        "No trait implementations (impl Trait for Type) were found between project types.\n",
    );
    out.push('\n');
    out.push_str("For trait implementations and design patterns, use:\n");
    out.push_str("  tldr patterns <path>        # Design patterns including trait usage\n");
    out.push_str("  tldr deps <path>            # Module dependency graph\n");
    out.push_str("  tldr coupling <path>        # Module coupling metrics\n");

    out
}

/// Format the inheritance report as human-readable text
pub fn format_text(report: &InheritanceReport) -> String {
    // Rust-specific: when all types are independent roots, show a helpful notice
    if is_rust_all_roots(report) {
        return format_rust_notice(report);
    }

    let mut out = String::new();

    // Header
    out.push_str("=== Inheritance Graph ===\n\n");

    out.push_str(&format!("Project: {}\n", report.project_path.display()));
    out.push_str(&format!("Classes found: {}\n", report.count));
    out.push_str(&format!(
        "Languages: {}\n",
        format_languages(&report.languages)
    ));
    out.push_str(&format!("Scan time: {}ms\n", report.scan_time_ms));
    out.push('\n');

    // Diamond warnings
    if !report.diamonds.is_empty() {
        out.push_str("!!! Diamond Inheritance Detected !!!\n");
        for diamond in &report.diamonds {
            out.push_str(&format!(
                "  {} has multiple paths to {}\n",
                diamond.class_name, diamond.common_ancestor
            ));
            for (i, path) in diamond.paths.iter().enumerate() {
                out.push_str(&format!("    Path {}: {}\n", i + 1, path.join(" -> ")));
            }
        }
        out.push('\n');
    }

    // Roots (classes with no parents in project)
    out.push_str("Roots (no project parents):\n");
    if report.roots.is_empty() {
        out.push_str("  (none)\n");
    } else {
        for root in &report.roots {
            let node = report.nodes.iter().find(|n| &n.name == root);
            let info = node.map(format_node_info).unwrap_or_default();
            out.push_str(&format!("  {} {}\n", root, info));
        }
    }
    out.push('\n');

    // Leaves (classes with no children)
    out.push_str("Leaves (no children):\n");
    if report.leaves.is_empty() {
        out.push_str("  (none)\n");
    } else {
        for leaf in &report.leaves {
            let node = report.nodes.iter().find(|n| &n.name == leaf);
            let info = node.map(format_node_info).unwrap_or_default();
            out.push_str(&format!("  {} {}\n", leaf, info));
        }
    }
    out.push('\n');

    // Hierarchy tree
    out.push_str("Hierarchy:\n");
    let trees = build_text_trees(report);
    for tree in trees {
        out.push_str(&tree);
    }

    // Mixins
    let mixins: Vec<_> = report
        .nodes
        .iter()
        .filter(|n| n.mixin == Some(true))
        .collect();
    if !mixins.is_empty() {
        out.push_str("\nMixins:\n");
        for mixin in mixins {
            out.push_str(&format!("  {} ({})\n", mixin.name, mixin.file.display()));
        }
    }

    out
}

fn format_languages(languages: &[crate::types::Language]) -> String {
    languages
        .iter()
        .map(|l| format!("{:?}", l).to_lowercase())
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_node_info(node: &InheritanceNode) -> String {
    let mut tags = Vec::new();

    if node.is_abstract == Some(true) {
        tags.push("abstract");
    }
    if node.protocol == Some(true) {
        tags.push("protocol");
    }
    if node.interface == Some(true) {
        tags.push("interface");
    }
    if node.mixin == Some(true) {
        tags.push("mixin");
    }

    if tags.is_empty() {
        String::new()
    } else {
        format!("[{}]", tags.join(", "))
    }
}

fn build_text_trees(report: &InheritanceReport) -> Vec<String> {
    // Build a simple indented tree starting from roots
    let mut trees = Vec::new();

    // Build child map
    let mut children_map: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for edge in &report.edges {
        if !edge.external {
            children_map
                .entry(edge.parent.clone())
                .or_default()
                .push(edge.child.clone());
        }
    }

    // Start from roots
    for root in &report.roots {
        let mut tree = String::new();
        build_tree_recursive(
            root,
            &children_map,
            &report.nodes,
            0,
            &mut tree,
            &mut std::collections::HashSet::new(),
        );
        trees.push(tree);
    }

    trees
}

fn build_tree_recursive(
    name: &str,
    children_map: &std::collections::HashMap<String, Vec<String>>,
    nodes: &[InheritanceNode],
    depth: usize,
    out: &mut String,
    visited: &mut std::collections::HashSet<String>,
) {
    // Prevent infinite loops in case of cycles
    if visited.contains(name) {
        return;
    }
    visited.insert(name.to_string());

    let indent = "  ".repeat(depth);
    let node = nodes.iter().find(|n| n.name == name);
    let info = node.map(format_node_info).unwrap_or_default();

    out.push_str(&format!("{}{} {}\n", indent, name, info));

    if let Some(children) = children_map.get(name) {
        let mut sorted_children = children.clone();
        sorted_children.sort();
        for child in sorted_children {
            build_tree_recursive(&child, children_map, nodes, depth + 1, out, visited);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{DiamondPattern, InheritanceEdge, Language};
    use std::path::PathBuf;

    fn create_test_report() -> InheritanceReport {
        let mut report = InheritanceReport::new(PathBuf::from("/test/project"));
        report.count = 3;
        report.languages = vec![Language::Python];
        report.scan_time_ms = 42;

        let mut animal =
            InheritanceNode::new("Animal", PathBuf::from("animals.py"), 1, Language::Python);
        animal.is_abstract = Some(true);
        report.nodes.push(animal);

        report.nodes.push(InheritanceNode::new(
            "Dog",
            PathBuf::from("animals.py"),
            10,
            Language::Python,
        ));

        report.nodes.push(InheritanceNode::new(
            "Cat",
            PathBuf::from("animals.py"),
            20,
            Language::Python,
        ));

        report.edges.push(InheritanceEdge::project(
            "Dog",
            "Animal",
            PathBuf::from("animals.py"),
            10,
            PathBuf::from("animals.py"),
            1,
        ));

        report.edges.push(InheritanceEdge::project(
            "Cat",
            "Animal",
            PathBuf::from("animals.py"),
            20,
            PathBuf::from("animals.py"),
            1,
        ));

        report.roots = vec!["Animal".to_string()];
        report.leaves = vec!["Dog".to_string(), "Cat".to_string()];

        report
    }

    #[test]
    fn test_escape_dot_string() {
        assert_eq!(escape_dot_string("Hello"), "Hello");
        assert_eq!(escape_dot_string("Hello\"World"), "Hello\\\"World");
        assert_eq!(escape_dot_string("Line1\nLine2"), "Line1\\nLine2");
        assert_eq!(escape_dot_string("A<B>C"), "A\\<B\\>C");
    }

    #[test]
    fn test_format_dot_basic() {
        let report = create_test_report();
        let dot = format_dot(&report);

        assert!(dot.starts_with("digraph inheritance"));
        assert!(dot.contains("rankdir=BT"));
        assert!(dot.contains("\"Dog\" -> \"Animal\""));
        assert!(dot.contains("\"Cat\" -> \"Animal\""));
        assert!(dot.contains("<<abstract>>"));
    }

    #[test]
    fn test_format_text_basic() {
        let report = create_test_report();
        let text = format_text(&report);

        assert!(text.contains("Inheritance Graph"));
        assert!(text.contains("Classes found: 3"));
        assert!(text.contains("Animal"));
        assert!(text.contains("Dog"));
        assert!(text.contains("Cat"));
    }

    #[test]
    fn test_format_text_diamonds() {
        let mut report = create_test_report();
        report.diamonds.push(DiamondPattern {
            class_name: "D".to_string(),
            common_ancestor: "A".to_string(),
            paths: vec![
                vec!["D".to_string(), "B".to_string(), "A".to_string()],
                vec!["D".to_string(), "C".to_string(), "A".to_string()],
            ],
        });

        let text = format_text(&report);
        assert!(text.contains("Diamond Inheritance Detected"));
        assert!(text.contains("D has multiple paths to A"));
    }

    /// When Rust code has no hierarchy (all nodes are roots), output should
    /// contain a Rust-specific notice instead of a useless flat list.
    #[test]
    fn test_format_text_rust_all_roots_shows_notice() {
        let mut report = InheritanceReport::new(PathBuf::from("/test/rust-project"));
        report.count = 3;
        report.languages = vec![Language::Rust];
        report.scan_time_ms = 10;

        // Three independent structs - no edges, all roots
        report.nodes.push(InheritanceNode::new(
            "Config",
            PathBuf::from("config.rs"),
            1,
            Language::Rust,
        ));
        report.nodes.push(InheritanceNode::new(
            "State",
            PathBuf::from("state.rs"),
            1,
            Language::Rust,
        ));
        report.nodes.push(InheritanceNode::new(
            "Error",
            PathBuf::from("error.rs"),
            1,
            Language::Rust,
        ));

        report.roots = vec![
            "Config".to_string(),
            "State".to_string(),
            "Error".to_string(),
        ];
        report.leaves = vec![
            "Config".to_string(),
            "State".to_string(),
            "Error".to_string(),
        ];
        // No edges

        let text = format_text(&report);

        // Should contain the Rust notice
        assert!(
            text.contains("Rust does not use class-based inheritance"),
            "Expected Rust notice in output, got:\n{}",
            text
        );
        // Should suggest alternative commands
        assert!(
            text.contains("tldr patterns"),
            "Expected 'tldr patterns' suggestion in output, got:\n{}",
            text
        );
        // Should NOT show the flat "Roots (no project parents):" list
        assert!(
            !text.contains("Roots (no project parents):"),
            "Should not show generic Roots section for Rust all-roots case, got:\n{}",
            text
        );
    }

    /// When Rust code HAS actual hierarchy (trait impls), the normal output
    /// should appear without the Rust notice.
    #[test]
    fn test_format_text_rust_with_hierarchy_no_notice() {
        let mut report = InheritanceReport::new(PathBuf::from("/test/rust-project"));
        report.count = 2;
        report.languages = vec![Language::Rust];
        report.scan_time_ms = 10;

        // A trait and a struct implementing it
        let mut animal =
            InheritanceNode::new("Animal", PathBuf::from("traits.rs"), 1, Language::Rust);
        animal.interface = Some(true);
        animal.is_abstract = Some(true);
        report.nodes.push(animal);

        report.nodes.push(InheritanceNode::new(
            "Dog",
            PathBuf::from("dog.rs"),
            1,
            Language::Rust,
        ));

        report.edges.push(InheritanceEdge::project(
            "Dog",
            "Animal",
            PathBuf::from("dog.rs"),
            1,
            PathBuf::from("traits.rs"),
            1,
        ));

        report.roots = vec!["Animal".to_string()];
        report.leaves = vec!["Dog".to_string()];

        let text = format_text(&report);

        // Should NOT contain the Rust notice (hierarchy exists)
        assert!(
            !text.contains("Rust does not use class-based inheritance"),
            "Should not show Rust notice when hierarchy exists, got:\n{}",
            text
        );
        // Should show normal output
        assert!(
            text.contains("Roots (no project parents):"),
            "Expected normal Roots section, got:\n{}",
            text
        );
    }
}
