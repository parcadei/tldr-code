//! PHP class extraction for inheritance analysis
//!
//! Extracts class, interface, and trait definitions from PHP source code
//! using tree-sitter. Handles:
//! - Class inheritance (extends)
//! - Interface implementation (implements)
//! - Interface declarations (extends other interfaces)
//! - Trait declarations (treated as interfaces/mixins)
//! - Abstract classes
//! - Qualified/namespaced names

use std::path::Path;

use tree_sitter::Node;

use crate::ast::parser::ParserPool;
use crate::types::{InheritanceNode, Language};
use crate::TldrResult;

/// Extract class, interface, and trait definitions from PHP source code
pub fn extract_classes(
    source: &str,
    file_path: &Path,
    parser_pool: &ParserPool,
) -> TldrResult<Vec<InheritanceNode>> {
    let tree = parser_pool.parse(source, Language::Php)?;
    let mut classes = Vec::new();

    let root = tree.root_node();
    visit_node(&root, source, file_path, &mut classes);

    Ok(classes)
}

fn visit_node(node: &Node, source: &str, file_path: &Path, classes: &mut Vec<InheritanceNode>) {
    match node.kind() {
        "class_declaration" => {
            if let Some(class) = extract_class_declaration(node, source, file_path) {
                classes.push(class);
            }
        }
        "interface_declaration" => {
            if let Some(iface) = extract_interface_declaration(node, source, file_path) {
                classes.push(iface);
            }
        }
        "trait_declaration" => {
            if let Some(t) = extract_trait_declaration(node, source, file_path) {
                classes.push(t);
            }
        }
        _ => {}
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        visit_node(&child, source, file_path, classes);
    }
}

/// Extract a class_declaration node.
///
/// PHP tree-sitter grammar:
/// - `name` field -> name (identifier)
/// - `base_clause` child -> extends clause
/// - `class_interface_clause` child -> implements clause
/// - modifiers child may contain "abstract" keyword
fn extract_class_declaration(
    node: &Node,
    source: &str,
    file_path: &Path,
) -> Option<InheritanceNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source.as_bytes()).ok()?.to_string();

    let line = node.start_position().row as u32 + 1;
    let mut class_node = InheritanceNode::new(name, file_path.to_path_buf(), line, Language::Php);

    let mut bases = Vec::new();

    // Walk children to find base_clause and class_interface_clause
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "base_clause" => {
                extract_names_from_clause(&child, source, &mut bases);
            }
            "class_interface_clause" => {
                extract_names_from_clause(&child, source, &mut bases);
            }
            _ => {}
        }
    }

    class_node.bases = bases;

    // Check for abstract modifier
    if has_modifier(node, source, "abstract") {
        class_node.is_abstract = Some(true);
    }

    Some(class_node)
}

/// Extract an interface_declaration node.
///
/// PHP grammar:
/// - `name` field -> name (identifier)
/// - `base_clause` child -> extends clause (interfaces can extend other interfaces)
fn extract_interface_declaration(
    node: &Node,
    source: &str,
    file_path: &Path,
) -> Option<InheritanceNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source.as_bytes()).ok()?.to_string();

    let line = node.start_position().row as u32 + 1;
    let mut iface_node = InheritanceNode::new(name, file_path.to_path_buf(), line, Language::Php);
    iface_node.interface = Some(true);

    let mut bases = Vec::new();

    // Interfaces can extend other interfaces via base_clause
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "base_clause" {
            extract_names_from_clause(&child, source, &mut bases);
        }
    }

    iface_node.bases = bases;

    Some(iface_node)
}

/// Extract a trait_declaration node.
///
/// PHP grammar:
/// - `name` field -> name (identifier)
///
/// Traits are like mixins, so we mark them as interfaces.
fn extract_trait_declaration(
    node: &Node,
    source: &str,
    file_path: &Path,
) -> Option<InheritanceNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source.as_bytes()).ok()?.to_string();

    let line = node.start_position().row as u32 + 1;
    let mut trait_node = InheritanceNode::new(name, file_path.to_path_buf(), line, Language::Php);
    trait_node.interface = Some(true);

    Some(trait_node)
}

/// Extract type names from a base_clause or class_interface_clause.
///
/// These clauses contain `name` or `qualified_name` children mixed with
/// keywords ("extends", "implements") and comma separators.
fn extract_names_from_clause(node: &Node, source: &str, bases: &mut Vec<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "name" => {
                if let Ok(text) = child.utf8_text(source.as_bytes()) {
                    bases.push(text.to_string());
                }
            }
            "qualified_name" => {
                if let Ok(text) = child.utf8_text(source.as_bytes()) {
                    bases.push(text.to_string());
                }
            }
            _ => {}
        }
    }
}

/// Check if a declaration has a specific modifier keyword (e.g., "abstract", "final")
fn has_modifier(node: &Node, source: &str, modifier: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        // PHP may use various modifier node kinds
        if let Ok(text) = child.utf8_text(source.as_bytes()) {
            if text == modifier {
                return true;
            }
        }
        // Check inside modifier nodes
        if child.kind() == "modifier" || child.kind() == "abstract_modifier" {
            if let Ok(text) = child.utf8_text(source.as_bytes()) {
                if text == modifier {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn parse_and_extract(source: &str) -> Vec<InheritanceNode> {
        let pool = ParserPool::new();
        extract_classes(source, &PathBuf::from("Test.php"), &pool).unwrap()
    }

    #[test]
    fn test_simple_class() {
        let source = r#"<?php
class Animal {
    public function speak() {}
}
?>"#;
        let classes = parse_and_extract(source);
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, "Animal");
        assert!(classes[0].bases.is_empty());
        assert_eq!(classes[0].language, Language::Php);
    }

    #[test]
    fn test_class_extends() {
        let source = r#"<?php
class Animal {}

class Dog extends Animal {
    public function bark() {}
}
?>"#;
        let classes = parse_and_extract(source);
        assert_eq!(classes.len(), 2);

        let dog = classes.iter().find(|c| c.name == "Dog").unwrap();
        assert!(dog.bases.contains(&"Animal".to_string()));
    }

    #[test]
    fn test_interface_declaration() {
        let source = r#"<?php
interface Serializable {
    public function serialize();
}
?>"#;
        let classes = parse_and_extract(source);
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, "Serializable");
        assert_eq!(classes[0].interface, Some(true));
    }

    #[test]
    fn test_class_implements_interface() {
        let source = r#"<?php
interface Serializable {
    public function serialize();
}

class Dog implements Serializable {
    public function serialize() { return "{}"; }
}
?>"#;
        let classes = parse_and_extract(source);
        let dog = classes.iter().find(|c| c.name == "Dog").unwrap();
        assert!(dog.bases.contains(&"Serializable".to_string()));
    }

    #[test]
    fn test_class_extends_and_implements() {
        let source = r#"<?php
class Animal {}
interface Serializable {}
interface Cloneable {}

class Dog extends Animal implements Serializable, Cloneable {
}
?>"#;
        let classes = parse_and_extract(source);
        let dog = classes.iter().find(|c| c.name == "Dog").unwrap();
        assert!(dog.bases.contains(&"Animal".to_string()));
        assert!(dog.bases.contains(&"Serializable".to_string()));
        assert!(dog.bases.contains(&"Cloneable".to_string()));
        assert_eq!(dog.bases.len(), 3);
    }

    #[test]
    fn test_abstract_class() {
        let source = r#"<?php
abstract class Shape {
    abstract public function area();
}
?>"#;
        let classes = parse_and_extract(source);
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, "Shape");
        assert_eq!(classes[0].is_abstract, Some(true));
    }

    #[test]
    fn test_trait_declaration() {
        let source = r#"<?php
trait Loggable {
    public function log() {}
}
?>"#;
        let classes = parse_and_extract(source);
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, "Loggable");
        assert_eq!(classes[0].interface, Some(true));
    }

    #[test]
    fn test_interface_extends_interface() {
        let source = r#"<?php
interface Base {
    public function doBase();
}

interface Extended extends Base {
    public function doExtended();
}
?>"#;
        let classes = parse_and_extract(source);
        let extended = classes.iter().find(|c| c.name == "Extended").unwrap();
        assert!(extended.bases.contains(&"Base".to_string()));
        assert_eq!(extended.interface, Some(true));
    }
}
