//! Multi-language interface extraction tests
//!
//! These tests validate that tree-sitter correctly parses different languages
//! and that our node kind assumptions are correct.

use tldr_core::ast::ParserPool;
use tldr_core::types::Language;

fn node_text<'a>(node: tree_sitter::Node, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
}

// =============================================================================
// Python
// =============================================================================

#[test]
fn test_python_function_definition_node_kinds() {
    let source = r#"
def public_func():
    """A public function."""
    pass

def _private_func():
    pass
"#;
    let pool = ParserPool::new();
    let tree = pool.parse(source, Language::Python).unwrap();
    let root = tree.root_node();

    let mut func_count = 0;
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "function_definition" {
            func_count += 1;
            let name = child
                .child_by_field_name("name")
                .map(|n| node_text(n, source.as_bytes()))
                .unwrap_or("");
            assert!(!name.is_empty(), "Function should have a name");
        }
    }
    assert_eq!(func_count, 2, "Should find 2 functions");
}

#[test]
fn test_python_class_definition_node_kinds() {
    let source = r#"
class PublicClass:
    def public_method(self):
        pass

    def _private_method(self):
        pass
"#;
    let pool = ParserPool::new();
    let tree = pool.parse(source, Language::Python).unwrap();
    let root = tree.root_node();

    let mut class_count = 0;
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "class_definition" {
            class_count += 1;
            let name = child
                .child_by_field_name("name")
                .map(|n| node_text(n, source.as_bytes()))
                .unwrap_or("");
            assert_eq!(name, "PublicClass");

            // Check body for methods
            let body = child.child_by_field_name("body").unwrap();
            let mut method_count = 0;
            let mut body_cursor = body.walk();
            for body_child in body.children(&mut body_cursor) {
                if body_child.kind() == "function_definition" {
                    method_count += 1;
                }
            }
            assert_eq!(method_count, 2, "Should find 2 methods in class body");
        }
    }
    assert_eq!(class_count, 1, "Should find 1 class");
}

// =============================================================================
// Rust
// =============================================================================

#[test]
fn test_rust_function_item_node_kinds() {
    let source = r#"
/// Adds two numbers.
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn private_helper() -> bool {
    true
}

pub async fn async_fetch() -> String {
    String::new()
}
"#;
    let pool = ParserPool::new();
    let tree = pool.parse(source, Language::Rust).unwrap();
    let root = tree.root_node();

    let mut pub_funcs = Vec::new();
    let mut priv_funcs = Vec::new();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        if child.kind() == "function_item" {
            let name = child
                .child_by_field_name("name")
                .map(|n| node_text(n, source.as_bytes()).to_string())
                .unwrap_or_default();

            // Check for visibility_modifier
            let mut is_pub = false;
            for i in 0..child.child_count() {
                if let Some(gc) = child.child(i) {
                    if gc.kind() == "visibility_modifier" {
                        is_pub = true;
                        break;
                    }
                }
            }

            if is_pub {
                pub_funcs.push(name);
            } else {
                priv_funcs.push(name);
            }
        }
    }

    assert_eq!(
        pub_funcs.len(),
        2,
        "Should find 2 pub functions: {:?}",
        pub_funcs
    );
    assert!(pub_funcs.contains(&"add".to_string()));
    assert!(pub_funcs.contains(&"async_fetch".to_string()));
    assert_eq!(priv_funcs.len(), 1, "Should find 1 private function");
    assert!(priv_funcs.contains(&"private_helper".to_string()));
}

#[test]
fn test_rust_struct_impl_trait_node_kinds() {
    let source = r#"
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub fn new(x: f64, y: f64) -> Self {
        Point { x, y }
    }

    fn internal(&self) {}
}

pub trait Drawable {
    fn draw(&self);
}
"#;
    let pool = ParserPool::new();
    let tree = pool.parse(source, Language::Rust).unwrap();
    let root = tree.root_node();

    let mut struct_count = 0;
    let mut impl_count = 0;
    let mut trait_count = 0;
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        match child.kind() {
            "struct_item" => struct_count += 1,
            "impl_item" => {
                impl_count += 1;
                // Check that impl has declaration_list body
                let mut has_decl_list = false;
                let mut inner_cursor = child.walk();
                for gc in child.children(&mut inner_cursor) {
                    if gc.kind() == "declaration_list" {
                        has_decl_list = true;
                        // Count pub methods
                        let mut pub_methods = 0;
                        let mut priv_methods = 0;
                        let mut decl_cursor = gc.walk();
                        for method in gc.children(&mut decl_cursor) {
                            if method.kind() == "function_item" {
                                let mut is_pub = false;
                                for k in 0..method.child_count() {
                                    if let Some(mk) = method.child(k) {
                                        if mk.kind() == "visibility_modifier" {
                                            is_pub = true;
                                            break;
                                        }
                                    }
                                }
                                if is_pub {
                                    pub_methods += 1;
                                } else {
                                    priv_methods += 1;
                                }
                            }
                        }
                        assert_eq!(pub_methods, 1, "Should find 1 pub method in impl");
                        assert_eq!(priv_methods, 1, "Should find 1 private method in impl");
                    }
                }
                assert!(has_decl_list, "impl should have declaration_list");
            }
            "trait_item" => trait_count += 1,
            _ => {}
        }
    }

    assert_eq!(struct_count, 1, "Should find 1 struct");
    assert_eq!(impl_count, 1, "Should find 1 impl");
    assert_eq!(trait_count, 1, "Should find 1 trait");
}

#[test]
fn test_rust_doc_comments() {
    let source = r#"
/// Adds two numbers.
/// This is a multi-line doc comment.
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
"#;
    let pool = ParserPool::new();
    let tree = pool.parse(source, Language::Rust).unwrap();
    let root = tree.root_node();

    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "function_item" {
            // Check for preceding doc comments
            let mut comments = Vec::new();
            let mut prev = child.prev_sibling();
            while let Some(sib) = prev {
                if sib.kind() == "line_comment" {
                    let text = node_text(sib, source.as_bytes());
                    if text.starts_with("///") {
                        comments.push(text.to_string());
                    }
                } else {
                    break;
                }
                prev = sib.prev_sibling();
            }
            assert!(
                !comments.is_empty(),
                "Should find doc comments before function"
            );
        }
    }
}

// =============================================================================
// Go
// =============================================================================

#[test]
fn test_go_function_declaration_node_kinds() {
    let source = r#"
package main

// ProcessData handles data processing.
func ProcessData(input string) (string, error) {
    return input, nil
}

func internalHelper() bool {
    return true
}
"#;
    let pool = ParserPool::new();
    let tree = pool.parse(source, Language::Go).unwrap();
    let root = tree.root_node();

    let mut funcs = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "function_declaration" {
            let name = child
                .child_by_field_name("name")
                .map(|n| node_text(n, source.as_bytes()).to_string())
                .unwrap_or_default();
            funcs.push(name);
        }
    }

    assert_eq!(funcs.len(), 2, "Should find 2 functions: {:?}", funcs);
    assert!(funcs.contains(&"ProcessData".to_string()));
    assert!(funcs.contains(&"internalHelper".to_string()));
}

#[test]
fn test_go_exported_vs_unexported() {
    let source = r#"
package main

func Exported() {}
func unexported() {}
"#;
    let pool = ParserPool::new();
    let tree = pool.parse(source, Language::Go).unwrap();
    let root = tree.root_node();

    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "function_declaration" {
            let name = child
                .child_by_field_name("name")
                .map(|n| node_text(n, source.as_bytes()))
                .unwrap_or("");
            let first_char = name.chars().next().unwrap();
            if name == "Exported" {
                assert!(first_char.is_uppercase());
            } else {
                assert!(first_char.is_lowercase());
            }
        }
    }
}

// =============================================================================
// TypeScript
// =============================================================================

#[test]
fn test_typescript_class_and_function_node_kinds() {
    let source = r#"
class UserService {
    async fetchUser(id: string): Promise<User> {
        return {} as User;
    }
}

function processData(input: string): number {
    return input.length;
}

interface User {
    id: string;
    name: string;
}

type Status = "active" | "inactive";
"#;
    let pool = ParserPool::new();
    let tree = pool.parse(source, Language::TypeScript).unwrap();
    let root = tree.root_node();

    let mut class_count = 0;
    let mut func_count = 0;
    let mut interface_count = 0;
    let mut type_alias_count = 0;
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        match child.kind() {
            "class_declaration" => class_count += 1,
            "function_declaration" => func_count += 1,
            "interface_declaration" => interface_count += 1,
            "type_alias_declaration" => type_alias_count += 1,
            _ => {}
        }
    }

    assert_eq!(class_count, 1, "Should find 1 class");
    assert_eq!(func_count, 1, "Should find 1 function");
    assert_eq!(interface_count, 1, "Should find 1 interface");
    assert_eq!(type_alias_count, 1, "Should find 1 type alias");
}

// =============================================================================
// Java
// =============================================================================

#[test]
fn test_java_class_declaration_node_kinds() {
    let source = r#"
public class UserService {
    public String getUser(String id) {
        return id;
    }

    private void internalCleanup() {}
}
"#;
    let pool = ParserPool::new();
    let tree = pool.parse(source, Language::Java).unwrap();
    let root = tree.root_node();

    let mut class_count = 0;
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "class_declaration" {
            class_count += 1;
            let name = child
                .child_by_field_name("name")
                .map(|n| node_text(n, source.as_bytes()))
                .unwrap_or("");
            assert_eq!(name, "UserService");

            // Check modifiers
            if let Some(mods) = child.child_by_field_name("modifiers") {
                assert!(
                    node_text(mods, source.as_bytes()).contains("public"),
                    "Class should have public modifier"
                );
            }

            // Check body - Java uses "class_body" not "body" field
            // Try both "body" field and explicit class_body search
            let mut c2 = child.walk();
            let class_body = child
                .children(&mut c2)
                .find(|gc| gc.kind() == "class_body");
            let body = child.child_by_field_name("body").or(class_body);
            assert!(
                body.is_some(),
                "Should find class body. Children: {:?}",
                (0..child.child_count())
                    .map(|i| child.child(i).map(|c| c.kind().to_string()))
                    .collect::<Vec<_>>()
            );
            let body = body.unwrap();
            let mut pub_methods = 0;
            let mut priv_methods = 0;
            let mut body_cursor = body.walk();
            for bc in body.children(&mut body_cursor) {
                if bc.kind() == "method_declaration" {
                    // Check modifiers - try field name and direct child search
                    let mod_text = bc
                        .child_by_field_name("modifiers")
                        .or_else(|| {
                            for i in 0..bc.child_count() {
                                if let Some(gc) = bc.child(i) {
                                    if gc.kind() == "modifiers" {
                                        return Some(gc);
                                    }
                                }
                            }
                            None
                        })
                        .map(|m| node_text(m, source.as_bytes()).to_string())
                        .unwrap_or_default();

                    if mod_text.contains("public") {
                        pub_methods += 1;
                    } else if mod_text.contains("private") {
                        priv_methods += 1;
                    }
                }
            }
            assert_eq!(pub_methods, 1, "Should find 1 public method");
            assert_eq!(priv_methods, 1, "Should find 1 private method");
        }
    }
    assert_eq!(class_count, 1, "Should find 1 class");
}

// =============================================================================
// C
// =============================================================================

#[test]
fn test_c_function_definition_node_kinds() {
    let source = r#"
int add(int a, int b) {
    return a + b;
}

static int internal_helper(void) {
    return 42;
}
"#;
    let pool = ParserPool::new();
    let tree = pool.parse(source, Language::C).unwrap();
    let root = tree.root_node();

    let mut funcs = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "function_definition" {
            // Get name from declarator
            let mut is_static = false;
            for i in 0..child.child_count() {
                if let Some(gc) = child.child(i) {
                    if gc.kind() == "storage_class_specifier"
                        && node_text(gc, source.as_bytes()) == "static"
                    {
                        is_static = true;
                    }
                }
            }

            // Get function name
            let name = if let Some(declarator) = child.child_by_field_name("declarator") {
                get_c_func_name(declarator, source.as_bytes())
            } else {
                String::new()
            };

            funcs.push((name, is_static));
        }
    }

    assert_eq!(funcs.len(), 2, "Should find 2 functions: {:?}", funcs);
    assert!(funcs.iter().any(|(n, s)| n == "add" && !s));
    assert!(funcs.iter().any(|(n, s)| n == "internal_helper" && *s));
}

fn get_c_func_name(declarator: tree_sitter::Node, source: &[u8]) -> String {
    if declarator.kind() == "identifier" {
        return node_text(declarator, source).to_string();
    }
    if let Some(inner) = declarator.child_by_field_name("declarator") {
        return get_c_func_name(inner, source);
    }
    if let Some(first) = declarator.child(0) {
        if first.kind() == "identifier" {
            return node_text(first, source).to_string();
        }
    }
    String::new()
}

// =============================================================================
// Ruby
// =============================================================================

#[test]
fn test_ruby_class_and_method_node_kinds() {
    let source = r#"
class UserManager
  def find_user(id)
    # find user
  end

  def _private_method
    # private
  end
end
"#;
    let pool = ParserPool::new();
    let tree = pool.parse(source, Language::Ruby).unwrap();
    let root = tree.root_node();

    let mut class_count = 0;
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "class" {
            class_count += 1;
            let name = child
                .child_by_field_name("name")
                .map(|n| node_text(n, source.as_bytes()))
                .unwrap_or("");
            assert_eq!(name, "UserManager");

            // Ruby class body: methods may be in a body_statement child or direct
            let mut method_count = 0;
            // First print all children to understand structure
            let child_kinds: Vec<String> = (0..child.child_count())
                .filter_map(|i| child.child(i).map(|c| c.kind().to_string()))
                .collect();
            // Try direct children first
            let mut inner_cursor = child.walk();
            for gc in child.children(&mut inner_cursor) {
                if gc.kind() == "method" {
                    method_count += 1;
                }
                // Also check inside body_statement
                if gc.kind() == "body_statement" {
                    let mut body_cursor = gc.walk();
                    for bc in gc.children(&mut body_cursor) {
                        if bc.kind() == "method" {
                            method_count += 1;
                        }
                    }
                }
            }
            assert_eq!(
                method_count, 2,
                "Should find 2 methods in class body. Children: {:?}",
                child_kinds
            );
        }
    }
    assert_eq!(class_count, 1, "Should find 1 class");
}

// =============================================================================
// C#
// =============================================================================

#[test]
fn test_csharp_class_node_kinds() {
    let source = r#"
public class UserService {
    public string GetUser(string id) {
        return id;
    }

    private void InternalCleanup() {}
}
"#;
    let pool = ParserPool::new();
    let tree = pool.parse(source, Language::CSharp).unwrap();
    let root = tree.root_node();

    let mut class_count = 0;
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "class_declaration" {
            class_count += 1;
        }
    }
    assert_eq!(class_count, 1, "Should find 1 C# class");
}

// =============================================================================
// Scala
// =============================================================================

#[test]
fn test_scala_class_node_kinds() {
    let source = r#"
class UserService {
  def getUser(id: String): String = {
    id
  }
}

object Config {
  val name = "test"
}
"#;
    let pool = ParserPool::new();
    let tree = pool.parse(source, Language::Scala).unwrap();
    let root = tree.root_node();

    let mut class_count = 0;
    let mut object_count = 0;
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        match child.kind() {
            "class_definition" => class_count += 1,
            "object_definition" => object_count += 1,
            _ => {}
        }
    }
    assert_eq!(class_count, 1, "Should find 1 Scala class");
    assert_eq!(object_count, 1, "Should find 1 Scala object");
}
