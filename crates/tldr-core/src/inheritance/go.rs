//! Go struct embedding extraction (A14)
//!
//! Go doesn't have class inheritance, but uses struct embedding for composition.
//! This module extracts embedded structs and models them as "Embeds" edges.
//!
//! Example:
//! ```go
//! type Animal struct {
//!     Name string
//! }
//!
//! type Dog struct {
//!     Animal      // Embedded - acts like inheritance
//!     Breed string
//! }
//! ```

use std::path::Path;

use tree_sitter::{Node, Tree};

use crate::ast::parser::ParserPool;
use crate::types::{InheritanceNode, Language};
use crate::TldrResult;

/// Extract struct definitions with embedded types from Go source code
pub fn extract_classes(
    source: &str,
    file_path: &Path,
    parser_pool: &ParserPool,
) -> TldrResult<Vec<InheritanceNode>> {
    let tree = parser_pool.parse(source, Language::Go)?;
    let mut classes = Vec::new();

    extract_type_declarations(&tree, source, file_path, &mut classes);

    Ok(classes)
}

fn extract_type_declarations(
    tree: &Tree,
    source: &str,
    file_path: &Path,
    classes: &mut Vec<InheritanceNode>,
) {
    let root = tree.root_node();
    visit_node(&root, source, file_path, classes);
}

fn visit_node(node: &Node, source: &str, file_path: &Path, classes: &mut Vec<InheritanceNode>) {
    if node.kind() == "type_declaration" {
        // type_declaration contains type_spec children
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if child.kind() == "type_spec" {
                    if let Some(class) = extract_type_spec(&child, source, file_path) {
                        classes.push(class);
                    }
                }
            }
        }
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        visit_node(&child, source, file_path, classes);
    }
}

fn extract_type_spec(node: &Node, source: &str, file_path: &Path) -> Option<InheritanceNode> {
    // Get type name
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source.as_bytes()).ok()?.to_string();

    let line = node.start_position().row as u32 + 1;

    // Get the type definition
    let type_node = node.child_by_field_name("type")?;

    // Only process struct types
    if type_node.kind() != "struct_type" {
        // For interfaces, we could model them separately
        if type_node.kind() == "interface_type" {
            let mut iface = InheritanceNode::new(name, file_path.to_path_buf(), line, Language::Go);
            iface.interface = Some(true);

            // Extract embedded interfaces
            let bases = extract_interface_embeds(&type_node, source);
            iface.bases = bases;

            return Some(iface);
        }
        return None;
    }

    let mut class_node = InheritanceNode::new(name, file_path.to_path_buf(), line, Language::Go);

    // Extract embedded structs (anonymous fields)
    let bases = extract_struct_embeds(&type_node, source);
    class_node.bases = bases;

    Some(class_node)
}

/// Extract embedded structs from struct type
/// Embedded fields are anonymous (no name, just type)
fn extract_struct_embeds(struct_node: &Node, source: &str) -> Vec<String> {
    let mut embeds = Vec::new();

    // Look for field_declaration_list
    for i in 0..struct_node.child_count() {
        if let Some(child) = struct_node.child(i) {
            if child.kind() == "field_declaration_list" {
                for j in 0..child.child_count() {
                    if let Some(field) = child.child(j) {
                        if field.kind() == "field_declaration" {
                            if let Some(embed) = extract_embed_from_field(&field, source) {
                                embeds.push(embed);
                            }
                        }
                    }
                }
            }
        }
    }

    embeds
}

/// Extract embedded type from field declaration
/// An embedded field has a type but no name
fn extract_embed_from_field(field: &Node, source: &str) -> Option<String> {
    // Check if this is an embedded field (no name, just type)
    // In tree-sitter-go, embedded fields have the type as the first significant child

    let mut has_name = false;
    let mut type_name = None;

    for i in 0..field.child_count() {
        if let Some(child) = field.child(i) {
            match child.kind() {
                "field_identifier" => {
                    has_name = true;
                }
                "type_identifier" => {
                    type_name = child
                        .utf8_text(source.as_bytes())
                        .ok()
                        .map(|s| s.to_string());
                }
                "pointer_type" => {
                    // *EmbeddedType
                    if let Some(inner) = child.child_by_field_name("type") {
                        if inner.kind() == "type_identifier" {
                            type_name = inner
                                .utf8_text(source.as_bytes())
                                .ok()
                                .map(|s| s.to_string());
                        }
                    }
                }
                "qualified_type" => {
                    // package.Type
                    if let Some(name) = child.child_by_field_name("name") {
                        type_name = name
                            .utf8_text(source.as_bytes())
                            .ok()
                            .map(|s| s.to_string());
                    }
                }
                _ => {}
            }
        }
    }

    // Only return if it's an embedded field (no explicit name)
    if !has_name {
        type_name
    } else {
        None
    }
}

/// Extract embedded interfaces from interface type
fn extract_interface_embeds(iface_node: &Node, source: &str) -> Vec<String> {
    let mut embeds = Vec::new();

    // Walk all children recursively to find embedded types
    visit_interface_children(iface_node, source, &mut embeds);

    embeds
}

fn visit_interface_children(node: &Node, source: &str, embeds: &mut Vec<String>) {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            match child.kind() {
                // Embedded interface types directly
                "type_identifier" => {
                    if let Ok(name) = child.utf8_text(source.as_bytes()) {
                        embeds.push(name.to_string());
                    }
                }
                "qualified_type" => {
                    if let Some(name) = child.child_by_field_name("name") {
                        if let Ok(n) = name.utf8_text(source.as_bytes()) {
                            embeds.push(n.to_string());
                        }
                    }
                }
                // In tree-sitter-go, interface bodies may have embedded types
                // that look like method_spec but without parameters
                "method_spec" => {
                    // Check if this is actually an embedded type (just a name, no signature)
                    // In tree-sitter-go, embedded types in interfaces are not method_spec
                    // They should be type_identifier directly. But let's handle variations.
                    visit_interface_children(&child, source, embeds);
                }
                _ => {
                    // Recurse into other nodes
                    visit_interface_children(&child, source, embeds);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn parse_and_extract(source: &str) -> Vec<InheritanceNode> {
        let pool = ParserPool::new();
        extract_classes(source, &PathBuf::from("test.go"), &pool).unwrap()
    }

    #[test]
    fn test_simple_struct() {
        let source = r#"
package main

type Animal struct {
    Name string
}
"#;
        let classes = parse_and_extract(source);
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, "Animal");
        assert!(classes[0].bases.is_empty());
    }

    #[test]
    fn test_struct_embedding() {
        let source = r#"
package main

type Animal struct {
    Name string
}

type Dog struct {
    Animal
    Breed string
}
"#;
        let classes = parse_and_extract(source);
        assert_eq!(classes.len(), 2);

        let dog = classes.iter().find(|c| c.name == "Dog").unwrap();
        assert!(dog.bases.contains(&"Animal".to_string()));
    }

    #[test]
    fn test_multiple_embedding() {
        let source = r#"
package main

type Walker struct {}
type Talker struct {}

type Robot struct {
    Walker
    Talker
    ID int
}
"#;
        let classes = parse_and_extract(source);
        let robot = classes.iter().find(|c| c.name == "Robot").unwrap();
        assert!(robot.bases.contains(&"Walker".to_string()));
        assert!(robot.bases.contains(&"Talker".to_string()));
    }

    #[test]
    fn test_interface() {
        let source = r#"
package main

type Reader interface {
    Read(p []byte) (n int, err error)
}
"#;
        let classes = parse_and_extract(source);
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, "Reader");
        assert_eq!(classes[0].interface, Some(true));
    }

    #[test]
    fn test_interface_embedding() {
        let source = r#"
package main

type Reader interface {
    Read(p []byte) (n int, err error)
}

type Writer interface {
    Write(p []byte) (n int, err error)
}

type ReadWriter interface {
    Reader
    Writer
}
"#;
        let classes = parse_and_extract(source);
        let rw = classes.iter().find(|c| c.name == "ReadWriter").unwrap();
        assert_eq!(rw.interface, Some(true));
        assert!(rw.bases.contains(&"Reader".to_string()));
        assert!(rw.bases.contains(&"Writer".to_string()));
    }
}
