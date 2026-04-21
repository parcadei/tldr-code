//! Inheritance analysis types for class hierarchy extraction
//!
//! This module defines types for the `inheritance` command (Phase 7-9).
//! Addresses blockers: A9 (InheritanceNode type not defined)

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use super::Language;

// =============================================================================
// Inheritance Node Types (A9)
// =============================================================================

/// A class or interface node in the inheritance graph
///
/// Represents a single class/interface with its inheritance relationships.
/// Supports Python, TypeScript, Go (struct embedding), and Rust (traits).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InheritanceNode {
    /// Class/interface name
    pub name: String,
    /// File containing this class
    pub file: PathBuf,
    /// Line number of class definition
    pub line: u32,
    /// Language of the source file
    pub language: Language,
    /// Base classes/interfaces this class extends
    pub bases: Vec<String>,
    /// Whether this is an abstract class (ABC in Python, abstract class in TS)
    /// Note: `abstract` is a reserved keyword in Rust
    #[serde(rename = "abstract")]
    pub is_abstract: Option<bool>,
    /// Whether this is a Protocol (Python typing.Protocol)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<bool>,
    /// Whether this is an interface (TypeScript interface)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interface: Option<bool>,
    /// Whether this is a mixin class
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mixin: Option<bool>,
    /// Python metaclass if specified (A12: Python metaclass support)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metaclass: Option<String>,
}

impl InheritanceNode {
    /// Create a new basic inheritance node
    pub fn new(name: impl Into<String>, file: PathBuf, line: u32, language: Language) -> Self {
        Self {
            name: name.into(),
            file,
            line,
            language,
            bases: Vec::new(),
            is_abstract: None,
            protocol: None,
            interface: None,
            mixin: None,
            metaclass: None,
        }
    }

    /// Add a base class
    pub fn with_base(mut self, base: impl Into<String>) -> Self {
        self.bases.push(base.into());
        self
    }

    /// Add multiple base classes
    pub fn with_bases(mut self, bases: Vec<String>) -> Self {
        self.bases = bases;
        self
    }

    /// Mark as abstract
    pub fn as_abstract(mut self) -> Self {
        self.is_abstract = Some(true);
        self
    }

    /// Mark as protocol
    pub fn as_protocol(mut self) -> Self {
        self.protocol = Some(true);
        self
    }

    /// Mark as interface
    pub fn as_interface(mut self) -> Self {
        self.interface = Some(true);
        self
    }

    /// Mark as mixin
    pub fn as_mixin(mut self) -> Self {
        self.mixin = Some(true);
        self
    }
}

// =============================================================================
// Inheritance Edge Types
// =============================================================================

/// Kind of inheritance relationship
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum InheritanceKind {
    /// Class extends another class (Python, TypeScript, Java)
    Extends,
    /// Class implements an interface (TypeScript, Java, Go)
    Implements,
    /// Struct embeds another struct (Go struct embedding - A14)
    Embeds,
}

/// Resolution status for a base class
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BaseResolution {
    /// Base class found in project
    Project,
    /// Base class is from standard library (Exception, ABC, etc.)
    Stdlib,
    /// External (third-party library) - not resolved
    Unresolved,
}

/// An edge in the inheritance graph (child -> parent)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InheritanceEdge {
    /// Child class name
    pub child: String,
    /// Parent class name
    pub parent: String,
    /// File containing the child class
    pub child_file: PathBuf,
    /// Line number of child class definition
    pub child_line: u32,
    /// File containing the parent class (None if external)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_file: Option<PathBuf>,
    /// Line number of parent class definition (None if external)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_line: Option<u32>,
    /// Kind of inheritance relationship
    pub kind: InheritanceKind,
    /// Whether the parent is external (not in project)
    pub external: bool,
    /// Resolution status of the base class
    pub resolution: BaseResolution,
}

impl InheritanceEdge {
    /// Create a new project-internal inheritance edge
    pub fn project(
        child: impl Into<String>,
        parent: impl Into<String>,
        child_file: PathBuf,
        child_line: u32,
        parent_file: PathBuf,
        parent_line: u32,
    ) -> Self {
        Self {
            child: child.into(),
            parent: parent.into(),
            child_file,
            child_line,
            parent_file: Some(parent_file),
            parent_line: Some(parent_line),
            kind: InheritanceKind::Extends,
            external: false,
            resolution: BaseResolution::Project,
        }
    }

    /// Create an edge to a stdlib base class
    pub fn stdlib(
        child: impl Into<String>,
        parent: impl Into<String>,
        child_file: PathBuf,
        child_line: u32,
    ) -> Self {
        Self {
            child: child.into(),
            parent: parent.into(),
            child_file,
            child_line,
            parent_file: None,
            parent_line: None,
            kind: InheritanceKind::Extends,
            external: true,
            resolution: BaseResolution::Stdlib,
        }
    }

    /// Create an edge to an unresolved (external) base class
    pub fn unresolved(
        child: impl Into<String>,
        parent: impl Into<String>,
        child_file: PathBuf,
        child_line: u32,
    ) -> Self {
        Self {
            child: child.into(),
            parent: parent.into(),
            child_file,
            child_line,
            parent_file: None,
            parent_line: None,
            kind: InheritanceKind::Extends,
            external: true,
            resolution: BaseResolution::Unresolved,
        }
    }

    /// Set the inheritance kind
    pub fn with_kind(mut self, kind: InheritanceKind) -> Self {
        self.kind = kind;
        self
    }
}

// =============================================================================
// Diamond Pattern Detection
// =============================================================================

/// A diamond inheritance pattern detected in the hierarchy
///
/// Diamond: Class D inherits from B and C, both of which inherit from A
/// ```text
///        A          <- common_ancestor
///       / \
///      B   C
///       \ /
///        D          <- class_name
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiamondPattern {
    /// The class at the bottom of the diamond
    pub class_name: String,
    /// The common ancestor at the top of the diamond
    pub common_ancestor: String,
    /// Multiple paths from class_name to common_ancestor
    /// Each path is a list of class names from child to ancestor
    pub paths: Vec<Vec<String>>,
}

impl DiamondPattern {
    /// Create a new diamond pattern
    pub fn new(class_name: impl Into<String>, common_ancestor: impl Into<String>) -> Self {
        Self {
            class_name: class_name.into(),
            common_ancestor: common_ancestor.into(),
            paths: Vec::new(),
        }
    }

    /// Add a path through the diamond
    pub fn with_path(mut self, path: Vec<String>) -> Self {
        self.paths.push(path);
        self
    }
}

// =============================================================================
// Inheritance Report
// =============================================================================

/// Complete inheritance analysis report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InheritanceReport {
    /// All inheritance edges (child -> parent relationships)
    pub edges: Vec<InheritanceEdge>,
    /// All class/interface nodes
    pub nodes: Vec<InheritanceNode>,
    /// Root classes (no parents in project)
    pub roots: Vec<String>,
    /// Leaf classes (no children)
    pub leaves: Vec<String>,
    /// Total number of classes analyzed
    pub count: usize,
    /// Languages found in the analysis
    pub languages: Vec<Language>,
    /// Detected diamond inheritance patterns
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diamonds: Vec<DiamondPattern>,
    /// Project root path
    pub project_path: PathBuf,
    /// Time taken for the scan in milliseconds
    pub scan_time_ms: u64,
}

impl InheritanceReport {
    /// Create a new empty report
    pub fn new(project_path: PathBuf) -> Self {
        Self {
            edges: Vec::new(),
            nodes: Vec::new(),
            roots: Vec::new(),
            leaves: Vec::new(),
            count: 0,
            languages: Vec::new(),
            diamonds: Vec::new(),
            project_path,
            scan_time_ms: 0,
        }
    }
}

// =============================================================================
// Inheritance Graph (Runtime structure, not serialized)
// =============================================================================

/// In-memory graph structure for inheritance analysis
///
/// Used during computation; InheritanceReport is the serializable output.
#[derive(Debug, Clone, Default)]
pub struct InheritanceGraph {
    /// Map from class name to node
    pub nodes: HashMap<String, InheritanceNode>,
    /// Edges: child -> list of parents
    pub parents: HashMap<String, Vec<String>>,
    /// Reverse edges: parent -> list of children
    pub children: HashMap<String, Vec<String>>,
}

impl InheritanceGraph {
    /// Create a new empty graph
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a node to the graph
    pub fn add_node(&mut self, node: InheritanceNode) {
        let name = node.name.clone();
        self.nodes.insert(name, node);
    }

    /// Add an edge (child extends/implements parent)
    pub fn add_edge(&mut self, child: &str, parent: &str) {
        self.parents
            .entry(child.to_string())
            .or_default()
            .push(parent.to_string());
        self.children
            .entry(parent.to_string())
            .or_default()
            .push(child.to_string());
    }

    /// Get all classes that have multiple parents (potential diamond sources)
    pub fn multi_parent_classes(&self) -> impl Iterator<Item = (&String, &Vec<String>)> {
        self.parents
            .iter()
            .filter(|(_, parents)| parents.len() >= 2)
    }

    /// Get ancestors of a class using BFS
    pub fn ancestors_bfs(&self, class_name: &str) -> std::collections::HashSet<String> {
        use std::collections::{HashSet, VecDeque};

        let mut ancestors = HashSet::new();
        let mut queue = VecDeque::new();

        if let Some(parents) = self.parents.get(class_name) {
            for parent in parents {
                queue.push_back(parent.clone());
            }
        }

        while let Some(current) = queue.pop_front() {
            if ancestors.insert(current.clone()) {
                if let Some(parents) = self.parents.get(&current) {
                    for parent in parents {
                        queue.push_back(parent.clone());
                    }
                }
            }
        }

        ancestors
    }

    /// Find root classes (no parents in the graph)
    pub fn find_roots(&self) -> Vec<String> {
        self.nodes
            .keys()
            .filter(|name| {
                self.parents
                    .get(*name)
                    .is_none_or(|parents| parents.is_empty())
            })
            .cloned()
            .collect()
    }

    /// Find leaf classes (no children in the graph)
    pub fn find_leaves(&self) -> Vec<String> {
        self.nodes
            .keys()
            .filter(|name| {
                self.children
                    .get(*name)
                    .is_none_or(|children| children.is_empty())
            })
            .cloned()
            .collect()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inheritance_node_serialization() {
        let node = InheritanceNode::new(
            "UserService",
            PathBuf::from("services/user.py"),
            10,
            Language::Python,
        )
        .with_bases(vec!["BaseService".to_string(), "ABC".to_string()])
        .as_abstract();

        let json = serde_json::to_string_pretty(&node).unwrap();
        assert!(json.contains("\"name\": \"UserService\""));
        assert!(json.contains("\"abstract\": true"));
        assert!(json.contains("\"BaseService\""));

        let parsed: InheritanceNode = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "UserService");
        assert_eq!(parsed.is_abstract, Some(true));
        assert_eq!(parsed.bases.len(), 2);
    }

    #[test]
    fn test_inheritance_edge_serialization() {
        let edge = InheritanceEdge::project(
            "UserService",
            "BaseService",
            PathBuf::from("services/user.py"),
            10,
            PathBuf::from("services/base.py"),
            5,
        );

        let json = serde_json::to_string(&edge).unwrap();
        let parsed: InheritanceEdge = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.child, "UserService");
        assert_eq!(parsed.parent, "BaseService");
        assert!(!parsed.external);
        assert_eq!(parsed.resolution, BaseResolution::Project);
    }

    #[test]
    fn test_inheritance_edge_stdlib() {
        let edge =
            InheritanceEdge::stdlib("CustomError", "Exception", PathBuf::from("errors.py"), 5);

        assert!(edge.external);
        assert_eq!(edge.resolution, BaseResolution::Stdlib);
        assert!(edge.parent_file.is_none());
    }

    #[test]
    fn test_diamond_pattern_serialization() {
        let diamond = DiamondPattern::new("D", "A")
            .with_path(vec!["D".to_string(), "B".to_string(), "A".to_string()])
            .with_path(vec!["D".to_string(), "C".to_string(), "A".to_string()]);

        let json = serde_json::to_string_pretty(&diamond).unwrap();
        assert!(json.contains("\"class_name\": \"D\""));
        assert!(json.contains("\"common_ancestor\": \"A\""));

        let parsed: DiamondPattern = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.paths.len(), 2);
    }

    #[test]
    fn test_inheritance_graph_multi_parent() {
        let mut graph = InheritanceGraph::new();

        graph.add_node(InheritanceNode::new(
            "A",
            PathBuf::from("a.py"),
            1,
            Language::Python,
        ));
        graph.add_node(InheritanceNode::new(
            "B",
            PathBuf::from("b.py"),
            1,
            Language::Python,
        ));
        graph.add_node(InheritanceNode::new(
            "C",
            PathBuf::from("c.py"),
            1,
            Language::Python,
        ));
        graph.add_node(InheritanceNode::new(
            "D",
            PathBuf::from("d.py"),
            1,
            Language::Python,
        ));

        graph.add_edge("B", "A");
        graph.add_edge("C", "A");
        graph.add_edge("D", "B");
        graph.add_edge("D", "C");

        let multi_parent: Vec<_> = graph.multi_parent_classes().collect();
        assert_eq!(multi_parent.len(), 1);
        assert_eq!(multi_parent[0].0, "D");
    }

    #[test]
    fn test_inheritance_graph_ancestors_bfs() {
        let mut graph = InheritanceGraph::new();

        graph.add_node(InheritanceNode::new(
            "A",
            PathBuf::from("a.py"),
            1,
            Language::Python,
        ));
        graph.add_node(InheritanceNode::new(
            "B",
            PathBuf::from("b.py"),
            1,
            Language::Python,
        ));
        graph.add_node(InheritanceNode::new(
            "C",
            PathBuf::from("c.py"),
            1,
            Language::Python,
        ));
        graph.add_node(InheritanceNode::new(
            "D",
            PathBuf::from("d.py"),
            1,
            Language::Python,
        ));

        graph.add_edge("B", "A");
        graph.add_edge("C", "B");
        graph.add_edge("D", "C");

        let ancestors = graph.ancestors_bfs("D");
        assert!(ancestors.contains("A"));
        assert!(ancestors.contains("B"));
        assert!(ancestors.contains("C"));
        assert!(!ancestors.contains("D"));
    }

    #[test]
    fn test_inheritance_report_serialization() {
        let report = InheritanceReport::new(PathBuf::from("/project"));

        let json = serde_json::to_string(&report).unwrap();
        let parsed: InheritanceReport = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.project_path, PathBuf::from("/project"));
    }
}
