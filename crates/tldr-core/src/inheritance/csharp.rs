//! C# class extraction for inheritance analysis
//!
//! Extracts class, interface, struct, and record definitions from C# source code
//! using tree-sitter. Handles:
//! - Class inheritance (: BaseClass)
//! - Interface implementation (: IInterface)
//! - Interface declarations (: IParentInterface)
//! - Struct declarations (: IInterface)
//! - Abstract classes
//! - Generic type parameters (stripped to base type)

use std::path::Path;

use tree_sitter::Node;

use crate::ast::parser::ParserPool;
use crate::types::{InheritanceNode, Language};
use crate::TldrResult;

/// Extract class, interface, and struct definitions from C# source code
pub fn extract_classes(
    source: &str,
    file_path: &Path,
    parser_pool: &ParserPool,
) -> TldrResult<Vec<InheritanceNode>> {
    let tree = parser_pool.parse(source, Language::CSharp)?;
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
        "struct_declaration" => {
            if let Some(s) = extract_struct_declaration(node, source, file_path) {
                classes.push(s);
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
/// C# tree-sitter grammar:
/// - `name` field -> identifier
/// - `bases` field -> base_list containing comma-separated types
/// - modifiers child may contain "abstract" keyword
fn extract_class_declaration(
    node: &Node,
    source: &str,
    file_path: &Path,
) -> Option<InheritanceNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source.as_bytes()).ok()?.to_string();

    let line = node.start_position().row as u32 + 1;
    let mut class_node =
        InheritanceNode::new(name, file_path.to_path_buf(), line, Language::CSharp);

    // Extract bases from base_list child
    class_node.bases = find_and_extract_base_list(node, source);

    // Check for abstract modifier
    if has_modifier(node, source, "abstract") {
        class_node.is_abstract = Some(true);
    }

    Some(class_node)
}

/// Extract an interface_declaration node.
///
/// C# grammar:
/// - `name` field -> identifier
/// - `base_list` child (interface can extend other interfaces)
fn extract_interface_declaration(
    node: &Node,
    source: &str,
    file_path: &Path,
) -> Option<InheritanceNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source.as_bytes()).ok()?.to_string();

    let line = node.start_position().row as u32 + 1;
    let mut iface_node =
        InheritanceNode::new(name, file_path.to_path_buf(), line, Language::CSharp);
    iface_node.interface = Some(true);

    // Interfaces can extend other interfaces
    iface_node.bases = find_and_extract_base_list(node, source);

    Some(iface_node)
}

/// Extract a struct_declaration node.
///
/// C# grammar:
/// - `name` field -> identifier
/// - `base_list` child (structs can implement interfaces)
fn extract_struct_declaration(
    node: &Node,
    source: &str,
    file_path: &Path,
) -> Option<InheritanceNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source.as_bytes()).ok()?.to_string();

    let line = node.start_position().row as u32 + 1;
    let mut struct_node =
        InheritanceNode::new(name, file_path.to_path_buf(), line, Language::CSharp);

    // Structs can implement interfaces
    struct_node.bases = find_and_extract_base_list(node, source);

    Some(struct_node)
}

/// Find the base_list child node and extract types from it
fn find_and_extract_base_list(node: &Node, source: &str) -> Vec<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "base_list" {
            return extract_types_from_base_list(&child, source);
        }
    }
    Vec::new()
}

/// Extract all parent type names from a base_list node.
///
/// C# base_list structure:
/// ```text
/// base_list
///   ":"
///   identifier "Animal"
///   ","
///   identifier "ISerializable"
/// ```
///
/// Types in the base_list can be:
/// - `identifier` -> simple name
/// - `generic_name` -> generic type (e.g., IList<T>)
/// - `qualified_name` -> dotted name (e.g., System.IO.Stream)
fn extract_types_from_base_list(node: &Node, source: &str) -> Vec<String> {
    let mut bases = Vec::new();

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                if let Ok(text) = child.utf8_text(source.as_bytes()) {
                    bases.push(text.to_string());
                }
            }
            "generic_name" => {
                if let Some(name) = extract_type_name_from_generic(&child, source) {
                    bases.push(name);
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

    bases
}

/// Extract the base type name from a generic_name node.
///
/// generic_name structure:
/// ```text
/// generic_name
///   identifier "IList"
///   type_argument_list "<T>"
/// ```
fn extract_type_name_from_generic(node: &Node, source: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" {
            return child
                .utf8_text(source.as_bytes())
                .ok()
                .map(|s| s.to_string());
        }
    }
    None
}

/// Check if a declaration has a specific modifier keyword (e.g., "abstract", "static")
fn has_modifier(node: &Node, source: &str, modifier: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "modifier" {
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
        extract_classes(source, &PathBuf::from("Test.cs"), &pool).unwrap()
    }

    #[test]
    fn test_simple_class() {
        let source = r#"
public class Animal {
    public void Speak() {}
}
"#;
        let classes = parse_and_extract(source);
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, "Animal");
        assert!(classes[0].bases.is_empty());
        assert_eq!(classes[0].language, Language::CSharp);
    }

    #[test]
    fn test_class_extends() {
        let source = r#"
public class Animal {}

public class Dog : Animal {
    public void Bark() {}
}
"#;
        let classes = parse_and_extract(source);
        assert_eq!(classes.len(), 2);

        let dog = classes.iter().find(|c| c.name == "Dog").unwrap();
        assert!(dog.bases.contains(&"Animal".to_string()));
    }

    #[test]
    fn test_interface_declaration() {
        let source = r#"
public interface ISerializable {
    string Serialize();
}
"#;
        let classes = parse_and_extract(source);
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, "ISerializable");
        assert_eq!(classes[0].interface, Some(true));
    }

    #[test]
    fn test_class_implements_interface() {
        let source = r#"
public interface ISerializable {
    string Serialize();
}

public class Dog : ISerializable {
    public string Serialize() { return "{}"; }
}
"#;
        let classes = parse_and_extract(source);
        let dog = classes.iter().find(|c| c.name == "Dog").unwrap();
        assert!(dog.bases.contains(&"ISerializable".to_string()));
    }

    #[test]
    fn test_class_extends_and_implements() {
        let source = r#"
public class Animal {}
public interface ISerializable {}
public interface ICloneable {}

public class Dog : Animal, ISerializable, ICloneable {
}
"#;
        let classes = parse_and_extract(source);
        let dog = classes.iter().find(|c| c.name == "Dog").unwrap();
        assert!(dog.bases.contains(&"Animal".to_string()));
        assert!(dog.bases.contains(&"ISerializable".to_string()));
        assert!(dog.bases.contains(&"ICloneable".to_string()));
        assert_eq!(dog.bases.len(), 3);
    }

    #[test]
    fn test_abstract_class() {
        let source = r#"
public abstract class Shape {
    public abstract double Area();
}
"#;
        let classes = parse_and_extract(source);
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, "Shape");
        assert_eq!(classes[0].is_abstract, Some(true));
    }

    #[test]
    fn test_struct_declaration() {
        let source = r#"
public struct Point {
    public double X;
    public double Y;
}
"#;
        let classes = parse_and_extract(source);
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, "Point");
    }

    #[test]
    fn test_interface_extends_interface() {
        let source = r#"
public interface IBase {
    void DoBase();
}

public interface IExtended : IBase {
    void DoExtended();
}
"#;
        let classes = parse_and_extract(source);
        let extended = classes.iter().find(|c| c.name == "IExtended").unwrap();
        assert!(extended.bases.contains(&"IBase".to_string()));
        assert_eq!(extended.interface, Some(true));
    }
}
