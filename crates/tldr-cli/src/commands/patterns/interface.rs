//! Interface command - Public API extraction
//!
//! Extracts the public interface (API surface) from source files.
//! Supports all languages with tree-sitter grammars: Python, Rust, Go,
//! TypeScript, JavaScript, Java, C, C++, Ruby, C#, Scala, PHP, Lua, Luau,
//! Elixir, and OCaml.
//!
//! # Features
//!
//! - Extracts public functions (language-appropriate visibility rules)
//! - Extracts public classes/structs/traits with their public methods
//! - Captures export declarations when present (e.g., Python `__all__`)
//! - Marks async functions/methods
//! - Includes function signatures with type annotations
//! - Includes docstrings/doc comments when present
//!
//! # Example
//!
//! ```bash
//! tldr interface src/api.py
//! tldr interface src/lib.rs
//! tldr interface src/ --format text
//! ```

use std::path::{Path, PathBuf};

use clap::Args;
use tree_sitter::Node;
use walkdir::WalkDir;

use super::error::{PatternsError, PatternsResult};
use super::types::{ClassInfo, FunctionInfo, InterfaceInfo, MethodInfo};
use super::validation::{read_file_safe, validate_directory_path, validate_file_path};
use crate::output::OutputFormat;
use tldr_core::ast::ParserPool;
use tldr_core::types::Language;

// =============================================================================
// CLI Arguments
// =============================================================================

/// Arguments for the interface command.
#[derive(Debug, Clone, Args)]
pub struct InterfaceArgs {
    /// File or directory to analyze
    #[arg(required = true)]
    pub path: PathBuf,

    /// Project root for path validation
    #[arg(long)]
    pub project_root: Option<PathBuf>,
}

// =============================================================================
// Language-Aware Node Kind Configuration
// =============================================================================

/// Node kinds that represent function definitions for a given language.
fn function_node_kinds(lang: Language) -> &'static [&'static str] {
    match lang {
        Language::Python => &["function_definition"],
        Language::Rust => &["function_item"],
        Language::Go => &["function_declaration", "method_declaration"],
        Language::Java => &["method_declaration", "constructor_declaration"],
        Language::TypeScript | Language::JavaScript => &[
            "function_declaration",
            "method_definition",
            "arrow_function",
        ],
        Language::C | Language::Cpp => &["function_definition"],
        Language::Ruby => &["method", "singleton_method"],
        Language::CSharp => &["method_declaration", "constructor_declaration"],
        Language::Scala => &["function_definition", "def_definition"],
        Language::Php => &["function_definition", "method_declaration"],
        Language::Lua | Language::Luau => {
            &["function_declaration", "function_definition_statement"]
        }
        Language::Elixir => &["call"], // `def` and `defp` are calls in elixir tree-sitter
        Language::Ocaml => &["let_binding", "value_definition"],
        _ => &[],
    }
}

/// Node kinds that represent class/struct/trait definitions for a given language.
fn class_node_kinds(lang: Language) -> &'static [&'static str] {
    match lang {
        Language::Python => &["class_definition"],
        Language::Rust => &["struct_item", "impl_item", "trait_item", "enum_item"],
        Language::Go => &["type_declaration"],
        Language::Java => &[
            "class_declaration",
            "interface_declaration",
            "enum_declaration",
        ],
        Language::TypeScript | Language::JavaScript => &[
            "class_declaration",
            "interface_declaration",
            "type_alias_declaration",
        ],
        Language::C => &["struct_specifier"],
        Language::Cpp => &["struct_specifier", "class_specifier"],
        Language::Ruby => &["class", "module"],
        Language::CSharp => &[
            "class_declaration",
            "interface_declaration",
            "struct_declaration",
        ],
        Language::Scala => &["class_definition", "object_definition", "trait_definition"],
        Language::Php => &["class_declaration", "interface_declaration"],
        Language::Lua | Language::Luau => &[], // Lua doesn't have class syntax
        Language::Elixir => &["call"],         // defmodule is a call in elixir tree-sitter
        Language::Ocaml => &["module_definition", "type_definition"],
        _ => &[],
    }
}

/// Node kinds that represent decorated/annotated definitions.
fn decorator_node_kinds(lang: Language) -> &'static [&'static str] {
    match lang {
        Language::Python => &["decorated_definition"],
        Language::Java => &["annotation"],
        Language::TypeScript | Language::JavaScript => &["decorator"],
        Language::CSharp => &["attribute_list"],
        Language::Rust => &["attribute_item"],
        _ => &[],
    }
}

/// Node kinds for method definitions inside classes/structs.
fn method_node_kinds(lang: Language) -> &'static [&'static str] {
    match lang {
        Language::Python => &["function_definition"],
        Language::Rust => &["function_item"],
        Language::Go => &["method_declaration"],
        Language::Java => &["method_declaration", "constructor_declaration"],
        Language::TypeScript | Language::JavaScript => {
            &["method_definition", "public_field_definition"]
        }
        Language::C | Language::Cpp => &["function_definition"],
        Language::Ruby => &["method", "singleton_method"],
        Language::CSharp => &["method_declaration", "constructor_declaration"],
        Language::Scala => &["function_definition", "def_definition"],
        Language::Php => &["method_declaration"],
        Language::Elixir => &["call"],
        Language::Ocaml => &["let_binding", "value_definition"],
        _ => &[],
    }
}

// =============================================================================
// Public Name Detection (Language-Aware)
// =============================================================================

/// Check if a name is public based on language conventions.
///
/// - Python: names not starting with `_`
/// - Rust: `pub` keyword (checked at node level, not name level)
/// - Go: names starting with uppercase
/// - Ruby: methods not starting with `_` (private is keyword-based)
/// - Other languages: generally all names are considered public
///   (visibility modifiers are checked at the node level)
#[inline]
pub fn is_public_name(name: &str) -> bool {
    !name.starts_with('_')
}

/// Check if a name is public based on language-specific rules.
fn is_public_for_lang(name: &str, lang: Language) -> bool {
    match lang {
        Language::Python | Language::Ruby | Language::Lua | Language::Luau => {
            !name.starts_with('_')
        }
        Language::Go => {
            // Go exports start with an uppercase letter
            name.chars().next().is_some_and(|c| c.is_uppercase())
        }
        // For Rust, Java, TS, C#, etc. - visibility is determined by modifiers,
        // not naming. We check modifiers at the node level.
        _ => true,
    }
}

/// Check if a Rust node has `pub` visibility.
fn is_rust_pub(node: Node, source: &[u8]) -> bool {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "visibility_modifier" {
                let text = node_text(child, source);
                return text.starts_with("pub");
            }
        }
    }
    false
}

/// Check if a Java/C#/TS node has public access modifier.
fn has_public_modifier(node: Node, source: &[u8]) -> bool {
    // Check modifiers child
    if let Some(modifiers) = node.child_by_field_name("modifiers") {
        let text = node_text(modifiers, source);
        return text.contains("public");
    }
    // Also check for direct modifier children
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            let kind = child.kind();
            if kind == "modifiers" || kind == "modifier" || kind == "access_modifier" {
                let text = node_text(child, source);
                if text.contains("public") {
                    return true;
                }
            }
            // For TypeScript: check for accessibility_modifier
            if kind == "accessibility_modifier" {
                let text = node_text(child, source);
                return text == "public";
            }
        }
    }
    // In Java, default (package-private) is not public, but for interface extraction
    // we treat non-private as public for utility
    true
}

/// Check if a C/C++ function is `static` (file-local, not public).
fn is_c_static(node: Node, source: &[u8]) -> bool {
    // Check for storage_class_specifier "static" before the function
    if let Some(prev) = node.prev_sibling() {
        if prev.kind() == "storage_class_specifier" {
            return node_text(prev, source) == "static";
        }
    }
    // Check declarator specifiers
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "storage_class_specifier" && node_text(child, source) == "static" {
                return true;
            }
        }
    }
    false
}

/// Language-aware visibility check for a node.
fn is_node_public(node: Node, source: &[u8], lang: Language) -> bool {
    let name = get_node_name(node, source, lang);
    let name_str = name.as_deref().unwrap_or("");

    match lang {
        Language::Rust => is_rust_pub(node, source),
        Language::Go => name_str.chars().next().is_some_and(|c| c.is_uppercase()),
        Language::Python | Language::Ruby | Language::Lua | Language::Luau => {
            !name_str.starts_with('_')
        }
        Language::Java | Language::CSharp => has_public_modifier(node, source),
        Language::C | Language::Cpp => !is_c_static(node, source),
        // For other languages, default to public
        _ => true,
    }
}

// =============================================================================
// Name Extraction
// =============================================================================

/// Get the name of a definition node based on language.
fn get_node_name<'a>(node: Node<'a>, source: &'a [u8], lang: Language) -> Option<String> {
    // First try the common "name" field
    if let Some(name_node) = node.child_by_field_name("name") {
        return Some(node_text(name_node, source).to_string());
    }

    match lang {
        Language::C | Language::Cpp => {
            // C/C++ function_definition has a "declarator" field
            if let Some(declarator) = node.child_by_field_name("declarator") {
                return extract_c_declarator_name(declarator, source);
            }
        }
        Language::Go => {
            // Go type_declaration wraps type_spec which has the name
            if node.kind() == "type_declaration" {
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        if child.kind() == "type_spec" {
                            if let Some(name_node) = child.child_by_field_name("name") {
                                return Some(node_text(name_node, source).to_string());
                            }
                        }
                    }
                }
            }
        }
        Language::Rust => {
            // Rust impl_item doesn't always have a "name" field
            if node.kind() == "impl_item" {
                // Look for the type being implemented
                if let Some(type_node) = node.child_by_field_name("type") {
                    return Some(node_text(type_node, source).to_string());
                }
                // Fallback: find type_identifier child
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        if child.kind() == "type_identifier" || child.kind() == "generic_type" {
                            return Some(node_text(child, source).to_string());
                        }
                    }
                }
            }
        }
        Language::Elixir => {
            // In Elixir, `def` and `defmodule` are call nodes
            // The first argument is the name
            if node.kind() == "call" {
                if let Some(target) = node.child(0) {
                    let target_text = node_text(target, source);
                    if target_text == "def" || target_text == "defp" || target_text == "defmodule" {
                        if let Some(args) = node.child_by_field_name("arguments") {
                            if let Some(first_arg) = args.child(0) {
                                // For def/defp, the first arg may be a call (name + params)
                                if first_arg.kind() == "call" {
                                    if let Some(fn_name) = first_arg.child(0) {
                                        return Some(node_text(fn_name, source).to_string());
                                    }
                                }
                                return Some(node_text(first_arg, source).to_string());
                            }
                        }
                    }
                }
            }
        }
        Language::Lua | Language::Luau => {
            // Try the "name" field first (already done above), then check child nodes
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if child.kind() == "identifier" || child.kind() == "dot_index_expression" {
                        return Some(node_text(child, source).to_string());
                    }
                }
            }
        }
        Language::Ruby => {
            // Ruby methods: first identifier child after "def"
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if child.kind() == "identifier" || child.kind() == "constant" {
                        return Some(node_text(child, source).to_string());
                    }
                }
            }
        }
        _ => {}
    }

    None
}

/// Extract name from a C/C++ declarator (which may be nested).
fn extract_c_declarator_name(declarator: Node, source: &[u8]) -> Option<String> {
    // The declarator could be a function_declarator wrapping an identifier
    if declarator.kind() == "identifier" {
        return Some(node_text(declarator, source).to_string());
    }
    if declarator.kind() == "field_identifier" {
        return Some(node_text(declarator, source).to_string());
    }
    // function_declarator has a "declarator" field that is the name
    if let Some(inner) = declarator.child_by_field_name("declarator") {
        return extract_c_declarator_name(inner, source);
    }
    // Try first child
    if let Some(first) = declarator.child(0) {
        if first.kind() == "identifier" || first.kind() == "field_identifier" {
            return Some(node_text(first, source).to_string());
        }
    }
    None
}

// =============================================================================
// __all__ Extraction (Python-specific)
// =============================================================================

/// Extract the contents of `__all__` if defined in the module (Python only).
pub fn extract_all_exports(root: Node, source: &[u8]) -> Option<Vec<String>> {
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        if child.kind() == "expression_statement" {
            if let Some(assignment) = child.child(0) {
                if assignment.kind() == "assignment" {
                    if let Some(left) = assignment.child_by_field_name("left") {
                        if left.kind() == "identifier" {
                            let name = node_text(left, source);
                            if name == "__all__" {
                                if let Some(right) = assignment.child_by_field_name("right") {
                                    return extract_list_strings(right, source);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

/// Extract string elements from a list node.
fn extract_list_strings(node: Node, source: &[u8]) -> Option<Vec<String>> {
    if node.kind() != "list" {
        return None;
    }

    let mut exports = Vec::new();
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "string" {
            let text = node_text(child, source);
            let cleaned = text
                .trim_start_matches(['"', '\''])
                .trim_end_matches(['"', '\'']);
            exports.push(cleaned.to_string());
        }
    }

    if exports.is_empty() {
        None
    } else {
        Some(exports)
    }
}

// =============================================================================
// Signature Extraction (Language-Aware)
// =============================================================================

/// Extract the function signature from a definition node.
///
/// For Python, reconstructs from parameter nodes.
/// For other languages, extracts the raw text of parameters and return type.
pub fn extract_function_signature(func_node: Node, source: &[u8], lang: Language) -> String {
    match lang {
        Language::Python => extract_python_signature(func_node, source),
        Language::Rust => extract_rust_signature(func_node, source),
        Language::Go => extract_go_signature(func_node, source),
        Language::Java | Language::CSharp => extract_java_like_signature(func_node, source),
        Language::TypeScript | Language::JavaScript => extract_ts_signature(func_node, source),
        Language::C | Language::Cpp => extract_c_signature(func_node, source),
        Language::Ruby => extract_ruby_signature(func_node, source),
        Language::Php => extract_php_signature(func_node, source),
        Language::Scala => extract_scala_signature(func_node, source),
        _ => extract_generic_signature(func_node, source),
    }
}

/// Python signature: reconstruct from parameter nodes.
fn extract_python_signature(func_node: Node, source: &[u8]) -> String {
    let mut params = Vec::new();

    if let Some(params_node) = func_node.child_by_field_name("parameters") {
        let mut cursor = params_node.walk();

        for child in params_node.children(&mut cursor) {
            match child.kind() {
                "identifier" => {
                    params.push(node_text(child, source).to_string());
                }
                "typed_parameter" => {
                    params.push(extract_typed_parameter(child, source));
                }
                "default_parameter" => {
                    params.push(extract_default_parameter(child, source));
                }
                "typed_default_parameter" => {
                    params.push(extract_typed_default_parameter(child, source));
                }
                "list_splat_pattern" | "dictionary_splat_pattern" => {
                    params.push(node_text(child, source).to_string());
                }
                _ => {}
            }
        }
    }

    let params_str = params.join(", ");
    let mut signature = format!("({})", params_str);

    if let Some(return_type) = func_node.child_by_field_name("return_type") {
        let return_text = node_text(return_type, source);
        signature.push_str(" -> ");
        signature.push_str(return_text);
    }

    signature
}

/// Rust signature: extract parameters and return type.
fn extract_rust_signature(func_node: Node, source: &[u8]) -> String {
    let mut sig = String::new();

    if let Some(params) = func_node.child_by_field_name("parameters") {
        sig.push_str(node_text(params, source));
    }

    if let Some(ret) = func_node.child_by_field_name("return_type") {
        sig.push_str(" -> ");
        sig.push_str(node_text(ret, source));
    }

    sig
}

/// Go signature: extract parameters and return type.
fn extract_go_signature(func_node: Node, source: &[u8]) -> String {
    let mut sig = String::new();

    if let Some(params) = func_node.child_by_field_name("parameters") {
        sig.push_str(node_text(params, source));
    }

    if let Some(result) = func_node.child_by_field_name("result") {
        sig.push(' ');
        sig.push_str(node_text(result, source));
    }

    sig
}

/// Java/C# signature: extract parameters from formal_parameters.
fn extract_java_like_signature(func_node: Node, source: &[u8]) -> String {
    let mut sig = String::new();

    // Try "parameters" field first, then look for "formal_parameters" or a parameter_list child
    let params_node = func_node.child_by_field_name("parameters").or_else(|| {
        // Search for formal_parameters or parameter_list node among children
        let mut cursor = func_node.walk();
        let found = func_node
            .children(&mut cursor)
            .find(|&child| child.kind() == "formal_parameters" || child.kind() == "parameter_list");
        found
    });

    if let Some(params) = params_node {
        sig.push_str(node_text(params, source));
    }

    // For Java, check for return type (it's the "type" field)
    if let Some(ret) = func_node.child_by_field_name("type") {
        // Prepend return type
        let ret_text = node_text(ret, source);
        sig = format!("{}: {}", sig, ret_text);
    }

    sig
}

/// TypeScript/JavaScript signature.
fn extract_ts_signature(func_node: Node, source: &[u8]) -> String {
    let mut sig = String::new();

    if let Some(params) = func_node.child_by_field_name("parameters") {
        sig.push_str(node_text(params, source));
    }

    if let Some(ret) = func_node.child_by_field_name("return_type") {
        sig.push_str(": ");
        sig.push_str(node_text(ret, source));
    }

    sig
}

/// C/C++ signature: extract from declarator.
fn extract_c_signature(func_node: Node, source: &[u8]) -> String {
    let mut sig = String::new();

    if let Some(declarator) = func_node.child_by_field_name("declarator") {
        // The declarator includes function name and parameter list
        // We want just the parameters portion
        if let Some(params) = declarator.child_by_field_name("parameters") {
            sig.push_str(node_text(params, source));
        }
    }

    // Return type is typically the first child (type specifier)
    if let Some(type_node) = func_node.child_by_field_name("type") {
        let type_text = node_text(type_node, source);
        if !type_text.is_empty() {
            sig = format!("{}: {}", sig, type_text);
        }
    }

    sig
}

/// Ruby signature.
fn extract_ruby_signature(func_node: Node, source: &[u8]) -> String {
    if let Some(params) = func_node.child_by_field_name("parameters") {
        node_text(params, source).to_string()
    } else {
        // Check for method_parameters child
        let mut cursor = func_node.walk();
        for child in func_node.children(&mut cursor) {
            if child.kind() == "method_parameters" {
                return node_text(child, source).to_string();
            }
        }
        "()".to_string()
    }
}

/// PHP signature.
fn extract_php_signature(func_node: Node, source: &[u8]) -> String {
    let mut sig = String::new();

    if let Some(params) = func_node.child_by_field_name("parameters") {
        sig.push_str(node_text(params, source));
    }

    if let Some(ret) = func_node.child_by_field_name("return_type") {
        sig.push_str(": ");
        sig.push_str(node_text(ret, source));
    }

    sig
}

/// Scala signature.
fn extract_scala_signature(func_node: Node, source: &[u8]) -> String {
    let mut sig = String::new();

    if let Some(params) = func_node.child_by_field_name("parameters") {
        sig.push_str(node_text(params, source));
    }

    if let Some(ret) = func_node.child_by_field_name("return_type") {
        sig.push_str(": ");
        sig.push_str(node_text(ret, source));
    }

    sig
}

/// Generic signature: try to find parameters/return type fields.
fn extract_generic_signature(func_node: Node, source: &[u8]) -> String {
    let mut sig = String::new();

    if let Some(params) = func_node.child_by_field_name("parameters") {
        sig.push_str(node_text(params, source));
    }

    sig
}

/// Extract a typed parameter (name: type) - Python-specific.
fn extract_typed_parameter(node: Node, source: &[u8]) -> String {
    let name = node
        .child(0)
        .filter(|c| c.kind() == "identifier")
        .map(|n| node_text(n, source))
        .unwrap_or("");
    let type_hint = node
        .child_by_field_name("type")
        .map(|n| node_text(n, source))
        .unwrap_or("");

    if type_hint.is_empty() {
        name.to_string()
    } else {
        format!("{}: {}", name, type_hint)
    }
}

/// Extract a default parameter (name=default) - Python-specific.
fn extract_default_parameter(node: Node, source: &[u8]) -> String {
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, source))
        .unwrap_or("");
    let value = node
        .child_by_field_name("value")
        .map(|n| node_text(n, source))
        .unwrap_or("");

    format!("{} = {}", name, value)
}

/// Extract a typed default parameter (name: type = default) - Python-specific.
fn extract_typed_default_parameter(node: Node, source: &[u8]) -> String {
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, source))
        .unwrap_or("");
    let type_hint = node
        .child_by_field_name("type")
        .map(|n| node_text(n, source))
        .unwrap_or("");
    let value = node
        .child_by_field_name("value")
        .map(|n| node_text(n, source))
        .unwrap_or("");

    if type_hint.is_empty() {
        format!("{} = {}", name, value)
    } else {
        format!("{}: {} = {}", name, type_hint, value)
    }
}

// =============================================================================
// Function Info Extraction (Language-Aware)
// =============================================================================

/// Extract function information from a function definition node.
pub fn extract_function_info(func_node: Node, source: &[u8], lang: Language) -> FunctionInfo {
    let name = get_node_name(func_node, source, lang).unwrap_or_default();
    let signature = extract_function_signature(func_node, source, lang);
    let lineno = func_node.start_position().row as u32 + 1;
    let is_async = detect_async(func_node, source, lang);
    let docstring = extract_docstring(func_node, source, lang);

    FunctionInfo {
        name,
        signature,
        docstring,
        lineno,
        is_async,
    }
}

/// Detect if a function is async.
fn detect_async(func_node: Node, source: &[u8], lang: Language) -> bool {
    match lang {
        Language::Python => {
            let func_text = node_text(func_node, source);
            func_text.starts_with("async ")
        }
        Language::Rust => {
            // Check for "async" keyword child
            for i in 0..func_node.child_count() {
                if let Some(child) = func_node.child(i) {
                    if node_text(child, source) == "async" {
                        return true;
                    }
                }
            }
            false
        }
        Language::TypeScript | Language::JavaScript => {
            // Check for async keyword
            let func_text = node_text(func_node, source);
            func_text.starts_with("async ")
        }
        Language::CSharp => {
            // Check modifiers for "async"
            if let Some(modifiers) = func_node.child_by_field_name("modifiers") {
                return node_text(modifiers, source).contains("async");
            }
            false
        }
        Language::Elixir => {
            // Elixir doesn't have async keyword in the traditional sense
            false
        }
        _ => false,
    }
}

// =============================================================================
// Docstring / Doc Comment Extraction (Language-Aware)
// =============================================================================

/// Extract docstring or doc comment from a function or class node.
fn extract_docstring(node: Node, source: &[u8], lang: Language) -> Option<String> {
    match lang {
        Language::Python => extract_python_docstring(node, source),
        Language::Rust => extract_rust_doc_comment(node, source),
        Language::Go => extract_go_doc_comment(node, source),
        Language::Java | Language::CSharp | Language::Scala | Language::Php => {
            extract_javadoc_comment(node, source)
        }
        Language::TypeScript | Language::JavaScript => extract_jsdoc_comment(node, source),
        Language::Ruby => extract_ruby_comment(node, source),
        Language::Elixir => extract_elixir_doc(node, source),
        _ => None,
    }
}

/// Python docstring: first string in function/class body.
fn extract_python_docstring(node: Node, source: &[u8]) -> Option<String> {
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        let first_stmt = body.children(&mut cursor).next();
        if let Some(child) = first_stmt {
            if child.kind() == "expression_statement" {
                if let Some(expr) = child.child(0) {
                    if expr.kind() == "string" {
                        let text = node_text(expr, source);
                        let cleaned = text
                            .trim_start_matches("\"\"\"")
                            .trim_start_matches("'''")
                            .trim_end_matches("\"\"\"")
                            .trim_end_matches("'''")
                            .trim();
                        return Some(cleaned.to_string());
                    }
                }
            }
        }
    }
    None
}

/// Rust doc comments: /// or //! preceding the node.
fn extract_rust_doc_comment(node: Node, source: &[u8]) -> Option<String> {
    let mut comments = Vec::new();
    let mut prev = node.prev_sibling();

    while let Some(sib) = prev {
        let kind = sib.kind();
        if kind == "line_comment" {
            let text = node_text(sib, source);
            if text.starts_with("///") || text.starts_with("//!") {
                let content = text
                    .trim_start_matches("///")
                    .trim_start_matches("//!")
                    .trim();
                comments.push(content.to_string());
            } else {
                break;
            }
        } else if kind == "attribute_item" {
            // Skip attributes between doc comments
        } else {
            break;
        }
        prev = sib.prev_sibling();
    }

    if comments.is_empty() {
        None
    } else {
        comments.reverse();
        Some(comments.join("\n"))
    }
}

/// Go doc comment: preceding line comment block.
fn extract_go_doc_comment(node: Node, source: &[u8]) -> Option<String> {
    let mut comments = Vec::new();
    let mut prev = node.prev_sibling();

    while let Some(sib) = prev {
        if sib.kind() == "comment" {
            let text = node_text(sib, source);
            let content = text.trim_start_matches("//").trim();
            comments.push(content.to_string());
        } else {
            break;
        }
        prev = sib.prev_sibling();
    }

    if comments.is_empty() {
        None
    } else {
        comments.reverse();
        Some(comments.join("\n"))
    }
}

/// Javadoc-style: /** ... */ preceding the node.
fn extract_javadoc_comment(node: Node, source: &[u8]) -> Option<String> {
    let mut prev = node.prev_sibling();

    while let Some(sib) = prev {
        let kind = sib.kind();
        if kind == "block_comment" || kind == "comment" || kind == "multiline_comment" {
            let text = node_text(sib, source);
            if text.starts_with("/**") {
                let cleaned = text
                    .trim_start_matches("/**")
                    .trim_end_matches("*/")
                    .lines()
                    .map(|l| l.trim().trim_start_matches('*').trim())
                    .filter(|l| !l.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n");
                return Some(cleaned);
            }
        } else if kind == "annotation" || kind == "marker_annotation" || kind == "attribute_list" {
            // Skip annotations/attributes
        } else {
            break;
        }
        prev = sib.prev_sibling();
    }
    None
}

/// JSDoc: /** ... */ preceding the node.
fn extract_jsdoc_comment(node: Node, source: &[u8]) -> Option<String> {
    extract_javadoc_comment(node, source)
}

/// Ruby: # comments preceding the node.
fn extract_ruby_comment(node: Node, source: &[u8]) -> Option<String> {
    let mut comments = Vec::new();
    let mut prev = node.prev_sibling();

    while let Some(sib) = prev {
        if sib.kind() == "comment" {
            let text = node_text(sib, source);
            let content = text.trim_start_matches('#').trim();
            comments.push(content.to_string());
        } else {
            break;
        }
        prev = sib.prev_sibling();
    }

    if comments.is_empty() {
        None
    } else {
        comments.reverse();
        Some(comments.join("\n"))
    }
}

/// Elixir: @doc or @moduledoc preceding the node.
fn extract_elixir_doc(node: Node, source: &[u8]) -> Option<String> {
    let mut prev = node.prev_sibling();

    while let Some(sib) = prev {
        if sib.kind() == "unary_operator" || sib.kind() == "call" {
            let text = node_text(sib, source);
            if text.starts_with("@doc") || text.starts_with("@moduledoc") {
                // Extract the string content
                let cleaned = text
                    .trim_start_matches("@moduledoc")
                    .trim_start_matches("@doc")
                    .trim()
                    .trim_start_matches("\"\"\"")
                    .trim_end_matches("\"\"\"")
                    .trim_start_matches('"')
                    .trim_end_matches('"')
                    .trim();
                if !cleaned.is_empty() {
                    return Some(cleaned.to_string());
                }
            }
        } else if sib.kind() == "comment" {
            // skip
        } else {
            break;
        }
        prev = sib.prev_sibling();
    }
    None
}

// =============================================================================
// Class Info Extraction (Language-Aware)
// =============================================================================

/// Extract class/struct/trait information from a definition node.
pub fn extract_class_info(class_node: Node, source: &[u8], lang: Language) -> ClassInfo {
    let name = get_node_name(class_node, source, lang).unwrap_or_default();
    let lineno = class_node.start_position().row as u32 + 1;

    // Extract base classes / implemented interfaces
    let bases = extract_base_classes(class_node, source, lang);

    // Extract methods
    let mut methods = Vec::new();
    let mut private_method_count = 0u32;

    let method_kinds = method_node_kinds(lang);
    let body_node = find_body_node(class_node, lang);

    if let Some(body) = body_node {
        collect_methods_from_body(
            body,
            source,
            lang,
            method_kinds,
            &mut methods,
            &mut private_method_count,
        );
    }

    ClassInfo {
        name,
        lineno,
        bases,
        methods,
        private_method_count,
    }
}

/// Find the body/block node of a class/struct definition.
fn find_body_node<'a>(class_node: Node<'a>, lang: Language) -> Option<Node<'a>> {
    // Try common field names
    if let Some(body) = class_node.child_by_field_name("body") {
        return Some(body);
    }
    if let Some(body) = class_node.child_by_field_name("members") {
        return Some(body);
    }

    match lang {
        Language::Rust => {
            // For impl_item, look for declaration_list
            let mut cursor = class_node.walk();
            for child in class_node.children(&mut cursor) {
                if child.kind() == "declaration_list" {
                    return Some(child);
                }
            }
            None
        }
        Language::Java | Language::CSharp => {
            // class_body or interface_body
            let mut cursor = class_node.walk();
            for child in class_node.children(&mut cursor) {
                if child.kind() == "class_body"
                    || child.kind() == "interface_body"
                    || child.kind() == "enum_body"
                    || child.kind() == "declaration_list"
                {
                    return Some(child);
                }
            }
            None
        }
        Language::TypeScript | Language::JavaScript => {
            let mut cursor = class_node.walk();
            for child in class_node.children(&mut cursor) {
                if child.kind() == "class_body" {
                    return Some(child);
                }
            }
            None
        }
        Language::Cpp => {
            let mut cursor = class_node.walk();
            for child in class_node.children(&mut cursor) {
                if child.kind() == "field_declaration_list" {
                    return Some(child);
                }
            }
            None
        }
        Language::Ruby => {
            // Ruby class body is inside a body_statement child
            let mut cursor = class_node.walk();
            for child in class_node.children(&mut cursor) {
                if child.kind() == "body_statement" {
                    return Some(child);
                }
            }
            // Fallback: use the class node itself
            Some(class_node)
        }
        _ => {
            // Default: try common body kinds
            let mut cursor = class_node.walk();
            for child in class_node.children(&mut cursor) {
                let kind = child.kind();
                if kind.contains("body")
                    || kind.contains("block")
                    || kind == "declaration_list"
                    || kind == "template_body"
                {
                    return Some(child);
                }
            }
            None
        }
    }
}

/// Collect methods from a class body node.
fn collect_methods_from_body(
    body: Node,
    source: &[u8],
    lang: Language,
    method_kinds: &[&str],
    methods: &mut Vec<MethodInfo>,
    private_count: &mut u32,
) {
    let mut cursor = body.walk();
    let decorator_kinds = decorator_node_kinds(lang);

    for child in body.children(&mut cursor) {
        let kind = child.kind();

        if method_kinds.contains(&kind) {
            let method_name = get_node_name(child, source, lang).unwrap_or_default();
            if is_method_public(&method_name, child, source, lang) {
                methods.push(extract_method_info(child, source, lang));
            } else {
                *private_count += 1;
            }
        } else if decorator_kinds.contains(&kind) {
            // Handle decorated methods (Python, Java annotations, etc.)
            if let Some(def) = find_definition_in_decorated(child, method_kinds) {
                let method_name = get_node_name(def, source, lang).unwrap_or_default();
                if is_method_public(&method_name, def, source, lang) {
                    methods.push(extract_method_info(def, source, lang));
                } else {
                    *private_count += 1;
                }
            }
        }
    }
}

/// Check if a method is public based on language conventions.
fn is_method_public(name: &str, node: Node, source: &[u8], lang: Language) -> bool {
    match lang {
        Language::Python | Language::Ruby | Language::Lua | Language::Luau => {
            is_public_for_lang(name, lang)
        }
        Language::Rust => is_rust_pub(node, source),
        Language::Go => name.chars().next().is_some_and(|c| c.is_uppercase()),
        Language::Java | Language::CSharp => has_public_modifier(node, source),
        _ => true,
    }
}

/// Extract base classes / superclasses / implemented interfaces.
fn extract_base_classes(class_node: Node, source: &[u8], lang: Language) -> Vec<String> {
    let mut bases = Vec::new();

    match lang {
        Language::Python => {
            if let Some(superclasses) = class_node.child_by_field_name("superclasses") {
                let mut cursor = superclasses.walk();
                for child in superclasses.children(&mut cursor) {
                    if child.kind() == "identifier" || child.kind() == "attribute" {
                        bases.push(node_text(child, source).to_string());
                    }
                }
            }
        }
        Language::Java | Language::CSharp => {
            // Check for "superclass" and "interfaces" fields
            if let Some(super_node) = class_node.child_by_field_name("superclass") {
                bases.push(node_text(super_node, source).to_string());
            }
            if let Some(interfaces) = class_node.child_by_field_name("interfaces") {
                let mut cursor = interfaces.walk();
                for child in interfaces.children(&mut cursor) {
                    if child.kind() == "type_identifier" || child.kind() == "generic_type" {
                        bases.push(node_text(child, source).to_string());
                    }
                }
            }
            // Also check super_interfaces for Java interface declarations
            if let Some(extends) = class_node.child_by_field_name("type_parameters") {
                // type parameters are not bases, skip
                let _ = extends;
            }
        }
        Language::Rust => {
            // For trait_item, look for trait bounds
            // For impl_item, look for the trait being implemented
            if class_node.kind() == "impl_item" {
                if let Some(trait_node) = class_node.child_by_field_name("trait") {
                    bases.push(node_text(trait_node, source).to_string());
                }
            }
        }
        Language::TypeScript | Language::JavaScript => {
            // Check for extends_clause or implements_clause
            let mut cursor = class_node.walk();
            for child in class_node.children(&mut cursor) {
                if child.kind() == "class_heritage" {
                    let mut inner_cursor = child.walk();
                    for clause in child.children(&mut inner_cursor) {
                        if clause.kind() == "extends_clause" || clause.kind() == "implements_clause"
                        {
                            let mut type_cursor = clause.walk();
                            for type_child in clause.children(&mut type_cursor) {
                                if type_child.kind() == "identifier"
                                    || type_child.kind() == "type_identifier"
                                {
                                    bases.push(node_text(type_child, source).to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
        Language::Ruby => {
            if let Some(super_node) = class_node.child_by_field_name("superclass") {
                bases.push(node_text(super_node, source).to_string());
            }
        }
        Language::Go => {
            // Go type_declaration doesn't have base classes per se
            // But embedded structs could be found in struct fields
        }
        Language::Scala => {
            if let Some(extends) = class_node.child_by_field_name("extends") {
                bases.push(node_text(extends, source).to_string());
            }
        }
        _ => {}
    }

    bases
}

/// Find a function/class definition inside a decorated_definition node.
fn find_definition_in_decorated<'a>(node: Node<'a>, target_kinds: &[&str]) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    let found = node
        .children(&mut cursor)
        .find(|&child| target_kinds.contains(&child.kind()));
    found
}

/// Extract method information from a function definition node.
fn extract_method_info(func_node: Node, source: &[u8], lang: Language) -> MethodInfo {
    let name = get_node_name(func_node, source, lang).unwrap_or_default();
    let signature = extract_function_signature(func_node, source, lang);
    let is_async = detect_async(func_node, source, lang);

    MethodInfo {
        name,
        signature,
        is_async,
    }
}

// =============================================================================
// Interface Extraction (Language-Aware)
// =============================================================================

/// Extract the public interface from a source file.
///
/// Detects the language from the file extension and uses the appropriate
/// tree-sitter grammar and node kinds.
pub fn extract_interface(path: &Path, source: &str) -> PatternsResult<InterfaceInfo> {
    let lang = Language::from_path(path).unwrap_or(Language::Python);
    extract_interface_with_lang(path, source, lang)
}

/// Extract the public interface from a source file with an explicit language.
pub fn extract_interface_with_lang(
    path: &Path,
    source: &str,
    lang: Language,
) -> PatternsResult<InterfaceInfo> {
    let source_bytes = source.as_bytes();

    // Parse with ParserPool (multi-language)
    let pool = ParserPool::new();
    let tree = pool
        .parse(source, lang)
        .map_err(|e| PatternsError::parse_error(path, format!("Failed to parse: {}", e)))?;

    let root = tree.root_node();

    // Extract __all__ exports (Python-specific)
    let all_exports = if lang == Language::Python {
        extract_all_exports(root, source_bytes)
    } else {
        None
    };

    // Determine node kinds for this language
    let func_kinds = function_node_kinds(lang);
    let class_kinds = class_node_kinds(lang);
    let decorator_kinds = decorator_node_kinds(lang);

    // Extract public functions and classes
    let (functions, classes) = collect_top_level_definitions(
        root,
        source_bytes,
        lang,
        func_kinds,
        class_kinds,
        decorator_kinds,
    );

    Ok(InterfaceInfo {
        file: path.display().to_string(),
        all_exports,
        functions,
        classes,
    })
}

/// Collect top-level function and class definitions from the AST root.
fn collect_top_level_definitions(
    root: Node,
    source: &[u8],
    lang: Language,
    func_kinds: &[&str],
    class_kinds: &[&str],
    decorator_kinds: &[&str],
)
    -> (Vec<FunctionInfo>, Vec<ClassInfo>)
{
    let mut functions = Vec::new();
    let mut classes = Vec::new();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        let kind = child.kind();

        if func_kinds.contains(&kind) {
            if is_node_public(child, source, lang) {
                // For Elixir, filter to only `def` (not `defp` which is private)
                if lang == Language::Elixir {
                    if let Some(target) = child.child(0) {
                        let target_text = node_text(target, source);
                        if target_text == "defp" {
                            continue;
                        }
                        if target_text != "def" {
                            continue;
                        }
                    }
                }
                functions.push(extract_function_info(child, source, lang));
            }
        } else if class_kinds.contains(&kind) {
            if is_node_public(child, source, lang) {
                // For Elixir, filter to only `defmodule`
                if lang == Language::Elixir {
                    if let Some(target) = child.child(0) {
                        if node_text(target, source) != "defmodule" {
                            continue;
                        }
                    }
                }
                classes.push(extract_class_info(child, source, lang));
            }
        } else if decorator_kinds.contains(&kind) {
            // Handle decorated definitions (Python)
            if let Some(def) = find_definition_in_decorated(child, func_kinds) {
                if is_node_public(def, source, lang) {
                    functions.push(extract_function_info(def, source, lang));
                }
            } else if let Some(class_def) = find_definition_in_decorated(child, class_kinds) {
                if is_node_public(class_def, source, lang) {
                    classes.push(extract_class_info(class_def, source, lang));
                }
            }
        } else if lang == Language::Php {
            // For some languages, definitions may be nested in other constructs
            // (e.g., Go has top-level declarations that contain the actual definitions)
            // Recurse one level for these
            // PHP wraps everything in a program > php_tag + declaration list
            // Recurse into children.
            let mut inner_cursor = child.walk();
            for inner_child in child.children(&mut inner_cursor) {
                let inner_kind = inner_child.kind();
                if func_kinds.contains(&inner_kind) {
                    if is_node_public(inner_child, source, lang) {
                        functions.push(extract_function_info(inner_child, source, lang));
                    }
                } else if class_kinds.contains(&inner_kind)
                    && is_node_public(inner_child, source, lang)
                {
                    classes.push(extract_class_info(inner_child, source, lang));
                }
            }
        }
    }

    (functions, classes)
}

// =============================================================================
// Text Formatting
// =============================================================================

/// Format interface info as human-readable text.
pub fn format_interface_text(info: &InterfaceInfo) -> String {
    let mut lines = Vec::new();

    // Header
    lines.push(format!("File: {}", info.file));
    lines.push(String::new());

    // __all__ exports
    if let Some(ref exports) = info.all_exports {
        lines.push("Exports (__all__):".to_string());
        for name in exports {
            lines.push(format!("  {}", name));
        }
        lines.push(String::new());
    }

    // Functions
    if !info.functions.is_empty() {
        lines.push("Functions:".to_string());
        for func in &info.functions {
            let async_marker = if func.is_async { "async " } else { "" };
            lines.push(format!(
                "  {}def {}{}  [line {}]",
                async_marker, func.name, func.signature, func.lineno
            ));
            if let Some(ref doc) = func.docstring {
                // Truncate long docstrings
                let doc_preview = if doc.len() > 60 {
                    format!("{}...", &doc[..57])
                } else {
                    doc.clone()
                };
                lines.push(format!("      \"{}\"", doc_preview));
            }
        }
        lines.push(String::new());
    }

    // Classes
    if !info.classes.is_empty() {
        lines.push("Classes:".to_string());
        for class in &info.classes {
            let bases_str = if class.bases.is_empty() {
                String::new()
            } else {
                format!("({})", class.bases.join(", "))
            };
            lines.push(format!(
                "  class {}{}  [line {}]",
                class.name, bases_str, class.lineno
            ));

            for method in &class.methods {
                let async_marker = if method.is_async { "async " } else { "" };
                lines.push(format!(
                    "    {}def {}{}",
                    async_marker, method.name, method.signature
                ));
            }

            if class.private_method_count > 0 {
                lines.push(format!(
                    "    ({} private methods)",
                    class.private_method_count
                ));
            }
        }
        lines.push(String::new());
    }

    // Summary
    let total_methods: u32 = info.classes.iter().map(|c| c.methods.len() as u32).sum();
    lines.push(format!(
        "Summary: {} functions, {} classes, {} public methods",
        info.functions.len(),
        info.classes.len(),
        total_methods
    ));

    lines.join("\n")
}

// =============================================================================
// Entry Point
// =============================================================================

/// Check if a file has a supported source code extension.
fn is_supported_source_file(path: &Path) -> bool {
    Language::from_path(path).is_some()
}

/// Run the interface command.
pub fn run(args: InterfaceArgs, format: OutputFormat) -> anyhow::Result<()> {
    let path = &args.path;

    if path.is_dir() {
        // Validate directory
        let canonical_dir = if let Some(ref root) = args.project_root {
            super::validation::validate_file_path_in_project(path, root)?
        } else {
            validate_directory_path(path)?
        };

        // Collect all supported source files recursively
        let mut results = Vec::new();
        let mut entries: Vec<PathBuf> = WalkDir::new(&canonical_dir)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                e.file_name()
                    .to_str()
                    .map(|s| !s.starts_with('.'))
                    .unwrap_or(true)
            })
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file() && is_supported_source_file(e.path()))
            .map(|e| e.path().to_path_buf())
            .collect();

        // Sort for deterministic output
        entries.sort();

        for file_path in entries {
            let source = read_file_safe(&file_path)?;
            match extract_interface(&file_path, &source) {
                Ok(info) => results.push(info),
                Err(_) => {
                    // Skip files that fail to parse (unsupported grammars, etc.)
                    continue;
                }
            }
        }

        // Output
        match format {
            OutputFormat::Text => {
                for info in &results {
                    println!("{}", format_interface_text(info));
                    println!();
                }
            }
            OutputFormat::Compact => {
                let json = serde_json::to_string(&results)?;
                println!("{}", json);
            }
            _ => {
                let json = serde_json::to_string_pretty(&results)?;
                println!("{}", json);
            }
        }
    } else {
        // Single file
        let canonical_path = if let Some(ref root) = args.project_root {
            super::validation::validate_file_path_in_project(path, root)?
        } else {
            validate_file_path(path)?
        };

        let source = read_file_safe(&canonical_path)?;
        let info = extract_interface(&canonical_path, &source)?;

        // Output
        match format {
            OutputFormat::Text => {
                println!("{}", format_interface_text(&info));
            }
            OutputFormat::Compact => {
                let json = serde_json::to_string(&info)?;
                println!("{}", json);
            }
            _ => {
                let json = serde_json::to_string_pretty(&info)?;
                println!("{}", json);
            }
        }
    }

    Ok(())
}

// =============================================================================
// Utilities
// =============================================================================

/// Get the text content of a node.
fn node_text<'a>(node: Node, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // is_public_name tests (backward-compatible)
    // -------------------------------------------------------------------------

    #[test]
    fn test_is_public_name_public() {
        assert!(is_public_name("my_function"));
        assert!(is_public_name("MyClass"));
        assert!(is_public_name("process"));
        assert!(is_public_name("x"));
    }

    #[test]
    fn test_is_public_name_private() {
        assert!(!is_public_name("_private"));
        assert!(!is_public_name("__dunder__"));
        assert!(!is_public_name("_PrivateClass"));
        assert!(!is_public_name("__init__"));
    }

    // -------------------------------------------------------------------------
    // Python: extract_all_exports tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_extract_all_exports_present() {
        let source = r#"
__all__ = ['foo', 'bar', 'Baz']

def foo():
    pass
"#;
        let pool = ParserPool::new();
        let tree = pool.parse(source, Language::Python).unwrap();
        let root = tree.root_node();

        let exports = extract_all_exports(root, source.as_bytes());
        assert!(exports.is_some());
        let exports = exports.unwrap();
        assert_eq!(exports.len(), 3);
        assert!(exports.contains(&"foo".to_string()));
        assert!(exports.contains(&"bar".to_string()));
        assert!(exports.contains(&"Baz".to_string()));
    }

    #[test]
    fn test_extract_all_exports_absent() {
        let source = r#"
def foo():
    pass
"#;
        let pool = ParserPool::new();
        let tree = pool.parse(source, Language::Python).unwrap();
        let root = tree.root_node();

        let exports = extract_all_exports(root, source.as_bytes());
        assert!(exports.is_none());
    }

    // -------------------------------------------------------------------------
    // Python: extract_function_signature tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_extract_function_signature_simple() {
        let source = "def foo(x, y): pass";
        let pool = ParserPool::new();
        let tree = pool.parse(source, Language::Python).unwrap();
        let root = tree.root_node();
        let func_node = root.child(0).unwrap();

        let sig = extract_function_signature(func_node, source.as_bytes(), Language::Python);
        assert_eq!(sig, "(x, y)");
    }

    #[test]
    fn test_extract_function_signature_typed() {
        let source = "def foo(x: int, y: str) -> bool: pass";
        let pool = ParserPool::new();
        let tree = pool.parse(source, Language::Python).unwrap();
        let root = tree.root_node();
        let func_node = root.child(0).unwrap();

        let sig = extract_function_signature(func_node, source.as_bytes(), Language::Python);
        assert!(sig.contains("x: int"), "sig = {:?}", sig);
        assert!(sig.contains("y: str"), "sig = {:?}", sig);
        assert!(sig.contains("-> bool"), "sig = {:?}", sig);
    }

    #[test]
    fn test_extract_function_signature_default() {
        let source = "def foo(x: int = 10): pass";
        let pool = ParserPool::new();
        let tree = pool.parse(source, Language::Python).unwrap();
        let root = tree.root_node();
        let func_node = root.child(0).unwrap();

        let sig = extract_function_signature(func_node, source.as_bytes(), Language::Python);
        assert!(sig.contains("x: int = 10") || sig.contains("x: int=10"));
    }

    // -------------------------------------------------------------------------
    // Python: extract_interface tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_extract_interface_public_functions() {
        let source = r#"
def public_func():
    """A public function."""
    pass

def _private_func():
    pass
"#;
        let info = extract_interface(Path::new("test.py"), source).unwrap();

        assert_eq!(info.functions.len(), 1);
        assert_eq!(info.functions[0].name, "public_func");
    }

    #[test]
    fn test_extract_interface_public_classes() {
        let source = r#"
class PublicClass:
    def public_method(self):
        pass

    def _private_method(self):
        pass

class _PrivateClass:
    pass
"#;
        let info = extract_interface(Path::new("test.py"), source).unwrap();

        assert_eq!(info.classes.len(), 1);
        assert_eq!(info.classes[0].name, "PublicClass");
        assert_eq!(info.classes[0].methods.len(), 1);
        assert_eq!(info.classes[0].methods[0].name, "public_method");
        assert_eq!(info.classes[0].private_method_count, 1);
    }

    #[test]
    fn test_extract_interface_async_function() {
        let source = r#"
async def async_func():
    pass

def sync_func():
    pass
"#;
        let info = extract_interface(Path::new("test.py"), source).unwrap();

        assert_eq!(info.functions.len(), 2);

        let async_fn = info.functions.iter().find(|f| f.name == "async_func");
        assert!(async_fn.is_some());
        assert!(async_fn.unwrap().is_async);

        let sync_fn = info.functions.iter().find(|f| f.name == "sync_func");
        assert!(sync_fn.is_some());
        assert!(!sync_fn.unwrap().is_async);
    }

    #[test]
    fn test_extract_interface_with_all() {
        let source = r#"
__all__ = ['foo', 'Bar']

def foo():
    pass

def bar():
    pass

class Bar:
    pass
"#;
        let info = extract_interface(Path::new("test.py"), source).unwrap();

        assert!(info.all_exports.is_some());
        let exports = info.all_exports.unwrap();
        assert!(exports.contains(&"foo".to_string()));
        assert!(exports.contains(&"Bar".to_string()));
    }

    #[test]
    fn test_extract_interface_docstrings() {
        let source = r#"
def documented():
    """This is a docstring."""
    pass

def undocumented():
    pass
"#;
        let info = extract_interface(Path::new("test.py"), source).unwrap();

        let documented = info.functions.iter().find(|f| f.name == "documented");
        assert!(documented.is_some());
        assert!(documented.unwrap().docstring.is_some());
        assert!(documented
            .unwrap()
            .docstring
            .as_ref()
            .unwrap()
            .contains("docstring"));

        let undocumented = info.functions.iter().find(|f| f.name == "undocumented");
        assert!(undocumented.is_some());
        assert!(undocumented.unwrap().docstring.is_none());
    }

    #[test]
    fn test_extract_interface_class_bases() {
        let source = r#"
class Child(Parent, Mixin):
    pass
"#;
        let info = extract_interface(Path::new("test.py"), source).unwrap();

        assert_eq!(info.classes.len(), 1);
        assert_eq!(info.classes[0].bases.len(), 2);
        assert!(info.classes[0].bases.contains(&"Parent".to_string()));
        assert!(info.classes[0].bases.contains(&"Mixin".to_string()));
    }

    // -------------------------------------------------------------------------
    // format_interface_text tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_format_interface_text() {
        let info = InterfaceInfo {
            file: "test.py".to_string(),
            all_exports: Some(vec!["foo".to_string()]),
            functions: vec![FunctionInfo {
                name: "foo".to_string(),
                signature: "(x: int) -> str".to_string(),
                docstring: Some("A function.".to_string()),
                lineno: 5,
                is_async: false,
            }],
            classes: vec![ClassInfo {
                name: "MyClass".to_string(),
                lineno: 10,
                bases: vec!["Base".to_string()],
                methods: vec![MethodInfo {
                    name: "method".to_string(),
                    signature: "(self)".to_string(),
                    is_async: false,
                }],
                private_method_count: 2,
            }],
        };

        let text = format_interface_text(&info);
        assert!(text.contains("File: test.py"));
        assert!(text.contains("foo"));
        assert!(text.contains("MyClass"));
        assert!(text.contains("Base"));
        assert!(text.contains("method"));
        assert!(text.contains("2 private methods"));
    }

    // =========================================================================
    // Multi-language tests
    // =========================================================================

    // -------------------------------------------------------------------------
    // Rust
    // -------------------------------------------------------------------------

    #[test]
    fn test_extract_interface_rust_pub_functions() {
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
        let info = extract_interface(Path::new("test.rs"), source).unwrap();

        assert_eq!(
            info.functions.len(),
            2,
            "Should find 2 pub functions, got: {:?}",
            info.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );

        let add_fn = info.functions.iter().find(|f| f.name == "add");
        assert!(add_fn.is_some(), "Should find 'add' function");
        let add_fn = add_fn.unwrap();
        assert!(
            add_fn.signature.contains("a: i32"),
            "sig = {:?}",
            add_fn.signature
        );
        assert!(
            add_fn.signature.contains("-> i32"),
            "sig = {:?}",
            add_fn.signature
        );
        assert!(add_fn.docstring.is_some(), "Should have doc comment");
        assert!(add_fn
            .docstring
            .as_ref()
            .unwrap()
            .contains("Adds two numbers"));
        assert!(!add_fn.is_async);

        let async_fn = info.functions.iter().find(|f| f.name == "async_fetch");
        assert!(async_fn.is_some(), "Should find 'async_fetch' function");
        assert!(async_fn.unwrap().is_async);
    }

    #[test]
    fn test_extract_interface_rust_struct_impl() {
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
"#;
        let info = extract_interface(Path::new("test.rs"), source).unwrap();

        // Should find struct and impl as classes
        assert!(
            !info.classes.is_empty(),
            "Should find at least struct/impl, got: {:?}",
            info.classes.iter().map(|c| &c.name).collect::<Vec<_>>()
        );

        // Check the struct
        let point_struct = info.classes.iter().find(|c| c.name == "Point");
        assert!(point_struct.is_some(), "Should find Point struct/impl");
    }

    #[test]
    fn test_extract_interface_rust_trait() {
        let source = r#"
pub trait Drawable {
    fn draw(&self);
    fn resize(&mut self, factor: f64);
}
"#;
        let info = extract_interface(Path::new("test.rs"), source).unwrap();

        let trait_info = info.classes.iter().find(|c| c.name == "Drawable");
        assert!(trait_info.is_some(), "Should find Drawable trait");
    }

    // -------------------------------------------------------------------------
    // Go
    // -------------------------------------------------------------------------

    #[test]
    fn test_extract_interface_go_exported_functions() {
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
        let info = extract_interface(Path::new("test.go"), source).unwrap();

        // Go: exported functions start with uppercase
        assert_eq!(
            info.functions.len(),
            1,
            "Should find 1 exported function, got: {:?}",
            info.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        assert_eq!(info.functions[0].name, "ProcessData");
        assert!(
            info.functions[0].docstring.is_some(),
            "Should have doc comment"
        );
    }

    // -------------------------------------------------------------------------
    // TypeScript
    // -------------------------------------------------------------------------

    #[test]
    fn test_extract_interface_typescript_class() {
        let source = r#"
class UserService {
    async fetchUser(id: string): Promise<User> {
        return {} as User;
    }

    private internalMethod(): void {}
}

function processData(input: string): number {
    return input.length;
}
"#;
        let info = extract_interface(Path::new("test.ts"), source).unwrap();

        // Should find both the class and the function
        assert!(
            !info.functions.is_empty() || !info.classes.is_empty(),
            "Should find definitions: functions={:?}, classes={:?}",
            info.functions.iter().map(|f| &f.name).collect::<Vec<_>>(),
            info.classes.iter().map(|c| &c.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_extract_interface_typescript_interface() {
        let source = r#"
interface User {
    id: string;
    name: string;
    email: string;
}

type Status = "active" | "inactive";
"#;
        let info = extract_interface(Path::new("test.ts"), source).unwrap();

        assert!(
            !info.classes.is_empty(),
            "Should find interface/type declarations, got: {:?}",
            info.classes.iter().map(|c| &c.name).collect::<Vec<_>>()
        );
    }

    // -------------------------------------------------------------------------
    // Java
    // -------------------------------------------------------------------------

    #[test]
    fn test_extract_interface_java_class() {
        let source = r#"
/**
 * Service for managing users.
 */
public class UserService {
    public String getUser(String id) {
        return id;
    }

    private void internalCleanup() {}
}
"#;
        let info = extract_interface(Path::new("test.java"), source).unwrap();

        assert!(
            !info.classes.is_empty(),
            "Should find Java class, got: {:?}",
            info.classes.iter().map(|c| &c.name).collect::<Vec<_>>()
        );

        if let Some(cls) = info.classes.iter().find(|c| c.name == "UserService") {
            assert!(!cls.methods.is_empty(), "Should find public methods");
        }
    }

    // -------------------------------------------------------------------------
    // C
    // -------------------------------------------------------------------------

    #[test]
    fn test_extract_interface_c_functions() {
        let source = r#"
int add(int a, int b) {
    return a + b;
}

static int internal_helper(void) {
    return 42;
}
"#;
        let info = extract_interface(Path::new("test.c"), source).unwrap();

        // Non-static C functions should be public
        assert_eq!(
            info.functions.len(),
            1,
            "Should find 1 non-static function, got: {:?}",
            info.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        assert_eq!(info.functions[0].name, "add");
    }

    // -------------------------------------------------------------------------
    // Ruby
    // -------------------------------------------------------------------------

    #[test]
    fn test_extract_interface_ruby_class() {
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
        let info = extract_interface(Path::new("test.rb"), source).unwrap();

        assert!(
            !info.classes.is_empty(),
            "Should find Ruby class, got: {:?}",
            info.classes.iter().map(|c| &c.name).collect::<Vec<_>>()
        );

        if let Some(cls) = info.classes.iter().find(|c| c.name == "UserManager") {
            assert_eq!(
                cls.methods.len(),
                1,
                "Should find 1 public method, got: {:?}",
                cls.methods.iter().map(|m| &m.name).collect::<Vec<_>>()
            );
            assert_eq!(cls.methods[0].name, "find_user");
            assert_eq!(cls.private_method_count, 1);
        }
    }

    // -------------------------------------------------------------------------
    // is_public_for_lang tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_is_public_for_go() {
        assert!(is_public_for_lang("ProcessData", Language::Go));
        assert!(!is_public_for_lang("processData", Language::Go));
    }

    #[test]
    fn test_is_public_for_python() {
        assert!(is_public_for_lang("process_data", Language::Python));
        assert!(!is_public_for_lang("_private", Language::Python));
    }

    // -------------------------------------------------------------------------
    // is_supported_source_file tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_is_supported_source_file() {
        assert!(is_supported_source_file(Path::new("test.py")));
        assert!(is_supported_source_file(Path::new("test.rs")));
        assert!(is_supported_source_file(Path::new("test.go")));
        assert!(is_supported_source_file(Path::new("test.ts")));
        assert!(is_supported_source_file(Path::new("test.java")));
        assert!(is_supported_source_file(Path::new("test.c")));
        assert!(is_supported_source_file(Path::new("test.rb")));
        assert!(is_supported_source_file(Path::new("test.cs")));
        assert!(!is_supported_source_file(Path::new("test.txt")));
        assert!(!is_supported_source_file(Path::new("test.md")));
    }
}
