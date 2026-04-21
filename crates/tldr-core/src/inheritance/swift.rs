//! Swift class extraction for inheritance analysis
//!
//! Extracts class, protocol, struct, and enum definitions from Swift source code
//! using tree-sitter. Handles:
//! - Class inheritance (: SuperClass)
//! - Protocol conformance (: Protocol1, Protocol2)
//! - Protocol inheritance (: ParentProtocol)
//! - Struct protocol conformance (: Protocol)
//! - Enum protocol conformance (: Protocol)
//! - Multiple inheritance specifiers
//! - Attribute annotations (@unchecked, @available, etc. are skipped)

use std::path::Path;

use tree_sitter::Node;

use crate::ast::parser::ParserPool;
use crate::types::{InheritanceNode, Language};
use crate::TldrResult;

/// Extract class, protocol, struct, and enum definitions from Swift source code
pub fn extract_classes(
    source: &str,
    file_path: &Path,
    parser_pool: &ParserPool,
) -> TldrResult<Vec<InheritanceNode>> {
    let tree = parser_pool.parse(source, Language::Swift)?;
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
        "protocol_declaration" => {
            if let Some(proto) = extract_protocol_declaration(node, source, file_path) {
                classes.push(proto);
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
/// Swift tree-sitter grammar uses `class_declaration` for classes, structs, enums, and actors.
/// The `declaration_kind` field distinguishes them: "class", "struct", "enum", "actor", "extension".
///
/// Fields:
/// - `name` -> type_identifier or user_type
/// - `declaration_kind` -> keyword
/// - `body` -> class_body or enum_class_body
///
/// Children:
/// - `inheritance_specifier` nodes (each has `inherits_from` field -> user_type)
/// - `modifiers` node
/// - `attribute` nodes
fn extract_class_declaration(
    node: &Node,
    source: &str,
    file_path: &Path,
) -> Option<InheritanceNode> {
    // Check declaration_kind - skip extensions
    let kind_node = node.child_by_field_name("declaration_kind")?;
    let kind = kind_node.utf8_text(source.as_bytes()).ok()?;
    if kind == "extension" {
        return None;
    }

    let name_node = node.child_by_field_name("name")?;
    let name = extract_type_name(&name_node, source)?;

    let line = node.start_position().row as u32 + 1;
    let mut class_node = InheritanceNode::new(name, file_path.to_path_buf(), line, Language::Swift);

    // Extract inheritance specifiers
    class_node.bases = extract_inheritance_specifiers(node, source);

    // Check for abstract-like annotations (Swift doesn't have abstract classes,
    // but we can detect if it has only protocol-like behavior)

    Some(class_node)
}

/// Extract a protocol_declaration node.
///
/// Swift grammar:
/// - `name` -> type_identifier
/// - children include `inheritance_specifier` for protocol inheritance
fn extract_protocol_declaration(
    node: &Node,
    source: &str,
    file_path: &Path,
) -> Option<InheritanceNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source.as_bytes()).ok()?.to_string();

    let line = node.start_position().row as u32 + 1;
    let mut proto_node = InheritanceNode::new(name, file_path.to_path_buf(), line, Language::Swift);
    proto_node.interface = Some(true);
    proto_node.protocol = Some(true);

    // Protocols can inherit from other protocols
    proto_node.bases = extract_inheritance_specifiers(node, source);

    Some(proto_node)
}

/// Extract type name from a name node.
///
/// The name field can be:
/// - `type_identifier` -> direct name string
/// - `user_type` -> contains type_identifier children
fn extract_type_name(node: &Node, source: &str) -> Option<String> {
    match node.kind() {
        "type_identifier" => node
            .utf8_text(source.as_bytes())
            .ok()
            .map(|s| s.to_string()),
        "user_type" => {
            // user_type may contain nested type_identifiers
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "type_identifier" {
                    return child
                        .utf8_text(source.as_bytes())
                        .ok()
                        .map(|s| s.to_string());
                }
            }
            // Fall back to the full text
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

/// Extract all inheritance specifiers from a declaration node.
///
/// Swift AST structure:
/// ```text
/// class_declaration
///   "class"
///   type_identifier "Dog"
///   inheritance_specifier
///     inherits_from: user_type
///       type_identifier "Animal"
///   inheritance_specifier
///     inherits_from: user_type
///       type_identifier "Sendable"
///   class_body { ... }
/// ```
///
/// Each `inheritance_specifier` child has an `inherits_from` field.
fn extract_inheritance_specifiers(node: &Node, source: &str) -> Vec<String> {
    let mut bases = Vec::new();

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "inheritance_specifier" {
            if let Some(inherits_from) = child.child_by_field_name("inherits_from") {
                if let Some(name) = extract_inherited_type_name(&inherits_from, source) {
                    // Skip attribute-like type annotations (e.g., @unchecked Sendable)
                    // The name itself should be a clean identifier
                    let name = name.trim().to_string();
                    if !name.is_empty() && !name.starts_with('@') {
                        bases.push(name);
                    }
                }
            }
        }
    }

    bases
}

/// Extract the type name from an inherits_from node.
///
/// The inherits_from node can be:
/// - `user_type` -> contains type_identifier(s)
/// - `function_type` -> skip (not a class/protocol)
fn extract_inherited_type_name(node: &Node, source: &str) -> Option<String> {
    match node.kind() {
        "user_type" => {
            // user_type contains type_identifier children
            // For simple types: user_type > type_identifier "Animal"
            // For generic types: user_type > type_identifier "Array" > type_arguments
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "type_identifier" {
                    return child
                        .utf8_text(source.as_bytes())
                        .ok()
                        .map(|s| s.to_string());
                }
            }
            // Fall back to the full text of the user_type
            node.utf8_text(source.as_bytes())
                .ok()
                .map(|s| s.to_string())
        }
        "type_identifier" => node
            .utf8_text(source.as_bytes())
            .ok()
            .map(|s| s.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn parse_and_extract(source: &str) -> Vec<InheritanceNode> {
        let pool = ParserPool::new();
        extract_classes(source, &PathBuf::from("Test.swift"), &pool).unwrap()
    }

    #[test]
    fn test_simple_class() {
        let source = r#"
class Animal {
    func speak() -> String {
        return "..."
    }
}
"#;
        let classes = parse_and_extract(source);
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, "Animal");
        assert!(classes[0].bases.is_empty());
        assert_eq!(classes[0].language, Language::Swift);
    }

    #[test]
    fn test_class_inherits() {
        let source = r#"
class Animal {
    func speak() -> String { return "..." }
}

class Dog: Animal {
    override func speak() -> String { return "Woof" }
}
"#;
        let classes = parse_and_extract(source);
        assert_eq!(classes.len(), 2);

        let dog = classes.iter().find(|c| c.name == "Dog").unwrap();
        assert!(dog.bases.contains(&"Animal".to_string()));
    }

    #[test]
    fn test_protocol_declaration() {
        let source = r#"
protocol Serializable {
    func serialize() -> String
}
"#;
        let classes = parse_and_extract(source);
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, "Serializable");
        assert_eq!(classes[0].protocol, Some(true));
        assert_eq!(classes[0].interface, Some(true));
    }

    #[test]
    fn test_class_with_protocol() {
        let source = r#"
protocol ParameterEncoder {
    func encode() -> String
}

class JSONParameterEncoder: ParameterEncoder {
    func encode() -> String { return "{}" }
}
"#;
        let classes = parse_and_extract(source);
        let encoder = classes
            .iter()
            .find(|c| c.name == "JSONParameterEncoder")
            .unwrap();
        assert!(encoder.bases.contains(&"ParameterEncoder".to_string()));
    }

    #[test]
    fn test_class_inherits_and_conforms() {
        let source = r#"
class Request {
    var url: String = ""
}

protocol Sendable {}

class DownloadRequest: Request, Sendable {
    var destination: String = ""
}
"#;
        let classes = parse_and_extract(source);
        let download = classes
            .iter()
            .find(|c| c.name == "DownloadRequest")
            .unwrap();
        assert!(download.bases.contains(&"Request".to_string()));
        assert!(download.bases.contains(&"Sendable".to_string()));
        assert_eq!(download.bases.len(), 2);
    }

    #[test]
    fn test_protocol_inherits_protocol() {
        let source = r#"
protocol Base {
    func id() -> String
}

protocol Extended: Base {
    func name() -> String
}
"#;
        let classes = parse_and_extract(source);
        let extended = classes.iter().find(|c| c.name == "Extended").unwrap();
        assert!(extended.bases.contains(&"Base".to_string()));
        assert_eq!(extended.protocol, Some(true));
    }

    #[test]
    fn test_struct_with_protocol() {
        let source = r#"
protocol Encodable {}

struct Options: Encodable {
    var rawValue: Int = 0
}
"#;
        let classes = parse_and_extract(source);
        let opts = classes.iter().find(|c| c.name == "Options").unwrap();
        assert!(opts.bases.contains(&"Encodable".to_string()));
    }

    #[test]
    fn test_enum_declaration() {
        let source = r#"
protocol RawRepresentable {}

enum Direction: RawRepresentable {
    case north
    case south
}
"#;
        let classes = parse_and_extract(source);
        let direction = classes.iter().find(|c| c.name == "Direction").unwrap();
        assert!(direction.bases.contains(&"RawRepresentable".to_string()));
    }
}
