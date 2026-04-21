//! Scala class extraction for inheritance analysis
//!
//! Extracts class, trait, object, and case class definitions from Scala source code
//! using tree-sitter. Handles:
//! - Class inheritance (extends)
//! - Trait extension (extends)
//! - Trait mixin (with)
//! - Object inheritance (extends)
//! - Case class definitions
//! - Abstract classes (with abstract modifier)
//! - Generic type parameters (stripped to base type)

use std::path::Path;

use tree_sitter::Node;

use crate::ast::parser::ParserPool;
use crate::types::{InheritanceNode, Language};
use crate::TldrResult;

/// Extract class, trait, and object definitions from Scala source code
pub fn extract_classes(
    source: &str,
    file_path: &Path,
    parser_pool: &ParserPool,
) -> TldrResult<Vec<InheritanceNode>> {
    let tree = parser_pool.parse(source, Language::Scala)?;
    let mut classes = Vec::new();

    let root = tree.root_node();
    visit_node(&root, source, file_path, &mut classes);

    Ok(classes)
}

fn visit_node(node: &Node, source: &str, file_path: &Path, classes: &mut Vec<InheritanceNode>) {
    match node.kind() {
        "class_definition" => {
            if let Some(class) = extract_class_definition(node, source, file_path) {
                classes.push(class);
            }
        }
        "trait_definition" => {
            if let Some(trait_node) = extract_trait_definition(node, source, file_path) {
                classes.push(trait_node);
            }
        }
        "object_definition" => {
            if let Some(obj) = extract_object_definition(node, source, file_path) {
                classes.push(obj);
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

/// Extract a class_definition node (includes case classes).
///
/// Scala tree-sitter grammar:
/// - `name` field -> identifier
/// - `extend` field -> extends_clause (contains parent types)
/// - modifiers child may contain "abstract", "case" keywords
fn extract_class_definition(
    node: &Node,
    source: &str,
    file_path: &Path,
) -> Option<InheritanceNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source.as_bytes()).ok()?.to_string();

    let line = node.start_position().row as u32 + 1;
    let mut class_node = InheritanceNode::new(name, file_path.to_path_buf(), line, Language::Scala);

    // Extract bases from extends clause
    if let Some(extends) = node.child_by_field_name("extend") {
        class_node.bases = extract_types_from_extends_clause(&extends, source);
    }

    // Check for abstract modifier
    if has_modifier(node, source, "abstract") {
        class_node.is_abstract = Some(true);
    }

    Some(class_node)
}

/// Extract a trait_definition node.
///
/// Scala grammar:
/// - `name` field -> identifier
/// - `extend` field -> extends_clause (trait can extend other traits)
fn extract_trait_definition(
    node: &Node,
    source: &str,
    file_path: &Path,
) -> Option<InheritanceNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source.as_bytes()).ok()?.to_string();

    let line = node.start_position().row as u32 + 1;
    let mut trait_node = InheritanceNode::new(name, file_path.to_path_buf(), line, Language::Scala);
    trait_node.interface = Some(true);

    // Traits can extend other traits
    if let Some(extends) = node.child_by_field_name("extend") {
        trait_node.bases = extract_types_from_extends_clause(&extends, source);
    }

    Some(trait_node)
}

/// Extract an object_definition node.
///
/// Scala grammar:
/// - `name` field -> identifier
/// - `extend` field -> extends_clause
fn extract_object_definition(
    node: &Node,
    source: &str,
    file_path: &Path,
) -> Option<InheritanceNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source.as_bytes()).ok()?.to_string();

    let line = node.start_position().row as u32 + 1;
    let mut obj_node = InheritanceNode::new(name, file_path.to_path_buf(), line, Language::Scala);

    // Objects can extend classes/traits
    if let Some(extends) = node.child_by_field_name("extend") {
        obj_node.bases = extract_types_from_extends_clause(&extends, source);
    }

    Some(obj_node)
}

/// Extract all parent type names from an extends_clause.
///
/// Scala extends_clause structure:
/// ```text
/// extends_clause
///   "extends"
///   type_identifier "Animal"     (or generic_type, stable_type_identifier, etc.)
///   arguments "(name)"           (constructor args - skip)
///   "with"
///   type_identifier "Serializable"
/// ```
///
/// The `type` field on extends_clause contains all the parent types.
/// We extract the base type name from each, handling:
/// - `type_identifier` -> direct name
/// - `generic_type` -> extract the base name (e.g., Generic[T] -> Generic)
/// - `stable_type_identifier` -> extract the last component (e.g., pkg.Class -> Class)
fn extract_types_from_extends_clause(node: &Node, source: &str) -> Vec<String> {
    let mut bases = Vec::new();

    // The extends_clause uses the "type" field for parent types
    // We iterate over all children with field name "type"
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        // Skip non-type nodes (keywords "extends", "with", arguments, etc.)
        match child.kind() {
            "type_identifier" => {
                if let Ok(text) = child.utf8_text(source.as_bytes()) {
                    bases.push(text.to_string());
                }
            }
            "generic_type" => {
                // Extract the base type name from generic_type (e.g., Generic[T] -> Generic)
                if let Some(name) = extract_type_name_from_generic(&child, source) {
                    bases.push(name);
                }
            }
            "stable_type_identifier" => {
                // Qualified names like pkg.Class -> extract the last part
                if let Some(name) = extract_last_identifier(&child, source) {
                    bases.push(name);
                }
            }
            _ => {}
        }
    }

    bases
}

/// Extract the base type name from a generic_type node.
///
/// generic_type structure:
/// ```text
/// generic_type
///   type_identifier "Generic"
///   type_arguments "[T]"
/// ```
fn extract_type_name_from_generic(node: &Node, source: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_identifier" {
            return child
                .utf8_text(source.as_bytes())
                .ok()
                .map(|s| s.to_string());
        }
    }
    None
}

/// Extract the last identifier from a stable_type_identifier or similar qualified type.
fn extract_last_identifier(node: &Node, source: &str) -> Option<String> {
    let mut last_id = None;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" || child.kind() == "type_identifier" {
            last_id = child
                .utf8_text(source.as_bytes())
                .ok()
                .map(|s| s.to_string());
        }
    }
    last_id
}

/// Check if a declaration has a specific modifier keyword (e.g., "abstract", "case")
fn has_modifier(node: &Node, source: &str, modifier: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "modifiers" {
            return check_modifier_recursive(&child, source, modifier);
        }
        // Also check direct "case" keyword for case classes
        if child.kind() == modifier {
            return true;
        }
    }

    // Also check the text of the full node for the keyword before the class/trait name
    // This handles cases where the modifier might not be in a modifiers node
    false
}

/// Recursively check a modifiers node for a specific keyword
fn check_modifier_recursive(node: &Node, source: &str, modifier: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Ok(text) = child.utf8_text(source.as_bytes()) {
            if text == modifier {
                return true;
            }
        }
        if check_modifier_recursive(&child, source, modifier) {
            return true;
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
        extract_classes(source, &PathBuf::from("Test.scala"), &pool).unwrap()
    }

    #[test]
    fn test_simple_class() {
        let source = r#"
class Animal {
  def speak(): String = "..."
}
"#;
        let classes = parse_and_extract(source);
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, "Animal");
        assert!(classes[0].bases.is_empty());
        assert_eq!(classes[0].language, Language::Scala);
    }

    #[test]
    fn test_class_extends() {
        let source = r#"
class Animal(val name: String)

class Dog(name: String) extends Animal(name) {
  def bark(): String = "Woof"
}
"#;
        let classes = parse_and_extract(source);
        assert_eq!(classes.len(), 2);

        let dog = classes.iter().find(|c| c.name == "Dog").unwrap();
        assert!(dog.bases.contains(&"Animal".to_string()));
    }

    #[test]
    fn test_trait_definition() {
        let source = r#"
trait Serializable {
  def serialize(): String
}
"#;
        let classes = parse_and_extract(source);
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, "Serializable");
        assert_eq!(classes[0].interface, Some(true));
    }

    #[test]
    fn test_trait_extends_trait() {
        let source = r#"
trait Base {
  def id: String
}

trait Extended extends Base {
  def name: String
}
"#;
        let classes = parse_and_extract(source);
        let extended = classes.iter().find(|c| c.name == "Extended").unwrap();
        assert!(extended.bases.contains(&"Base".to_string()));
        assert_eq!(extended.interface, Some(true));
    }

    #[test]
    fn test_object_extends() {
        let source = r#"
trait Compress {
  def headers: Seq[String]
}

object Gzip extends Compress {
  def headers = Seq("gzip")
}
"#;
        let classes = parse_and_extract(source);
        let gzip = classes.iter().find(|c| c.name == "Gzip").unwrap();
        assert!(gzip.bases.contains(&"Compress".to_string()));
    }

    #[test]
    fn test_class_with_mixin() {
        let source = r#"
class Animal(val name: String)

trait Serializable {
  def serialize(): String
}

class Dog(name: String) extends Animal(name) with Serializable {
  def serialize() = s"Dog($name)"
}
"#;
        let classes = parse_and_extract(source);
        let dog = classes.iter().find(|c| c.name == "Dog").unwrap();
        assert!(dog.bases.contains(&"Animal".to_string()));
        assert!(dog.bases.contains(&"Serializable".to_string()));
        assert_eq!(dog.bases.len(), 2);
    }

    #[test]
    fn test_case_class() {
        let source = r#"
case class Request(url: String, method: String)
"#;
        let classes = parse_and_extract(source);
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, "Request");
    }

    #[test]
    fn test_abstract_class() {
        let source = r#"
abstract class Shape {
  def area(): Double
}
"#;
        let classes = parse_and_extract(source);
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, "Shape");
        assert_eq!(classes[0].is_abstract, Some(true));
    }
}
