//! Rust trait extraction (A16)
//!
//! Extracts trait definitions and impl blocks from Rust source code.
//! Models:
//! - `trait Foo` definitions as interface-like nodes
//! - `impl Trait for Type` blocks as Implements edges
//!
//! Note: Generic trait bounds like `fn foo<T: Animal>()` are NOT modeled
//! as inheritance relationships.

use std::path::Path;

use tree_sitter::{Node, Tree};

use crate::ast::parser::ParserPool;
use crate::types::{InheritanceNode, Language};
use crate::TldrResult;

/// Extract trait definitions and struct/enum types with impl blocks
pub fn extract_classes(
    source: &str,
    file_path: &Path,
    parser_pool: &ParserPool,
) -> TldrResult<Vec<InheritanceNode>> {
    let tree = parser_pool.parse(source, Language::Rust)?;
    let mut classes = Vec::new();
    let mut impl_map: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();

    // First pass: collect all traits, structs, enums
    extract_definitions(&tree, source, file_path, &mut classes);

    // Second pass: collect impl blocks to add bases
    collect_impl_blocks(&tree, source, &mut impl_map);

    // Apply impl blocks to types
    for class in &mut classes {
        if let Some(traits) = impl_map.get(&class.name) {
            class.bases = traits.clone();
        }
    }

    Ok(classes)
}

fn extract_definitions(
    tree: &Tree,
    source: &str,
    file_path: &Path,
    classes: &mut Vec<InheritanceNode>,
) {
    let root = tree.root_node();
    visit_for_definitions(&root, source, file_path, classes);
}

fn visit_for_definitions(
    node: &Node,
    source: &str,
    file_path: &Path,
    classes: &mut Vec<InheritanceNode>,
) {
    match node.kind() {
        "trait_item" => {
            if let Some(trait_node) = extract_trait(node, source, file_path) {
                classes.push(trait_node);
            }
        }
        "struct_item" => {
            if let Some(struct_node) = extract_struct(node, source, file_path) {
                classes.push(struct_node);
            }
        }
        "enum_item" => {
            if let Some(enum_node) = extract_enum(node, source, file_path) {
                classes.push(enum_node);
            }
        }
        _ => {}
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        visit_for_definitions(&child, source, file_path, classes);
    }
}

fn extract_trait(node: &Node, source: &str, file_path: &Path) -> Option<InheritanceNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source.as_bytes()).ok()?.to_string();

    let line = node.start_position().row as u32 + 1;

    let mut trait_node = InheritanceNode::new(name, file_path.to_path_buf(), line, Language::Rust);
    trait_node.interface = Some(true); // Traits are like interfaces
    trait_node.is_abstract = Some(true); // Cannot instantiate traits

    // Extract super traits (trait Foo: Bar + Baz)
    let mut supers = Vec::new();
    if let Some(bounds) = node.child_by_field_name("bounds") {
        extract_trait_bounds(&bounds, source, &mut supers);
    }
    trait_node.bases = supers;

    Some(trait_node)
}

fn extract_trait_bounds(node: &Node, source: &str, supers: &mut Vec<String>) {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if let Some(name) = extract_type_from_bound(&child, source) {
                supers.push(name);
            }
        }
    }
}

fn extract_type_from_bound(node: &Node, source: &str) -> Option<String> {
    match node.kind() {
        "type_identifier" => node
            .utf8_text(source.as_bytes())
            .ok()
            .map(|s| s.to_string()),
        "generic_type" => {
            let type_name = node.child_by_field_name("type")?;
            type_name
                .utf8_text(source.as_bytes())
                .ok()
                .map(|s| s.to_string())
        }
        "scoped_type_identifier" => {
            // path::Trait -> "Trait"
            let name = node.child_by_field_name("name")?;
            name.utf8_text(source.as_bytes())
                .ok()
                .map(|s| s.to_string())
        }
        _ => None,
    }
}

fn extract_struct(node: &Node, source: &str, file_path: &Path) -> Option<InheritanceNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source.as_bytes()).ok()?.to_string();

    let line = node.start_position().row as u32 + 1;

    Some(InheritanceNode::new(
        name,
        file_path.to_path_buf(),
        line,
        Language::Rust,
    ))
}

fn extract_enum(node: &Node, source: &str, file_path: &Path) -> Option<InheritanceNode> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source.as_bytes()).ok()?.to_string();

    let line = node.start_position().row as u32 + 1;

    Some(InheritanceNode::new(
        name,
        file_path.to_path_buf(),
        line,
        Language::Rust,
    ))
}

fn collect_impl_blocks(
    tree: &Tree,
    source: &str,
    impl_map: &mut std::collections::HashMap<String, Vec<String>>,
) {
    let root = tree.root_node();
    visit_for_impls(&root, source, impl_map);
}

fn visit_for_impls(
    node: &Node,
    source: &str,
    impl_map: &mut std::collections::HashMap<String, Vec<String>>,
) {
    if node.kind() == "impl_item" {
        // Check if this is `impl Trait for Type`
        if let Some((type_name, trait_name)) = extract_impl_for(node, source) {
            impl_map.entry(type_name).or_default().push(trait_name);
        }
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        visit_for_impls(&child, source, impl_map);
    }
}

fn extract_impl_for(node: &Node, source: &str) -> Option<(String, String)> {
    // Look for impl Trait for Type pattern
    let mut trait_name: Option<String> = None;
    let mut type_name: Option<String> = None;

    // In tree-sitter-rust, impl_item has:
    // - trait: the trait being implemented (optional)
    // - type: the type implementing the trait
    if let Some(trait_node) = node.child_by_field_name("trait") {
        trait_name = extract_type_name(&trait_node, source);
    }

    if let Some(type_node) = node.child_by_field_name("type") {
        type_name = extract_type_name(&type_node, source);
    }

    // Only return if both trait and type are present (impl Trait for Type)
    match (type_name, trait_name) {
        (Some(t), Some(tr)) => Some((t, tr)),
        _ => None,
    }
}

fn extract_type_name(node: &Node, source: &str) -> Option<String> {
    match node.kind() {
        "type_identifier" => node
            .utf8_text(source.as_bytes())
            .ok()
            .map(|s| s.to_string()),
        "generic_type" => {
            let type_name = node.child_by_field_name("type")?;
            type_name
                .utf8_text(source.as_bytes())
                .ok()
                .map(|s| s.to_string())
        }
        "scoped_type_identifier" => {
            let name = node.child_by_field_name("name")?;
            name.utf8_text(source.as_bytes())
                .ok()
                .map(|s| s.to_string())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn parse_and_extract(source: &str) -> Vec<InheritanceNode> {
        let pool = ParserPool::new();
        extract_classes(source, &PathBuf::from("test.rs"), &pool).unwrap()
    }

    #[test]
    fn test_simple_struct() {
        let source = r#"
struct Animal {
    name: String,
}
"#;
        let classes = parse_and_extract(source);
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, "Animal");
        assert!(classes[0].bases.is_empty());
    }

    #[test]
    fn test_trait_definition() {
        let source = r#"
trait Animal {
    fn speak(&self) -> String;
}
"#;
        let classes = parse_and_extract(source);
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, "Animal");
        assert_eq!(classes[0].interface, Some(true));
        assert_eq!(classes[0].is_abstract, Some(true));
    }

    #[test]
    fn test_impl_trait_for_struct() {
        let source = r#"
trait Animal {
    fn speak(&self) -> String;
}

struct Dog {
    name: String,
}

impl Animal for Dog {
    fn speak(&self) -> String {
        "woof".to_string()
    }
}
"#;
        let classes = parse_and_extract(source);
        assert_eq!(classes.len(), 2);

        let dog = classes.iter().find(|c| c.name == "Dog").unwrap();
        assert!(dog.bases.contains(&"Animal".to_string()));
    }

    #[test]
    fn test_trait_super_traits() {
        let source = r#"
trait Base {}

trait Derived: Base {
    fn method(&self);
}
"#;
        let classes = parse_and_extract(source);
        let derived = classes.iter().find(|c| c.name == "Derived").unwrap();
        assert!(derived.bases.contains(&"Base".to_string()));
    }

    #[test]
    fn test_enum() {
        let source = r#"
enum Color {
    Red,
    Green,
    Blue,
}
"#;
        let classes = parse_and_extract(source);
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, "Color");
    }

    #[test]
    fn test_multiple_trait_impls() {
        let source = r#"
trait Display {
    fn display(&self) -> String;
}

trait Debug {
    fn debug(&self) -> String;
}

struct Point {
    x: i32,
    y: i32,
}

impl Display for Point {
    fn display(&self) -> String {
        format!("({}, {})", self.x, self.y)
    }
}

impl Debug for Point {
    fn debug(&self) -> String {
        format!("Point {{ x: {}, y: {} }}", self.x, self.y)
    }
}
"#;
        let classes = parse_and_extract(source);
        let point = classes.iter().find(|c| c.name == "Point").unwrap();
        assert!(point.bases.contains(&"Display".to_string()));
        assert!(point.bases.contains(&"Debug".to_string()));
    }
}
