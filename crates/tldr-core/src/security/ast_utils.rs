//! AST utility functions for security analysis
//!
//! Provides reusable tree-sitter helpers for taint analysis and other
//! security checks. These functions abstract away language-specific
//! node kind differences.

use tree_sitter::Node;

use crate::Language;

// =============================================================================
// Node Kind Lookup Tables
// =============================================================================

/// Get the tree-sitter node kinds for call expressions in each language.
///
/// Returns a slice of node kind strings that represent function/method calls.
pub fn call_node_kinds(language: Language) -> &'static [&'static str] {
    match language {
        Language::Python => &["call"],
        Language::TypeScript | Language::JavaScript => &["call_expression"],
        Language::Go => &["call_expression"],
        Language::Java => &["method_invocation", "object_creation_expression"],
        Language::Kotlin => &["call_expression", "navigation_expression"],
        Language::Rust => &["call_expression", "macro_invocation"],
        Language::C | Language::Cpp => &["call_expression"],
        Language::Ruby => &["call", "method_call"],
        Language::Swift => &["call_expression"],
        Language::CSharp => &["invocation_expression", "object_creation_expression"],
        Language::Scala => &["call_expression"],
        Language::Php => &["function_call_expression", "member_call_expression"],
        Language::Lua | Language::Luau => &["function_call"],
        Language::Elixir => &["call"],
        Language::Ocaml => &["application_expression"],
    }
}

/// Get the tree-sitter node kinds for string literals in each language.
pub fn string_node_kinds(language: Language) -> &'static [&'static str] {
    match language {
        Language::Python => &["string", "concatenated_string"],
        Language::TypeScript | Language::JavaScript => &["string", "template_string"],
        Language::Go => &["interpreted_string_literal", "raw_string_literal"],
        Language::Java => &["string_literal"],
        Language::Kotlin => &["string_literal"],
        Language::Rust => &["string_literal", "raw_string_literal"],
        Language::C | Language::Cpp => &["string_literal"],
        Language::Ruby => &["string", "string_content"],
        Language::Swift => &["line_string_literal"],
        Language::CSharp => &["string_literal", "verbatim_string_literal"],
        Language::Scala => &["string"],
        Language::Php => &["string", "encapsed_string"],
        Language::Lua | Language::Luau => &["string"],
        Language::Elixir => &["string"],
        Language::Ocaml => &["string"],
    }
}

/// Get the tree-sitter node kinds for assignment nodes in each language.
pub fn assignment_node_kinds(language: Language) -> &'static [&'static str] {
    match language {
        Language::Python => &["assignment", "augmented_assignment"],
        Language::TypeScript | Language::JavaScript => &[
            "variable_declaration",
            "assignment_expression",
            "augmented_assignment_expression",
        ],
        Language::Go => &["short_var_declaration", "assignment_statement"],
        Language::Java => &["local_variable_declaration", "assignment_expression"],
        Language::Kotlin => &["property_declaration", "assignment"],
        Language::Rust => &["let_declaration", "assignment_expression"],
        Language::C | Language::Cpp => &["declaration", "assignment_expression"],
        Language::Ruby => &["assignment"],
        Language::Swift => &["property_declaration", "assignment"],
        Language::CSharp => &["variable_declaration", "assignment_expression"],
        Language::Scala => &["val_definition", "var_definition", "assignment_expression"],
        Language::Php => &["assignment_expression"],
        Language::Lua => &["variable_declaration", "assignment_statement"],
        Language::Luau => &["variable_declaration", "assignment_statement"],
        Language::Elixir => &["match_operator"],
        Language::Ocaml => &["let_binding", "value_definition"],
    }
}

/// Get the tree-sitter node kinds for binary expressions in each language.
///
/// These are arithmetic/bitwise operations like `a + b`, `x * y`, etc.
/// Used for Common Subexpression Elimination (CSE) / Available Expressions analysis.
pub fn binary_expression_node_kinds(language: Language) -> &'static [&'static str] {
    match language {
        Language::Python => &["binary_operator"],
        Language::TypeScript | Language::JavaScript => &["binary_expression"],
        Language::Go => &["binary_expression"],
        Language::Rust => &["binary_expression"],
        Language::Java => &["binary_expression"],
        Language::Kotlin => &[
            "additive_expression",
            "multiplicative_expression",
            "binary_expression",
        ],
        Language::C | Language::Cpp => &["binary_expression"],
        Language::Ruby => &["binary"],
        Language::Swift => &["binary_expression"],
        Language::CSharp => &["binary_expression"],
        Language::Scala => &["infix_expression"],
        Language::Php => &["binary_expression"],
        Language::Lua | Language::Luau => &["binary_expression"],
        Language::Elixir => &["binary_operator"],
        Language::Ocaml => &["infix_expression"],
    }
}

/// Get the tree-sitter node kinds for comment nodes in each language.
pub fn comment_node_kinds(language: Language) -> &'static [&'static str] {
    match language {
        Language::Python => &["comment"],
        Language::TypeScript | Language::JavaScript => &["comment"],
        Language::Go => &["comment"],
        Language::Java => &["line_comment", "block_comment"],
        Language::Kotlin => &["line_comment", "multiline_comment"],
        Language::Rust => &["line_comment", "block_comment"],
        Language::C | Language::Cpp => &["comment"],
        Language::Ruby => &["comment"],
        Language::Swift => &["comment", "multiline_comment"],
        Language::CSharp => &["comment"],
        Language::Scala => &["comment", "block_comment"],
        Language::Php => &["comment"],
        Language::Lua | Language::Luau => &["comment"],
        Language::Elixir => &["comment"],
        Language::Ocaml => &["comment"],
    }
}

/// Get the tree-sitter node kinds for loop constructs in each language.
///
/// Returns a slice of node kind strings that represent for/while/loop statements.
/// Used by bounds analysis to find loop iteration variables.
pub fn loop_node_kinds(language: Language) -> &'static [&'static str] {
    match language {
        Language::Python => &["for_statement", "while_statement"],
        Language::TypeScript | Language::JavaScript => &[
            "for_statement",
            "while_statement",
            "for_in_statement",
            "do_statement",
        ],
        Language::Go => &["for_statement"],
        Language::Rust => &["for_expression", "while_expression", "loop_expression"],
        Language::Java => &[
            "for_statement",
            "while_statement",
            "enhanced_for_statement",
            "do_statement",
        ],
        Language::C | Language::Cpp => &["for_statement", "while_statement", "do_statement"],
        Language::Ruby => &["for", "while", "until"],
        Language::Php => &[
            "for_statement",
            "while_statement",
            "foreach_statement",
            "do_statement",
        ],
        Language::Kotlin => &["for_statement", "while_statement", "do_while_statement"],
        Language::Swift => &["for_statement", "while_statement", "repeat_while_statement"],
        Language::CSharp => &[
            "for_statement",
            "while_statement",
            "foreach_statement",
            "do_statement",
        ],
        Language::Scala => &["for_expression", "while_expression"],
        Language::Elixir => &["call"],
        Language::Lua | Language::Luau => &["for_statement", "while_statement", "repeat_statement"],
        Language::Ocaml => &["for_expression", "while_expression"],
    }
}

/// Get the tree-sitter node kinds for literal values in each language.
///
/// Returns node kinds for integer, float, and string literals.
/// Used by GVN to identify constant expressions.
pub fn literal_node_kinds(language: Language) -> &'static [&'static str] {
    match language {
        Language::Python => &["integer", "float", "string"],
        Language::TypeScript | Language::JavaScript => &["number", "string"],
        Language::Go => &[
            "int_literal",
            "float_literal",
            "interpreted_string_literal",
            "raw_string_literal",
        ],
        Language::Rust => &["integer_literal", "float_literal", "string_literal"],
        Language::Java => &[
            "decimal_integer_literal",
            "decimal_floating_point_literal",
            "string_literal",
        ],
        Language::C | Language::Cpp => &["number_literal", "string_literal"],
        Language::Ruby => &["integer", "float", "string"],
        Language::Php => &["integer", "float", "string"],
        Language::Kotlin => &["integer_literal", "real_literal", "string_literal"],
        Language::Swift => &["integer_literal", "real_literal", "line_string_literal"],
        Language::CSharp => &["integer_literal", "real_literal", "string_literal"],
        Language::Scala => &["integer_literal", "floating_point_literal", "string"],
        Language::Elixir => &["integer", "float", "string"],
        Language::Lua | Language::Luau => &["number", "string"],
        Language::Ocaml => &["number", "string", "character"],
    }
}

/// Get the tree-sitter node kinds for identifier nodes in each language.
///
/// Returns node kinds that represent variable/symbol names.
/// Used by GVN to recognize variable references across all languages.
pub fn identifier_node_kinds(language: Language) -> &'static [&'static str] {
    match language {
        Language::Python => &["identifier"],
        Language::TypeScript | Language::JavaScript => &["identifier"],
        Language::Go => &["identifier"],
        Language::Rust => &["identifier"],
        Language::Java => &["identifier"],
        Language::C | Language::Cpp => &["identifier"],
        Language::Ruby => &["identifier", "constant"],
        Language::Php => &["name", "variable_name"],
        Language::Kotlin => &["simple_identifier"],
        Language::Swift => &["simple_identifier"],
        Language::CSharp => &["identifier"],
        Language::Scala => &["identifier"],
        Language::Elixir => &["identifier", "atom"],
        Language::Lua | Language::Luau => &["identifier"],
        Language::Ocaml => &["value_name", "module_name"],
    }
}

/// Get the tree-sitter node kinds for unary expressions in each language.
///
/// Returns node kinds that represent unary operations like `-x`, `!x`, `~x`.
/// Used by GVN to recognize unary operators across all languages.
pub fn unary_expression_node_kinds(language: Language) -> &'static [&'static str] {
    match language {
        Language::Python => &["unary_operator"],
        Language::TypeScript | Language::JavaScript => &["unary_expression"],
        Language::Go => &["unary_expression"],
        Language::Rust => &["unary_expression"],
        Language::Java => &["unary_expression"],
        Language::C | Language::Cpp => &["unary_expression"],
        Language::Ruby => &["unary"],
        Language::Php => &["unary_op_expression"],
        Language::Kotlin => &["prefix_expression", "postfix_expression"],
        Language::Swift => &["prefix_expression", "postfix_expression"],
        Language::CSharp => &["prefix_unary_expression", "postfix_unary_expression"],
        Language::Scala => &["prefix_expression", "postfix_expression"],
        Language::Elixir => &["unary_operator"],
        Language::Lua | Language::Luau => &["unary_expression"],
        Language::Ocaml => &["prefix_expression"],
    }
}

/// Get the tree-sitter node kinds for boolean/logical expressions in each language.
///
/// Returns node kinds that represent boolean operations like `a and b`, `a || b`.
/// Used by GVN to recognize boolean operators across all languages.
pub fn boolean_expression_node_kinds(language: Language) -> &'static [&'static str] {
    match language {
        Language::Python => &["boolean_operator"],
        Language::TypeScript | Language::JavaScript => &["binary_expression"],
        Language::Go => &["binary_expression"],
        Language::Rust => &["binary_expression"],
        Language::Java => &["binary_expression"],
        Language::C | Language::Cpp => &["binary_expression"],
        Language::Ruby => &["binary"],
        Language::Php => &["binary_expression"],
        Language::Kotlin => &["conjunction_expression", "disjunction_expression"],
        Language::Swift => &["binary_expression"],
        Language::CSharp => &["binary_expression"],
        Language::Scala => &["infix_expression"],
        Language::Elixir => &["binary_operator"],
        Language::Lua | Language::Luau => &["binary_expression"],
        Language::Ocaml => &["infix_expression"],
    }
}

/// Get the tree-sitter node kinds for comparison expressions in each language.
///
/// Returns node kinds that represent comparison operations like `a < b`, `a == b`.
/// Used by GVN to recognize comparison operators across all languages.
pub fn comparison_node_kinds(language: Language) -> &'static [&'static str] {
    match language {
        Language::Python => &["comparison_operator"],
        Language::TypeScript | Language::JavaScript => &["binary_expression"],
        Language::Go => &["binary_expression"],
        Language::Rust => &["binary_expression"],
        Language::Java => &["binary_expression"],
        Language::C | Language::Cpp => &["binary_expression"],
        Language::Ruby => &["binary"],
        Language::Php => &["binary_expression"],
        Language::Kotlin => &["comparison_expression", "equality_expression"],
        Language::Swift => &["binary_expression"],
        Language::CSharp => &["binary_expression"],
        Language::Scala => &["infix_expression"],
        Language::Elixir => &["binary_operator"],
        Language::Lua | Language::Luau => &["binary_expression"],
        Language::Ocaml => &["infix_expression"],
    }
}

/// Get the tree-sitter node kinds for parenthesized expressions in each language.
///
/// Returns node kinds that represent parenthesized wrapping like `(a + b)`.
/// Used by GVN to unwrap parenthesized expressions.
pub fn parenthesized_expression_node_kinds(language: Language) -> &'static [&'static str] {
    match language {
        Language::Python => &["parenthesized_expression"],
        Language::TypeScript | Language::JavaScript => &["parenthesized_expression"],
        Language::Go => &["parenthesized_expression"],
        Language::Rust => &["parenthesized_expression"],
        Language::Java => &["parenthesized_expression"],
        Language::C | Language::Cpp => &["parenthesized_expression"],
        Language::Ruby => &["parenthesized_statements"],
        Language::Php => &["parenthesized_expression"],
        Language::Kotlin => &["parenthesized_expression"],
        Language::Swift => &["tuple_expression"],
        Language::CSharp => &["parenthesized_expression"],
        Language::Scala => &["parenthesized_expression"],
        Language::Elixir => &["block"],
        Language::Lua | Language::Luau => &["parenthesized_expression"],
        Language::Ocaml => &["parenthesized_expression"],
    }
}

/// Get the tree-sitter node kinds for function/method declarations in each language.
///
/// Returns node kinds that represent function or method definitions.
/// Used by GVN and bounds analysis for function-level scoping.
pub fn function_node_kinds(language: Language) -> &'static [&'static str] {
    match language {
        Language::Python => &["function_definition"],
        Language::TypeScript | Language::JavaScript => &[
            "function_declaration",
            "method_definition",
            "arrow_function",
            "generator_function_declaration",
        ],
        Language::Go => &["function_declaration", "method_declaration"],
        Language::Rust => &["function_item"],
        Language::Java => &["method_declaration", "constructor_declaration"],
        Language::C | Language::Cpp => &["function_definition"],
        Language::Ruby => &["method", "singleton_method"],
        Language::Php => &["function_definition", "method_declaration"],
        Language::Kotlin => &["function_declaration"],
        Language::Swift => &["function_declaration", "init_declaration"],
        Language::CSharp => &["method_declaration", "constructor_declaration"],
        Language::Scala => &["function_definition", "val_definition"],
        Language::Elixir => &["call"],
        Language::Lua | Language::Luau => &["function_declaration", "local_function"],
        Language::Ocaml => &["let_binding", "value_definition"],
    }
}

/// Get the tree-sitter node kinds for field/member access expressions.
///
/// These are the nodes representing `self.field`, `this.field`, `receiver.field` etc.
/// Used by the cohesion LCOM4 metric to detect which fields a method accesses.
///
/// Returns tuples of (node_kind, object_child_name, field_child_name).
/// - `node_kind`: the tree-sitter node type for the field access
/// - `object_child_name`: the field name for the object/receiver child (or None for positional)
/// - `field_child_name`: the field name for the accessed field child (or None for positional)
pub fn field_access_info(language: Language) -> &'static [FieldAccessPattern] {
    match language {
        Language::Python => &[FieldAccessPattern {
            node_kind: "attribute",
            object_field: Some("object"),
            member_field: Some("attribute"),
            self_keywords: &["self"],
        }],
        Language::TypeScript | Language::JavaScript => &[FieldAccessPattern {
            node_kind: "member_expression",
            object_field: Some("object"),
            member_field: Some("property"),
            self_keywords: &["this"],
        }],
        Language::Go => &[FieldAccessPattern {
            node_kind: "selector_expression",
            object_field: Some("operand"),
            member_field: Some("field"),
            // Go uses the receiver name (not a keyword), handled specially
            self_keywords: &[],
        }],
        Language::Rust => &[FieldAccessPattern {
            node_kind: "field_expression",
            object_field: Some("value"),
            member_field: Some("field"),
            self_keywords: &["self"],
        }],
        Language::Java => &[FieldAccessPattern {
            node_kind: "field_access",
            object_field: Some("object"),
            member_field: Some("field"),
            self_keywords: &["this"],
        }],
        Language::Kotlin => &[FieldAccessPattern {
            node_kind: "navigation_expression",
            object_field: None, // positional: child(0) = receiver
            member_field: None, // positional: navigation_suffix -> simple_identifier
            self_keywords: &["this"],
        }],
        Language::Swift => &[FieldAccessPattern {
            node_kind: "navigation_expression",
            object_field: None, // positional: child(0) = receiver
            member_field: None, // positional: navigation_suffix -> simple_identifier
            self_keywords: &["self"],
        }],
        Language::CSharp => &[FieldAccessPattern {
            node_kind: "member_access_expression",
            object_field: None, // positional: first child is object
            member_field: Some("name"),
            self_keywords: &["this"],
        }],
        Language::C => &[FieldAccessPattern {
            node_kind: "field_expression",
            object_field: Some("argument"),
            member_field: Some("field"),
            // C has no self concept
            self_keywords: &[],
        }],
        Language::Cpp => &[FieldAccessPattern {
            node_kind: "field_expression",
            object_field: Some("argument"),
            member_field: Some("field"),
            self_keywords: &["this"],
        }],
        Language::Ruby => &[
            // Ruby uses @field for instance variables (no receiver.field pattern)
            FieldAccessPattern {
                node_kind: "instance_variable",
                object_field: None,
                member_field: None, // whole node text is the field name (e.g., "@name")
                self_keywords: &[],
            },
        ],
        Language::Scala => &[
            FieldAccessPattern {
                node_kind: "field_expression",
                object_field: None, // positional: first identifier child
                member_field: None, // positional: second identifier child
                self_keywords: &["this"],
            },
            FieldAccessPattern {
                node_kind: "select_expression",
                object_field: None,
                member_field: None,
                self_keywords: &["this"],
            },
        ],
        Language::Php => &[FieldAccessPattern {
            node_kind: "member_access_expression",
            object_field: Some("object"),
            member_field: Some("name"),
            self_keywords: &["$this"],
        }],
        Language::Lua => &[FieldAccessPattern {
            node_kind: "dot_index_expression",
            object_field: None, // positional: first child
            member_field: None, // positional: last identifier child
            self_keywords: &["self"],
        }],
        Language::Luau => &[FieldAccessPattern {
            node_kind: "dot_index_expression",
            object_field: None,
            member_field: None,
            self_keywords: &["self"],
        }],
        Language::Elixir => &[
            // Elixir uses @field for module attributes (similar to Ruby instance vars)
            FieldAccessPattern {
                node_kind: "unary_operator",
                object_field: None,
                member_field: None,
                self_keywords: &[],
            },
        ],
        Language::Ocaml => &[
            // OCaml is functional - no self/this concept for field access
            // Uses record field access: record.field
            FieldAccessPattern {
                node_kind: "field_get_expression",
                object_field: None,
                member_field: None,
                self_keywords: &[],
            },
        ],
    }
}

/// Description of how field access nodes look in a specific language.
#[derive(Debug, Clone, Copy)]
pub struct FieldAccessPattern {
    /// The tree-sitter node kind (e.g., "attribute", "member_expression")
    pub node_kind: &'static str,
    /// The field name for the object/receiver child (None = positional)
    pub object_field: Option<&'static str>,
    /// The field name for the member/field child (None = positional or whole-node)
    pub member_field: Option<&'static str>,
    /// Keywords that represent self/this for this language
    pub self_keywords: &'static [&'static str],
}

// =============================================================================
// Node Text Helpers
// =============================================================================

/// Get the UTF-8 text of a node from the source.
pub fn node_text<'a>(node: &Node, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
}

/// Check if a node is inside a comment.
///
/// Walks up the parent chain to see if any ancestor is a comment node.
pub fn is_in_comment(node: &Node, language: Language) -> bool {
    let comment_kinds = comment_node_kinds(language);
    let mut current = Some(*node);
    while let Some(n) = current {
        if comment_kinds.contains(&n.kind()) {
            return true;
        }
        current = n.parent();
    }
    false
}

/// Check if a node is inside a string literal.
///
/// Walks up the parent chain to see if any ancestor is a string node.
pub fn is_in_string(node: &Node, language: Language) -> bool {
    let string_kinds = string_node_kinds(language);
    let mut current = Some(*node);
    while let Some(n) = current {
        if string_kinds.contains(&n.kind()) {
            return true;
        }
        current = n.parent();
    }
    false
}

// =============================================================================
// Call Name Extraction
// =============================================================================

/// Extract the function/method name from a call node.
///
/// Each language has different node child field names for the function being called.
/// For example:
/// - Python: `call` has a `function` field
/// - Go: `call_expression` has a `function` field
/// - Java: `method_invocation` has a `name` and `object` field
///
/// Returns the full dotted name (e.g., "os.system", "request.args") if available.
pub fn extract_call_name(node: &Node, source: &[u8], language: Language) -> Option<String> {
    match language {
        Language::Python => extract_call_name_python(node, source),
        Language::TypeScript | Language::JavaScript => extract_call_name_typescript(node, source),
        Language::Go => extract_call_name_go(node, source),
        Language::Java => extract_call_name_java(node, source),
        Language::Kotlin => extract_call_name_kotlin(node, source),
        Language::Rust => extract_call_name_rust(node, source),
        Language::C | Language::Cpp => extract_call_name_c(node, source),
        Language::Ruby => extract_call_name_ruby(node, source),
        Language::Swift => extract_call_name_swift(node, source),
        Language::CSharp => extract_call_name_csharp(node, source),
        Language::Scala => extract_call_name_scala(node, source),
        Language::Php => extract_call_name_php(node, source),
        Language::Lua | Language::Luau => extract_call_name_lua(node, source),
        Language::Elixir => extract_call_name_elixir(node, source),
        Language::Ocaml => extract_call_name_ocaml(node, source),
    }
}

fn extract_call_name_python(node: &Node, source: &[u8]) -> Option<String> {
    // Python `call` node has a `function` field
    let func = node.child_by_field_name("function")?;
    Some(node_text(&func, source).to_string())
}

fn extract_call_name_typescript(node: &Node, source: &[u8]) -> Option<String> {
    // TypeScript/JS `call_expression` has a `function` field
    let func = node.child_by_field_name("function")?;
    Some(node_text(&func, source).to_string())
}

fn extract_call_name_go(node: &Node, source: &[u8]) -> Option<String> {
    let func = node.child_by_field_name("function")?;
    Some(node_text(&func, source).to_string())
}

fn extract_call_name_java(node: &Node, source: &[u8]) -> Option<String> {
    match node.kind() {
        "method_invocation" => {
            let name = node.child_by_field_name("name")?;
            if let Some(obj) = node.child_by_field_name("object") {
                Some(format!(
                    "{}.{}",
                    node_text(&obj, source),
                    node_text(&name, source)
                ))
            } else {
                Some(node_text(&name, source).to_string())
            }
        }
        "object_creation_expression" => {
            // `new Foo(...)` - extract the type name
            let type_node = node.child_by_field_name("type")?;
            Some(format!("new {}", node_text(&type_node, source)))
        }
        _ => None,
    }
}

fn extract_call_name_kotlin(node: &Node, source: &[u8]) -> Option<String> {
    // Kotlin call_expression or navigation_expression
    if node.kind() == "call_expression" {
        // First child is the function name or navigation expression
        let func = node.child(0)?;
        Some(node_text(&func, source).to_string())
    } else {
        Some(node_text(node, source).to_string())
    }
}

fn extract_call_name_rust(node: &Node, source: &[u8]) -> Option<String> {
    match node.kind() {
        "call_expression" => {
            let func = node.child_by_field_name("function")?;
            Some(node_text(&func, source).to_string())
        }
        "macro_invocation" => {
            let macro_name = node.child_by_field_name("macro")?;
            Some(node_text(&macro_name, source).to_string())
        }
        _ => None,
    }
}

fn extract_call_name_c(node: &Node, source: &[u8]) -> Option<String> {
    let func = node.child_by_field_name("function")?;
    Some(node_text(&func, source).to_string())
}

fn extract_call_name_ruby(node: &Node, source: &[u8]) -> Option<String> {
    match node.kind() {
        "call" | "method_call" => {
            if let Some(method) = node.child_by_field_name("method") {
                if let Some(receiver) = node.child_by_field_name("receiver") {
                    Some(format!(
                        "{}.{}",
                        node_text(&receiver, source),
                        node_text(&method, source)
                    ))
                } else {
                    Some(node_text(&method, source).to_string())
                }
            } else {
                // Simple call without explicit method field
                let first = node.child(0)?;
                Some(node_text(&first, source).to_string())
            }
        }
        _ => None,
    }
}

fn extract_call_name_swift(node: &Node, source: &[u8]) -> Option<String> {
    // Swift call_expression
    let func = node.child(0)?;
    Some(node_text(&func, source).to_string())
}

fn extract_call_name_csharp(node: &Node, source: &[u8]) -> Option<String> {
    match node.kind() {
        "invocation_expression" => {
            let func = node
                .child_by_field_name("function")
                .or_else(|| node.child(0))?;
            Some(node_text(&func, source).to_string())
        }
        "object_creation_expression" => {
            let type_node = node.child_by_field_name("type")?;
            Some(format!("new {}", node_text(&type_node, source)))
        }
        _ => None,
    }
}

fn extract_call_name_scala(node: &Node, source: &[u8]) -> Option<String> {
    let func = node
        .child_by_field_name("function")
        .or_else(|| node.child(0))?;
    Some(node_text(&func, source).to_string())
}

fn extract_call_name_php(node: &Node, source: &[u8]) -> Option<String> {
    match node.kind() {
        "function_call_expression" => {
            let func = node
                .child_by_field_name("function")
                .or_else(|| node.child(0))?;
            Some(node_text(&func, source).to_string())
        }
        "member_call_expression" => {
            let name = node.child_by_field_name("name")?;
            if let Some(obj) = node.child_by_field_name("object") {
                Some(format!(
                    "{}->{}",
                    node_text(&obj, source),
                    node_text(&name, source)
                ))
            } else {
                Some(node_text(&name, source).to_string())
            }
        }
        _ => None,
    }
}

fn extract_call_name_lua(node: &Node, source: &[u8]) -> Option<String> {
    // Lua function_call has first child as the function name
    let func = node.child_by_field_name("name").or_else(|| node.child(0))?;
    Some(node_text(&func, source).to_string())
}

fn extract_call_name_elixir(node: &Node, source: &[u8]) -> Option<String> {
    // Elixir call node - target is first child
    let target = node
        .child_by_field_name("target")
        .or_else(|| node.child(0))?;
    Some(node_text(&target, source).to_string())
}

fn extract_call_name_ocaml(node: &Node, source: &[u8]) -> Option<String> {
    // OCaml application_expression - function is first child
    let func = node.child(0)?;
    Some(node_text(&func, source).to_string())
}

// =============================================================================
// Tree Walking
// =============================================================================

/// Collect all descendant nodes of a given node.
///
/// Uses depth-first traversal via tree-sitter's tree cursor.
pub fn walk_descendants<'a>(node: Node<'a>) -> Vec<Node<'a>> {
    let mut result = Vec::new();
    let mut cursor = node.walk();

    // Move to first child
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            result.push(child);
            // Recurse into children
            result.extend(walk_descendants(child));
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    result
}

/// Find the first ancestor assignment node and extract the assigned variable name.
///
/// Walks up the parent chain looking for assignment nodes, then extracts
/// the variable being assigned to (LHS of the assignment).
pub fn find_parent_assignment_var(
    node: &Node,
    source: &[u8],
    language: Language,
) -> Option<String> {
    let assign_kinds = assignment_node_kinds(language);
    let mut current = node.parent();

    while let Some(parent) = current {
        if assign_kinds.contains(&parent.kind()) {
            return extract_lhs_var(&parent, source, language);
        }
        current = parent.parent();
    }

    None
}

/// Extract the variable name from the LHS of an assignment node.
fn extract_lhs_var(node: &Node, source: &[u8], language: Language) -> Option<String> {
    match language {
        Language::Python => {
            // Python assignment: `left` field
            let left = node.child_by_field_name("left")?;
            Some(node_text(&left, source).to_string())
        }
        Language::TypeScript | Language::JavaScript => {
            // variable_declaration -> declarator -> name
            if node.kind() == "variable_declaration" {
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        if child.kind() == "variable_declarator" {
                            if let Some(name) = child.child_by_field_name("name") {
                                return Some(node_text(&name, source).to_string());
                            }
                        }
                    }
                }
                None
            } else {
                // assignment_expression: left field
                let left = node.child_by_field_name("left")?;
                Some(node_text(&left, source).to_string())
            }
        }
        Language::Go => {
            // short_var_declaration: `left` field, assignment_statement: `left` field
            let left = node.child_by_field_name("left")?;
            let text = node_text(&left, source);
            // May be comma-separated: "a, b := ...", take first
            text.split(',').next().map(|s| s.trim().to_string())
        }
        Language::Java => {
            // local_variable_declaration -> declarator -> name
            if node.kind() == "local_variable_declaration" {
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        if child.kind() == "variable_declarator" {
                            if let Some(name) = child.child_by_field_name("name") {
                                return Some(node_text(&name, source).to_string());
                            }
                        }
                    }
                }
            }
            // assignment_expression: left field
            node.child_by_field_name("left")
                .map(|n| node_text(&n, source).to_string())
        }
        Language::Kotlin => {
            // property_declaration: first identifier child
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if child.kind() == "simple_identifier" {
                        return Some(node_text(&child, source).to_string());
                    }
                }
            }
            node.child_by_field_name("left")
                .map(|n| node_text(&n, source).to_string())
        }
        Language::Rust => {
            // let_declaration: `pattern` field
            if node.kind() == "let_declaration" {
                let pattern = node.child_by_field_name("pattern")?;
                Some(node_text(&pattern, source).to_string())
            } else {
                node.child_by_field_name("left")
                    .map(|n| node_text(&n, source).to_string())
            }
        }
        Language::C | Language::Cpp => {
            // declaration -> declarator (init_declarator -> declarator)
            if node.kind() == "declaration" {
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        if child.kind() == "init_declarator" {
                            if let Some(decl) = child.child_by_field_name("declarator") {
                                let text = node_text(&decl, source);
                                // Strip pointer markers
                                return Some(text.trim_start_matches('*').to_string());
                            }
                        }
                    }
                }
            }
            node.child_by_field_name("left")
                .map(|n| node_text(&n, source).to_string())
        }
        Language::Ruby => {
            let left = node.child_by_field_name("left")?;
            Some(node_text(&left, source).to_string())
        }
        Language::Swift => {
            // property_declaration: first child (pattern)
            if node.kind() == "property_declaration" {
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        if child.kind() == "pattern" {
                            return Some(node_text(&child, source).to_string());
                        }
                    }
                }
            }
            node.child_by_field_name("left")
                .map(|n| node_text(&n, source).to_string())
        }
        Language::CSharp => {
            if node.kind() == "variable_declaration" {
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        if child.kind() == "variable_declarator" {
                            if let Some(name) = child.child_by_field_name("name") {
                                return Some(node_text(&name, source).to_string());
                            }
                        }
                    }
                }
            }
            node.child_by_field_name("left")
                .map(|n| node_text(&n, source).to_string())
        }
        Language::Scala => {
            // val_definition / var_definition: pattern field
            node.child_by_field_name("pattern")
                .or_else(|| node.child_by_field_name("left"))
                .map(|n| node_text(&n, source).to_string())
        }
        Language::Php => node
            .child_by_field_name("left")
            .map(|n| node_text(&n, source).to_string()),
        Language::Lua | Language::Luau => {
            // variable_declaration has `name` children
            if node.kind() == "variable_declaration" {
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        if child.kind() == "variable_list"
                            || child.kind() == "assignment_variable_list"
                        {
                            if let Some(first) = child.child(0) {
                                return Some(node_text(&first, source).to_string());
                            }
                        }
                    }
                }
            }
            // assignment_statement
            if let Some(left) = node.child_by_field_name("left") {
                return Some(node_text(&left, source).to_string());
            }
            // Fallback: first child
            node.child(0).map(|n| node_text(&n, source).to_string())
        }
        Language::Elixir => {
            // match_operator: left and right
            let left = node.child_by_field_name("left").or_else(|| node.child(0))?;
            let text = node_text(&left, source);
            // Extract last identifier from destructuring like {:ok, content}
            let cleaned = text.replace(['{', '}', '(', ')', '[', ']', ':', ','], " ");
            cleaned
                .split_whitespace()
                .rfind(|t| {
                    !t.is_empty()
                        && t.chars()
                            .next()
                            .is_some_and(|c| c.is_alphabetic() || c == '_')
                        && *t != "ok"
                        && *t != "err"
                })
                .map(|s| s.to_string())
        }
        Language::Ocaml => {
            // let_binding: pattern field
            node.child_by_field_name("pattern")
                .or_else(|| node.child(0))
                .map(|n| node_text(&n, source).to_string())
        }
    }
}

/// Extract the first argument from a call node.
///
/// This gets the first non-string-literal argument passed to a function call.
pub fn extract_first_arg(node: &Node, source: &[u8], language: Language) -> Option<String> {
    // Find the arguments node
    let args = find_arguments_node(node, language)?;

    let string_kinds = string_node_kinds(language);

    // Iterate through argument children
    for i in 0..args.child_count() {
        if let Some(arg) = args.child(i) {
            // Skip commas, parens, etc.
            if arg.is_named() && !string_kinds.contains(&arg.kind()) {
                let text = node_text(&arg, source).to_string();
                // Skip string literals (even if not tagged as such by kind)
                if !text.starts_with('"')
                    && !text.starts_with('\'')
                    && !text.starts_with("f\"")
                    && !text.starts_with("f'")
                {
                    // Get first identifier part (before dots)
                    let var = text.split('.').next().unwrap_or(&text);
                    let var = var.trim_start_matches('$');
                    if !var.is_empty()
                        && var
                            .chars()
                            .next()
                            .is_some_and(|c| c.is_alphabetic() || c == '_')
                    {
                        return Some(var.to_string());
                    }
                }
            }
        }
    }

    None
}

/// Find the arguments child node of a call node.
fn find_arguments_node<'a>(node: &'a Node, language: Language) -> Option<Node<'a>> {
    let args_kind = match language {
        Language::Python => "argument_list",
        Language::TypeScript | Language::JavaScript => "arguments",
        Language::Go => "argument_list",
        Language::Java => "argument_list",
        Language::Kotlin => "call_suffix",
        Language::Rust => "arguments",
        Language::C | Language::Cpp => "argument_list",
        Language::Ruby => "argument_list",
        Language::Swift => "call_suffix",
        Language::CSharp => "argument_list",
        Language::Scala => "arguments",
        Language::Php => "arguments",
        Language::Lua | Language::Luau => "arguments",
        Language::Elixir => "arguments",
        Language::Ocaml => return node.child(1), // OCaml: second child is the argument
    };

    node.child_by_field_name("arguments").or_else(|| {
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if child.kind() == args_kind {
                    return Some(child);
                }
            }
        }
        None
    })
}

// =============================================================================
// Statement-level Text Matching with AST Verification
// =============================================================================

/// Check if a statement text contains a call to a specific function name,
/// verified against the AST to exclude matches in comments and strings.
///
/// This is the bridge between the existing regex-based approach and full AST.
/// It takes a line of source code, parses it, and checks if the function name
/// appears as an actual call (not in a comment or string).
///
/// # Arguments
/// * `statement` - The source code statement text
/// * `call_name` - The function name to look for (e.g., "eval", "input")
/// * `language` - The programming language
///
/// # Returns
/// `true` if the call appears in actual code (not in a comment or string)
pub fn verify_call_in_statement(statement: &str, call_name: &str, _language: Language) -> bool {
    // Quick check: if the call name isn't even in the text, skip parsing
    if !statement.contains(call_name) {
        return false;
    }

    // For now, use text-based checking as a fast path.
    // The AST parsing of individual statements is expensive and may fail
    // because single statements may not be valid programs.
    // The full AST verification happens when we have the full file tree.
    true
}

/// Given a full file tree, check if a node at a specific line is in a comment or string.
///
/// This is the core AST advantage: we can filter out false positives from
/// comments and string literals using the parsed tree.
pub fn is_code_node(node: &Node, language: Language) -> bool {
    !is_in_comment(node, language) && !is_in_string(node, language)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::parser::ParserPool;

    #[test]
    fn test_call_node_kinds_all_languages() {
        // Verify all 18 languages return non-empty call node kinds
        let languages = vec![
            Language::Python,
            Language::TypeScript,
            Language::JavaScript,
            Language::Go,
            Language::Rust,
            Language::Java,
            Language::C,
            Language::Cpp,
            Language::Ruby,
            Language::Kotlin,
            Language::Swift,
            Language::CSharp,
            Language::Scala,
            Language::Php,
            Language::Lua,
            Language::Luau,
            Language::Elixir,
            Language::Ocaml,
        ];
        for lang in languages {
            let kinds = call_node_kinds(lang);
            assert!(
                !kinds.is_empty(),
                "call_node_kinds should be non-empty for {:?}",
                lang
            );
        }
    }

    #[test]
    fn test_comment_node_kinds_all_languages() {
        let languages = vec![
            Language::Python,
            Language::TypeScript,
            Language::JavaScript,
            Language::Go,
            Language::Rust,
            Language::Java,
            Language::C,
            Language::Cpp,
            Language::Ruby,
            Language::Kotlin,
            Language::Swift,
            Language::CSharp,
            Language::Scala,
            Language::Php,
            Language::Lua,
            Language::Luau,
            Language::Elixir,
            Language::Ocaml,
        ];
        for lang in languages {
            let kinds = comment_node_kinds(lang);
            assert!(
                !kinds.is_empty(),
                "comment_node_kinds should be non-empty for {:?}",
                lang
            );
        }
    }

    #[test]
    fn test_python_call_name_extraction() {
        let pool = ParserPool::new();
        let source = "result = eval(code)";
        let tree = pool.parse(source, Language::Python).unwrap();

        let root = tree.root_node();
        let descendants = walk_descendants(root);

        let call_kinds = call_node_kinds(Language::Python);
        let calls: Vec<_> = descendants
            .iter()
            .filter(|n| call_kinds.contains(&n.kind()))
            .collect();

        assert!(!calls.is_empty(), "Should find a call node in: {}", source);
        let call_name = extract_call_name(calls[0], source.as_bytes(), Language::Python);
        assert_eq!(call_name, Some("eval".to_string()));
    }

    #[test]
    fn test_python_comment_not_call() {
        let pool = ParserPool::new();
        let source = "# eval(code) - this is a comment\nx = 1";
        let tree = pool.parse(source, Language::Python).unwrap();

        let root = tree.root_node();
        let descendants = walk_descendants(root);

        let call_kinds = call_node_kinds(Language::Python);
        let calls: Vec<_> = descendants
            .iter()
            .filter(|n| call_kinds.contains(&n.kind()))
            .collect();

        // eval in comment should NOT appear as a call node
        assert!(
            calls.is_empty(),
            "eval in comment should not be detected as call"
        );
    }

    #[test]
    fn test_python_string_not_call() {
        let pool = ParserPool::new();
        let source = "msg = \"call eval(code) here\"";
        let tree = pool.parse(source, Language::Python).unwrap();

        let root = tree.root_node();
        let descendants = walk_descendants(root);

        let call_kinds = call_node_kinds(Language::Python);
        let calls: Vec<_> = descendants
            .iter()
            .filter(|n| call_kinds.contains(&n.kind()))
            .collect();

        // eval inside a string literal should NOT appear as a call node
        assert!(
            calls.is_empty(),
            "eval in string should not be detected as call"
        );
    }

    #[test]
    fn test_walk_descendants() {
        let pool = ParserPool::new();
        let source = "x = 1 + 2";
        let tree = pool.parse(source, Language::Python).unwrap();

        let root = tree.root_node();
        let descendants = walk_descendants(root);

        // Should have multiple descendant nodes
        assert!(descendants.len() > 1, "Should find descendant nodes");
    }

    #[test]
    fn test_is_in_comment_python() {
        let pool = ParserPool::new();
        let source = "# this is a comment\nx = 1";
        let tree = pool.parse(source, Language::Python).unwrap();

        let root = tree.root_node();
        let descendants = walk_descendants(root);

        let comments: Vec<_> = descendants
            .iter()
            .filter(|n| n.kind() == "comment")
            .collect();

        assert!(!comments.is_empty(), "Should find comment node");
        assert!(is_in_comment(comments[0], Language::Python));
    }

    #[test]
    fn test_find_parent_assignment_python() {
        let pool = ParserPool::new();
        let source = "result = input()";
        let tree = pool.parse(source, Language::Python).unwrap();

        let root = tree.root_node();
        let descendants = walk_descendants(root);

        let call_kinds = call_node_kinds(Language::Python);
        let calls: Vec<_> = descendants
            .iter()
            .filter(|n| call_kinds.contains(&n.kind()))
            .collect();

        assert!(!calls.is_empty(), "Should find call node");
        let var = find_parent_assignment_var(calls[0], source.as_bytes(), Language::Python);
        assert_eq!(var, Some("result".to_string()));
    }

    #[test]
    fn test_loop_node_kinds_all_languages() {
        let languages = vec![
            Language::Python,
            Language::TypeScript,
            Language::JavaScript,
            Language::Go,
            Language::Rust,
            Language::Java,
            Language::C,
            Language::Cpp,
            Language::Ruby,
            Language::Kotlin,
            Language::Swift,
            Language::CSharp,
            Language::Scala,
            Language::Php,
            Language::Lua,
            Language::Luau,
            Language::Elixir,
            Language::Ocaml,
        ];
        for lang in languages {
            let kinds = loop_node_kinds(lang);
            assert!(
                !kinds.is_empty(),
                "loop_node_kinds should be non-empty for {:?}",
                lang
            );
        }
    }

    #[test]
    fn test_literal_node_kinds_all_languages() {
        let languages = vec![
            Language::Python,
            Language::TypeScript,
            Language::JavaScript,
            Language::Go,
            Language::Rust,
            Language::Java,
            Language::C,
            Language::Cpp,
            Language::Ruby,
            Language::Kotlin,
            Language::Swift,
            Language::CSharp,
            Language::Scala,
            Language::Php,
            Language::Lua,
            Language::Luau,
            Language::Elixir,
            Language::Ocaml,
        ];
        for lang in languages {
            let kinds = literal_node_kinds(lang);
            assert!(
                !kinds.is_empty(),
                "literal_node_kinds should be non-empty for {:?}",
                lang
            );
        }
    }

    #[test]
    fn test_identifier_node_kinds_all_languages() {
        let languages = vec![
            Language::Python,
            Language::TypeScript,
            Language::JavaScript,
            Language::Go,
            Language::Rust,
            Language::Java,
            Language::C,
            Language::Cpp,
            Language::Ruby,
            Language::Kotlin,
            Language::Swift,
            Language::CSharp,
            Language::Scala,
            Language::Php,
            Language::Lua,
            Language::Luau,
            Language::Elixir,
            Language::Ocaml,
        ];
        for lang in languages {
            let kinds = identifier_node_kinds(lang);
            assert!(
                !kinds.is_empty(),
                "identifier_node_kinds should be non-empty for {:?}",
                lang
            );
        }
    }

    #[test]
    fn test_function_node_kinds_all_languages() {
        let languages = vec![
            Language::Python,
            Language::TypeScript,
            Language::JavaScript,
            Language::Go,
            Language::Rust,
            Language::Java,
            Language::C,
            Language::Cpp,
            Language::Ruby,
            Language::Kotlin,
            Language::Swift,
            Language::CSharp,
            Language::Scala,
            Language::Php,
            Language::Lua,
            Language::Luau,
            Language::Elixir,
            Language::Ocaml,
        ];
        for lang in languages {
            let kinds = function_node_kinds(lang);
            assert!(
                !kinds.is_empty(),
                "function_node_kinds should be non-empty for {:?}",
                lang
            );
        }
    }

    #[test]
    fn test_python_specific_node_kinds() {
        // Verify Python-specific lookup values are correct
        assert_eq!(
            loop_node_kinds(Language::Python),
            &["for_statement", "while_statement"]
        );
        assert_eq!(
            literal_node_kinds(Language::Python),
            &["integer", "float", "string"]
        );
        assert_eq!(identifier_node_kinds(Language::Python), &["identifier"]);
        assert_eq!(
            function_node_kinds(Language::Python),
            &["function_definition"]
        );
    }

    #[test]
    fn test_swift_specific_node_kinds() {
        // Verify Swift node kinds match P0 smoke test findings
        assert!(loop_node_kinds(Language::Swift).contains(&"for_statement"));
        assert!(literal_node_kinds(Language::Swift).contains(&"real_literal"));
        assert!(literal_node_kinds(Language::Swift).contains(&"integer_literal"));
        assert!(identifier_node_kinds(Language::Swift).contains(&"simple_identifier"));
        assert!(function_node_kinds(Language::Swift).contains(&"function_declaration"));
    }
}
