//! Ruby class extraction for inheritance analysis
//!
//! Extracts class and module definitions from Ruby source code
//! using tree-sitter. Handles:
//! - Class inheritance (class Dog < Animal)
//! - Module definitions (treated as interfaces/mixins)
//! - Scope resolution for namespaced classes (ActiveRecord::Base)

use std::path::Path;

use tree_sitter::Node;

use crate::ast::parser::ParserPool;
use crate::types::{InheritanceNode, Language};
use crate::TldrResult;

/// Extract class and module definitions from Ruby source code
pub fn extract_classes(
    source: &str,
    file_path: &Path,
    parser_pool: &ParserPool,
) -> TldrResult<Vec<InheritanceNode>> {
    let tree = parser_pool.parse(source, Language::Ruby)?;
    let mut classes = Vec::new();

    let root = tree.root_node();
    visit_node(&root, source, file_path, &mut classes);

    Ok(classes)
}

fn visit_node(node: &Node, source: &str, file_path: &Path, classes: &mut Vec<InheritanceNode>) {
    match node.kind() {
        "class" => {
            if let Some(class) = extract_class_definition(node, source, file_path) {
                classes.push(class);
            }
        }
        "module" => {
            if let Some(module) = extract_module_definition(node, source, file_path) {
                classes.push(module);
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

/// Extract a class node.
///
/// Ruby tree-sitter grammar:
/// - `name` field -> constant (class name)
/// - `superclass` field -> superclass node containing the parent class
///
/// The superclass field value is a `superclass` node that contains
/// a constant or scope_resolution child representing the parent class name.
fn extract_class_definition(
    node: &Node,
    source: &str,
    file_path: &Path,
) -> Option<InheritanceNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = extract_constant_name(&name_node, source)?;

    let line = node.start_position().row as u32 + 1;
    let mut class_node = InheritanceNode::new(name, file_path.to_path_buf(), line, Language::Ruby);

    // Extract superclass from the superclass field
    if let Some(superclass_node) = node.child_by_field_name("superclass") {
        if let Some(parent_name) = extract_superclass_name(&superclass_node, source) {
            class_node.bases.push(parent_name);
        }
    }

    Some(class_node)
}

/// Extract a module node.
///
/// Ruby grammar:
/// - `name` field -> constant (module name)
///
/// Modules have no inheritance but serve as mixins, so we mark them as interfaces.
fn extract_module_definition(
    node: &Node,
    source: &str,
    file_path: &Path,
) -> Option<InheritanceNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = extract_constant_name(&name_node, source)?;

    let line = node.start_position().row as u32 + 1;
    let mut module_node = InheritanceNode::new(name, file_path.to_path_buf(), line, Language::Ruby);
    module_node.interface = Some(true);

    Some(module_node)
}

/// Extract the name from a constant or scope_resolution node.
///
/// - `constant` -> "Animal"
/// - `scope_resolution` -> "ActiveRecord::Base" (we return the last component)
fn extract_constant_name(node: &Node, source: &str) -> Option<String> {
    match node.kind() {
        "constant" => node
            .utf8_text(source.as_bytes())
            .ok()
            .map(|s| s.to_string()),
        "scope_resolution" => {
            // For namespaced constants like ActiveRecord::Base, return the full text
            node.utf8_text(source.as_bytes())
                .ok()
                .map(|s| s.to_string())
        }
        _ => node
            .utf8_text(source.as_bytes())
            .ok()
            .map(|s| s.to_string()),
    }
}

/// Extract the superclass name from a superclass node.
///
/// The superclass node wraps the actual type:
/// ```text
/// superclass
///   "<"
///   constant "Animal"
/// ```
/// or:
/// ```text
/// superclass
///   "<"
///   scope_resolution
///     constant "ActiveRecord"
///     "::"
///     constant "Base"
/// ```
fn extract_superclass_name(node: &Node, source: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "constant" => {
                return child
                    .utf8_text(source.as_bytes())
                    .ok()
                    .map(|s| s.to_string());
            }
            "scope_resolution" => {
                return child
                    .utf8_text(source.as_bytes())
                    .ok()
                    .map(|s| s.to_string());
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn parse_and_extract(source: &str) -> Vec<InheritanceNode> {
        let pool = ParserPool::new();
        extract_classes(source, &PathBuf::from("Test.rb"), &pool).unwrap()
    }

    #[test]
    fn test_simple_class() {
        let source = r#"
class Animal
  def speak
    "..."
  end
end
"#;
        let classes = parse_and_extract(source);
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, "Animal");
        assert!(classes[0].bases.is_empty());
        assert_eq!(classes[0].language, Language::Ruby);
    }

    #[test]
    fn test_class_inherits() {
        let source = r#"
class Animal
end

class Dog < Animal
end
"#;
        let classes = parse_and_extract(source);
        assert_eq!(classes.len(), 2);

        let dog = classes.iter().find(|c| c.name == "Dog").unwrap();
        assert!(dog.bases.contains(&"Animal".to_string()));
    }

    #[test]
    fn test_module_definition() {
        let source = r#"
module Serializable
  def serialize
    "{}"
  end
end
"#;
        let classes = parse_and_extract(source);
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, "Serializable");
        assert_eq!(classes[0].interface, Some(true));
    }

    #[test]
    fn test_namespaced_superclass() {
        let source = r#"
class User < ActiveRecord::Base
end
"#;
        let classes = parse_and_extract(source);
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, "User");
        assert!(classes[0].bases.contains(&"ActiveRecord::Base".to_string()));
    }

    #[test]
    fn test_nested_classes() {
        let source = r#"
class Outer
  class Inner < Outer
  end
end
"#;
        let classes = parse_and_extract(source);
        assert_eq!(classes.len(), 2);

        let inner = classes.iter().find(|c| c.name == "Inner").unwrap();
        assert!(inner.bases.contains(&"Outer".to_string()));
    }
}
