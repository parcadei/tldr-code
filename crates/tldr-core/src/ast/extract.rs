//! Full file extraction (spec Section 2.1.3)
//!
//! Extracts complete module information from a single file including:
//! - Module docstring
//! - All imports
//! - Function details (name, params, return type, docstring, decorators)
//! - Class details (name, bases, methods)
//! - Intra-file call graph

use std::collections::HashMap;
use std::path::Path;

use tree_sitter::{Node, Tree};

use crate::error::TldrError;
use crate::types::{ClassInfo, FieldInfo, FunctionInfo, IntraFileCallGraph, Language, ModuleInfo};
use crate::TldrResult;

use super::imports::extract_imports_from_tree;
use super::parser::parse_file;

/// Extract complete module information from a file.
///
/// # Arguments
/// * `file_path` - Path to the source file
/// * `base_path` - Optional base path for relative file paths in output
///
/// # Returns
/// * `Ok(ModuleInfo)` - Complete module information
/// * `Err(TldrError::PathNotFound)` - File doesn't exist
/// * `Err(TldrError::PathTraversal)` - Path escapes base_path
/// * `Err(TldrError::UnsupportedLanguage)` - Unknown file extension
/// * `Err(TldrError::ParseError)` - Syntax error
pub fn extract_file(file_path: &Path, base_path: Option<&Path>) -> TldrResult<ModuleInfo> {
    // Check for path traversal if base_path provided
    if let Some(base) = base_path {
        let canonical_file = dunce::canonicalize(file_path)
            .map_err(|_| TldrError::PathNotFound(file_path.to_path_buf()))?;
        let canonical_base =
            dunce::canonicalize(base).map_err(|_| TldrError::PathNotFound(base.to_path_buf()))?;

        if !canonical_file.starts_with(&canonical_base) {
            return Err(TldrError::PathTraversal(file_path.to_path_buf()));
        }
    }

    let (tree, source, language) = parse_file(file_path)?;

    extract_from_tree(&tree, &source, language, file_path, base_path)
}

/// Extract complete module information from a pre-parsed syntax tree.
///
/// This function is useful when you already have a parsed tree and want to extract
/// module information without re-parsing. This enables combined passes where parsing
/// happens once and multiple extractions can be performed.
///
/// # Arguments
/// * `tree` - Pre-parsed syntax tree
/// * `source` - Source code text
/// * `language` - Programming language of the source
/// * `file_path` - Path to the source file (used for output path)
/// * `base_path` - Optional base path for relative file paths in output
///
/// # Returns
/// * `Ok(ModuleInfo)` - Complete module information
/// * `Err(TldrError)` - Extraction error
pub fn extract_from_tree(
    tree: &Tree,
    source: &str,
    language: Language,
    file_path: &Path,
    base_path: Option<&Path>,
) -> TldrResult<ModuleInfo> {
    // Compute relative path if base provided
    let output_path = if let Some(base) = base_path {
        file_path
            .strip_prefix(base)
            .unwrap_or(file_path)
            .to_path_buf()
    } else {
        file_path.to_path_buf()
    };

    // Extract module docstring
    let docstring = extract_module_docstring(tree, source, language);

    // Extract imports
    let imports = extract_imports_from_tree(tree, source, language)?;

    // Extract functions with full details
    let functions = extract_functions_detailed(tree, source, language);

    // Extract classes with full details
    let classes = extract_classes_detailed(tree, source, language);

    // Extract module-level constants (Gap 3)
    let constants = extract_module_constants(tree, source, language);

    // Build intra-file call graph
    let call_graph = build_intra_file_call_graph(tree, source, language, &functions, &classes);

    Ok(ModuleInfo {
        file_path: output_path,
        language,
        docstring,
        imports,
        functions,
        classes,
        constants,
        call_graph,
    })
}

/// Extract module-level docstring
fn extract_module_docstring(tree: &Tree, source: &str, language: Language) -> Option<String> {
    let root = tree.root_node();

    match language {
        Language::Python => {
            // First expression statement that is a string
            let mut cursor = root.walk();
            for child in root.children(&mut cursor) {
                if child.kind() == "expression_statement" {
                    if let Some(expr) = child.child(0) {
                        if expr.kind() == "string" {
                            return Some(extract_string_content(&expr, source));
                        }
                    }
                } else if child.kind() != "comment" {
                    // Stop at first non-comment, non-docstring
                    break;
                }
            }
            None
        }
        Language::TypeScript | Language::JavaScript => {
            // JSDoc comment at start
            let mut cursor = root.walk();
            for child in root.children(&mut cursor) {
                if child.kind() == "comment" {
                    let text = get_node_text(&child, source);
                    if text.starts_with("/**") {
                        return Some(text);
                    }
                } else {
                    break;
                }
            }
            None
        }
        Language::Rust => {
            // //! or /*! doc comments
            let mut cursor = root.walk();
            let mut doc_lines = Vec::new();
            for child in root.children(&mut cursor) {
                let text = get_node_text(&child, source);
                if child.kind() == "line_comment" && text.starts_with("//!") {
                    doc_lines.push(text.trim_start_matches("//!").trim().to_string());
                } else if child.kind() == "block_comment" && text.starts_with("/*!") {
                    return Some(text);
                } else if !doc_lines.is_empty() || child.kind() != "line_comment" {
                    break;
                }
            }
            if doc_lines.is_empty() {
                None
            } else {
                Some(doc_lines.join("\n"))
            }
        }
        _ => None,
    }
}

/// Parse a `/** ... */` block doc comment, stripping delimiters and leading `*` per line.
fn parse_block_doc_comment(text: &str) -> Option<String> {
    let inner = text.trim_start_matches("/**").trim_end_matches("*/");
    let cleaned: Vec<String> = inner
        .lines()
        .map(|l| {
            let t = l.trim();
            let t = t
                .strip_prefix("* ")
                .unwrap_or(t.strip_prefix('*').unwrap_or(t));
            t.to_string()
        })
        .collect();
    let start = cleaned
        .iter()
        .position(|l| !l.is_empty())
        .unwrap_or(cleaned.len());
    let end = cleaned
        .iter()
        .rposition(|l| !l.is_empty())
        .map(|i| i + 1)
        .unwrap_or(0);
    if start >= end {
        None
    } else {
        Some(cleaned[start..end].join("\n"))
    }
}

/// Extract functions with full details
fn extract_functions_detailed(tree: &Tree, source: &str, language: Language) -> Vec<FunctionInfo> {
    let mut functions = Vec::new();
    let root = tree.root_node();

    match language {
        Language::Python => extract_python_functions_detailed(&root, source, &mut functions, false),
        Language::TypeScript | Language::JavaScript => {
            extract_ts_functions_detailed(&root, source, &mut functions, false)
        }
        Language::Go => extract_go_functions_detailed(&root, source, &mut functions),
        Language::Rust => extract_rust_functions_detailed(&root, source, &mut functions),
        Language::Java => extract_java_functions_detailed(&root, source, &mut functions),
        Language::C => extract_c_functions_detailed(&root, source, &mut functions),
        Language::Cpp => extract_cpp_functions_detailed(&root, source, &mut functions),
        Language::Ruby => extract_ruby_functions_detailed(&root, source, &mut functions),
        Language::Php => extract_php_functions_detailed(&root, source, &mut functions),
        Language::CSharp => extract_csharp_functions_detailed(&root, source, &mut functions),
        Language::Kotlin => extract_kotlin_functions_detailed(&root, source, &mut functions),
        Language::Scala => extract_scala_functions_detailed(&root, source, &mut functions),
        Language::Elixir => extract_elixir_functions_detailed(&root, source, &mut functions),
        Language::Lua => extract_lua_functions_detailed(&root, source, &mut functions),
        Language::Luau => extract_luau_functions_detailed(&root, source, &mut functions),
        Language::Swift => extract_swift_functions_detailed(&root, source, &mut functions),
        Language::Ocaml => extract_ocaml_functions_detailed(&root, source, &mut functions),
    }

    functions
}

/// Extract classes with full details
fn extract_classes_detailed(tree: &Tree, source: &str, language: Language) -> Vec<ClassInfo> {
    let mut classes = Vec::new();
    let root = tree.root_node();

    match language {
        Language::Python => extract_python_classes_detailed(&root, source, &mut classes),
        Language::TypeScript | Language::JavaScript => {
            extract_ts_classes_detailed(&root, source, &mut classes)
        }
        Language::Rust => extract_rust_structs_detailed(&root, source, &mut classes),
        Language::Java => extract_java_classes_detailed(&root, source, &mut classes),
        Language::Cpp => extract_cpp_classes_detailed(&root, source, &mut classes),
        Language::Ruby => extract_ruby_classes_detailed(&root, source, &mut classes),
        Language::Php => extract_php_classes_detailed(&root, source, &mut classes),
        Language::CSharp => extract_csharp_classes_detailed(&root, source, &mut classes),
        Language::Kotlin => extract_kotlin_classes_detailed(&root, source, &mut classes),
        Language::Scala => extract_scala_classes_detailed(&root, source, &mut classes),
        Language::Elixir => extract_elixir_classes_detailed(&root, source, &mut classes),
        Language::Go => extract_go_structs_detailed(&root, source, &mut classes),
        Language::Swift => extract_swift_classes_detailed(&root, source, &mut classes),
        Language::C | Language::Lua | Language::Luau | Language::Ocaml => {} // No classes
    }

    classes
}

// =============================================================================
// Python detailed extraction
// =============================================================================

fn extract_python_functions_detailed(
    node: &Node,
    source: &str,
    functions: &mut Vec<FunctionInfo>,
    is_method: bool,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                // Check if this is inside a class
                let in_class = is_inside_class(&child);
                if in_class && !is_method {
                    continue; // Skip methods when extracting functions
                }
                if !in_class && is_method {
                    continue; // Skip functions when extracting methods
                }

                let info = extract_python_function_info(&child, source, in_class);
                functions.push(info);
            }
            "decorated_definition" => {
                // Handle decorated functions
                if let Some(def) = child.child_by_field_name("definition") {
                    if def.kind() == "function_definition" {
                        let in_class = is_inside_class(&child);
                        if (in_class && is_method) || (!in_class && !is_method) {
                            let mut info = extract_python_function_info(&def, source, in_class);
                            info.decorators = extract_decorators(&child, source);
                            functions.push(info);
                        }
                    }
                }
            }
            "class_definition" => {
                // Don't recurse into classes for top-level functions
                if !is_method {
                    continue;
                }
                if let Some(body) = child.child_by_field_name("body") {
                    extract_python_functions_detailed(&body, source, functions, true);
                }
            }
            _ => {
                if !is_method {
                    extract_python_functions_detailed(&child, source, functions, false);
                }
            }
        }
    }
}

fn extract_python_function_info(node: &Node, source: &str, is_method: bool) -> FunctionInfo {
    let name = node
        .child_by_field_name("name")
        .map(|n| get_node_text(&n, source))
        .unwrap_or_default();

    let params = extract_python_params(node, source);
    let return_type = node
        .child_by_field_name("return_type")
        .map(|n| get_node_text(&n, source));

    let docstring = extract_python_docstring(node, source);
    let is_async = node
        .prev_sibling()
        .map(|s| s.kind() == "async")
        .unwrap_or(false)
        || has_async_keyword(node, source);

    let line_number = node.start_position().row as u32 + 1;

    FunctionInfo {
        name,
        params,
        return_type,
        docstring,
        is_method,
        is_async,
        decorators: Vec::new(), // Set by caller for decorated functions
        line_number,
    }
}

fn extract_python_params(node: &Node, source: &str) -> Vec<String> {
    let mut params = Vec::new();

    if let Some(params_node) = node.child_by_field_name("parameters") {
        let mut cursor = params_node.walk();
        for child in params_node.children(&mut cursor) {
            match child.kind() {
                "identifier" => {
                    params.push(get_node_text(&child, source));
                }
                "typed_parameter" | "default_parameter" | "typed_default_parameter" => {
                    // The identifier is the first child, not a named field
                    let mut inner_cursor = child.walk();
                    for inner_child in child.children(&mut inner_cursor) {
                        if inner_child.kind() == "identifier" {
                            params.push(get_node_text(&inner_child, source));
                            break;
                        }
                    }
                }
                "list_splat_pattern" | "dictionary_splat_pattern" => {
                    params.push(get_node_text(&child, source));
                }
                _ => {}
            }
        }
    }

    params
}

fn extract_python_docstring(node: &Node, source: &str) -> Option<String> {
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        let mut children = body.children(&mut cursor);
        if let Some(child) = children.next() {
            if child.kind() == "expression_statement" {
                if let Some(expr) = child.child(0) {
                    if expr.kind() == "string" {
                        return Some(extract_string_content(&expr, source));
                    }
                }
            }
        }
    }
    None
}

fn extract_decorators(node: &Node, source: &str) -> Vec<String> {
    let mut decorators = Vec::new();
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "decorator" {
            let text = get_node_text(&child, source);
            // Remove leading @
            decorators.push(text.trim_start_matches('@').to_string());
        }
    }

    decorators
}

fn extract_python_classes_detailed(node: &Node, source: &str, classes: &mut Vec<ClassInfo>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_definition" => {
                let info = extract_python_class_info(&child, source);
                classes.push(info);
            }
            "decorated_definition" => {
                if let Some(def) = child.child_by_field_name("definition") {
                    if def.kind() == "class_definition" {
                        let mut info = extract_python_class_info(&def, source);
                        info.decorators = extract_decorators(&child, source);
                        classes.push(info);
                    }
                }
            }
            _ => {
                extract_python_classes_detailed(&child, source, classes);
            }
        }
    }
}

fn extract_python_class_info(node: &Node, source: &str) -> ClassInfo {
    let name = node
        .child_by_field_name("name")
        .map(|n| get_node_text(&n, source))
        .unwrap_or_default();

    let bases = extract_python_bases(node, source);
    let docstring = extract_python_class_docstring(node, source);
    let line_number = node.start_position().row as u32 + 1;

    // Extract methods
    let mut methods = Vec::new();
    if let Some(body) = node.child_by_field_name("body") {
        extract_python_functions_detailed(&body, source, &mut methods, true);
    }

    // Extract class fields (Gap 3)
    let fields = extract_python_class_fields(node, source);

    ClassInfo {
        name,
        bases,
        docstring,
        methods,
        fields,
        decorators: Vec::new(),
        line_number,
    }
}

fn extract_python_bases(node: &Node, source: &str) -> Vec<String> {
    let mut bases = Vec::new();

    if let Some(superclasses) = node.child_by_field_name("superclasses") {
        let mut cursor = superclasses.walk();
        for child in superclasses.children(&mut cursor) {
            if child.kind() == "identifier" || child.kind() == "attribute" {
                bases.push(get_node_text(&child, source));
            }
        }
    }

    bases
}

fn extract_python_class_docstring(node: &Node, source: &str) -> Option<String> {
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        let mut children = body.children(&mut cursor);
        if let Some(child) = children.next() {
            if child.kind() == "expression_statement" {
                if let Some(expr) = child.child(0) {
                    if expr.kind() == "string" {
                        return Some(extract_string_content(&expr, source));
                    }
                }
            }
        }
    }
    None
}

// =============================================================================
// Gap 3: Python class field extraction
// =============================================================================

/// Helper: check if a name is UPPER_CASE (constant convention)
pub(crate) fn is_upper_case_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_uppercase() || c == '_' || c.is_ascii_digit())
        && name.chars().any(|c| c.is_alphabetic())
}

/// Extract class-level fields and __init__ self.x assignments from a Python class
fn extract_python_class_fields(node: &Node, source: &str) -> Vec<FieldInfo> {
    let mut fields = Vec::new();

    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            match child.kind() {
                "expression_statement" => {
                    // Class-level assignment: x = 10 or x: int = 5
                    if let Some(inner) = child.child(0) {
                        if inner.kind() == "assignment" {
                            if let Some(field) =
                                extract_python_field_from_assignment(&inner, source, true)
                            {
                                fields.push(field);
                            }
                        }
                    }
                }
                "function_definition" | "decorated_definition" => {
                    // Check for __init__ and extract self.x assignments
                    let def_node = if child.kind() == "decorated_definition" {
                        child.child_by_field_name("definition")
                    } else {
                        Some(child)
                    };
                    if let Some(def) = def_node {
                        if def.kind() == "function_definition" {
                            let fname = def
                                .child_by_field_name("name")
                                .map(|n| get_node_text(&n, source));
                            if fname.as_deref() == Some("__init__") {
                                extract_python_init_fields(&def, source, &mut fields);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fields
}

/// Extract a field from a Python assignment node (class-level)
fn extract_python_field_from_assignment(
    node: &Node,
    source: &str,
    is_static: bool,
) -> Option<FieldInfo> {
    // Assignment: left = right  OR  left: type = right
    let left = node.child_by_field_name("left")?;

    // Only handle simple identifiers (not self.x or tuple unpacking)
    if left.kind() != "identifier" {
        return None;
    }

    let name = get_node_text(&left, source);

    // Extract type annotation if present
    let field_type = node
        .child_by_field_name("type")
        .map(|n| get_node_text(&n, source));

    // Extract default value
    let default_value = node
        .child_by_field_name("right")
        .map(|n| get_node_text(&n, source));

    let is_constant = is_upper_case_name(&name);
    let visibility = if name.starts_with('_') {
        Some("private".to_string())
    } else {
        Some("public".to_string())
    };
    let line_number = node.start_position().row as u32 + 1;

    Some(FieldInfo {
        name,
        field_type,
        default_value,
        is_static,
        is_constant,
        visibility,
        line_number,
    })
}

/// Extract self.x assignments from __init__ method body
fn extract_python_init_fields(init_node: &Node, source: &str, fields: &mut Vec<FieldInfo>) {
    if let Some(body) = init_node.child_by_field_name("body") {
        extract_python_self_assignments(&body, source, fields);
    }
}

/// Recursively walk a function body to find self.x = ... assignments
fn extract_python_self_assignments(node: &Node, source: &str, fields: &mut Vec<FieldInfo>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "expression_statement" {
            if let Some(inner) = child.child(0) {
                if inner.kind() == "assignment" {
                    if let Some(left) = inner.child_by_field_name("left") {
                        if left.kind() == "attribute" {
                            // Check if it's self.x
                            if let Some(obj) = left.child_by_field_name("object") {
                                if get_node_text(&obj, source) == "self" {
                                    if let Some(attr) = left.child_by_field_name("attribute") {
                                        let name = get_node_text(&attr, source);
                                        let default_value = inner
                                            .child_by_field_name("right")
                                            .map(|n| get_node_text(&n, source));
                                        let visibility = if name.starts_with('_') {
                                            Some("private".to_string())
                                        } else {
                                            Some("public".to_string())
                                        };
                                        let line_number = inner.start_position().row as u32 + 1;

                                        fields.push(FieldInfo {
                                            name,
                                            field_type: None,
                                            default_value,
                                            is_static: false,
                                            is_constant: false,
                                            visibility,
                                            line_number,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        // Recurse into if/for/try blocks inside __init__
        extract_python_self_assignments(&child, source, fields);
    }
}

// =============================================================================
// Gap 3: Module-level constants extraction
// =============================================================================

/// Extract module-level constants for all languages
fn extract_module_constants(tree: &Tree, source: &str, language: Language) -> Vec<FieldInfo> {
    let root = tree.root_node();
    match language {
        Language::Python => extract_python_module_constants(&root, source),
        Language::Rust => extract_rust_module_constants(&root, source),
        Language::Go => extract_go_module_constants(&root, source),
        Language::TypeScript | Language::JavaScript => extract_ts_module_constants(&root, source),
        Language::Java => Vec::new(), // Java constants are always in classes
        Language::C => extract_c_module_constants(&root, source),
        Language::Cpp => extract_cpp_module_constants(&root, source),
        Language::Ruby => extract_ruby_module_constants(&root, source),
        Language::Kotlin => extract_kotlin_module_constants(&root, source),
        Language::Swift => extract_swift_module_constants(&root, source),
        Language::CSharp => extract_csharp_module_constants(&root, source),
        Language::Scala => extract_scala_module_constants(&root, source),
        Language::Php => extract_php_module_constants(&root, source),
        Language::Lua => extract_lua_module_constants(&root, source),
        Language::Luau => extract_luau_module_constants(&root, source),
        Language::Elixir => extract_elixir_module_constants(&root, source),
        Language::Ocaml => extract_ocaml_module_constants(&root, source),
    }
}

/// Extract UPPER_CASE top-level assignments as Python module constants
fn extract_python_module_constants(root: &Node, source: &str) -> Vec<FieldInfo> {
    let mut constants = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "expression_statement" {
            if let Some(inner) = child.child(0) {
                if inner.kind() == "assignment" {
                    if let Some(left) = inner.child_by_field_name("left") {
                        if left.kind() == "identifier" {
                            let name = get_node_text(&left, source);
                            if is_upper_case_name(&name) {
                                let default_value = inner
                                    .child_by_field_name("right")
                                    .map(|n| get_node_text(&n, source));
                                let field_type = inner
                                    .child_by_field_name("type")
                                    .map(|n| get_node_text(&n, source));
                                let line_number = inner.start_position().row as u32 + 1;
                                constants.push(FieldInfo {
                                    name,
                                    field_type,
                                    default_value,
                                    is_static: true,
                                    is_constant: true,
                                    visibility: None,
                                    line_number,
                                });
                            }
                        }
                    }
                }
            }
        }
    }
    constants
}

/// Extract const/static items as Rust module constants
fn extract_rust_module_constants(root: &Node, source: &str) -> Vec<FieldInfo> {
    let mut constants = Vec::new();
    extract_rust_constants_recursive(root, source, &mut constants);
    constants
}

fn extract_rust_constants_recursive(node: &Node, source: &str, constants: &mut Vec<FieldInfo>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "const_item" => {
                if let Some(field) = extract_rust_const_or_static(&child, source, true) {
                    constants.push(field);
                }
            }
            "static_item" => {
                if let Some(field) = extract_rust_const_or_static(&child, source, false) {
                    constants.push(field);
                }
            }
            _ => {
                // Don't recurse into functions/impl blocks
                if child.kind() != "function_item"
                    && child.kind() != "impl_item"
                    && child.kind() != "struct_item"
                {
                    extract_rust_constants_recursive(&child, source, constants);
                }
            }
        }
    }
}

fn extract_rust_const_or_static(node: &Node, source: &str, is_const: bool) -> Option<FieldInfo> {
    let name = node
        .child_by_field_name("name")
        .map(|n| get_node_text(&n, source))?;

    let field_type = node
        .child_by_field_name("type")
        .map(|n| get_node_text(&n, source));

    let default_value = node
        .child_by_field_name("value")
        .map(|n| get_node_text(&n, source));

    let visibility = node
        .children(&mut node.walk())
        .find(|c| c.kind() == "visibility_modifier")
        .map(|n| {
            let text = get_node_text(&n, source);
            if text == "pub" {
                "public".to_string()
            } else {
                text
            }
        })
        .or_else(|| Some("private".to_string()));

    let line_number = node.start_position().row as u32 + 1;

    Some(FieldInfo {
        name,
        field_type,
        default_value,
        is_static: !is_const, // static items are is_static, const items are not (they're inlined)
        is_constant: true,
        visibility,
        line_number,
    })
}

/// Extract Go const declarations as module constants
fn extract_go_module_constants(root: &Node, source: &str) -> Vec<FieldInfo> {
    let mut constants = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "const_declaration" {
            let mut spec_cursor = child.walk();
            for spec in child.children(&mut spec_cursor) {
                if spec.kind() == "const_spec" {
                    if let Some(field) = extract_go_const_spec(&spec, source) {
                        constants.push(field);
                    }
                }
            }
        } else if child.kind() == "var_declaration" {
            // Go package-level var declarations: `var X = value` or `var ( ... )`
            // Single var: var_declaration -> var_spec (direct child)
            // Grouped var: var_declaration -> var_spec_list -> var_spec (nested)
            extract_go_var_specs(&child, source, &mut constants);
        }
    }
    constants
}

fn extract_go_const_spec(node: &Node, source: &str) -> Option<FieldInfo> {
    let name = node
        .child_by_field_name("name")
        .map(|n| get_node_text(&n, source))?;

    let field_type = node
        .child_by_field_name("type")
        .map(|n| get_node_text(&n, source));

    let default_value = node
        .child_by_field_name("value")
        .map(|n| get_node_text(&n, source));

    let visibility = if name
        .chars()
        .next()
        .map(|c| c.is_uppercase())
        .unwrap_or(false)
    {
        Some("public".to_string())
    } else {
        Some("private".to_string())
    };

    let line_number = node.start_position().row as u32 + 1;

    Some(FieldInfo {
        name,
        field_type,
        default_value,
        is_static: true,
        is_constant: true,
        visibility,
        line_number,
    })
}

/// Recursively extract var_spec nodes from a var_declaration or var_spec_list.
///
/// Handles both single `var X = value` (var_spec is a direct child of
/// var_declaration) and grouped `var ( ... )` (var_spec nodes are inside
/// a var_spec_list wrapper).
fn extract_go_var_specs(node: &Node, source: &str, constants: &mut Vec<FieldInfo>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "var_spec" {
            if let Some(field) = extract_go_var_spec(&child, source) {
                constants.push(field);
            }
        } else if child.kind() == "var_spec_list" {
            // Grouped var block: recurse into the list
            extract_go_var_specs(&child, source, constants);
        }
    }
}

/// Extract a single Go var_spec as a FieldInfo.
///
/// Go var_spec AST structure (parallel to const_spec):
/// ```text
/// var_spec
///   name: identifier "ErrNotFound"
///   type: type_identifier? "error"
///   value: expression_list? (call_expression ...)
/// ```
fn extract_go_var_spec(node: &Node, source: &str) -> Option<FieldInfo> {
    let name = node
        .child_by_field_name("name")
        .map(|n| get_node_text(&n, source))?;

    let field_type = node
        .child_by_field_name("type")
        .map(|n| get_node_text(&n, source));

    let default_value = node
        .child_by_field_name("value")
        .map(|n| get_node_text(&n, source));

    let visibility = if name
        .chars()
        .next()
        .map(|c| c.is_uppercase())
        .unwrap_or(false)
    {
        Some("public".to_string())
    } else {
        Some("private".to_string())
    };

    let line_number = node.start_position().row as u32 + 1;

    Some(FieldInfo {
        name,
        field_type,
        default_value,
        is_static: true,
        is_constant: false, // var, not const
        visibility,
        line_number,
    })
}

/// Extract UPPER_CASE const declarations as TS/JS module constants
fn extract_ts_module_constants(root: &Node, source: &str) -> Vec<FieldInfo> {
    let mut constants = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "lexical_declaration" {
            // Check if it's a `const` declaration
            let text = get_node_text(&child, source);
            if text.starts_with("const ") {
                let mut decl_cursor = child.walk();
                for decl_child in child.children(&mut decl_cursor) {
                    if decl_child.kind() == "variable_declarator" {
                        let name = decl_child
                            .child_by_field_name("name")
                            .map(|n| get_node_text(&n, source));
                        if let Some(name) = name {
                            if is_upper_case_name(&name) {
                                let default_value = decl_child
                                    .child_by_field_name("value")
                                    .map(|n| get_node_text(&n, source));
                                let line_number = decl_child.start_position().row as u32 + 1;
                                constants.push(FieldInfo {
                                    name,
                                    field_type: None,
                                    default_value,
                                    is_static: true,
                                    is_constant: true,
                                    visibility: None,
                                    line_number,
                                });
                            }
                        }
                    }
                }
            }
        } else if child.kind() == "export_statement" {
            // Handle: export const X = ...
            let mut export_cursor = child.walk();
            for export_child in child.children(&mut export_cursor) {
                if export_child.kind() == "lexical_declaration" {
                    let text = get_node_text(&export_child, source);
                    if text.starts_with("const ") {
                        let mut decl_cursor = export_child.walk();
                        for decl_child in export_child.children(&mut decl_cursor) {
                            if decl_child.kind() == "variable_declarator" {
                                let name = decl_child
                                    .child_by_field_name("name")
                                    .map(|n| get_node_text(&n, source));
                                if let Some(name) = name {
                                    if is_upper_case_name(&name) {
                                        let default_value = decl_child
                                            .child_by_field_name("value")
                                            .map(|n| get_node_text(&n, source));
                                        let line_number =
                                            decl_child.start_position().row as u32 + 1;
                                        constants.push(FieldInfo {
                                            name,
                                            field_type: None,
                                            default_value,
                                            is_static: true,
                                            is_constant: true,
                                            visibility: None,
                                            line_number,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    constants
}

/// Extract C module constants: `#define UPPER_CASE value` and `const type UPPER_CASE = value;`
fn extract_c_module_constants(root: &Node, source: &str) -> Vec<FieldInfo> {
    extract_c_cpp_module_constants(root, source, &["const"])
}

/// Extract C++ module constants: same as C plus `constexpr` declarations
fn extract_cpp_module_constants(root: &Node, source: &str) -> Vec<FieldInfo> {
    extract_c_cpp_module_constants(root, source, &["const", "constexpr"])
}

/// Shared extraction for C and C++ module constants.
///
/// Handles `#define UPPER_CASE value` and `declaration` nodes with const/constexpr qualifiers.
fn extract_c_cpp_module_constants(
    root: &Node,
    source: &str,
    const_qualifiers: &[&str],
) -> Vec<FieldInfo> {
    let mut constants = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        match child.kind() {
            "preproc_def" => {
                // #define NAME value -- children: #define, identifier, preproc_arg
                extract_preproc_def_constant(&child, source, &mut constants);
            }
            "declaration" => {
                // Check for a type_qualifier matching one of the allowed qualifiers
                let has_qualifier = {
                    let mut inner_cursor = child.walk();
                    let mut found = false;
                    for c in child.children(&mut inner_cursor) {
                        if c.kind() == "type_qualifier" {
                            let text = get_node_text(&c, source);
                            if const_qualifiers.contains(&text.as_str()) {
                                found = true;
                                break;
                            }
                        }
                    }
                    found
                };
                if has_qualifier {
                    extract_c_const_declaration(&child, source, &mut constants);
                }
            }
            _ => {}
        }
    }
    constants
}

/// Extract an UPPER_CASE `#define` preprocessor constant
fn extract_preproc_def_constant(node: &Node, source: &str, constants: &mut Vec<FieldInfo>) {
    let mut inner_cursor = node.walk();
    let mut name = None;
    let mut default_value = None;
    for inner in node.children(&mut inner_cursor) {
        match inner.kind() {
            "identifier" => name = Some(get_node_text(&inner, source)),
            "preproc_arg" => default_value = Some(get_node_text(&inner, source).trim().to_string()),
            _ => {}
        }
    }
    if let Some(name) = name {
        if is_upper_case_name(&name) {
            let line_number = node.start_position().row as u32 + 1;
            constants.push(FieldInfo {
                name,
                field_type: None,
                default_value,
                is_static: true,
                is_constant: true,
                visibility: None,
                line_number,
            });
        }
    }
}

/// Extract name and value from a C/C++ `const`/`constexpr` declaration via init_declarator
fn extract_c_const_declaration(node: &Node, source: &str, constants: &mut Vec<FieldInfo>) {
    let field_type = node
        .child_by_field_name("type")
        .map(|n| get_node_text(&n, source));

    // Find init_declarator children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "init_declarator" {
            // Fields: declarator (identifier), value (literal)
            let name = child
                .child_by_field_name("declarator")
                .map(|n| get_node_text(&n, source));
            let default_value = child
                .child_by_field_name("value")
                .map(|n| get_node_text(&n, source));

            if let Some(name) = name {
                if is_upper_case_name(&name) {
                    let line_number = node.start_position().row as u32 + 1;
                    constants.push(FieldInfo {
                        name,
                        field_type: field_type.clone(),
                        default_value,
                        is_static: true,
                        is_constant: true,
                        visibility: None,
                        line_number,
                    });
                }
            }
        }
    }
}

/// Extract Ruby module constants: UPPER_CASE assignments at top level
///
/// In Ruby, constants are identifiers starting with uppercase. The tree-sitter
/// grammar uses `constant` node kind (vs `identifier` for lowercase).
fn extract_ruby_module_constants(root: &Node, source: &str) -> Vec<FieldInfo> {
    let mut constants = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "assignment" {
            // Left child is either `constant` (UPPER_CASE) or `identifier` (lowercase)
            let mut inner_cursor = child.walk();
            let mut left_node = None;
            let mut right_text = None;
            let mut seen_equals = false;
            for inner in child.children(&mut inner_cursor) {
                if !seen_equals && inner.kind() == "constant" {
                    left_node = Some(inner);
                } else if inner.kind() == "=" {
                    seen_equals = true;
                } else if seen_equals && right_text.is_none() {
                    right_text = Some(get_node_text(&inner, source));
                }
            }
            if let Some(left) = left_node {
                let name = get_node_text(&left, source);
                if is_upper_case_name(&name) {
                    let line_number = child.start_position().row as u32 + 1;
                    constants.push(FieldInfo {
                        name,
                        field_type: None,
                        default_value: right_text,
                        is_static: true,
                        is_constant: true,
                        visibility: None,
                        line_number,
                    });
                }
            }
        }
    }
    constants
}

/// Extract Kotlin module constants: `const val NAME`, `val UPPER_NAME`
///
/// AST: property_declaration > [modifiers("const")] [val/var] variable_declaration > simple_identifier
fn extract_kotlin_module_constants(root: &Node, source: &str) -> Vec<FieldInfo> {
    let mut constants = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "property_declaration" {
            let mut inner_cursor = child.walk();
            let mut is_val = false;
            let mut has_const_modifier = false;
            let mut name = None;
            let mut default_value = None;
            let mut seen_equals = false;

            for inner in child.children(&mut inner_cursor) {
                match inner.kind() {
                    "modifiers" => {
                        let mod_text = get_node_text(&inner, source);
                        if mod_text.contains("const") {
                            has_const_modifier = true;
                        }
                    }
                    "val" => is_val = true,
                    "variable_declaration" => {
                        // The variable_declaration contains an identifier child
                        let mut var_cursor = inner.walk();
                        for var_child in inner.children(&mut var_cursor) {
                            if var_child.kind() == "identifier"
                                || var_child.kind() == "simple_identifier"
                            {
                                name = Some(get_node_text(&var_child, source));
                            }
                        }
                    }
                    "=" => seen_equals = true,
                    _ => {
                        if seen_equals && default_value.is_none() {
                            default_value = Some(get_node_text(&inner, source));
                        }
                    }
                }
            }

            if let Some(name) = name {
                // Extract if: const val (explicit const), or val with UPPER_CASE name
                if has_const_modifier || (is_val && is_upper_case_name(&name)) {
                    let line_number = child.start_position().row as u32 + 1;
                    constants.push(FieldInfo {
                        name,
                        field_type: None,
                        default_value,
                        is_static: true,
                        is_constant: true,
                        visibility: None,
                        line_number,
                    });
                }
            }
        }
    }
    constants
}

/// Extract Swift module constants: `let UPPER_NAME = value`
///
/// AST: property_declaration > value_binding_pattern("let"/"var") + pattern > simple_identifier
fn extract_swift_module_constants(root: &Node, source: &str) -> Vec<FieldInfo> {
    let mut constants = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "property_declaration" {
            let mut inner_cursor = child.walk();
            let mut is_let = false;
            let mut name = None;
            let mut default_value = None;
            let mut seen_equals = false;

            for inner in child.children(&mut inner_cursor) {
                match inner.kind() {
                    "value_binding_pattern" => {
                        let text = get_node_text(&inner, source);
                        is_let = text == "let";
                    }
                    "pattern" => {
                        // Contains simple_identifier
                        let mut pat_cursor = inner.walk();
                        for pat_child in inner.children(&mut pat_cursor) {
                            if pat_child.kind() == "simple_identifier" {
                                name = Some(get_node_text(&pat_child, source));
                            }
                        }
                    }
                    "=" => seen_equals = true,
                    _ => {
                        if seen_equals && default_value.is_none() {
                            default_value = Some(get_node_text(&inner, source));
                        }
                    }
                }
            }

            if let Some(name) = name {
                if is_let && is_upper_case_name(&name) {
                    let line_number = child.start_position().row as u32 + 1;
                    constants.push(FieldInfo {
                        name,
                        field_type: None,
                        default_value,
                        is_static: true,
                        is_constant: true,
                        visibility: None,
                        line_number,
                    });
                }
            }
        }
    }
    constants
}

/// Extract C# module constants: `const type NAME = value;` at top-level
///
/// AST: global_statement > local_declaration_statement > modifier("const") +
///      variable_declaration > variable_declarator(name=identifier)
fn extract_csharp_module_constants(root: &Node, source: &str) -> Vec<FieldInfo> {
    let mut constants = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "global_statement" {
            // Look for local_declaration_statement with const modifier
            let mut stmt_cursor = child.walk();
            for stmt_child in child.children(&mut stmt_cursor) {
                if stmt_child.kind() == "local_declaration_statement" {
                    let has_const = {
                        let mut mod_cursor = stmt_child.walk();
                        let mut found = false;
                        for c in stmt_child.children(&mut mod_cursor) {
                            if c.kind() == "modifier" && get_node_text(&c, source).contains("const")
                            {
                                found = true;
                                break;
                            }
                        }
                        found
                    };
                    if has_const {
                        extract_csharp_const_from_declaration(&stmt_child, source, &mut constants);
                    }
                }
            }
        }
    }
    constants
}

/// Extract variable names from a C# local_declaration_statement with const modifier
fn extract_csharp_const_from_declaration(
    node: &Node,
    source: &str,
    constants: &mut Vec<FieldInfo>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declaration" {
            let field_type = child
                .child_by_field_name("type")
                .map(|n| get_node_text(&n, source));

            let mut decl_cursor = child.walk();
            for decl_child in child.children(&mut decl_cursor) {
                if decl_child.kind() == "variable_declarator" {
                    let name = decl_child
                        .child_by_field_name("name")
                        .map(|n| get_node_text(&n, source));

                    if let Some(name) = name {
                        if is_upper_case_name(&name) {
                            // Get the value after the equals sign
                            let default_value = {
                                let mut val_cursor = decl_child.walk();
                                let mut found_eq = false;
                                let mut val = None;
                                for vc in decl_child.children(&mut val_cursor) {
                                    if vc.kind() == "=" {
                                        found_eq = true;
                                    } else if found_eq && val.is_none() {
                                        val = Some(get_node_text(&vc, source));
                                    }
                                }
                                val
                            };
                            let line_number = node.start_position().row as u32 + 1;
                            constants.push(FieldInfo {
                                name,
                                field_type: field_type.clone(),
                                default_value,
                                is_static: true,
                                is_constant: true,
                                visibility: None,
                                line_number,
                            });
                        }
                    }
                }
            }
        }
    }
}

/// Extract Scala module constants: `val UPPER_NAME = value` at top level
///
/// AST: val_definition > identifier + value, var_definition > identifier + value
fn extract_scala_module_constants(root: &Node, source: &str) -> Vec<FieldInfo> {
    let mut constants = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "val_definition" {
            // val_definition children: val, identifier, =, literal
            let mut inner_cursor = child.walk();
            let mut name = None;
            let mut default_value = None;
            let mut seen_equals = false;

            for inner in child.children(&mut inner_cursor) {
                match inner.kind() {
                    "identifier" if name.is_none() => {
                        name = Some(get_node_text(&inner, source));
                    }
                    "=" => seen_equals = true,
                    _ => {
                        if seen_equals && default_value.is_none() && inner.kind() != "val" {
                            default_value = Some(get_node_text(&inner, source));
                        }
                    }
                }
            }

            if let Some(name) = name {
                if is_upper_case_name(&name) {
                    let line_number = child.start_position().row as u32 + 1;
                    constants.push(FieldInfo {
                        name,
                        field_type: None,
                        default_value,
                        is_static: true,
                        is_constant: true,
                        visibility: None,
                        line_number,
                    });
                }
            }
        }
        // var_definition is mutable, so we skip it
    }
    constants
}

/// Extract PHP module constants: `const NAME = value;` and `define('NAME', value);`
///
/// AST: const_declaration > const_element > name; expression_statement > function_call_expression
fn extract_php_module_constants(root: &Node, source: &str) -> Vec<FieldInfo> {
    let mut constants = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        match child.kind() {
            "const_declaration" => {
                // const NAME = value;
                let mut inner_cursor = child.walk();
                for inner in child.children(&mut inner_cursor) {
                    if inner.kind() == "const_element" {
                        let mut elem_cursor = inner.walk();
                        let mut name = None;
                        let mut default_value = None;
                        let mut seen_equals = false;
                        for elem in inner.children(&mut elem_cursor) {
                            match elem.kind() {
                                "name" => name = Some(get_node_text(&elem, source)),
                                "=" => seen_equals = true,
                                _ => {
                                    if seen_equals && default_value.is_none() {
                                        default_value = Some(get_node_text(&elem, source));
                                    }
                                }
                            }
                        }
                        if let Some(name) = name {
                            let line_number = child.start_position().row as u32 + 1;
                            constants.push(FieldInfo {
                                name,
                                field_type: None,
                                default_value,
                                is_static: true,
                                is_constant: true,
                                visibility: None,
                                line_number,
                            });
                        }
                    }
                }
            }
            "expression_statement" => {
                // define('NAME', value);
                let mut inner_cursor = child.walk();
                for inner in child.children(&mut inner_cursor) {
                    if inner.kind() == "function_call_expression" {
                        let func_name = inner
                            .child_by_field_name("function")
                            .map(|n| get_node_text(&n, source));
                        if func_name.as_deref() == Some("define") {
                            if let Some(args) = inner.child_by_field_name("arguments") {
                                extract_php_define_call(&args, source, &mut constants, &child);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    constants
}

/// Extract name and value from a PHP define('NAME', value) call arguments
fn extract_php_define_call(
    args: &Node,
    source: &str,
    constants: &mut Vec<FieldInfo>,
    parent: &Node,
) {
    // Arguments: ( argument(string('NAME')) , argument(value) )
    let mut arg_cursor = args.walk();
    let mut first_arg = None;
    let mut second_arg = None;
    let mut arg_count = 0;
    for arg in args.children(&mut arg_cursor) {
        if arg.kind() == "argument" {
            match arg_count {
                0 => first_arg = Some(arg),
                1 => second_arg = Some(arg),
                _ => {}
            }
            arg_count += 1;
        }
    }

    if let Some(first) = first_arg {
        // The first argument should be a string containing the constant name
        let full_text = get_node_text(&first, source);
        // Strip quotes: 'NAME' or "NAME"
        let name = full_text
            .trim_matches(|c| c == '\'' || c == '"')
            .to_string();

        if !name.is_empty() {
            let default_value = second_arg.map(|a| get_node_text(&a, source));
            let line_number = parent.start_position().row as u32 + 1;
            constants.push(FieldInfo {
                name,
                field_type: None,
                default_value,
                is_static: true,
                is_constant: true,
                visibility: None,
                line_number,
            });
        }
    }
}

/// Extract Lua module constants: UPPER_CASE assignments at top level
///
/// AST: assignment_statement > variable_list + expression_list
///      variable_declaration > local + assignment_statement
fn extract_lua_module_constants(root: &Node, source: &str) -> Vec<FieldInfo> {
    let mut constants = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        match child.kind() {
            "assignment_statement" => {
                // Top-level assignment: NAME = value
                extract_lua_constant_from_assignment(&child, source, &mut constants);
            }
            "variable_declaration" => {
                // local NAME = value
                // Contains assignment_statement as child
                let mut inner_cursor = child.walk();
                for inner in child.children(&mut inner_cursor) {
                    if inner.kind() == "assignment_statement" {
                        extract_lua_constant_from_assignment(&inner, source, &mut constants);
                    }
                }
            }
            _ => {}
        }
    }
    constants
}

/// Extract a constant from a Lua assignment_statement if the LHS is UPPER_CASE
fn extract_lua_constant_from_assignment(node: &Node, source: &str, constants: &mut Vec<FieldInfo>) {
    let mut cursor = node.walk();
    let mut var_list = None;
    let mut expr_list = None;
    for child in node.children(&mut cursor) {
        match child.kind() {
            "variable_list" => var_list = Some(child),
            "expression_list" => expr_list = Some(child),
            _ => {}
        }
    }

    if let Some(vars) = var_list {
        let mut var_cursor = vars.walk();
        for var_child in vars.children(&mut var_cursor) {
            if var_child.kind() == "identifier" {
                let name = get_node_text(&var_child, source);
                if is_upper_case_name(&name) {
                    let default_value = expr_list.as_ref().map(|e| get_node_text(e, source));
                    let line_number = node.start_position().row as u32 + 1;
                    constants.push(FieldInfo {
                        name,
                        field_type: None,
                        default_value,
                        is_static: true,
                        is_constant: true,
                        visibility: None,
                        line_number,
                    });
                }
            }
        }
    }
}

/// Extract Luau module constants: `local UPPER_NAME = value` at top level
///
/// Luau uses same AST as Lua: variable_declaration > local + assignment_statement
fn extract_luau_module_constants(root: &Node, source: &str) -> Vec<FieldInfo> {
    // Luau and Lua share the same AST structure for variable declarations
    extract_lua_module_constants(root, source)
}

/// Extract Elixir module constants: `@UPPER_NAME value` module attributes
///
/// AST: unary_operator(operator=@, operand=alias("UPPER_NAME"))
/// The value is the next sibling node.
fn extract_elixir_module_constants(root: &Node, source: &str) -> Vec<FieldInfo> {
    let mut constants = Vec::new();
    let child_count = root.child_count();
    for i in 0..child_count {
        if let Some(child) = root.child(i) {
            if child.kind() == "unary_operator" {
                let operator = child.child_by_field_name("operator");
                let operand = child.child_by_field_name("operand");

                if let (Some(op), Some(name_node)) = (operator, operand) {
                    if get_node_text(&op, source) == "@" {
                        let name = get_node_text(&name_node, source);
                        if is_upper_case_name(&name) {
                            // The value is the next sibling
                            let default_value =
                                root.child(i + 1).map(|n| get_node_text(&n, source));
                            let line_number = child.start_position().row as u32 + 1;
                            constants.push(FieldInfo {
                                name,
                                field_type: None,
                                default_value,
                                is_static: true,
                                is_constant: true,
                                visibility: None,
                                line_number,
                            });
                        }
                    }
                }
            }
        }
    }
    constants
}

/// Extract OCaml module constants: `let UPPER_NAME = value` at top level
///
/// AST: value_definition > let_binding(pattern=constructor_path/value_name, body=expr)
/// UPPER_CASE names are parsed as constructor_path > constructor_name.
fn extract_ocaml_module_constants(root: &Node, source: &str) -> Vec<FieldInfo> {
    let mut constants = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "value_definition" {
            let mut inner_cursor = child.walk();
            for inner in child.children(&mut inner_cursor) {
                if inner.kind() == "let_binding" {
                    // Only extract non-function bindings (no parameter children)
                    if ocaml_binding_has_params(&inner) {
                        continue;
                    }

                    // Get the pattern (name)
                    let name = inner
                        .child_by_field_name("pattern")
                        .map(|n| get_node_text(&n, source));

                    let default_value = inner
                        .child_by_field_name("body")
                        .map(|n| get_node_text(&n, source));

                    if let Some(name) = name {
                        if is_upper_case_name(&name) {
                            let line_number = child.start_position().row as u32 + 1;
                            constants.push(FieldInfo {
                                name,
                                field_type: None,
                                default_value,
                                is_static: true,
                                is_constant: true,
                                visibility: None,
                                line_number,
                            });
                        }
                    }
                }
            }
        }
    }
    constants
}

// =============================================================================
// TypeScript detailed extraction
// =============================================================================

fn extract_ts_functions_detailed(
    node: &Node,
    source: &str,
    functions: &mut Vec<FunctionInfo>,
    is_method: bool,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                if !is_method {
                    let info = extract_ts_function_info(&child, source, false);
                    functions.push(info);
                }
            }
            "method_definition" | "method_signature" => {
                if is_method {
                    let info = extract_ts_function_info(&child, source, true);
                    functions.push(info);
                }
            }
            "class_declaration" | "class" => {
                if is_method {
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_ts_functions_detailed(&body, source, functions, true);
                    }
                }
            }
            "lexical_declaration" | "variable_declaration" => {
                // Handle: const foo = () => {} or const foo = function() {}
                if !is_method {
                    extract_ts_variable_functions(&child, source, functions);
                }
                // Also recurse for nested declarations
                extract_ts_functions_detailed(&child, source, functions, is_method);
            }
            "export_statement" => {
                // Handle: export const foo = () => {} and export function foo() {}
                extract_ts_functions_detailed(&child, source, functions, is_method);
            }
            _ => {
                extract_ts_functions_detailed(&child, source, functions, is_method);
            }
        }
    }
}

/// Extract functions from variable declarations with arrow function or function expression values.
/// Handles patterns like: `const foo = () => {}`, `const foo = function() {}`,
/// `export const foo = async () => {}`
fn extract_ts_variable_functions(node: &Node, source: &str, functions: &mut Vec<FunctionInfo>) {
    // Extract docstring from the declaration node (JSDoc sits before the
    // lexical_declaration / variable_declaration, not the inner declarator).
    let decl_docstring = extract_jsdoc_docstring(node, source);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            let name = child
                .child_by_field_name("name")
                .map(|n| get_node_text(&n, source))
                .unwrap_or_default();

            if name.is_empty() {
                continue;
            }

            let value = child.child_by_field_name("value");
            if let Some(val) = value {
                let is_func = matches!(
                    val.kind(),
                    "arrow_function" | "function_expression" | "function"
                );
                if is_func {
                    let params = extract_ts_arrow_params(&val, source);
                    let return_type = val.child_by_field_name("return_type").map(|n| {
                        get_node_text(&n, source)
                            .trim_start_matches(':')
                            .trim()
                            .to_string()
                    });
                    let is_async = get_node_text(&val, source).starts_with("async")
                        || get_node_text(&child, source).starts_with("async");
                    let line_number = child.start_position().row as u32 + 1;

                    functions.push(FunctionInfo {
                        name,
                        params,
                        return_type,
                        docstring: decl_docstring.clone(),
                        is_method: false,
                        is_async,
                        decorators: Vec::new(),
                        line_number,
                    });
                }
            }
        }
    }
}

/// Extract parameters from an arrow function or function expression node.
fn extract_ts_arrow_params(node: &Node, source: &str) -> Vec<String> {
    let mut params = Vec::new();

    if let Some(params_node) = node.child_by_field_name("parameters") {
        let mut cursor = params_node.walk();
        for child in params_node.children(&mut cursor) {
            match child.kind() {
                "required_parameter" | "optional_parameter" => {
                    if let Some(pattern) = child.child_by_field_name("pattern") {
                        params.push(get_node_text(&pattern, source));
                    }
                }
                "identifier" => {
                    // Simple arrow function params: (x) => {} or x => {}
                    params.push(get_node_text(&child, source));
                }
                _ => {}
            }
        }
    } else if let Some(param) = node.child_by_field_name("parameter") {
        // Single-param arrow: x => {}
        params.push(get_node_text(&param, source));
    }

    params
}

fn extract_ts_function_info(node: &Node, source: &str, is_method: bool) -> FunctionInfo {
    let name = node
        .child_by_field_name("name")
        .map(|n| get_node_text(&n, source))
        .unwrap_or_default();

    let params = extract_ts_params(node, source);
    let return_type = node.child_by_field_name("return_type").map(|n| {
        get_node_text(&n, source)
            .trim_start_matches(':')
            .trim()
            .to_string()
    });

    let is_async = get_node_text(node, source).starts_with("async");
    let line_number = node.start_position().row as u32 + 1;

    FunctionInfo {
        name,
        params,
        return_type,
        docstring: extract_jsdoc_docstring(node, source),
        is_method,
        is_async,
        decorators: Vec::new(),
        line_number,
    }
}

fn extract_ts_params(node: &Node, source: &str) -> Vec<String> {
    let mut params = Vec::new();

    if let Some(params_node) = node.child_by_field_name("parameters") {
        let mut cursor = params_node.walk();
        for child in params_node.children(&mut cursor) {
            if child.kind() == "required_parameter" || child.kind() == "optional_parameter" {
                if let Some(pattern) = child.child_by_field_name("pattern") {
                    params.push(get_node_text(&pattern, source));
                }
            }
        }
    }

    params
}

/// Extract JSDoc comments (`/** */`) from preceding sibling nodes.
///
/// Walks up through wrapping constructs (export_statement, lexical_declaration)
/// to find the doc comment even when the declaration is nested.
fn extract_jsdoc_docstring(node: &Node, source: &str) -> Option<String> {
    let mut target = *node;
    for _ in 0..3 {
        if let Some(doc) = try_jsdoc_prev_sibling(&target, source) {
            return Some(doc);
        }
        if let Some(parent) = target.parent() {
            if matches!(
                parent.kind(),
                "export_statement"
                    | "lexical_declaration"
                    | "variable_declaration"
                    | "variable_declarator"
            ) {
                target = parent;
                continue;
            }
        }
        break;
    }
    None
}

/// Try to find a `/** */` JSDoc comment as the previous sibling of a node.
fn try_jsdoc_prev_sibling(node: &Node, source: &str) -> Option<String> {
    let prev = node.prev_sibling()?;
    if prev.kind() != "comment" {
        return None;
    }
    let text = get_node_text(&prev, source);
    if !text.starts_with("/**") {
        return None;
    }
    parse_block_doc_comment(&text)
}

fn extract_ts_classes_detailed(node: &Node, source: &str, classes: &mut Vec<ClassInfo>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_declaration" | "class" | "interface_declaration" => {
                let info = extract_ts_class_info(&child, source);
                classes.push(info);
            }
            "type_alias_declaration" => {
                // Type aliases like `type Foo = string | number` are represented
                // as ClassInfo entries so the surface extractor can detect them via
                // `determine_ts_class_kind` and tag them as TypeAlias.
                let name = child
                    .child_by_field_name("name")
                    .map(|n| get_node_text(&n, source))
                    .unwrap_or_default();
                let line_number = child.start_position().row as u32 + 1;
                classes.push(ClassInfo {
                    name,
                    bases: Vec::new(),
                    docstring: extract_jsdoc_docstring(&child, source),
                    methods: Vec::new(),
                    fields: Vec::new(),
                    decorators: Vec::new(),
                    line_number,
                });
            }
            _ => {
                extract_ts_classes_detailed(&child, source, classes);
            }
        }
    }
}

fn extract_ts_class_info(node: &Node, source: &str) -> ClassInfo {
    let name = node
        .child_by_field_name("name")
        .map(|n| get_node_text(&n, source))
        .unwrap_or_default();

    let line_number = node.start_position().row as u32 + 1;

    // Extract methods
    let mut methods = Vec::new();
    if let Some(body) = node.child_by_field_name("body") {
        extract_ts_functions_detailed(&body, source, &mut methods, true);
    }

    // Extract extends clause
    let mut bases = Vec::new();
    let mut c = node.walk();
    for child in node.children(&mut c) {
        if child.kind() == "class_heritage" {
            let text = get_node_text(&child, source);
            if text.starts_with("extends") {
                let base = text
                    .trim_start_matches("extends")
                    .split_whitespace()
                    .next();
                if let Some(b) = base {
                    bases.push(b.to_string());
                }
            }
        }
    }

    // Extract class fields (Gap 3)
    let fields = if let Some(body) = node.child_by_field_name("body") {
        extract_ts_class_fields(&body, source)
    } else {
        Vec::new()
    };

    ClassInfo {
        name,
        bases,
        docstring: extract_jsdoc_docstring(node, source),
        methods,
        fields,
        decorators: Vec::new(),
        line_number,
    }
}

// =============================================================================
// Gap 3: TypeScript class field extraction
// =============================================================================

/// Extract fields from a TypeScript class body
/// Looks for public_field_definition and property-like nodes
fn extract_ts_class_fields(body: &Node, source: &str) -> Vec<FieldInfo> {
    let mut fields = Vec::new();
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        // TypeScript/JavaScript field definitions:
        // public_field_definition or property_definition (depending on grammar)
        match child.kind() {
            "public_field_definition" | "field_definition" => {
                if let Some(field) = extract_ts_field_from_definition(&child, source) {
                    fields.push(field);
                }
            }
            // Handle property_identifier with modifiers
            _ => {}
        }
    }
    fields
}

fn extract_ts_field_from_definition(node: &Node, source: &str) -> Option<FieldInfo> {
    let name = node
        .child_by_field_name("name")
        .map(|n| get_node_text(&n, source));

    // Fallback: first identifier child
    let name = name.or_else(|| {
        let mut c = node.walk();
        for ch in node.children(&mut c) {
            if ch.kind() == "property_identifier" || ch.kind() == "identifier" {
                return Some(get_node_text(&ch, source));
            }
        }
        None
    })?;

    let field_type = node.child_by_field_name("type").map(|n| {
        let text = get_node_text(&n, source);
        text.trim_start_matches(':').trim().to_string()
    });

    let default_value = node
        .child_by_field_name("value")
        .map(|n| get_node_text(&n, source));

    // Check for static keyword
    let text = get_node_text(node, source);
    let is_static = text.starts_with("static ");

    // Check for visibility modifiers
    let visibility = if text.contains("private ") {
        Some("private".to_string())
    } else if text.contains("protected ") {
        Some("protected".to_string())
    } else {
        Some("public".to_string())
    };

    let line_number = node.start_position().row as u32 + 1;

    let is_constant = is_static && is_upper_case_name(&name);

    Some(FieldInfo {
        name,
        field_type,
        default_value,
        is_static,
        is_constant,
        visibility,
        line_number,
    })
}

// =============================================================================
// Go detailed extraction
// =============================================================================

fn extract_go_functions_detailed(node: &Node, source: &str, functions: &mut Vec<FunctionInfo>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "function_declaration" {
            // Only top-level functions; method_declaration nodes are handled
            // by extract_go_methods_to_classes and associated with their receiver structs.
            let info = extract_go_function_info(&child, source);
            functions.push(info);
        }
        extract_go_functions_detailed(&child, source, functions);
    }
}

fn extract_go_function_info(node: &Node, source: &str) -> FunctionInfo {
    let name = node
        .child_by_field_name("name")
        .map(|n| get_node_text(&n, source))
        .unwrap_or_default();

    let is_method = node.kind() == "method_declaration";
    let params = extract_go_params(node, source);
    let return_type = node
        .child_by_field_name("result")
        .map(|n| get_node_text(&n, source));

    let line_number = node.start_position().row as u32 + 1;
    let docstring = extract_go_docstring(node, source);

    FunctionInfo {
        name,
        params,
        return_type,
        docstring,
        is_method,
        is_async: false,
        decorators: Vec::new(),
        line_number,
    }
}

/// Extract Go doc comments from preceding sibling comment nodes.
///
/// Go doc comments are `//` line comments immediately preceding a declaration.
/// They are represented as `comment` sibling nodes in the tree-sitter Go grammar.
/// This function walks backwards from the given node collecting contiguous comment
/// siblings, then joins them with newlines after stripping the `// ` prefix.
fn extract_go_docstring(node: &Node, source: &str) -> Option<String> {
    let mut prev = node.prev_sibling();
    let mut comment_lines: Vec<String> = Vec::new();

    while let Some(prev_node) = prev {
        if prev_node.kind() == "comment" {
            let text = get_node_text(&prev_node, source);
            // Block comment: return immediately if no line comments collected yet
            if text.starts_with("/*") {
                if !comment_lines.is_empty() {
                    break;
                }
                // Strip /* and */ delimiters, trim whitespace
                let inner = text
                    .trim_start_matches("/*")
                    .trim_end_matches("*/")
                    .trim();
                if inner.is_empty() {
                    return None;
                }
                return Some(inner.to_string());
            }
            // Line comment: strip "// " or "//" prefix
            if text.starts_with("//") {
                let stripped = text.strip_prefix("// ").unwrap_or(
                    text.strip_prefix("//").unwrap_or(&text),
                );
                comment_lines.push(stripped.to_string());
            } else {
                break;
            }
            prev = prev_node.prev_sibling();
        } else {
            break;
        }
    }

    if comment_lines.is_empty() {
        None
    } else {
        comment_lines.reverse();
        Some(comment_lines.join("\n"))
    }
}

fn extract_go_params(node: &Node, source: &str) -> Vec<String> {
    let mut params = Vec::new();

    if let Some(params_node) = node.child_by_field_name("parameters") {
        let mut cursor = params_node.walk();
        for child in params_node.children(&mut cursor) {
            if child.kind() == "parameter_declaration" {
                // Go allows grouped parameters: `a, b, c int` produces a single
                // parameter_declaration with multiple identifier children as names.
                // child_by_field_name("name") only returns the first one, so we
                // iterate all children to collect every identifier (name).
                let mut inner_cursor = child.walk();
                let mut found_any = false;
                for inner in child.children(&mut inner_cursor) {
                    if inner.kind() == "identifier" {
                        params.push(get_node_text(&inner, source));
                        found_any = true;
                    }
                }
                // Fallback: if no identifier children found, try field_identifier
                // (used in some tree-sitter-go versions)
                if !found_any {
                    if let Some(name) = child.child_by_field_name("name") {
                        params.push(get_node_text(&name, source));
                    }
                }
            }
        }
    }

    params
}

// =============================================================================
// Gap 2+3: Go struct/interface extraction and method association
// =============================================================================

/// Extract Go struct and interface type declarations as ClassInfo, then
/// associate method_declaration nodes with their receiver types (two-pass).
///
/// Pass 1: Walk the AST for type_declaration nodes to find structs and interfaces.
///   - Structs become ClassInfo with empty methods (fields extracted).
///   - Interfaces become ClassInfo with methods extracted from method_spec nodes.
///
/// Pass 2: Walk the AST for method_declaration nodes (Go methods with receivers).
///   - Extract the receiver type (normalizing pointer receivers: *Server -> Server).
///   - Find or auto-vivify a ClassInfo for the receiver type.
///   - Add the method to its ClassInfo.methods.
fn extract_go_structs_detailed(node: &Node, source: &str, classes: &mut Vec<ClassInfo>) {
    // Pass 1: Extract structs and interfaces
    extract_go_types_pass1(node, source, classes);

    // Pass 2: Associate methods with receiver types
    extract_go_methods_to_classes(node, source, classes);
}

/// Pass 1: Extract Go struct and interface type declarations as ClassInfo.
fn extract_go_types_pass1(node: &Node, source: &str, classes: &mut Vec<ClassInfo>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_declaration" {
            // Doc comments are siblings of type_declaration, not type_spec
            let docstring = extract_go_docstring(&child, source);
            let mut spec_cursor = child.walk();
            for spec in child.children(&mut spec_cursor) {
                if spec.kind() == "type_spec" {
                    let name = spec
                        .child_by_field_name("name")
                        .map(|n| get_node_text(&n, source))
                        .unwrap_or_default();
                    let type_node = spec.child_by_field_name("type");
                    if let Some(tn) = type_node {
                        if tn.kind() == "struct_type" {
                            let line_number = spec.start_position().row as u32 + 1;
                            let fields = extract_go_struct_fields(&tn, source);
                            classes.push(ClassInfo {
                                name,
                                bases: Vec::new(),
                                docstring: docstring.clone(),
                                methods: Vec::new(),
                                fields,
                                decorators: Vec::new(),
                                line_number,
                            });
                        } else if tn.kind() == "interface_type" {
                            let line_number = spec.start_position().row as u32 + 1;
                            let methods = extract_go_interface_methods(&tn, source);
                            classes.push(ClassInfo {
                                name,
                                bases: Vec::new(),
                                docstring: docstring.clone(),
                                methods,
                                fields: Vec::new(),
                                decorators: Vec::new(),
                                line_number,
                            });
                        }
                    }
                }
            }
        }
    }
}

/// Extract method signatures from a Go interface_type node.
///
/// Go interfaces contain method_spec nodes (method signatures without bodies):
/// ```text
/// interface_type
///   method_spec_list (or direct children)
///     method_spec
///       name: field_identifier "Handle"
///       parameters: parameter_list
///       result: ...
/// ```
fn extract_go_interface_methods(interface_node: &Node, source: &str) -> Vec<FunctionInfo> {
    let mut methods = Vec::new();
    // Walk all descendants looking for method_spec nodes
    extract_go_interface_methods_recursive(interface_node, source, &mut methods);
    methods
}

fn extract_go_interface_methods_recursive(
    node: &Node,
    source: &str,
    methods: &mut Vec<FunctionInfo>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "method_elem" || child.kind() == "method_spec" {
            // The method name is a field_identifier child node.
            // Extract it by finding the first field_identifier.
            let mut name = String::new();
            let mut params = Vec::new();
            let mut return_type = None;
            let line_number = child.start_position().row as u32 + 1;

            let mut inner_cursor = child.walk();
            for inner in child.children(&mut inner_cursor) {
                match inner.kind() {
                    "field_identifier" => {
                        name = get_node_text(&inner, source);
                    }
                    "parameter_list" => {
                        // Extract parameter names from the parameter list
                        let mut param_cursor = inner.walk();
                        for param in inner.children(&mut param_cursor) {
                            if param.kind() == "parameter_declaration" {
                                if let Some(pname) = param.child_by_field_name("name") {
                                    params.push(get_node_text(&pname, source));
                                }
                            }
                        }
                    }
                    "type_identifier" | "qualified_type" | "pointer_type" | "slice_type"
                    | "map_type" | "channel_type" | "function_type" | "interface_type"
                    | "struct_type" | "parenthesized_type" => {
                        // This is the return type (simple single return)
                        return_type = Some(get_node_text(&inner, source));
                    }
                    _ => {}
                }
            }

            // Also check for result field (tuple return types)
            if return_type.is_none() {
                if let Some(result) = child.child_by_field_name("result") {
                    return_type = Some(get_node_text(&result, source));
                }
            }

            if !name.is_empty() {
                methods.push(FunctionInfo {
                    name,
                    params,
                    return_type,
                    docstring: extract_go_docstring(&child, source),
                    is_method: true,
                    is_async: false,
                    decorators: Vec::new(),
                    line_number,
                });
            }
        } else {
            extract_go_interface_methods_recursive(&child, source, methods);
        }
    }
}

/// Pass 2: Walk the AST for method_declaration nodes, extract receiver type,
/// and associate each method with its receiver's ClassInfo.
///
/// If a receiver type has no matching ClassInfo (orphan method), a new ClassInfo
/// is auto-vivified for that type.
fn extract_go_methods_to_classes(node: &Node, source: &str, classes: &mut Vec<ClassInfo>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "method_declaration" {
            let method_info = extract_go_function_info(&child, source);
            let receiver_type = extract_go_receiver_type(&child, source);

            if !receiver_type.is_empty() {
                // Find existing ClassInfo or auto-vivify
                if let Some(class) = classes.iter_mut().find(|c| c.name == receiver_type) {
                    class.methods.push(method_info);
                } else {
                    // Auto-vivify: method for type not defined in this file
                    classes.push(ClassInfo {
                        name: receiver_type,
                        bases: Vec::new(),
                        docstring: None,
                        methods: vec![method_info],
                        fields: Vec::new(),
                        decorators: Vec::new(),
                        line_number: 0, // Unknown, defined elsewhere
                    });
                }
            }
        }
        extract_go_methods_to_classes(&child, source, classes);
    }
}

/// Extract the receiver type from a Go method_declaration node.
///
/// Go method_declaration AST structure:
/// ```text
/// method_declaration
///   receiver: parameter_list
///     parameter_declaration
///       name: identifier "s"
///       type: pointer_type → type_identifier "Server"   (pointer receiver)
///       type: type_identifier "Server"                   (value receiver)
/// ```
///
/// Normalizes pointer receivers: `*Server` -> `Server`.
fn extract_go_receiver_type(method_node: &Node, source: &str) -> String {
    if let Some(receiver) = method_node.child_by_field_name("receiver") {
        let mut cursor = receiver.walk();
        for child in receiver.children(&mut cursor) {
            if child.kind() == "parameter_declaration" {
                if let Some(type_node) = child.child_by_field_name("type") {
                    let type_text = get_node_text(&type_node, source);
                    // Normalize: strip pointer prefix "*Server" -> "Server"
                    return type_text.trim_start_matches('*').to_string();
                }
            }
        }
    }
    String::new()
}

/// Extract fields from a Go struct_type node
fn extract_go_struct_fields(struct_node: &Node, source: &str) -> Vec<FieldInfo> {
    let mut fields = Vec::new();
    let mut cursor = struct_node.walk();
    for child in struct_node.children(&mut cursor) {
        if child.kind() == "field_declaration_list" {
            let mut field_cursor = child.walk();
            for field in child.children(&mut field_cursor) {
                if field.kind() == "field_declaration" {
                    let field_type = field
                        .child_by_field_name("type")
                        .map(|n| get_node_text(&n, source));

                    // Go allows multiple names per field_declaration: X, Y int
                    // Collect all field_identifier children as separate FieldInfo
                    let mut names = Vec::new();
                    let mut name_cursor = field.walk();
                    for fc in field.children(&mut name_cursor) {
                        if fc.kind() == "field_identifier" {
                            names.push(get_node_text(&fc, source));
                        }
                    }

                    let line_number = field.start_position().row as u32 + 1;
                    for name in names {
                        let visibility = if name
                            .chars()
                            .next()
                            .map(|c| c.is_uppercase())
                            .unwrap_or(false)
                        {
                            Some("public".to_string())
                        } else {
                            Some("private".to_string())
                        };
                        fields.push(FieldInfo {
                            name,
                            field_type: field_type.clone(),
                            default_value: None,
                            is_static: false,
                            is_constant: false,
                            visibility,
                            line_number,
                        });
                    }
                }
            }
        }
    }
    fields
}

// =============================================================================
// Rust detailed extraction
// =============================================================================

fn extract_rust_functions_detailed(node: &Node, source: &str, functions: &mut Vec<FunctionInfo>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "function_item" {
            // Only top-level functions
            if !is_inside_impl(&child) {
                let info = extract_rust_function_info(&child, source, false);
                functions.push(info);
            }
        }
        extract_rust_functions_detailed(&child, source, functions);
    }
}

fn extract_rust_function_info(node: &Node, source: &str, is_method: bool) -> FunctionInfo {
    let name = node
        .child_by_field_name("name")
        .map(|n| get_node_text(&n, source))
        .unwrap_or_default();

    let params = extract_rust_params(node, source);
    let return_type = node.child_by_field_name("return_type").map(|n| {
        get_node_text(&n, source)
            .trim_start_matches("->")
            .trim()
            .to_string()
    });

    let is_async = get_node_text(node, source).contains("async fn");
    let line_number = node.start_position().row as u32 + 1;

    FunctionInfo {
        name,
        params,
        return_type,
        docstring: extract_rust_docstring(node, source),
        is_method,
        is_async,
        decorators: Vec::new(),
        line_number,
    }
}

fn extract_rust_params(node: &Node, source: &str) -> Vec<String> {
    let mut params = Vec::new();

    if let Some(params_node) = node.child_by_field_name("parameters") {
        let mut cursor = params_node.walk();
        for child in params_node.children(&mut cursor) {
            if child.kind() == "parameter" {
                if let Some(pattern) = child.child_by_field_name("pattern") {
                    params.push(get_node_text(&pattern, source));
                }
            } else if child.kind() == "self_parameter" {
                params.push(get_node_text(&child, source));
            }
        }
    }

    params
}

/// Extract Rust doc comments (`///` line comments or `/** */` blocks) from
/// preceding sibling nodes, skipping `#[...]` attribute items.
fn extract_rust_docstring(node: &Node, source: &str) -> Option<String> {
    let mut prev = node.prev_sibling();
    let mut comment_lines: Vec<String> = Vec::new();

    while let Some(prev_node) = prev {
        match prev_node.kind() {
            "line_comment" => {
                let text = get_node_text(&prev_node, source);
                if text.starts_with("///") {
                    let stripped = text
                        .strip_prefix("/// ")
                        .unwrap_or(text.strip_prefix("///").unwrap_or(&text));
                    comment_lines.push(stripped.to_string());
                } else {
                    break;
                }
            }
            "block_comment" => {
                let text = get_node_text(&prev_node, source);
                if text.starts_with("/**") {
                    if !comment_lines.is_empty() {
                        break;
                    }
                    return parse_block_doc_comment(&text);
                }
                break;
            }
            "attribute_item" => {
                // Skip #[...] attributes between doc comment and item
                prev = prev_node.prev_sibling();
                continue;
            }
            _ => break,
        }
        prev = prev_node.prev_sibling();
    }

    if comment_lines.is_empty() {
        None
    } else {
        comment_lines.reverse();
        Some(comment_lines.join("\n"))
    }
}

fn extract_rust_structs_detailed(node: &Node, source: &str, classes: &mut Vec<ClassInfo>) {
    // Two-pass approach (Gap 1):
    // Pass 1: Collect all struct/enum definitions
    // Pass 2: Walk impl blocks and associate methods with their target types

    // Pass 1: Collect structs and enums
    collect_rust_struct_defs(node, source, classes);

    // Build name -> index map for O(1) lookup during impl association
    let mut struct_map: HashMap<String, Vec<usize>> = HashMap::new();
    for (idx, class) in classes.iter().enumerate() {
        struct_map.entry(class.name.clone()).or_default().push(idx);
    }

    // Pass 2: Associate impl block methods with their target types
    associate_rust_impl_methods(node, source, classes, &struct_map);
}

/// Pass 1: Recursively collect struct/enum/trait definitions into ClassInfo entries
fn collect_rust_struct_defs(node: &Node, source: &str, classes: &mut Vec<ClassInfo>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "struct_item"
            || child.kind() == "enum_item"
            || child.kind() == "trait_item"
        {
            let name = child
                .child_by_field_name("name")
                .map(|n| get_node_text(&n, source))
                .unwrap_or_default();

            let line_number = child.start_position().row as u32 + 1;

            // Extract struct fields (Gap 3)
            let fields = if child.kind() == "struct_item" {
                extract_rust_struct_fields(&child, source)
            } else {
                Vec::new() // enum variants and trait items handled separately
            };

            // Extract methods declared directly in trait body (declaration_list)
            let methods = if child.kind() == "trait_item" {
                extract_methods_from_trait_body(&child, source)
            } else {
                Vec::new()
            };

            classes.push(ClassInfo {
                name,
                bases: Vec::new(),
                docstring: extract_rust_docstring(&child, source),
                methods,
                fields,
                decorators: Vec::new(),
                line_number,
            });
        }
        collect_rust_struct_defs(&child, source, classes);
    }
}

/// Extract method signatures from a trait body (`declaration_list`).
///
/// Trait methods can be either:
/// - `function_signature_item`: Declaration without body (e.g., `fn greet(&self) -> String;`)
/// - `function_item`: Default implementation (e.g., `fn default_greet(&self) -> String { ... }`)
fn extract_methods_from_trait_body(trait_node: &Node, source: &str) -> Vec<FunctionInfo> {
    let mut methods = Vec::new();
    let mut cursor = trait_node.walk();
    for child in trait_node.children(&mut cursor) {
        if child.kind() == "declaration_list" {
            let mut body_cursor = child.walk();
            for item in child.children(&mut body_cursor) {
                if item.kind() == "function_signature_item" || item.kind() == "function_item" {
                    if item.kind() == "function_item" {
                        let info = extract_rust_function_info(&item, source, true);
                        methods.push(info);
                    } else {
                        // function_signature_item: `fn greet(&self) -> String;`
                        let name = item
                            .child_by_field_name("name")
                            .map(|n| get_node_text(&n, source))
                            .unwrap_or_default();

                        let params = extract_rust_params(&item, source);
                        let return_type = item.child_by_field_name("return_type").map(|n| {
                            get_node_text(&n, source)
                                .trim_start_matches("->")
                                .trim()
                                .to_string()
                        });

                        let is_async = get_node_text(&item, source).contains("async fn");
                        let line_number = item.start_position().row as u32 + 1;

                        methods.push(FunctionInfo {
                            name,
                            params,
                            return_type,
                            docstring: extract_rust_docstring(&item, source),
                            is_method: true,
                            is_async,
                            decorators: Vec::new(),
                            line_number,
                        });
                    }
                }
            }
        }
    }
    methods
}

/// Pass 2: Walk all impl blocks and associate methods with matching ClassInfo (Gap 1)
fn associate_rust_impl_methods(
    node: &Node,
    source: &str,
    classes: &mut Vec<ClassInfo>,
    struct_map: &HashMap<String, Vec<usize>>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "impl_item" {
            if let Some(type_name) = get_impl_type_name(&child, source) {
                let methods = extract_methods_from_impl_body(&child, source);
                if let Some(indices) = struct_map.get(&type_name) {
                    // Associate with the first matching struct/enum
                    if let Some(&idx) = indices.first() {
                        classes[idx].methods.extend(methods);
                    }
                }
                // Orphan impls (no matching struct in file) are silently skipped
            }
        }
        associate_rust_impl_methods(&child, source, classes, struct_map);
    }
}

/// Extract the target type name from an impl block.
///
/// Handles:
/// - `impl Foo { ... }` -> "Foo"
/// - `impl Trait for Foo { ... }` -> "Foo" (the type, not the trait)
/// - `impl<T> Foo<T> { ... }` -> "Foo" (strips generic params)
/// - `impl std::fmt::Display for Foo { ... }` -> "Foo"
fn get_impl_type_name(impl_node: &Node, source: &str) -> Option<String> {
    let type_node = impl_node.child_by_field_name("type")?;

    match type_node.kind() {
        "type_identifier" => {
            // Simple case: `impl Foo { ... }` or `impl Trait for Foo { ... }`
            Some(get_node_text(&type_node, source))
        }
        "generic_type" => {
            // Generic case: `impl<T> Container<T> { ... }`
            // The type_identifier is nested under the "type" field of generic_type
            type_node
                .child_by_field_name("type")
                .map(|n| get_node_text(&n, source))
        }
        "scoped_type_identifier" => {
            // Scoped case: `impl some::module::Type { ... }`
            // Take the last segment (the actual type name)
            type_node
                .child_by_field_name("name")
                .map(|n| get_node_text(&n, source))
        }
        _ => {
            // Fallback: extract text and strip any generics
            let text = get_node_text(&type_node, source);
            let name = text.split('<').next().unwrap_or(&text).trim();
            if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            }
        }
    }
}

/// Extract all methods (FunctionInfo) from an impl block's body
fn extract_methods_from_impl_body(impl_node: &Node, source: &str) -> Vec<FunctionInfo> {
    let mut methods = Vec::new();

    if let Some(body) = impl_node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for item in body.children(&mut cursor) {
            if item.kind() == "function_item" {
                let info = extract_rust_function_info(&item, source, true);
                methods.push(info);
            }
        }
    }

    methods
}

// =============================================================================
// Gap 3: Rust struct field extraction
// =============================================================================

/// Extract fields from a Rust struct_item node
fn extract_rust_struct_fields(struct_node: &Node, source: &str) -> Vec<FieldInfo> {
    let mut fields = Vec::new();
    let mut cursor = struct_node.walk();
    for child in struct_node.children(&mut cursor) {
        if child.kind() == "field_declaration_list" {
            let mut field_cursor = child.walk();
            for field in child.children(&mut field_cursor) {
                if field.kind() == "field_declaration" {
                    let name = field
                        .child_by_field_name("name")
                        .map(|n| get_node_text(&n, source));
                    let field_type = field
                        .child_by_field_name("type")
                        .map(|n| get_node_text(&n, source));

                    if let Some(name) = name {
                        // Check for visibility modifier (pub, pub(crate), etc.)
                        let visibility = field
                            .children(&mut field.walk())
                            .find(|c| c.kind() == "visibility_modifier")
                            .map(|n| {
                                let text = get_node_text(&n, source);
                                if text == "pub" {
                                    "public".to_string()
                                } else {
                                    text
                                }
                            })
                            .or_else(|| Some("private".to_string()));

                        let line_number = field.start_position().row as u32 + 1;
                        fields.push(FieldInfo {
                            name,
                            field_type,
                            default_value: None,
                            is_static: false,
                            is_constant: false,
                            visibility,
                            line_number,
                        });
                    }
                }
            }
        }
    }
    fields
}

// =============================================================================
// Java detailed extraction
// =============================================================================

fn extract_java_functions_detailed(node: &Node, source: &str, functions: &mut Vec<FunctionInfo>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "method_declaration" {
            let info = extract_java_function_info(&child, source);
            functions.push(info);
        }
        extract_java_functions_detailed(&child, source, functions);
    }
}

fn extract_java_function_info(node: &Node, source: &str) -> FunctionInfo {
    let name = node
        .child_by_field_name("name")
        .map(|n| get_node_text(&n, source))
        .unwrap_or_default();

    let params = extract_java_params(node, source);
    let return_type = node
        .child_by_field_name("type")
        .map(|n| get_node_text(&n, source));

    let line_number = node.start_position().row as u32 + 1;

    FunctionInfo {
        name,
        params,
        return_type,
        docstring: extract_java_docstring(node, source),
        is_method: true,
        is_async: false,
        decorators: Vec::new(),
        line_number,
    }
}

fn extract_java_params(node: &Node, source: &str) -> Vec<String> {
    let mut params = Vec::new();

    if let Some(params_node) = node.child_by_field_name("parameters") {
        let mut cursor = params_node.walk();
        for child in params_node.children(&mut cursor) {
            if child.kind() == "formal_parameter" {
                if let Some(name) = child.child_by_field_name("name") {
                    params.push(get_node_text(&name, source));
                }
            }
        }
    }

    params
}

/// Extract Javadoc comments (`/** */`) from preceding sibling nodes,
/// skipping annotation nodes (`marker_annotation`, `annotation`).
fn extract_java_docstring(node: &Node, source: &str) -> Option<String> {
    let mut prev = node.prev_sibling();
    while let Some(prev_node) = prev {
        match prev_node.kind() {
            "block_comment" | "comment" => {
                let text = get_node_text(&prev_node, source);
                if text.starts_with("/**") {
                    return parse_block_doc_comment(&text);
                }
                return None;
            }
            "marker_annotation" | "annotation" => {
                // Skip @Entity, @Override etc. between Javadoc and declaration
                prev = prev_node.prev_sibling();
                continue;
            }
            _ => return None,
        }
    }
    None
}

fn extract_java_classes_detailed(node: &Node, source: &str, classes: &mut Vec<ClassInfo>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "class_declaration"
            || child.kind() == "interface_declaration"
            || child.kind() == "enum_declaration"
            || child.kind() == "record_declaration"
        {
            let name = child
                .child_by_field_name("name")
                .map(|n| get_node_text(&n, source))
                .unwrap_or_default();

            let line_number = child.start_position().row as u32 + 1;

            // Extract methods
            let mut methods = Vec::new();
            if let Some(body) = child.child_by_field_name("body") {
                extract_java_functions_detailed(&body, source, &mut methods);
            }

            // Extract extends/implements bases
            let bases = extract_java_class_bases(&child, source);

            // Extract class fields (Gap 3)
            let fields = if let Some(body) = child.child_by_field_name("body") {
                extract_java_class_fields(&body, source)
            } else {
                Vec::new()
            };

            classes.push(ClassInfo {
                name,
                bases,
                docstring: extract_java_docstring(&child, source),
                methods,
                fields,
                decorators: Vec::new(),
                line_number,
            });
        }
        extract_java_classes_detailed(&child, source, classes);
    }
}

/// Extract base class/interface names from a Java class/interface/enum/record declaration
fn extract_java_class_bases(node: &Node, source: &str) -> Vec<String> {
    let mut bases = Vec::new();

    // Extract superclass (extends for classes)
    if let Some(superclass) = node.child_by_field_name("superclass") {
        extract_java_type_names(&superclass, source, &mut bases);
    }

    // Extract implements (for classes, enums, records)
    if let Some(interfaces) = node.child_by_field_name("interfaces") {
        extract_java_type_list(&interfaces, source, &mut bases);
    }

    // Extract extends for interfaces (extends_interfaces child node)
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "extends_interfaces" {
            extract_java_type_list(&child, source, &mut bases);
        }
    }

    bases
}

/// Extract type names directly from a node's children
fn extract_java_type_names(node: &Node, source: &str, bases: &mut Vec<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(name) = extract_java_type_name(&child, source) {
            bases.push(name);
        }
    }
}

/// Extract types from a node containing a type_list child
fn extract_java_type_list(node: &Node, source: &str, bases: &mut Vec<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_list" {
            let mut inner_cursor = child.walk();
            for type_child in child.children(&mut inner_cursor) {
                if let Some(name) = extract_java_type_name(&type_child, source) {
                    bases.push(name);
                }
            }
        } else if let Some(name) = extract_java_type_name(&child, source) {
            bases.push(name);
        }
    }
}

/// Extract a single type name, handling generics and scoped types
fn extract_java_type_name(node: &Node, source: &str) -> Option<String> {
    match node.kind() {
        "type_identifier" => Some(get_node_text(node, source)),
        "generic_type" => {
            // Generic<T> -> base type name only
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    match child.kind() {
                        "type_identifier" => return Some(get_node_text(&child, source)),
                        "scoped_type_identifier" => return Some(get_node_text(&child, source)),
                        _ => {}
                    }
                }
            }
            None
        }
        "scoped_type_identifier" => Some(get_node_text(node, source)),
        _ => None,
    }
}

// =============================================================================
// Gap 3: Java class field extraction
// =============================================================================

/// Extract field declarations from a Java class body
fn extract_java_class_fields(body: &Node, source: &str) -> Vec<FieldInfo> {
    let mut fields = Vec::new();
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() == "field_declaration" {
            // field_declaration has modifiers, type, and declarator(s)
            let field_type = child
                .child_by_field_name("type")
                .map(|n| get_node_text(&n, source));

            // Check modifiers for static, final, visibility
            let text = get_node_text(&child, source);
            let is_static = text.contains("static ");
            let is_final = text.contains("final ");

            let visibility = if text.contains("private ") {
                Some("private".to_string())
            } else if text.contains("protected ") {
                Some("protected".to_string())
            } else if text.contains("public ") {
                Some("public".to_string())
            } else {
                Some("package".to_string())
            };

            // Extract each variable_declarator
            let mut decl_cursor = child.walk();
            for decl_child in child.children(&mut decl_cursor) {
                if decl_child.kind() == "variable_declarator" {
                    let name = decl_child
                        .child_by_field_name("name")
                        .map(|n| get_node_text(&n, source));

                    if let Some(name) = name {
                        let default_value = decl_child
                            .child_by_field_name("value")
                            .map(|n| get_node_text(&n, source));

                        let is_constant = is_static && is_final && is_upper_case_name(&name);
                        let line_number = child.start_position().row as u32 + 1;

                        fields.push(FieldInfo {
                            name,
                            field_type: field_type.clone(),
                            default_value,
                            is_static,
                            is_constant,
                            visibility: visibility.clone(),
                            line_number,
                        });
                    }
                }
            }
        }
    }
    fields
}

// =============================================================================
// Lua detailed extraction
// =============================================================================

fn extract_lua_functions_detailed(node: &Node, source: &str, functions: &mut Vec<FunctionInfo>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                // Named function: `function foo() end` or `local function foo() end`
                let info = extract_lua_function_info(&child, source);
                functions.push(info);
            }
            "assignment_statement" => {
                // Check for: M.func = function() end
                extract_lua_assignment_functions(&child, source, functions);
            }
            "variable_declaration" => {
                // Check for: local myFunc = function() end
                // variable_declaration wraps an assignment_statement
                let mut inner_cursor = child.walk();
                for inner in child.children(&mut inner_cursor) {
                    if inner.kind() == "assignment_statement" {
                        extract_lua_assignment_functions(&inner, source, functions);
                    }
                }
                // Do NOT recurse further -- the assignment_statement is fully handled above
            }
            _ => {
                extract_lua_functions_detailed(&child, source, functions);
            }
        }
    }
}

/// Extract a function from an assignment statement if the RHS is a function_definition.
/// Handles patterns like:
///   M.request = function(url) end        -- dot_index_expression LHS
///   myFunc = function(a, b) end          -- identifier LHS
fn extract_lua_assignment_functions(node: &Node, source: &str, functions: &mut Vec<FunctionInfo>) {
    // Find the variable_list and expression_list children
    let mut var_list = None;
    let mut expr_list = None;
    let mut assign_cursor = node.walk();
    for child in node.children(&mut assign_cursor) {
        match child.kind() {
            "variable_list" => var_list = Some(child),
            "expression_list" => expr_list = Some(child),
            _ => {}
        }
    }

    let (var_list, expr_list) = match (var_list, expr_list) {
        (Some(v), Some(e)) => (v, e),
        _ => return,
    };

    // Check if RHS contains a function_definition
    let mut func_def = None;
    let mut el_cursor = expr_list.walk();
    for child in expr_list.children(&mut el_cursor) {
        if child.kind() == "function_definition" {
            func_def = Some(child);
            break;
        }
    }

    let func_def = match func_def {
        Some(f) => f,
        None => return,
    };

    // Extract the name from the LHS
    let name = extract_lua_lhs_name(&var_list, source);
    if name.is_empty() {
        return;
    }

    let params = extract_lua_params(&func_def, source);
    let docstring = extract_lua_docstring_before(node, source);
    let line_number = node.start_position().row as u32 + 1;

    functions.push(FunctionInfo {
        name,
        params,
        return_type: None, // Lua is dynamically typed
        docstring,
        is_method: false,
        is_async: false,
        decorators: Vec::new(),
        line_number,
    });
}

/// Extract the function name from the LHS of a Lua assignment.
/// For `M.request`, returns "request". For `myFunc`, returns "myFunc".
fn extract_lua_lhs_name(var_list: &Node, source: &str) -> String {
    let mut vl_cursor = var_list.walk();
    for child in var_list.children(&mut vl_cursor) {
        match child.kind() {
            "dot_index_expression" => {
                // M.request -> extract "request" from the field
                if let Some(field) = child.child_by_field_name("field") {
                    return get_node_text(&field, source);
                }
                // Fallback: take text after last '.'
                let text = get_node_text(&child, source);
                if let Some(name) = text.rsplit('.').next() {
                    return name.to_string();
                }
            }
            "identifier" => {
                return get_node_text(&child, source);
            }
            _ => {}
        }
    }
    String::new()
}

fn extract_lua_function_info(node: &Node, source: &str) -> FunctionInfo {
    let name = node
        .child_by_field_name("name")
        .map(|n| get_node_text(&n, source))
        .unwrap_or_default();

    let params = extract_lua_params(node, source);
    let docstring = extract_lua_docstring_before(node, source);
    let line_number = node.start_position().row as u32 + 1;

    FunctionInfo {
        name,
        params,
        return_type: None, // Lua is dynamically typed
        docstring,
        is_method: false,
        is_async: false,
        decorators: Vec::new(),
        line_number,
    }
}

fn extract_lua_params(node: &Node, source: &str) -> Vec<String> {
    let mut params = Vec::new();

    if let Some(params_node) = node.child_by_field_name("parameters") {
        let mut cursor = params_node.walk();
        for child in params_node.children(&mut cursor) {
            match child.kind() {
                "identifier" => {
                    params.push(get_node_text(&child, source));
                }
                "spread" | "vararg_expression" => {
                    // ... varargs
                    params.push("...".to_string());
                }
                _ => {}
            }
        }
    }

    params
}

/// Extract doc comment lines preceding a node.
/// Lua doc comments use `---` (LuaDoc) or `--` (regular comment).
/// We collect consecutive comment nodes immediately before the target node.
fn extract_lua_docstring_before(node: &Node, source: &str) -> Option<String> {
    let mut doc_lines = Vec::new();
    let mut prev = node.prev_sibling();

    // Walk backwards through consecutive comment siblings
    while let Some(sibling) = prev {
        if sibling.kind() == "comment" {
            let text = get_node_text(&sibling, source);
            doc_lines.push(text);
            prev = sibling.prev_sibling();
        } else {
            break;
        }
    }

    if doc_lines.is_empty() {
        return None;
    }

    // Reverse since we collected bottom-to-top
    doc_lines.reverse();

    // Clean up: strip leading --, ---, and whitespace
    let cleaned: Vec<String> = doc_lines
        .iter()
        .map(|line| {
            let stripped = line.trim();
            let stripped = stripped.strip_prefix("---").unwrap_or(stripped);
            let stripped = stripped.strip_prefix("--").unwrap_or(stripped);
            stripped.trim().to_string()
        })
        .collect();

    Some(cleaned.join("\n"))
}

// =============================================================================
// Luau detailed extraction
// =============================================================================

fn extract_luau_functions_detailed(node: &Node, source: &str, functions: &mut Vec<FunctionInfo>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                let info = extract_luau_function_info(&child, source);
                functions.push(info);
            }
            "assignment_statement" | "variable_assignment" => {
                extract_luau_assignment_functions(&child, source, functions);
            }
            "variable_declaration" => {
                // local myFunc = function() end
                let mut inner_cursor = child.walk();
                for inner in child.children(&mut inner_cursor) {
                    if inner.kind() == "assignment_statement"
                        || inner.kind() == "variable_assignment"
                    {
                        extract_luau_assignment_functions(&inner, source, functions);
                    }
                }
                // Do NOT recurse further -- the assignment_statement is fully handled above
            }
            _ => {
                extract_luau_functions_detailed(&child, source, functions);
            }
        }
    }
}

/// Extract functions from Luau assignment statements (same pattern as Lua).
fn extract_luau_assignment_functions(node: &Node, source: &str, functions: &mut Vec<FunctionInfo>) {
    // Find the variable_list / assignment_variable_list and expression_list
    let mut var_list = None;
    let mut expr_list = None;
    let mut assign_cursor = node.walk();
    for child in node.children(&mut assign_cursor) {
        match child.kind() {
            "variable_list" | "assignment_variable_list" | "binding_list" => var_list = Some(child),
            "expression_list" | "assignment_expression_list" => expr_list = Some(child),
            _ => {}
        }
    }

    let (var_list, expr_list) = match (var_list, expr_list) {
        (Some(v), Some(e)) => (v, e),
        _ => return,
    };

    // Check if RHS contains a function_definition
    let mut func_def = None;
    let mut el_cursor = expr_list.walk();
    for child in expr_list.children(&mut el_cursor) {
        if child.kind() == "function_definition" {
            func_def = Some(child);
            break;
        }
    }

    let func_def = match func_def {
        Some(f) => f,
        None => return,
    };

    let name = extract_lua_lhs_name(&var_list, source);
    if name.is_empty() {
        return;
    }

    let params = extract_luau_params(&func_def, source);
    let return_type = extract_luau_return_type(&func_def, source);
    let docstring = extract_lua_docstring_before(node, source);
    let line_number = node.start_position().row as u32 + 1;

    functions.push(FunctionInfo {
        name,
        params,
        return_type,
        docstring,
        is_method: false,
        is_async: false,
        decorators: Vec::new(),
        line_number,
    });
}

fn extract_luau_function_info(node: &Node, source: &str) -> FunctionInfo {
    let name = node
        .child_by_field_name("name")
        .map(|n| get_node_text(&n, source))
        .unwrap_or_default();

    let params = extract_luau_params(node, source);
    let return_type = extract_luau_return_type(node, source);
    let docstring = extract_lua_docstring_before(node, source);
    let line_number = node.start_position().row as u32 + 1;

    FunctionInfo {
        name,
        params,
        return_type,
        docstring,
        is_method: false,
        is_async: false,
        decorators: Vec::new(),
        line_number,
    }
}

fn extract_luau_params(node: &Node, source: &str) -> Vec<String> {
    let mut params = Vec::new();

    if let Some(params_node) = node.child_by_field_name("parameters") {
        let mut cursor = params_node.walk();
        for child in params_node.children(&mut cursor) {
            match child.kind() {
                "identifier" => {
                    // Simple parameter without type annotation (fallback)
                    params.push(get_node_text(&child, source));
                }
                "parameter" => {
                    // Luau typed parameter: `name: Type`
                    // The first identifier child is the parameter name
                    let mut inner_cursor = child.walk();
                    for inner in child.children(&mut inner_cursor) {
                        if inner.kind() == "identifier" {
                            params.push(get_node_text(&inner, source));
                            break;
                        }
                    }
                }
                "spread" | "vararg_expression" => {
                    params.push("...".to_string());
                }
                _ => {}
            }
        }
    }

    params
}

/// Extract return type from a Luau function declaration.
/// In tree-sitter-luau, the return type appears as a `:` + type node
/// after the `parameters` node but before the `body` node.
fn extract_luau_return_type(node: &Node, source: &str) -> Option<String> {
    // Walk children: find `:` after parameters, then the next type node is the return type
    let mut found_params = false;
    let mut found_colon = false;
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "parameters" {
            found_params = true;
            continue;
        }
        if found_params && child.kind() == ":" {
            found_colon = true;
            continue;
        }
        if found_colon && child.kind() != "block" && child.kind() != "end" {
            // This should be the return type node
            let type_text = get_node_text(&child, source).trim().to_string();
            if !type_text.is_empty() {
                return Some(type_text);
            }
        }
        if child.kind() == "block" || child.kind() == "end" {
            break;
        }
    }

    None
}

// =============================================================================
// Swift detailed extraction
// =============================================================================
//
fn extract_swift_functions_detailed(node: &Node, source: &str, functions: &mut Vec<FunctionInfo>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                // Skip methods inside class/struct bodies -- those are handled
                // by extract_swift_classes_detailed
                if !is_inside_swift_type(&child) {
                    let info = extract_swift_function_info(&child, source, false);
                    functions.push(info);
                }
            }
            "class_declaration" | "struct_declaration" | "class_body" | "struct_body" => {
                // Don't recurse into class/struct bodies for top-level function extraction
            }
            _ => {
                extract_swift_functions_detailed(&child, source, functions);
            }
        }
    }
}

fn extract_swift_function_info(node: &Node, source: &str, is_method: bool) -> FunctionInfo {
    let name = node
        .child_by_field_name("name")
        .map(|n| get_node_text(&n, source))
        .unwrap_or_default();

    let params = extract_swift_params(node, source);
    let return_type = extract_swift_return_type(node, source);
    let docstring = extract_swift_docstring_before(node, source);
    let is_async = get_node_text(node, source).contains("async ");
    let line_number = node.start_position().row as u32 + 1;

    FunctionInfo {
        name,
        params,
        return_type,
        docstring,
        is_method,
        is_async,
        decorators: Vec::new(),
        line_number,
    }
}

fn extract_swift_params(node: &Node, source: &str) -> Vec<String> {
    let mut params = Vec::new();

    // Swift parameters are inside a parameter_clause or parameters node
    let params_node = node.child_by_field_name("parameters");

    let search_node = match params_node {
        Some(ref n) => n,
        None => {
            // Try to find parameter_clause child
            let mut cursor = node.walk();
            let mut found = None;
            for child in node.children(&mut cursor) {
                if child.kind() == "parameter_clause" || child.kind() == "parameter_list" {
                    found = Some(child);
                    break;
                }
            }
            match found {
                Some(ref _n) => {
                    // Extract params inline from the found node
                    let mut cursor2 = _n.walk();
                    for child in _n.children(&mut cursor2) {
                        if child.kind() == "parameter" {
                            let mut inner_cursor = child.walk();
                            for inner in child.children(&mut inner_cursor) {
                                if inner.kind() == "simple_identifier"
                                    || inner.kind() == "identifier"
                                {
                                    params.push(get_node_text(&inner, source));
                                    break;
                                }
                            }
                        }
                    }
                    return params;
                }
                None => return params,
            }
        }
    };

    let mut cursor = search_node.walk();
    for child in search_node.children(&mut cursor) {
        if child.kind() == "parameter" {
            let mut inner_cursor = child.walk();
            for inner in child.children(&mut inner_cursor) {
                if inner.kind() == "simple_identifier" || inner.kind() == "identifier" {
                    params.push(get_node_text(&inner, source));
                    break;
                }
            }
        }
    }

    params
}

fn extract_swift_return_type(node: &Node, source: &str) -> Option<String> {
    let mut found_arrow = false;
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "->" || get_node_text(&child, source) == "->" {
            found_arrow = true;
            continue;
        }
        if found_arrow {
            let kind = child.kind();
            // The next meaningful node after -> should be the return type
            if kind == "type_identifier"
                || kind == "type_annotation"
                || kind == "simple_identifier"
                || kind == "user_type"
                || kind == "optional_type"
                || kind == "array_type"
                || kind == "dictionary_type"
                || kind == "tuple_type"
            {
                return Some(get_node_text(&child, source));
            }
            // If it's the function body, stop
            if kind == "function_body" || kind == "code_block" {
                break;
            }
        }
    }

    None
}

fn extract_swift_docstring_before(node: &Node, source: &str) -> Option<String> {
    let mut doc_lines = Vec::new();
    let mut prev = node.prev_sibling();

    while let Some(sibling) = prev {
        if sibling.kind() == "comment" || sibling.kind() == "multiline_comment" {
            let text = get_node_text(&sibling, source);
            doc_lines.push(text);
            prev = sibling.prev_sibling();
        } else {
            break;
        }
    }

    if doc_lines.is_empty() {
        return None;
    }

    doc_lines.reverse();

    let cleaned: Vec<String> = doc_lines
        .iter()
        .map(|line| {
            let stripped = line.trim();
            // Handle /// doc comments
            if let Some(rest) = stripped.strip_prefix("///") {
                return rest.trim().to_string();
            }
            // Handle /** */ block comments
            if stripped.starts_with("/**") && stripped.ends_with("*/") {
                let inner = &stripped[3..stripped.len() - 2];
                return inner.trim().to_string();
            }
            stripped.to_string()
        })
        .collect();

    Some(cleaned.join("\n"))
}

fn is_inside_swift_type(node: &Node) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "class_declaration"
            | "struct_declaration"
            | "class_body"
            | "struct_body"
            | "extension_declaration"
            | "protocol_declaration" => return true,
            _ => current = parent.parent(),
        }
    }
    false
}

fn extract_swift_classes_detailed(node: &Node, source: &str, classes: &mut Vec<ClassInfo>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_declaration" | "struct_declaration" => {
                let info = extract_swift_class_info(&child, source);
                classes.push(info);
            }
            _ => {
                extract_swift_classes_detailed(&child, source, classes);
            }
        }
    }
}

fn extract_swift_class_info(node: &Node, source: &str) -> ClassInfo {
    let name = node
        .child_by_field_name("name")
        .map(|n| get_node_text(&n, source))
        .unwrap_or_else(|| {
            // Fallback: look for type_identifier child
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "type_identifier" || child.kind() == "simple_identifier" {
                    return get_node_text(&child, source);
                }
            }
            String::new()
        });

    let bases = extract_swift_bases(node, source);
    let docstring = extract_swift_docstring_before(node, source);
    let line_number = node.start_position().row as u32 + 1;

    // Extract methods from the body
    let mut methods = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();
        if kind == "class_body" || kind == "struct_body" || kind == "body" {
            let mut body_cursor = child.walk();
            for body_child in child.children(&mut body_cursor) {
                if body_child.kind() == "function_declaration" {
                    let info = extract_swift_function_info(&body_child, source, true);
                    methods.push(info);
                }
            }
        }
    }
    // Also check for body via field name
    if methods.is_empty() {
        if let Some(body) = node.child_by_field_name("body") {
            let mut body_cursor = body.walk();
            for body_child in body.children(&mut body_cursor) {
                if body_child.kind() == "function_declaration" {
                    let info = extract_swift_function_info(&body_child, source, true);
                    methods.push(info);
                }
            }
        }
    }

    ClassInfo {
        name,
        bases,
        docstring,
        methods,
        fields: Vec::new(),
        decorators: Vec::new(),
        line_number,
    }
}

fn extract_swift_bases(node: &Node, source: &str) -> Vec<String> {
    let mut bases = Vec::new();
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "inheritance_clause" || child.kind() == "type_inheritance_clause" {
            let mut inner_cursor = child.walk();
            for inner in child.children(&mut inner_cursor) {
                if inner.kind() == "type_identifier"
                    || inner.kind() == "user_type"
                    || inner.kind() == "simple_identifier"
                {
                    bases.push(get_node_text(&inner, source));
                }
                // Also check for inheritance_specifier wrapping type nodes
                if inner.kind() == "inheritance_specifier"
                    || inner.kind() == "annotated_inheritance_specifier"
                {
                    let mut spec_cursor = inner.walk();
                    for spec_child in inner.children(&mut spec_cursor) {
                        if spec_child.kind() == "type_identifier"
                            || spec_child.kind() == "user_type"
                            || spec_child.kind() == "simple_identifier"
                        {
                            bases.push(get_node_text(&spec_child, source));
                        }
                    }
                }
            }
        }
    }

    bases
}

// =============================================================================
// OCaml detailed extraction
// =============================================================================

fn extract_ocaml_functions_detailed(node: &Node, source: &str, functions: &mut Vec<FunctionInfo>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "value_definition" {
            // value_definition contains let_binding(s)
            let mut inner_cursor = child.walk();
            for inner in child.children(&mut inner_cursor) {
                if inner.kind() == "let_binding" {
                    // Only extract if it looks like a function (has parameters)
                    if ocaml_binding_has_params(&inner) {
                        let info = extract_ocaml_function_info(&inner, &child, source);
                        functions.push(info);
                    }
                }
            }
        }
        extract_ocaml_functions_detailed(&child, source, functions);
    }
}

/// Check if an OCaml let_binding has parameters (i.e., is a function definition).
/// A let_binding with just `pattern = body` and no `parameter` children is a value binding.
fn ocaml_binding_has_params(node: &Node) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "parameter" {
            return true;
        }
    }
    false
}

fn extract_ocaml_function_info(binding: &Node, definition: &Node, source: &str) -> FunctionInfo {
    // Name: the pattern field of the let_binding (value_name)
    let name = binding
        .child_by_field_name("pattern")
        .map(|n| get_node_text(&n, source))
        .unwrap_or_default();

    let params = extract_ocaml_params(binding, source);
    let return_type = extract_ocaml_return_type(binding, source);
    let docstring = extract_ocaml_docstring_before(definition, source);
    let line_number = definition.start_position().row as u32 + 1;

    FunctionInfo {
        name,
        params,
        return_type,
        docstring,
        is_method: false,
        is_async: false,
        decorators: Vec::new(),
        line_number,
    }
}

fn extract_ocaml_params(node: &Node, source: &str) -> Vec<String> {
    let mut params = Vec::new();

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "parameter" {
            // Parameter can have:
            //   - value_pattern (simple: `x`)
            //   - typed_pattern (typed: `(x : int)`)
            // Look for the pattern field
            if let Some(pattern) = child.child_by_field_name("pattern") {
                match pattern.kind() {
                    "value_pattern" => {
                        params.push(get_node_text(&pattern, source));
                    }
                    "typed_pattern" => {
                        // Inside typed_pattern, the pattern field holds the value_pattern
                        if let Some(inner_pat) = pattern.child_by_field_name("pattern") {
                            params.push(get_node_text(&inner_pat, source));
                        } else {
                            // Fallback: first value_pattern or identifier child
                            let mut inner_cursor = pattern.walk();
                            for inner in pattern.children(&mut inner_cursor) {
                                if inner.kind() == "value_pattern" || inner.kind() == "value_name" {
                                    params.push(get_node_text(&inner, source));
                                    break;
                                }
                            }
                        }
                    }
                    "tuple_pattern" | "cons_pattern" | "unit" => {
                        // Complex patterns -- use the whole text
                        params.push(get_node_text(&pattern, source));
                    }
                    _ => {
                        params.push(get_node_text(&pattern, source));
                    }
                }
            } else {
                // No pattern field, try first child that's a value_pattern
                let mut inner_cursor = child.walk();
                for inner in child.children(&mut inner_cursor) {
                    if inner.kind() == "value_pattern" || inner.kind() == "value_name" {
                        params.push(get_node_text(&inner, source));
                        break;
                    }
                }
            }
        }
    }

    params
}

/// Extract return type annotation from an OCaml let_binding.
/// Pattern: `let add (x : int) (y : int) : int = ...`
/// The `:` + type appears after all parameters and before `=`.
fn extract_ocaml_return_type(node: &Node, source: &str) -> Option<String> {
    // Walk children in order: look for `:` after the last parameter
    // and before `=`.
    let mut last_was_colon = false;
    let mut past_all_params = false;
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        let kind = child.kind();

        if kind == "parameter" {
            past_all_params = false; // Still have params
            last_was_colon = false;
            continue;
        }

        // After the last parameter
        if kind != "parameter" && !past_all_params {
            past_all_params = true;
        }

        if past_all_params && kind == ":" {
            last_was_colon = true;
            continue;
        }

        if last_was_colon && kind == "=" {
            // The colon was part of the binding, not a return type annotation
            return None;
        }

        if last_was_colon && kind != "=" {
            // This is the return type node
            let type_text = get_node_text(&child, source).trim().to_string();
            if !type_text.is_empty() {
                return Some(type_text);
            }
            last_was_colon = false;
        }

        if kind == "=" {
            break;
        }
    }

    None
}

/// Extract OCaml doc comment before a value_definition node.
/// OCaml doc comments use `(** ... *)` format.
fn extract_ocaml_docstring_before(node: &Node, source: &str) -> Option<String> {
    let mut prev = node.prev_sibling();

    while let Some(sibling) = prev {
        if sibling.kind() == "comment" {
            let text = get_node_text(&sibling, source);
            let trimmed = text.trim();
            if trimmed.starts_with("(**") {
                // OCaml doc comment
                let inner = trimmed
                    .strip_prefix("(**")
                    .and_then(|s| s.strip_suffix("*)"))
                    .unwrap_or(trimmed);
                return Some(inner.trim().to_string());
            }
            // Regular comment, keep looking
            prev = sibling.prev_sibling();
        } else {
            break;
        }
    }

    None
}

// =============================================================================
// Call graph building
// =============================================================================

fn build_intra_file_call_graph(
    tree: &Tree,
    source: &str,
    language: Language,
    functions: &[FunctionInfo],
    classes: &[ClassInfo],
) -> IntraFileCallGraph {
    let mut calls: HashMap<String, Vec<String>> = HashMap::new();
    let mut called_by: HashMap<String, Vec<String>> = HashMap::new();

    // Build set of known function and class names
    let known_functions: std::collections::HashSet<String> = functions
        .iter()
        .map(|f| f.name.clone())
        .chain(classes.iter().map(|c| c.name.clone()))
        .chain(
            classes
                .iter()
                .flat_map(|c| c.methods.iter().map(|m| m.name.clone())),
        )
        .collect();

    let root = tree.root_node();

    // Extract calls from each function
    for func in functions {
        let func_calls =
            extract_calls_in_function(&root, source, &func.name, &known_functions, language);
        if !func_calls.is_empty() {
            calls.insert(func.name.clone(), func_calls.clone());
            for callee in func_calls {
                called_by.entry(callee).or_default().push(func.name.clone());
            }
        }
    }

    // Extract calls from each method
    for class in classes {
        for method in &class.methods {
            let method_calls =
                extract_calls_in_function(&root, source, &method.name, &known_functions, language);
            if !method_calls.is_empty() {
                calls.insert(method.name.clone(), method_calls.clone());
                for callee in method_calls {
                    called_by
                        .entry(callee)
                        .or_default()
                        .push(method.name.clone());
                }
            }
        }
    }

    IntraFileCallGraph { calls, called_by }
}

fn extract_calls_in_function(
    root: &Node,
    source: &str,
    function_name: &str,
    known_functions: &std::collections::HashSet<String>,
    language: Language,
) -> Vec<String> {
    let mut calls = Vec::new();

    // Find the function node and extract calls from it
    find_and_extract_calls(
        root,
        source,
        function_name,
        known_functions,
        &mut calls,
        language,
    );

    calls.sort();
    calls.dedup();
    calls
}

fn find_and_extract_calls(
    node: &Node,
    source: &str,
    target_name: &str,
    known_functions: &std::collections::HashSet<String>,
    calls: &mut Vec<String>,
    language: Language,
) {
    let func_kinds: &[&str] = match language {
        Language::Python => &["function_definition"],
        Language::TypeScript | Language::JavaScript => {
            &["function_declaration", "method_definition"]
        }
        Language::Go => &["function_declaration", "method_declaration"],
        Language::Rust => &["function_item"],
        Language::Java => &["method_declaration"],
        _ => &[],
    };

    // Use cursor-based tree walk to find ALL functions matching the target name.
    // When multiple classes define methods with the same name, we must extract
    // calls from ALL of them (their calls get merged into one entry).
    let mut cursor = node.walk();
    let mut reached_root = false;
    loop {
        let walk_node = cursor.node();

        let is_matching_func = if func_kinds.contains(&walk_node.kind()) {
            // Standard function/method declaration
            walk_node
                .child_by_field_name("name")
                .is_some_and(|n| get_node_text(&n, source) == target_name)
        } else if walk_node.kind() == "variable_declarator" {
            // Arrow function or function expression: const foo = () => {}
            let name_matches = walk_node
                .child_by_field_name("name")
                .is_some_and(|n| get_node_text(&n, source) == target_name);
            let has_func_value = walk_node.child_by_field_name("value").is_some_and(|v| {
                matches!(
                    v.kind(),
                    "arrow_function" | "function_expression" | "function"
                )
            });
            name_matches && has_func_value
        } else {
            false
        };

        if is_matching_func {
            // Found a matching function, extract calls from its body
            extract_call_expressions(&walk_node, source, known_functions, calls, language);
            // Do NOT return early -- continue searching for other
            // functions with the same name (e.g., same-named methods
            // in different classes). Skip children of this function
            // to avoid re-processing.
            if !cursor.goto_next_sibling() {
                loop {
                    if !cursor.goto_parent() {
                        reached_root = true;
                        break;
                    }
                    if cursor.goto_next_sibling() {
                        break;
                    }
                }
                if reached_root {
                    break;
                }
            }
            continue;
        }

        // Advance cursor: depth-first traversal
        if cursor.goto_first_child() {
            continue;
        }
        if cursor.goto_next_sibling() {
            continue;
        }
        loop {
            if !cursor.goto_parent() {
                reached_root = true;
                break;
            }
            if cursor.goto_next_sibling() {
                break;
            }
        }
        if reached_root {
            break;
        }
    }
}

fn extract_call_expressions(
    node: &Node,
    source: &str,
    known_functions: &std::collections::HashSet<String>,
    calls: &mut Vec<String>,
    language: Language,
) {
    let call_kinds: &[&str] = match language {
        Language::Python => &["call"],
        Language::TypeScript | Language::JavaScript => &["call_expression"],
        Language::Go => &["call_expression"],
        Language::Rust => &["call_expression"],
        Language::Java => &["method_invocation"],
        _ => &[],
    };

    // Use cursor-based tree walk to visit ALL descendant nodes.
    // This is more robust than recursive children iteration and ensures
    // no nodes are missed inside conditional branches, loops, try/except,
    // match statements, comprehensions, or any other nested structure.
    let mut cursor = node.walk();
    let mut reached_root = false;
    loop {
        let walk_node = cursor.node();

        if call_kinds.contains(&walk_node.kind()) {
            // Get the function name being called
            let callee_name = match language {
                Language::Python => walk_node
                    .child_by_field_name("function")
                    .map(|n| get_node_text(&n, source)),
                Language::TypeScript | Language::JavaScript | Language::Go => walk_node
                    .child_by_field_name("function")
                    .map(|n| get_node_text(&n, source)),
                Language::Rust => walk_node
                    .child_by_field_name("function")
                    .map(|n| get_node_text(&n, source)),
                Language::Java => walk_node
                    .child_by_field_name("name")
                    .map(|n| get_node_text(&n, source)),
                _ => None,
            };

            if let Some(callee) = callee_name {
                // For cross-file call detection, we need ALL calls, not just local ones.
                // Include the full callee name (e.g., "module.func") for cross-file resolution,
                // and also the simple name for intra-file matching.
                let simple_name = callee
                    .split('.')
                    .next_back()
                    .unwrap_or(&callee)
                    .to_string();
                if known_functions.contains(&simple_name) {
                    // Local function call - use simple name
                    calls.push(simple_name);
                } else {
                    // Potentially cross-file call - preserve full callee name for resolution
                    calls.push(callee.to_string());
                }
            }
        }

        // Advance cursor: depth-first traversal
        if cursor.goto_first_child() {
            continue;
        }
        if cursor.goto_next_sibling() {
            continue;
        }
        // Walk back up until we can go to a sibling
        loop {
            if !cursor.goto_parent() {
                reached_root = true;
                break;
            }
            if cursor.goto_next_sibling() {
                break;
            }
        }
        if reached_root {
            break;
        }
    }
}

// =============================================================================
// Helper functions
// =============================================================================

fn get_node_text(node: &Node, source: &str) -> String {
    source[node.byte_range()].to_string()
}

fn extract_string_content(node: &Node, source: &str) -> String {
    let text = get_node_text(node, source);
    // Remove string delimiters
    text.trim_matches(|c| c == '"' || c == '\'' || c == '`')
        .trim_matches('"')
        .to_string()
}

fn is_inside_class(node: &Node) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "class_definition" | "class_declaration" | "class" | "class_body" => return true,
            _ => current = parent.parent(),
        }
    }
    false
}

fn is_inside_impl(node: &Node) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "impl_item" {
            return true;
        }
        current = parent.parent();
    }
    false
}

fn has_async_keyword(node: &Node, source: &str) -> bool {
    get_node_text(node, source).starts_with("async")
}

// =============================================================================
// C detailed extraction
// =============================================================================

fn extract_c_functions_detailed(node: &Node, source: &str, functions: &mut Vec<FunctionInfo>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "function_definition" {
            let info = extract_c_function_info(&child, source);
            functions.push(info);
        }
        extract_c_functions_detailed(&child, source, functions);
    }
}

fn extract_c_function_info(node: &Node, source: &str) -> FunctionInfo {
    let name = extract_c_function_name(node, source).unwrap_or_default();

    let params = extract_c_params(node, source);
    let return_type = node
        .child_by_field_name("type")
        .map(|n| get_node_text(&n, source));

    let docstring = extract_c_docstring(node, source);
    let line_number = node.start_position().row as u32 + 1;

    FunctionInfo {
        name,
        params,
        return_type,
        docstring,
        is_method: false,
        is_async: false,
        decorators: Vec::new(),
        line_number,
    }
}

/// Extract function name from C/C++ function_definition node.
/// Handles: `int foo(...)`, `void *foo(...)`, `int (*foo)(...)` patterns.
/// AST: function_definition -> declarator (function_declarator) -> declarator (identifier)
/// May have pointer_declarator wrapping the identifier.
fn extract_c_function_name(node: &Node, source: &str) -> Option<String> {
    let declarator = node.child_by_field_name("declarator")?;

    if declarator.kind() == "function_declarator" {
        return extract_name_from_function_declarator(&declarator, source);
    }

    // Sometimes the declarator is a pointer_declarator wrapping a function_declarator
    if declarator.kind() == "pointer_declarator" {
        let mut cursor = declarator.walk();
        for child in declarator.children(&mut cursor) {
            if child.kind() == "function_declarator" {
                return extract_name_from_function_declarator(&child, source);
            }
        }
    }

    // Fallback: declarator is directly an identifier (rare)
    if declarator.kind() == "identifier" {
        return Some(get_node_text(&declarator, source));
    }

    None
}

/// Extract the identifier name from a function_declarator node.
/// Handles pointer_declarator wrapping (e.g., `*get_ptr`).
fn extract_name_from_function_declarator(func_decl: &Node, source: &str) -> Option<String> {
    if let Some(name_node) = func_decl.child_by_field_name("declarator") {
        if name_node.kind() == "identifier" {
            return Some(get_node_text(&name_node, source));
        }
        // pointer_declarator wrapping: void *get_ptr(...)
        if name_node.kind() == "pointer_declarator" {
            let mut cursor = name_node.walk();
            for child in name_node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    return Some(get_node_text(&child, source));
                }
            }
        }
        // qualified_identifier for C++ (e.g., ClassName::method)
        if name_node.kind() == "qualified_identifier" {
            // Return the full qualified name
            return Some(get_node_text(&name_node, source));
        }
        // destructor_name for C++ (e.g., ~ClassName)
        if name_node.kind() == "destructor_name" {
            return Some(get_node_text(&name_node, source));
        }
    }
    None
}

fn extract_c_params(node: &Node, source: &str) -> Vec<String> {
    let mut params = Vec::new();

    // Navigate: function_definition -> declarator (function_declarator) -> parameters
    let declarator = match node.child_by_field_name("declarator") {
        Some(d) => d,
        None => return params,
    };

    let func_decl = if declarator.kind() == "function_declarator" {
        declarator
    } else if declarator.kind() == "pointer_declarator" {
        // Find function_declarator inside pointer_declarator
        let mut found = None;
        let mut cursor = declarator.walk();
        for child in declarator.children(&mut cursor) {
            if child.kind() == "function_declarator" {
                found = Some(child);
                break;
            }
        }
        match found {
            Some(f) => f,
            None => return params,
        }
    } else {
        return params;
    };

    if let Some(params_node) = func_decl.child_by_field_name("parameters") {
        let mut cursor = params_node.walk();
        for child in params_node.children(&mut cursor) {
            if child.kind() == "parameter_declaration" {
                // The parameter name is in the "declarator" field
                if let Some(decl) = child.child_by_field_name("declarator") {
                    let name = extract_c_param_name(&decl, source);
                    if !name.is_empty() {
                        params.push(name);
                    }
                }
                // If no declarator field, this is a type-only param (e.g., `void`)
            }
        }
    }

    params
}

/// Extract parameter name from a declarator node, handling pointer wrappers.
fn extract_c_param_name(decl: &Node, source: &str) -> String {
    match decl.kind() {
        "identifier" => get_node_text(decl, source),
        "pointer_declarator" => {
            // *name -> find the identifier inside
            let mut cursor = decl.walk();
            for child in decl.children(&mut cursor) {
                if child.kind() == "identifier" {
                    return get_node_text(&child, source);
                }
            }
            String::new()
        }
        "array_declarator" => {
            // name[] or name[N]
            if let Some(inner) = decl.child_by_field_name("declarator") {
                return extract_c_param_name(&inner, source);
            }
            String::new()
        }
        _ => get_node_text(decl, source),
    }
}

/// Extract docstring from comment node immediately before the function_definition.
/// Supports both /* ... */ block comments and consecutive // line comments.
fn extract_c_docstring(node: &Node, source: &str) -> Option<String> {
    let mut prev = node.prev_sibling();

    // Collect consecutive comment nodes immediately before the function
    let mut comment_lines: Vec<String> = Vec::new();

    while let Some(prev_node) = prev {
        if prev_node.kind() == "comment" {
            let text = get_node_text(&prev_node, source);
            // Block comment: return immediately
            if text.starts_with("/*") {
                // If we already collected line comments, those are closer to the function
                if !comment_lines.is_empty() {
                    break;
                }
                return Some(text);
            }
            // Line comment: collect (we're going backwards)
            if text.starts_with("//") {
                comment_lines.push(text);
            } else {
                break;
            }
            prev = prev_node.prev_sibling();
        } else {
            break;
        }
    }

    if comment_lines.is_empty() {
        None
    } else {
        comment_lines.reverse();
        Some(comment_lines.join("\n"))
    }
}

// =============================================================================
// C++ detailed extraction
// =============================================================================

fn extract_cpp_functions_detailed(node: &Node, source: &str, functions: &mut Vec<FunctionInfo>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "function_definition" {
            // Only top-level functions (not inside class/struct bodies)
            if !is_inside_cpp_class(&child) {
                let info = extract_cpp_function_info(&child, source, false);
                functions.push(info);
            }
        }
        // Recurse, but skip class/struct bodies (methods handled in class extraction)
        if child.kind() != "class_specifier"
            && child.kind() != "struct_specifier"
            && child.kind() != "field_declaration_list"
        {
            extract_cpp_functions_detailed(&child, source, functions);
        }
    }
}

fn extract_cpp_function_info(node: &Node, source: &str, is_method: bool) -> FunctionInfo {
    let name = extract_c_function_name(node, source).unwrap_or_default();

    let params = extract_c_params(node, source);
    let return_type = node
        .child_by_field_name("type")
        .map(|n| get_node_text(&n, source));

    let docstring = extract_c_docstring(node, source);
    let line_number = node.start_position().row as u32 + 1;

    // Check for virtual keyword in the source text of the function
    let text = get_node_text(node, source);
    let mut decorators = Vec::new();
    if text.contains("virtual ") {
        decorators.push("virtual".to_string());
    }
    if text.contains("static ") {
        decorators.push("static".to_string());
    }

    FunctionInfo {
        name,
        params,
        return_type,
        docstring,
        is_method,
        is_async: false,
        decorators,
        line_number,
    }
}

fn is_inside_cpp_class(node: &Node) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "class_specifier" | "struct_specifier" | "field_declaration_list" => return true,
            _ => current = parent.parent(),
        }
    }
    false
}

fn extract_cpp_classes_detailed(node: &Node, source: &str, classes: &mut Vec<ClassInfo>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "class_specifier" || child.kind() == "struct_specifier" {
            let info = extract_cpp_class_info(&child, source);
            // Only add named classes/structs (skip anonymous)
            if !info.name.is_empty() {
                classes.push(info);
            }
        }
        extract_cpp_classes_detailed(&child, source, classes);
    }
}

fn extract_cpp_class_info(node: &Node, source: &str) -> ClassInfo {
    let name = node
        .child_by_field_name("name")
        .map(|n| get_node_text(&n, source))
        .unwrap_or_default();

    let bases = extract_cpp_bases(node, source);
    let docstring = extract_c_docstring(node, source);
    let line_number = node.start_position().row as u32 + 1;

    // Extract methods from class body
    let mut methods = Vec::new();
    if let Some(body) = node.child_by_field_name("body") {
        extract_cpp_methods_from_body(&body, source, &mut methods);
    }

    ClassInfo {
        name,
        bases,
        docstring,
        methods,
        fields: Vec::new(),
        decorators: Vec::new(),
        line_number,
    }
}

/// Extract base classes from C++ class/struct specifier.
/// Looks for base_class_clause child, then extracts type_identifier children.
fn extract_cpp_bases(node: &Node, source: &str) -> Vec<String> {
    let mut bases = Vec::new();
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "base_class_clause" {
            let mut inner_cursor = child.walk();
            for inner in child.children(&mut inner_cursor) {
                if inner.kind() == "type_identifier"
                    || inner.kind() == "qualified_identifier"
                    || inner.kind() == "template_type"
                {
                    bases.push(get_node_text(&inner, source));
                }
            }
        }
    }

    bases
}

/// Extract method definitions from a C++ class body (field_declaration_list).
/// Skips access_specifier nodes (public/private/protected).
fn extract_cpp_methods_from_body(body: &Node, source: &str, methods: &mut Vec<FunctionInfo>) {
    let mut cursor = body.walk();

    for child in body.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                let info = extract_cpp_function_info(&child, source, true);
                methods.push(info);
            }
            "declaration" => {
                // Handle inline method declarations that have a body
                // e.g., `int foo() { ... }` inside a class that tree-sitter parses as declaration
                // Usually these are just declarations without body, skip them
            }
            "access_specifier" => {
                // Skip public:/private:/protected:
            }
            _ => {}
        }
    }
}

// =============================================================================
// Ruby detailed extraction
// =============================================================================

fn extract_ruby_functions_detailed(node: &Node, source: &str, functions: &mut Vec<FunctionInfo>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "method" | "singleton_method" => {
                // Only top-level functions (not inside class/module)
                if !is_inside_ruby_class(&child) {
                    let info = extract_ruby_function_info(&child, source, false);
                    functions.push(info);
                }
            }
            "class" | "module" => {
                // Don't recurse into classes for top-level function extraction
            }
            _ => {
                extract_ruby_functions_detailed(&child, source, functions);
            }
        }
    }
}

fn extract_ruby_function_info(node: &Node, source: &str, is_method: bool) -> FunctionInfo {
    let name = node
        .child_by_field_name("name")
        .map(|n| get_node_text(&n, source))
        .unwrap_or_default();

    let params = extract_ruby_params(node, source);
    let docstring = extract_ruby_docstring(node, source);
    let line_number = node.start_position().row as u32 + 1;

    // singleton_method => class method (self.foo)
    let is_singleton = node.kind() == "singleton_method";

    let mut decorators = Vec::new();
    if is_singleton {
        decorators.push("self".to_string());
    }

    FunctionInfo {
        name,
        params,
        return_type: None, // Ruby is dynamically typed
        docstring,
        is_method,
        is_async: false,
        decorators,
        line_number,
    }
}

fn extract_ruby_params(node: &Node, source: &str) -> Vec<String> {
    let mut params = Vec::new();

    if let Some(params_node) = node.child_by_field_name("parameters") {
        let mut cursor = params_node.walk();
        for child in params_node.children(&mut cursor) {
            match child.kind() {
                "identifier" => {
                    params.push(get_node_text(&child, source));
                }
                "optional_parameter" => {
                    // name = default_value
                    if let Some(name_node) = child.child_by_field_name("name") {
                        params.push(get_node_text(&name_node, source));
                    }
                }
                "splat_parameter" => {
                    // *args
                    let text = get_node_text(&child, source);
                    params.push(text);
                }
                "hash_splat_parameter" => {
                    // **kwargs
                    let text = get_node_text(&child, source);
                    params.push(text);
                }
                "block_parameter" => {
                    // &block
                    let text = get_node_text(&child, source);
                    params.push(text);
                }
                "keyword_parameter" => {
                    // name: or name: default
                    if let Some(name_node) = child.child_by_field_name("name") {
                        params.push(get_node_text(&name_node, source));
                    }
                }
                "destructured_parameter" => {
                    // (a, b) - destructured
                    let text = get_node_text(&child, source);
                    params.push(text);
                }
                _ => {}
            }
        }
    }

    params
}

/// Extract docstring from consecutive comment nodes immediately before the method.
/// Ruby uses # style comments. Consecutive # lines form a docstring.
/// Extract docstring from consecutive comment nodes immediately before the method.
/// Ruby uses # style comments. Consecutive # lines form a docstring.
///
/// In Ruby's tree-sitter grammar, methods inside a class are wrapped in
/// `body_statement`, but comments sit as siblings of `body_statement`
/// under the `class` node. So when `node.prev_sibling()` yields nothing
/// (method is first child of body_statement), we try
/// `node.parent(body_statement).prev_sibling()` to reach the comment.
fn extract_ruby_docstring(node: &Node, source: &str) -> Option<String> {
    // Try direct prev sibling first, then walk up through body_statement
    let first_prev = node
        .prev_sibling()
        .or_else(|| {
            node.parent()
                .filter(|p| p.kind() == "body_statement")
                .and_then(|p| p.prev_sibling())
        });
    let mut prev = first_prev;
    let mut comment_lines: Vec<String> = Vec::new();
    while let Some(prev_node) = prev {
        if prev_node.kind() == "comment" {
            let text = get_node_text(&prev_node, source);
            comment_lines.push(text);
            prev = prev_node.prev_sibling();
        } else {
            break;
        }
    }
    if comment_lines.is_empty() {
        None
    } else {
        comment_lines.reverse();
        Some(comment_lines.join("\n"))
    }
}

fn is_inside_ruby_class(node: &Node) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "class" | "module" | "body_statement" => {
                // body_statement is the body of a class/module
                // Check if its parent is a class/module
                if parent.kind() == "body_statement" {
                    if let Some(grandparent) = parent.parent() {
                        if grandparent.kind() == "class" || grandparent.kind() == "module" {
                            return true;
                        }
                    }
                    current = parent.parent();
                    continue;
                }
                return true;
            }
            _ => current = parent.parent(),
        }
    }
    false
}

fn extract_ruby_classes_detailed(node: &Node, source: &str, classes: &mut Vec<ClassInfo>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "class" => {
                let info = extract_ruby_class_info(&child, source);
                classes.push(info);
            }
            "module" => {
                // Treat modules as class-like constructs
                let info = extract_ruby_module_info(&child, source);
                classes.push(info);
            }
            _ => {
                extract_ruby_classes_detailed(&child, source, classes);
            }
        }
    }
}

fn extract_ruby_class_info(node: &Node, source: &str) -> ClassInfo {
    let name = node
        .child_by_field_name("name")
        .map(|n| get_node_text(&n, source))
        .unwrap_or_default();

    let mut bases = Vec::new();
    if let Some(superclass) = node.child_by_field_name("superclass") {
        bases.push(get_node_text(&superclass, source));
    }

    let docstring = extract_ruby_docstring(node, source);
    let line_number = node.start_position().row as u32 + 1;

    // Extract methods from class body
    let mut methods = Vec::new();
    if let Some(body) = node.child_by_field_name("body") {
        extract_ruby_methods_from_body(&body, source, &mut methods);
    }

    ClassInfo {
        name,
        bases,
        docstring,
        methods,
        fields: Vec::new(),
        decorators: Vec::new(),
        line_number,
    }
}

fn extract_ruby_module_info(node: &Node, source: &str) -> ClassInfo {
    let name = node
        .child_by_field_name("name")
        .map(|n| get_node_text(&n, source))
        .unwrap_or_default();

    let docstring = extract_ruby_docstring(node, source);
    let line_number = node.start_position().row as u32 + 1;

    // Extract methods from module body
    let mut methods = Vec::new();
    if let Some(body) = node.child_by_field_name("body") {
        extract_ruby_methods_from_body(&body, source, &mut methods);
    }

    ClassInfo {
        name,
        bases: Vec::new(), // Modules don't have superclasses
        docstring,
        methods,
        fields: Vec::new(),
        decorators: vec!["module".to_string()],
        line_number,
    }
}

/// Extract methods from a Ruby class/module body.
/// The body is a body_statement node containing method definitions.
fn extract_ruby_methods_from_body(body: &Node, source: &str, methods: &mut Vec<FunctionInfo>) {
    let mut cursor = body.walk();

    for child in body.children(&mut cursor) {
        match child.kind() {
            "method" | "singleton_method" => {
                let info = extract_ruby_function_info(&child, source, true);
                methods.push(info);
            }
            _ => {
                // Recurse to find methods in nested blocks (e.g., inside `begin`/`rescue`)
                extract_ruby_methods_from_body(&child, source, methods);
            }
        }
    }
}

// =============================================================================
// PHP detailed extraction
// =============================================================================

fn extract_php_functions_detailed(node: &Node, source: &str, functions: &mut Vec<FunctionInfo>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                // Standalone function (not a method)
                let info = extract_php_function_info(&child, source, false);
                functions.push(info);
            }
            "class_declaration" | "interface_declaration" | "trait_declaration" => {
                // Don't recurse into classes for top-level function extraction
            }
            _ => {
                extract_php_functions_detailed(&child, source, functions);
            }
        }
    }
}

fn extract_php_function_info(node: &Node, source: &str, is_method: bool) -> FunctionInfo {
    let name = node
        .child_by_field_name("name")
        .map(|n| get_node_text(&n, source))
        .unwrap_or_default();

    let params = extract_php_params(node, source);
    let return_type = extract_php_return_type(node, source);
    let docstring = extract_php_docstring(node, source);
    let line_number = node.start_position().row as u32 + 1;

    // Check for visibility and static modifiers on method_declaration
    let mut decorators = Vec::new();
    if is_method {
        let text = get_node_text(node, source);
        if text.starts_with("public ") || text.contains(" public ") {
            decorators.push("public".to_string());
        } else if text.starts_with("private ") || text.contains(" private ") {
            decorators.push("private".to_string());
        } else if text.starts_with("protected ") || text.contains(" protected ") {
            decorators.push("protected".to_string());
        }
        if text.contains("static ") {
            decorators.push("static".to_string());
        }
        if text.contains("abstract ") {
            decorators.push("abstract".to_string());
        }
    }

    FunctionInfo {
        name,
        params,
        return_type,
        docstring,
        is_method,
        is_async: false,
        decorators,
        line_number,
    }
}

fn extract_php_params(node: &Node, source: &str) -> Vec<String> {
    let mut params = Vec::new();

    if let Some(params_node) = node.child_by_field_name("parameters") {
        // params_node is formal_parameters
        let mut cursor = params_node.walk();
        for child in params_node.children(&mut cursor) {
            if child.kind() == "simple_parameter" || child.kind() == "variadic_parameter" {
                // The parameter name is in the "name" field (starts with $)
                if let Some(name_node) = child.child_by_field_name("name") {
                    params.push(get_node_text(&name_node, source));
                }
            } else if child.kind() == "property_promotion_parameter" {
                // PHP 8 constructor promotion: public readonly string $name
                if let Some(name_node) = child.child_by_field_name("name") {
                    params.push(get_node_text(&name_node, source));
                }
            }
        }
    }

    params
}

/// Extract return type from PHP function/method.
/// Looks for a return_type field on the node.
fn extract_php_return_type(node: &Node, source: &str) -> Option<String> {
    // tree-sitter-php uses "return_type" field
    if let Some(rt) = node.child_by_field_name("return_type") {
        let text = get_node_text(&rt, source);
        // Remove leading colon and whitespace if present
        let cleaned = text.trim_start_matches(':').trim().to_string();
        if !cleaned.is_empty() {
            return Some(cleaned);
        }
    }

    // Fallback: scan children for a ":" followed by a type node
    // This handles cases where the grammar doesn't expose a return_type field
    let mut cursor = node.walk();
    let mut found_colon = false;
    for child in node.children(&mut cursor) {
        if child.kind() == ":" {
            found_colon = true;
            continue;
        }
        if found_colon {
            let kind = child.kind();
            // Type nodes in PHP grammar
            if kind == "named_type"
                || kind == "primitive_type"
                || kind == "optional_type"
                || kind == "union_type"
                || kind == "intersection_type"
                || kind == "name"
                || kind == "qualified_name"
            {
                return Some(get_node_text(&child, source));
            }
            found_colon = false;
        }
    }

    None
}

/// Extract docstring from PHPDoc comment (/** ... */) immediately before the function/method.
fn extract_php_docstring(node: &Node, source: &str) -> Option<String> {
    if let Some(prev_node) = node.prev_sibling() {
        if prev_node.kind() == "comment" {
            let text = get_node_text(&prev_node, source);
            if text.starts_with("/**") {
                return Some(text);
            }
            // Regular // or /* comment - also accept as docstring
            if text.starts_with("/*") || text.starts_with("//") {
                return Some(text);
            }
        }
    }

    None
}

fn extract_php_classes_detailed(node: &Node, source: &str, classes: &mut Vec<ClassInfo>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_declaration" => {
                let info = extract_php_class_info(&child, source);
                classes.push(info);
            }
            "interface_declaration" => {
                let info = extract_php_interface_info(&child, source);
                classes.push(info);
            }
            "trait_declaration" => {
                let info = extract_php_trait_info(&child, source);
                classes.push(info);
            }
            _ => {
                extract_php_classes_detailed(&child, source, classes);
            }
        }
    }
}

fn extract_php_class_info(node: &Node, source: &str) -> ClassInfo {
    let name = node
        .child_by_field_name("name")
        .map(|n| get_node_text(&n, source))
        .unwrap_or_default();

    let bases = extract_php_bases(node, source);
    let docstring = extract_php_docstring(node, source);
    let line_number = node.start_position().row as u32 + 1;

    // Extract methods from class body
    let mut methods = Vec::new();
    if let Some(body) = node.child_by_field_name("body") {
        extract_php_methods_from_body(&body, source, &mut methods);
    }

    ClassInfo {
        name,
        bases,
        docstring,
        methods,
        fields: Vec::new(),
        decorators: Vec::new(),
        line_number,
    }
}

fn extract_php_interface_info(node: &Node, source: &str) -> ClassInfo {
    let name = node
        .child_by_field_name("name")
        .map(|n| get_node_text(&n, source))
        .unwrap_or_default();

    let docstring = extract_php_docstring(node, source);
    let line_number = node.start_position().row as u32 + 1;

    let mut methods = Vec::new();
    if let Some(body) = node.child_by_field_name("body") {
        extract_php_methods_from_body(&body, source, &mut methods);
    }

    ClassInfo {
        name,
        bases: Vec::new(),
        docstring,
        methods,
        fields: Vec::new(),
        decorators: vec!["interface".to_string()],
        line_number,
    }
}

fn extract_php_trait_info(node: &Node, source: &str) -> ClassInfo {
    let name = node
        .child_by_field_name("name")
        .map(|n| get_node_text(&n, source))
        .unwrap_or_default();

    let docstring = extract_php_docstring(node, source);
    let line_number = node.start_position().row as u32 + 1;

    let mut methods = Vec::new();
    if let Some(body) = node.child_by_field_name("body") {
        extract_php_methods_from_body(&body, source, &mut methods);
    }

    ClassInfo {
        name,
        bases: Vec::new(),
        docstring,
        methods,
        fields: Vec::new(),
        decorators: vec!["trait".to_string()],
        line_number,
    }
}

/// Extract base classes from PHP class_declaration.
/// Looks for base_clause (extends) and class_interface_clause (implements).
fn extract_php_bases(node: &Node, source: &str) -> Vec<String> {
    let mut bases = Vec::new();
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "base_clause" {
            // extends ClassName
            let mut inner_cursor = child.walk();
            for inner in child.children(&mut inner_cursor) {
                if inner.kind() == "name"
                    || inner.kind() == "qualified_name"
                    || inner.kind() == "named_type"
                {
                    bases.push(get_node_text(&inner, source));
                }
            }
        } else if child.kind() == "class_interface_clause" {
            // implements Interface1, Interface2
            let mut inner_cursor = child.walk();
            for inner in child.children(&mut inner_cursor) {
                if inner.kind() == "name"
                    || inner.kind() == "qualified_name"
                    || inner.kind() == "named_type"
                {
                    bases.push(get_node_text(&inner, source));
                }
            }
        }
    }

    bases
}

/// Extract method declarations from a PHP class body (declaration_list).
fn extract_php_methods_from_body(body: &Node, source: &str, methods: &mut Vec<FunctionInfo>) {
    let mut cursor = body.walk();

    for child in body.children(&mut cursor) {
        if child.kind() == "method_declaration" {
            let info = extract_php_function_info(&child, source, true);
            methods.push(info);
        }
    }
}

// =============================================================================
// CSharp detailed extraction
// =============================================================================

fn extract_csharp_functions_detailed(node: &Node, source: &str, functions: &mut Vec<FunctionInfo>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "method_declaration" | "constructor_declaration" => {
                let info = extract_csharp_function_info(&child, source);
                functions.push(info);
            }
            // Skip into class/struct/namespace bodies to find methods
            "class_declaration"
            | "struct_declaration"
            | "namespace_declaration"
            | "interface_declaration" => {
                if let Some(body) = child.child_by_field_name("body") {
                    extract_csharp_functions_detailed(&body, source, functions);
                }
            }
            _ => {
                extract_csharp_functions_detailed(&child, source, functions);
            }
        }
    }
}

fn extract_csharp_function_info(node: &Node, source: &str) -> FunctionInfo {
    let name = node
        .child_by_field_name("name")
        .map(|n| get_node_text(&n, source))
        .unwrap_or_else(|| {
            // For constructors, name might be the type_identifier child
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    return get_node_text(&child, source);
                }
            }
            String::new()
        });

    let params = extract_csharp_params(node, source);

    let return_type = node
        .child_by_field_name("type")
        .map(|n| get_node_text(&n, source));

    let docstring = extract_csharp_docstring(node, source);

    // Check for async modifier
    let is_async = {
        let mut cursor = node.walk();
        let mut found = false;
        for child in node.children(&mut cursor) {
            if child.kind() == "modifier" || child.kind() == "async" {
                let text = get_node_text(&child, source);
                if text == "async" {
                    found = true;
                    break;
                }
            }
        }
        found
    };

    let decorators = extract_csharp_attributes(node, source);
    let line_number = node.start_position().row as u32 + 1;

    FunctionInfo {
        name,
        params,
        return_type,
        docstring,
        is_method: true, // C# methods are always inside classes/structs
        is_async,
        decorators,
        line_number,
    }
}

fn extract_csharp_params(node: &Node, source: &str) -> Vec<String> {
    let mut params = Vec::new();

    if let Some(params_node) = node.child_by_field_name("parameters") {
        let mut cursor = params_node.walk();
        for child in params_node.children(&mut cursor) {
            if child.kind() == "parameter" {
                // Parameter name is the "name" field or the last identifier
                if let Some(name) = child.child_by_field_name("name") {
                    params.push(get_node_text(&name, source));
                } else {
                    // Fallback: find last identifier child
                    let mut inner_cursor = child.walk();
                    let mut last_ident = None;
                    for inner in child.children(&mut inner_cursor) {
                        if inner.kind() == "identifier" {
                            last_ident = Some(get_node_text(&inner, source));
                        }
                    }
                    if let Some(name) = last_ident {
                        params.push(name);
                    }
                }
            }
        }
    }

    params
}

fn extract_csharp_docstring(node: &Node, source: &str) -> Option<String> {
    // Look for preceding XML doc comments (/// comments)
    let mut prev = node.prev_sibling();
    let mut doc_lines = Vec::new();

    while let Some(sibling) = prev {
        if sibling.kind() == "comment" {
            let text = get_node_text(&sibling, source);
            if text.starts_with("///") {
                doc_lines.push(text.trim_start_matches("///").trim().to_string());
                prev = sibling.prev_sibling();
                continue;
            }
        }
        break;
    }

    if doc_lines.is_empty() {
        None
    } else {
        doc_lines.reverse();
        Some(doc_lines.join("\n"))
    }
}

fn extract_csharp_attributes(node: &Node, source: &str) -> Vec<String> {
    // Look for attribute_list siblings before the method
    let mut attrs = Vec::new();
    let mut prev = node.prev_sibling();

    while let Some(sibling) = prev {
        if sibling.kind() == "attribute_list" {
            let text = get_node_text(&sibling, source);
            // Remove surrounding brackets [...]
            let trimmed = text
                .trim_start_matches('[')
                .trim_end_matches(']')
                .to_string();
            attrs.push(trimmed);
            prev = sibling.prev_sibling();
            continue;
        }
        break;
    }

    attrs.reverse();
    attrs
}

fn extract_csharp_classes_detailed(node: &Node, source: &str, classes: &mut Vec<ClassInfo>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_declaration" | "struct_declaration" | "interface_declaration" => {
                let info = extract_csharp_class_info(&child, source);
                classes.push(info);
            }
            _ => {
                extract_csharp_classes_detailed(&child, source, classes);
            }
        }
    }
}

fn extract_csharp_class_info(node: &Node, source: &str) -> ClassInfo {
    let name = node
        .child_by_field_name("name")
        .map(|n| get_node_text(&n, source))
        .unwrap_or_default();

    let line_number = node.start_position().row as u32 + 1;

    // Extract base types from base_list
    let bases = extract_csharp_bases(node, source);

    // Extract methods from body
    let mut methods = Vec::new();
    if let Some(body) = node.child_by_field_name("body") {
        // Only extract methods directly inside this class body
        let mut body_cursor = body.walk();
        for body_child in body.children(&mut body_cursor) {
            if body_child.kind() == "method_declaration"
                || body_child.kind() == "constructor_declaration"
            {
                let info = extract_csharp_function_info(&body_child, source);
                methods.push(info);
            }
        }
    }

    let docstring = extract_csharp_docstring(node, source);

    ClassInfo {
        name,
        bases,
        docstring,
        methods,
        fields: Vec::new(),
        decorators: Vec::new(),
        line_number,
    }
}

fn extract_csharp_bases(node: &Node, source: &str) -> Vec<String> {
    let mut bases = Vec::new();

    if let Some(base_list) = node.child_by_field_name("bases") {
        let mut cursor = base_list.walk();
        for child in base_list.children(&mut cursor) {
            if child.kind() == "identifier"
                || child.kind() == "generic_name"
                || child.kind() == "qualified_name"
            {
                bases.push(get_node_text(&child, source));
            }
        }
    } else {
        // Fallback: look for base_list child node
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "base_list" {
                let mut inner = child.walk();
                for base_child in child.children(&mut inner) {
                    if base_child.kind() == "identifier"
                        || base_child.kind() == "generic_name"
                        || base_child.kind() == "qualified_name"
                    {
                        bases.push(get_node_text(&base_child, source));
                    }
                }
            }
        }
    }

    bases
}

// =============================================================================
// Kotlin detailed extraction
// =============================================================================

fn extract_kotlin_functions_detailed(node: &Node, source: &str, functions: &mut Vec<FunctionInfo>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                // Skip methods inside classes (they get extracted by class extractor)
                if !is_inside_kotlin_class(&child) {
                    let info = extract_kotlin_function_info(&child, source, false);
                    functions.push(info);
                }
            }
            "class_declaration" | "object_declaration" => {
                // Don't recurse into classes for top-level functions
            }
            _ => {
                extract_kotlin_functions_detailed(&child, source, functions);
            }
        }
    }
}

fn is_inside_kotlin_class(node: &Node) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "class_declaration" | "object_declaration" | "class_body" => return true,
            _ => current = parent.parent(),
        }
    }
    false
}

fn extract_kotlin_function_info(node: &Node, source: &str, is_method: bool) -> FunctionInfo {
    // Kotlin uses simple_identifier for function names, not a "name" field
    let name = node
        .child_by_field_name("name")
        .map(|n| get_node_text(&n, source))
        .unwrap_or_else(|| {
            // Fallback: find simple_identifier child
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "simple_identifier" {
                    return get_node_text(&child, source);
                }
            }
            String::new()
        });

    let params = extract_kotlin_params(node, source);
    let return_type = extract_kotlin_return_type(node, source);
    let docstring = extract_kotlin_docstring(node, source);

    // Check for suspend modifier (Kotlin's async)
    let is_async = {
        let mut cursor = node.walk();
        let mut found = false;
        for child in node.children(&mut cursor) {
            if child.kind() == "modifiers" {
                let mut mod_cursor = child.walk();
                for mod_child in child.children(&mut mod_cursor) {
                    let text = get_node_text(&mod_child, source);
                    if text == "suspend" {
                        found = true;
                        break;
                    }
                }
            }
        }
        found
    };

    let decorators = extract_kotlin_annotations(node, source);
    let line_number = node.start_position().row as u32 + 1;

    FunctionInfo {
        name,
        params,
        return_type,
        docstring,
        is_method,
        is_async,
        decorators,
        line_number,
    }
}

fn extract_kotlin_params(node: &Node, source: &str) -> Vec<String> {
    let mut params = Vec::new();

    // Look for function_value_parameters child
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "function_value_parameters" {
            let mut inner = child.walk();
            for param_wrapper in child.children(&mut inner) {
                if param_wrapper.kind() == "parameter" {
                    // Parameter has a simple_identifier as name
                    let mut param_cursor = param_wrapper.walk();
                    for param_child in param_wrapper.children(&mut param_cursor) {
                        if param_child.kind() == "simple_identifier" {
                            params.push(get_node_text(&param_child, source));
                            break;
                        }
                    }
                } else if param_wrapper.kind() == "function_value_parameter" {
                    // function_value_parameter wraps a parameter node
                    if let Some(param) = param_wrapper.child_by_field_name("parameter") {
                        let mut param_cursor = param.walk();
                        for param_child in param.children(&mut param_cursor) {
                            if param_child.kind() == "simple_identifier" {
                                params.push(get_node_text(&param_child, source));
                                break;
                            }
                        }
                    } else {
                        // Fallback: find simple_identifier directly
                        let mut param_cursor = param_wrapper.walk();
                        for param_child in param_wrapper.children(&mut param_cursor) {
                            if param_child.kind() == "simple_identifier" {
                                params.push(get_node_text(&param_child, source));
                                break;
                            }
                        }
                    }
                }
            }
            break;
        }
    }

    params
}

fn extract_kotlin_return_type(node: &Node, source: &str) -> Option<String> {
    // Look for user_type or type_reference after the colon following parameters
    let mut cursor = node.walk();
    let mut found_params = false;
    let mut found_colon = false;

    for child in node.children(&mut cursor) {
        if child.kind() == "function_value_parameters" {
            found_params = true;
            continue;
        }
        if found_params && get_node_text(&child, source) == ":" {
            found_colon = true;
            continue;
        }
        if found_colon {
            match child.kind() {
                "user_type" | "nullable_type" | "type_identifier" | "function_type"
                | "type_reference" => {
                    return Some(get_node_text(&child, source));
                }
                _ => {
                    // Might be the return type under a different node kind
                    if child.kind() != "function_body" && child.kind() != "{" {
                        return Some(get_node_text(&child, source));
                    }
                    break;
                }
            }
        }
    }

    None
}

fn extract_kotlin_docstring(node: &Node, source: &str) -> Option<String> {
    // KDoc: /** ... */ comment before the function
    let mut prev = node.prev_sibling();

    while let Some(sibling) = prev {
        if sibling.kind() == "multiline_comment" {
            let text = get_node_text(&sibling, source);
            if text.starts_with("/**") {
                return Some(text);
            }
        }
        // Skip over annotations/modifiers to find the doc comment
        if sibling.kind() == "modifiers" || sibling.kind() == "annotation" {
            prev = sibling.prev_sibling();
            continue;
        }
        break;
    }

    None
}

fn extract_kotlin_annotations(node: &Node, source: &str) -> Vec<String> {
    let mut annotations = Vec::new();

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "modifiers" {
            let mut mod_cursor = child.walk();
            for mod_child in child.children(&mut mod_cursor) {
                if mod_child.kind() == "annotation" {
                    let text = get_node_text(&mod_child, source);
                    // Remove leading @
                    annotations.push(text.trim_start_matches('@').to_string());
                }
            }
        }
    }

    annotations
}

fn extract_kotlin_classes_detailed(node: &Node, source: &str, classes: &mut Vec<ClassInfo>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_declaration" => {
                let info = extract_kotlin_class_info(&child, source);
                classes.push(info);
            }
            "object_declaration" => {
                let info = extract_kotlin_object_info(&child, source);
                classes.push(info);
            }
            _ => {
                extract_kotlin_classes_detailed(&child, source, classes);
            }
        }
    }
}

fn extract_kotlin_class_info(node: &Node, source: &str) -> ClassInfo {
    // Kotlin uses type_identifier for class name
    let name = {
        let mut cursor = node.walk();
        let mut found = String::new();
        for child in node.children(&mut cursor) {
            if child.kind() == "type_identifier" {
                found = get_node_text(&child, source);
                break;
            }
        }
        found
    };

    let line_number = node.start_position().row as u32 + 1;
    let bases = extract_kotlin_bases(node, source);
    let docstring = extract_kotlin_docstring(node, source);

    // Extract methods from class_body
    let mut methods = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "class_body" {
            let mut body_cursor = child.walk();
            for body_child in child.children(&mut body_cursor) {
                if body_child.kind() == "function_declaration" {
                    let info = extract_kotlin_function_info(&body_child, source, true);
                    methods.push(info);
                }
            }
        }
    }

    ClassInfo {
        name,
        bases,
        docstring,
        methods,
        fields: Vec::new(),
        decorators: Vec::new(),
        line_number,
    }
}

fn extract_kotlin_object_info(node: &Node, source: &str) -> ClassInfo {
    // Object declaration uses type_identifier for name
    let name = {
        let mut cursor = node.walk();
        let mut found = String::new();
        for child in node.children(&mut cursor) {
            if child.kind() == "type_identifier" {
                found = get_node_text(&child, source);
                break;
            }
        }
        found
    };

    let line_number = node.start_position().row as u32 + 1;

    // Extract methods from class_body
    let mut methods = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "class_body" {
            let mut body_cursor = child.walk();
            for body_child in child.children(&mut body_cursor) {
                if body_child.kind() == "function_declaration" {
                    let info = extract_kotlin_function_info(&body_child, source, true);
                    methods.push(info);
                }
            }
        }
    }

    ClassInfo {
        name,
        bases: Vec::new(),
        docstring: extract_kotlin_docstring(node, source),
        methods,
        fields: Vec::new(),
        decorators: Vec::new(),
        line_number,
    }
}

fn extract_kotlin_bases(node: &Node, source: &str) -> Vec<String> {
    let mut bases = Vec::new();

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "delegation_specifiers" {
            let mut inner = child.walk();
            for spec in child.children(&mut inner) {
                if spec.kind() == "delegation_specifier" {
                    // The user_type or constructor_invocation inside
                    let mut spec_cursor = spec.walk();
                    for spec_child in spec.children(&mut spec_cursor) {
                        if spec_child.kind() == "user_type"
                            || spec_child.kind() == "constructor_invocation"
                        {
                            // For constructor_invocation, get just the type name
                            let mut type_cursor = spec_child.walk();
                            for type_child in spec_child.children(&mut type_cursor) {
                                if type_child.kind() == "type_identifier"
                                    || type_child.kind() == "user_type"
                                {
                                    bases.push(get_node_text(&type_child, source));
                                    break;
                                }
                            }
                            break;
                        }
                        if spec_child.kind() == "type_identifier" {
                            bases.push(get_node_text(&spec_child, source));
                            break;
                        }
                    }
                }
                // Some grammars put user_type directly under delegation_specifiers
                if spec.kind() == "user_type" || spec.kind() == "constructor_invocation" {
                    let mut type_cursor = spec.walk();
                    for type_child in spec.children(&mut type_cursor) {
                        if type_child.kind() == "type_identifier" {
                            bases.push(get_node_text(&type_child, source));
                            break;
                        }
                    }
                }
            }
        }
    }

    bases
}

// =============================================================================
// Scala detailed extraction
// =============================================================================

fn extract_scala_functions_detailed(node: &Node, source: &str, functions: &mut Vec<FunctionInfo>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" | "function_declaration" => {
                // Skip methods inside classes (they get extracted by class extractor)
                if !is_inside_scala_class(&child) {
                    let info = extract_scala_function_info(&child, source, false);
                    functions.push(info);
                }
            }
            "class_definition" | "object_definition" | "trait_definition" => {
                // Don't recurse into class-like constructs for top-level functions
            }
            _ => {
                extract_scala_functions_detailed(&child, source, functions);
            }
        }
    }
}

fn is_inside_scala_class(node: &Node) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "class_definition" | "object_definition" | "trait_definition" | "template_body" => {
                return true
            }
            _ => current = parent.parent(),
        }
    }
    false
}

fn extract_scala_function_info(node: &Node, source: &str, is_method: bool) -> FunctionInfo {
    let name = node
        .child_by_field_name("name")
        .map(|n| get_node_text(&n, source))
        .unwrap_or_else(|| {
            // Fallback: find identifier child
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    return get_node_text(&child, source);
                }
            }
            String::new()
        });

    let params = extract_scala_params(node, source);
    let return_type = extract_scala_return_type(node, source);
    let docstring = extract_scala_docstring(node, source);
    let line_number = node.start_position().row as u32 + 1;

    FunctionInfo {
        name,
        params,
        return_type,
        docstring,
        is_method,
        is_async: false, // Scala handles async via Futures, not a keyword
        decorators: Vec::new(),
        line_number,
    }
}

fn extract_scala_params(node: &Node, source: &str) -> Vec<String> {
    let mut params = Vec::new();

    // Scala has parameters field or look for parameter lists
    if let Some(params_node) = node.child_by_field_name("parameters") {
        extract_scala_params_from_list(&params_node, source, &mut params);
    } else {
        // Look for parameters or class_parameters children
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "parameters" || child.kind() == "class_parameters" {
                extract_scala_params_from_list(&child, source, &mut params);
                break;
            }
        }
    }

    params
}

fn extract_scala_params_from_list(node: &Node, source: &str, params: &mut Vec<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_parameter" | "parameter" => {
                // Name is the first identifier
                if let Some(name) = child.child_by_field_name("name") {
                    params.push(get_node_text(&name, source));
                } else {
                    let mut inner = child.walk();
                    for inner_child in child.children(&mut inner) {
                        if inner_child.kind() == "identifier" {
                            params.push(get_node_text(&inner_child, source));
                            break;
                        }
                    }
                }
            }
            // Nested parameter lists (curried functions)
            "parameters" | "class_parameters" => {
                extract_scala_params_from_list(&child, source, params);
            }
            _ => {}
        }
    }
}

fn extract_scala_return_type(node: &Node, source: &str) -> Option<String> {
    // Look for the return type after `:` and before `=` or `{`
    let mut cursor = node.walk();
    let mut found_colon = false;

    for child in node.children(&mut cursor) {
        if get_node_text(&child, source) == ":" {
            found_colon = true;
            continue;
        }
        if found_colon {
            let text = get_node_text(&child, source);
            if text == "=" || text == "{" {
                break;
            }
            match child.kind() {
                "type_identifier"
                | "generic_type"
                | "compound_type"
                | "infix_type"
                | "tuple_type"
                | "function_type"
                | "parametrized_type"
                | "stable_type_identifier" => {
                    return Some(text);
                }
                _ => {
                    // Accept any non-punctuation node as potential return type
                    if !text.is_empty() && text != "=" && text != "{" {
                        return Some(text);
                    }
                    break;
                }
            }
        }
    }

    None
}

fn extract_scala_docstring(node: &Node, source: &str) -> Option<String> {
    // ScalaDoc: /** ... */ before the function
    let mut prev = node.prev_sibling();

    while let Some(sibling) = prev {
        match sibling.kind() {
            "comment" | "block_comment" => {
                let text = get_node_text(&sibling, source);
                if text.starts_with("/**") {
                    return Some(text);
                }
            }
            // Skip annotations/modifiers
            "annotation" | "modifiers" => {
                prev = sibling.prev_sibling();
                continue;
            }
            _ => break,
        }
        break;
    }

    None
}

fn extract_scala_classes_detailed(node: &Node, source: &str, classes: &mut Vec<ClassInfo>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_definition" => {
                let info = extract_scala_class_info(&child, source);
                classes.push(info);
            }
            "object_definition" => {
                let info = extract_scala_object_info(&child, source);
                classes.push(info);
            }
            "trait_definition" => {
                let info = extract_scala_trait_info(&child, source);
                classes.push(info);
            }
            _ => {
                extract_scala_classes_detailed(&child, source, classes);
            }
        }
    }
}

fn extract_scala_class_info(node: &Node, source: &str) -> ClassInfo {
    let name = node
        .child_by_field_name("name")
        .map(|n| get_node_text(&n, source))
        .unwrap_or_else(|| {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    return get_node_text(&child, source);
                }
            }
            String::new()
        });

    let line_number = node.start_position().row as u32 + 1;
    let bases = extract_scala_bases(node, source);
    let docstring = extract_scala_docstring(node, source);

    // Extract methods from template_body
    let mut methods = Vec::new();
    extract_scala_methods_from_body(node, source, &mut methods);

    ClassInfo {
        name,
        bases,
        docstring,
        methods,
        fields: Vec::new(),
        decorators: Vec::new(),
        line_number,
    }
}

fn extract_scala_object_info(node: &Node, source: &str) -> ClassInfo {
    let name = node
        .child_by_field_name("name")
        .map(|n| get_node_text(&n, source))
        .unwrap_or_else(|| {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    return get_node_text(&child, source);
                }
            }
            String::new()
        });

    let line_number = node.start_position().row as u32 + 1;
    let bases = extract_scala_bases(node, source);
    let docstring = extract_scala_docstring(node, source);

    let mut methods = Vec::new();
    extract_scala_methods_from_body(node, source, &mut methods);

    ClassInfo {
        name,
        bases,
        docstring,
        methods,
        fields: Vec::new(),
        decorators: Vec::new(),
        line_number,
    }
}

fn extract_scala_trait_info(node: &Node, source: &str) -> ClassInfo {
    let name = node
        .child_by_field_name("name")
        .map(|n| get_node_text(&n, source))
        .unwrap_or_else(|| {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    return get_node_text(&child, source);
                }
            }
            String::new()
        });

    let line_number = node.start_position().row as u32 + 1;
    let bases = extract_scala_bases(node, source);
    let docstring = extract_scala_docstring(node, source);

    let mut methods = Vec::new();
    extract_scala_methods_from_body(node, source, &mut methods);

    ClassInfo {
        name,
        bases,
        docstring,
        methods,
        fields: Vec::new(),
        decorators: Vec::new(),
        line_number,
    }
}

fn extract_scala_methods_from_body(node: &Node, source: &str, methods: &mut Vec<FunctionInfo>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "template_body" || child.kind() == "body" {
            let mut body_cursor = child.walk();
            for body_child in child.children(&mut body_cursor) {
                if body_child.kind() == "function_definition"
                    || body_child.kind() == "function_declaration"
                {
                    let info = extract_scala_function_info(&body_child, source, true);
                    methods.push(info);
                }
            }
        }
    }
}

fn extract_scala_bases(node: &Node, source: &str) -> Vec<String> {
    let mut bases = Vec::new();

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "extends_clause" {
            let mut inner = child.walk();
            for inner_child in child.children(&mut inner) {
                match inner_child.kind() {
                    "type_identifier" | "generic_type" | "stable_type_identifier" => {
                        bases.push(get_node_text(&inner_child, source));
                    }
                    _ => {}
                }
            }
        }
    }

    bases
}

// =============================================================================
// Elixir detailed extraction
// =============================================================================

fn extract_elixir_functions_detailed(node: &Node, source: &str, functions: &mut Vec<FunctionInfo>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "call" {
            if let Some(first) = child.child(0) {
                let text = get_node_text(&first, source);
                if text == "def" || text == "defp" {
                    let info = extract_elixir_function_info(&child, source);
                    functions.push(info);
                } else if text != "defmodule" {
                    // Recurse into non-module calls
                    extract_elixir_functions_detailed(&child, source, functions);
                }
                // For defmodule, recurse into its do_block to find nested functions
                if text == "defmodule" {
                    let mut mod_cursor = child.walk();
                    for mod_child in child.children(&mut mod_cursor) {
                        if mod_child.kind() == "do_block" {
                            extract_elixir_functions_detailed(&mod_child, source, functions);
                        }
                    }
                }
            } else {
                extract_elixir_functions_detailed(&child, source, functions);
            }
        } else {
            extract_elixir_functions_detailed(&child, source, functions);
        }
    }
}

fn extract_elixir_function_info(node: &Node, source: &str) -> FunctionInfo {
    // Structure: (call (identifier "def") (arguments (call (identifier "func_name") (arguments ...))))
    // Or: (call (identifier "def") (arguments (identifier "func_name")) (do_block ...))
    let mut name = String::new();
    let mut params = Vec::new();
    let is_private;

    // First child is "def" or "defp"
    if let Some(first) = node.child(0) {
        let text = get_node_text(&first, source);
        is_private = text == "defp";
    } else {
        is_private = false;
    }

    // Second child is arguments containing the function clause
    if let Some(args) = node.child(1) {
        if args.kind() == "arguments" {
            // First child of arguments could be an identifier (no-param function)
            // or a call (function with params)
            if let Some(first_arg) = args.child(0) {
                if first_arg.kind() == "identifier" {
                    name = get_node_text(&first_arg, source);
                } else if first_arg.kind() == "call" {
                    // call node: first child is function name, rest are arguments
                    if let Some(fname) = first_arg.child(0) {
                        if fname.kind() == "identifier" {
                            name = get_node_text(&fname, source);
                        }
                    }
                    // Extract params from the call's arguments
                    if let Some(call_args) = first_arg.child(1) {
                        if call_args.kind() == "arguments" {
                            params = extract_elixir_params(&call_args, source);
                        }
                    }
                } else if first_arg.kind() == "binary_operator" {
                    // Pattern: def func(args) when guard do ... end
                    // The binary_operator wraps the function clause with a guard
                    let mut bin_cursor = first_arg.walk();
                    for bin_child in first_arg.children(&mut bin_cursor) {
                        if bin_child.kind() == "call" {
                            if let Some(fname) = bin_child.child(0) {
                                if fname.kind() == "identifier" {
                                    name = get_node_text(&fname, source);
                                }
                            }
                            if let Some(call_args) = bin_child.child(1) {
                                if call_args.kind() == "arguments" {
                                    params = extract_elixir_params(&call_args, source);
                                }
                            }
                            break;
                        }
                        if bin_child.kind() == "identifier" && name.is_empty() {
                            name = get_node_text(&bin_child, source);
                        }
                    }
                }
            }
        } else if args.kind() == "call" {
            // Direct call without arguments wrapper
            if let Some(fname) = args.child(0) {
                if fname.kind() == "identifier" {
                    name = get_node_text(&fname, source);
                }
            }
            if let Some(call_args) = args.child(1) {
                if call_args.kind() == "arguments" {
                    params = extract_elixir_params(&call_args, source);
                }
            }
        } else if args.kind() == "identifier" {
            name = get_node_text(&args, source);
        }
    }

    let docstring = extract_elixir_docstring(node, source);
    let line_number = node.start_position().row as u32 + 1;

    let _ = is_private; // Could be used for decorators but not needed per spec

    FunctionInfo {
        name,
        params,
        return_type: None, // Elixir is dynamically typed
        docstring,
        is_method: false,
        is_async: false,
        decorators: Vec::new(),
        line_number,
    }
}

fn extract_elixir_params(node: &Node, source: &str) -> Vec<String> {
    let mut params = Vec::new();

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                params.push(get_node_text(&child, source));
            }
            "binary_operator" => {
                // Default value: param \\ default
                // Take the left side (identifier)
                if let Some(left) = child.child(0) {
                    if left.kind() == "identifier" {
                        params.push(get_node_text(&left, source));
                    }
                }
            }
            "unary_operator" => {
                // Pattern match like ^pin or \\ operator
                params.push(get_node_text(&child, source));
            }
            "tuple" | "map" | "list" | "sigil" | "string" | "atom" => {
                // Pattern-matched params - use the full text
                params.push(get_node_text(&child, source));
            }
            // Skip commas and parens
            "," | "(" | ")" => {}
            _ => {
                // For other patterns, include as-is
                let text = get_node_text(&child, source);
                if !text.is_empty() && text != "," && text != "(" && text != ")" {
                    params.push(text);
                }
            }
        }
    }

    params
}

fn extract_elixir_docstring(node: &Node, source: &str) -> Option<String> {
    // @doc attribute before the function
    // It's a call node with identifier "@doc" followed by the doc content
    let mut prev = node.prev_sibling();

    while let Some(sibling) = prev {
        if sibling.kind() == "call" || sibling.kind() == "unary_operator" {
            let text = get_node_text(&sibling, source);
            if text.starts_with("@doc") {
                // Extract the string content after @doc
                let doc = text.trim_start_matches("@doc").trim();
                if !doc.is_empty() {
                    return Some(doc.to_string());
                }
            }
        }
        // Skip past @spec and other attributes
        if sibling.kind() == "call" || sibling.kind() == "unary_operator" {
            let text = get_node_text(&sibling, source);
            if text.starts_with("@spec") || text.starts_with("@impl") {
                prev = sibling.prev_sibling();
                continue;
            }
        }
        break;
    }

    None
}

fn extract_elixir_classes_detailed(node: &Node, source: &str, classes: &mut Vec<ClassInfo>) {
    // Elixir modules (defmodule) are extracted as ClassInfo
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "call" {
            if let Some(first) = child.child(0) {
                let text = get_node_text(&first, source);
                if text == "defmodule" {
                    let info = extract_elixir_module_info(&child, source);
                    classes.push(info);
                } else {
                    extract_elixir_classes_detailed(&child, source, classes);
                }
            }
        } else {
            extract_elixir_classes_detailed(&child, source, classes);
        }
    }
}

fn extract_elixir_module_info(node: &Node, source: &str) -> ClassInfo {
    // defmodule Name do ... end
    // Structure: (call (identifier "defmodule") (arguments (alias "ModuleName")) (do_block ...))
    let mut name = String::new();

    if let Some(args) = node.child(1) {
        if args.kind() == "arguments" {
            let mut cursor = args.walk();
            for child in args.children(&mut cursor) {
                if child.kind() == "alias" {
                    name = get_node_text(&child, source);
                    break;
                }
                // Sometimes it's a dot-qualified alias
                if child.kind() == "call" || child.kind() == "dot" {
                    name = get_node_text(&child, source);
                    break;
                }
            }
        } else if args.kind() == "alias" {
            name = get_node_text(&args, source);
        }
    }

    let line_number = node.start_position().row as u32 + 1;

    // Extract functions from do_block
    let mut methods = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "do_block" {
            extract_elixir_module_functions(&child, source, &mut methods);
        }
    }

    let docstring = extract_elixir_module_docstring(node, source);

    ClassInfo {
        name,
        bases: Vec::new(), // Elixir doesn't have class inheritance
        docstring,
        methods,
        fields: Vec::new(),
        decorators: Vec::new(),
        line_number,
    }
}

fn extract_elixir_module_functions(node: &Node, source: &str, functions: &mut Vec<FunctionInfo>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call" {
            if let Some(first) = child.child(0) {
                let text = get_node_text(&first, source);
                if text == "def" || text == "defp" {
                    let info = extract_elixir_function_info(&child, source);
                    functions.push(info);
                }
            }
        }
        // Recurse into nested structures (but not nested modules)
        if child.kind() != "call" {
            extract_elixir_module_functions(&child, source, functions);
        }
    }
}

fn extract_elixir_module_docstring(node: &Node, source: &str) -> Option<String> {
    // @moduledoc inside the do_block
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "do_block" {
            let mut body_cursor = child.walk();
            for body_child in child.children(&mut body_cursor) {
                if body_child.kind() == "call" || body_child.kind() == "unary_operator" {
                    let text = get_node_text(&body_child, source);
                    if text.starts_with("@moduledoc") {
                        let doc = text.trim_start_matches("@moduledoc").trim();
                        if !doc.is_empty() {
                            return Some(doc.to_string());
                        }
                    }
                }
                // Only check the first few statements for moduledoc
                if body_child.kind() == "call" {
                    if let Some(first) = body_child.child(0) {
                        let first_text = get_node_text(&first, source);
                        if first_text == "def" || first_text == "defp" {
                            break;
                        }
                    }
                }
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_extract_python_file() {
        let mut file = NamedTempFile::with_suffix(".py").unwrap();
        write!(
            file,
            r#"
"""Module docstring."""

def foo():
    """Function docstring."""
    bar()

def bar():
    pass
"#
        )
        .unwrap();

        let info = extract_file(file.path(), None).unwrap();

        assert_eq!(info.language, Language::Python);
        assert!(info.docstring.is_some());
        assert_eq!(info.functions.len(), 2);
        assert!(info.functions.iter().any(|f| f.name == "foo"));
        assert!(info.call_graph.calls.contains_key("foo"));
    }

    #[test]
    fn test_extract_handles_file_not_found() {
        let result = extract_file(Path::new("/nonexistent/file.py"), None);
        assert!(matches!(result, Err(TldrError::PathNotFound(_))));
    }

    #[test]
    fn test_extract_handles_unsupported_language() {
        let mut file = NamedTempFile::with_suffix(".xyz").unwrap();
        write!(file, "unknown language").unwrap();

        let result = extract_file(file.path(), None);
        assert!(matches!(result, Err(TldrError::UnsupportedLanguage(_))));
    }

    #[test]
    fn test_extract_calls_in_conditional_branches() {
        let mut file = NamedTempFile::with_suffix(".py").unwrap();
        write!(
            file,
            r#"
def get_imports(file, lang):
    if lang == "python":
        return parse_imports(file)
    elif lang == "go":
        return parse_go_imports(file)
    elif lang == "java":
        return parse_java_imports(file)
    else:
        return default_imports(file)

def parse_imports(f):
    pass

def parse_go_imports(f):
    pass

def parse_java_imports(f):
    pass

def default_imports(f):
    pass
"#
        )
        .unwrap();

        let info = extract_file(file.path(), None).unwrap();
        let calls = info
            .call_graph
            .calls
            .get("get_imports")
            .expect("get_imports should have calls");
        assert!(
            calls.contains(&"parse_imports".to_string()),
            "should find parse_imports in if branch"
        );
        assert!(
            calls.contains(&"parse_go_imports".to_string()),
            "should find parse_go_imports in elif branch"
        );
        assert!(
            calls.contains(&"parse_java_imports".to_string()),
            "should find parse_java_imports in elif branch"
        );
        assert!(
            calls.contains(&"default_imports".to_string()),
            "should find default_imports in else branch"
        );
    }

    #[test]
    fn test_extract_calls_in_for_while_with_try() {
        let mut file = NamedTempFile::with_suffix(".py").unwrap();
        write!(
            file,
            r#"
def process(items):
    for item in items:
        transform(item)
    while check_pending():
        flush()
    with open_resource() as r:
        read_data(r)
    try:
        risky_op()
    except Exception:
        handle_error()

def transform(x):
    pass

def check_pending():
    pass

def flush():
    pass

def open_resource():
    pass

def read_data(r):
    pass

def risky_op():
    pass

def handle_error():
    pass
"#
        )
        .unwrap();

        let info = extract_file(file.path(), None).unwrap();
        let calls = info
            .call_graph
            .calls
            .get("process")
            .expect("process should have calls");
        assert!(
            calls.contains(&"transform".to_string()),
            "should find transform in for loop"
        );
        assert!(
            calls.contains(&"check_pending".to_string()),
            "should find check_pending in while"
        );
        assert!(
            calls.contains(&"flush".to_string()),
            "should find flush in while body"
        );
        assert!(
            calls.contains(&"open_resource".to_string()),
            "should find open_resource in with"
        );
        assert!(
            calls.contains(&"read_data".to_string()),
            "should find read_data in with body"
        );
        assert!(
            calls.contains(&"risky_op".to_string()),
            "should find risky_op in try"
        );
        assert!(
            calls.contains(&"handle_error".to_string()),
            "should find handle_error in except"
        );
    }

    #[test]
    fn test_extract_calls_duplicate_method_names_across_classes() {
        // BUG: When multiple classes have methods with the same name,
        // find_and_extract_calls only finds the FIRST matching method,
        // causing all same-named methods to share the same call list.
        let mut file = NamedTempFile::with_suffix(".py").unwrap();
        write!(
            file,
            r#"
class Alpha:
    def process(self):
        alpha_helper()

    def visit(self):
        visit_alpha()

class Beta:
    def process(self):
        beta_helper()

    def visit(self):
        visit_beta()

def alpha_helper():
    pass

def beta_helper():
    pass

def visit_alpha():
    pass

def visit_beta():
    pass
"#
        )
        .unwrap();

        let info = extract_file(file.path(), None).unwrap();
        let calls = &info.call_graph.calls;

        // The "process" key should contain calls from BOTH Alpha.process AND Beta.process
        let process_calls = calls.get("process").expect("process should have calls");
        assert!(
            process_calls.contains(&"alpha_helper".to_string()),
            "should find alpha_helper from Alpha.process, got: {:?}",
            process_calls
        );
        assert!(
            process_calls.contains(&"beta_helper".to_string()),
            "should find beta_helper from Beta.process, got: {:?}",
            process_calls
        );

        // The "visit" key should contain calls from BOTH Alpha.visit AND Beta.visit
        let visit_calls = calls.get("visit").expect("visit should have calls");
        assert!(
            visit_calls.contains(&"visit_alpha".to_string()),
            "should find visit_alpha from Alpha.visit, got: {:?}",
            visit_calls
        );
        assert!(
            visit_calls.contains(&"visit_beta".to_string()),
            "should find visit_beta from Beta.visit, got: {:?}",
            visit_calls
        );
    }

    #[test]
    fn test_extract_python_params() {
        use crate::ast::parser::parse;

        let source = r#"
def foo(x, y):
    pass

def bar(items: list) -> int:
    return 0

def baz(a: int, b: str = "default") -> None:
    pass
"#;
        let tree = parse(source, Language::Python).unwrap();
        let functions = extract_functions_detailed(&tree, source, Language::Python);

        let foo = functions.iter().find(|f| f.name == "foo").unwrap();
        assert_eq!(foo.params, vec!["x".to_string(), "y".to_string()]);

        let bar = functions.iter().find(|f| f.name == "bar").unwrap();
        assert_eq!(bar.params, vec!["items".to_string()]);

        let baz = functions.iter().find(|f| f.name == "baz").unwrap();
        assert_eq!(baz.params, vec!["a".to_string(), "b".to_string()]);
    }

    // =========================================================================
    // Lua extraction tests
    // =========================================================================

    #[test]
    fn test_extract_lua_named_functions() {
        use crate::ast::parser::parse;

        let source = r#"--- A docstring for greet
function greet(name, age)
    print("Hello " .. name)
end

local function helper(x)
    return x + 1
end
"#;
        let tree = parse(source, Language::Lua).unwrap();
        let functions = extract_functions_detailed(&tree, source, Language::Lua);

        assert_eq!(
            functions.len(),
            2,
            "Should find 2 named functions, got: {:?}",
            functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );

        let greet = functions.iter().find(|f| f.name == "greet").unwrap();
        assert_eq!(greet.params, vec!["name", "age"]);
        assert!(greet.docstring.is_some(), "greet should have a docstring");
        assert!(greet
            .docstring
            .as_ref()
            .unwrap()
            .contains("docstring for greet"));
        assert_eq!(greet.return_type, None);
        assert!(!greet.is_async);

        let helper = functions.iter().find(|f| f.name == "helper").unwrap();
        assert_eq!(helper.params, vec!["x"]);
    }

    #[test]
    fn test_extract_lua_assignment_functions() {
        use crate::ast::parser::parse;

        let source = r#"M.request = function(url, opts)
    return http.get(url)
end

local myFunc = function(a, b)
    return a + b
end
"#;
        let tree = parse(source, Language::Lua).unwrap();
        let functions = extract_functions_detailed(&tree, source, Language::Lua);

        assert!(
            functions.len() >= 2,
            "Should find at least 2 assignment functions, got: {:?}",
            functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );

        let request = functions.iter().find(|f| f.name == "request").unwrap();
        assert_eq!(request.params, vec!["url", "opts"]);

        let my_func = functions.iter().find(|f| f.name == "myFunc").unwrap();
        assert_eq!(my_func.params, vec!["a", "b"]);
    }

    #[test]
    fn test_extract_lua_file_integration() {
        let mut file = NamedTempFile::with_suffix(".lua").unwrap();
        write!(
            file,
            r#"--- Module function
function greet(name)
    print("Hello " .. name)
end

local function helper()
    return 42
end
"#
        )
        .unwrap();

        let info = extract_file(file.path(), None).unwrap();
        assert_eq!(info.language, Language::Lua);
        assert!(info.functions.len() >= 2);
        assert!(info.functions.iter().any(|f| f.name == "greet"));
        assert!(info.functions.iter().any(|f| f.name == "helper"));
    }

    // =========================================================================
    // Luau extraction tests
    // =========================================================================

    #[test]
    fn test_extract_luau_typed_functions() {
        use crate::ast::parser::parse;

        let source = r#"--- Typed function
function greet(name: string, age: number): string
    return "Hello " .. name
end

local function helper(x: number): number
    return x + 1
end
"#;
        let tree = parse(source, Language::Luau).unwrap();
        let functions = extract_functions_detailed(&tree, source, Language::Luau);

        assert_eq!(
            functions.len(),
            2,
            "Should find 2 functions, got: {:?}",
            functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );

        let greet = functions.iter().find(|f| f.name == "greet").unwrap();
        assert_eq!(greet.params, vec!["name", "age"]);
        assert!(
            greet.return_type.is_some(),
            "greet should have a return type"
        );
        let rt = greet.return_type.as_ref().unwrap();
        assert!(
            rt.contains("string"),
            "return type should contain 'string', got: {}",
            rt
        );
        assert!(greet.docstring.is_some(), "greet should have a docstring");

        let helper = functions.iter().find(|f| f.name == "helper").unwrap();
        assert_eq!(helper.params, vec!["x"]);
        assert!(helper.return_type.is_some());
    }

    #[test]
    fn test_extract_luau_file_integration() {
        let mut file = NamedTempFile::with_suffix(".luau").unwrap();
        write!(
            file,
            r#"--!strict
function add(a: number, b: number): number
    return a + b
end
"#
        )
        .unwrap();

        let info = extract_file(file.path(), None).unwrap();
        assert_eq!(info.language, Language::Luau);
        assert!(!info.functions.is_empty());
        let add = info.functions.iter().find(|f| f.name == "add").unwrap();
        assert_eq!(add.params, vec!["a", "b"]);
    }

    // =========================================================================
    // OCaml extraction tests
    // =========================================================================

    #[test]
    fn test_extract_ocaml_simple_functions() {
        use crate::ast::parser::parse;

        let source = r#"(** A greeting function *)
let greet name age =
  Printf.printf "Hello %s, age %d\n" name age

let rec factorial n =
  if n <= 1 then 1
  else n * factorial (n - 1)
"#;
        let tree = parse(source, Language::Ocaml).unwrap();
        let functions = extract_functions_detailed(&tree, source, Language::Ocaml);

        assert!(
            functions.len() >= 2,
            "Should find at least 2 functions, got: {:?}",
            functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );

        let greet = functions.iter().find(|f| f.name == "greet").unwrap();
        assert_eq!(greet.params, vec!["name", "age"]);
        assert!(greet.docstring.is_some(), "greet should have a docstring");
        assert!(greet
            .docstring
            .as_ref()
            .unwrap()
            .contains("greeting function"));
        assert_eq!(greet.return_type, None); // No return type annotation
        assert!(!greet.is_async);

        let factorial = functions.iter().find(|f| f.name == "factorial").unwrap();
        assert_eq!(factorial.params, vec!["n"]);
    }

    #[test]
    fn test_extract_ocaml_typed_functions() {
        use crate::ast::parser::parse;

        let source = r#"let add (x : int) (y : int) : int =
  x + y
"#;
        let tree = parse(source, Language::Ocaml).unwrap();
        let functions = extract_functions_detailed(&tree, source, Language::Ocaml);

        assert_eq!(functions.len(), 1, "Should find 1 function");
        let add = &functions[0];
        assert_eq!(add.name, "add");
        assert_eq!(add.params, vec!["x", "y"]);
        assert!(add.return_type.is_some(), "add should have a return type");
        assert_eq!(add.return_type.as_ref().unwrap(), "int");
    }

    #[test]
    fn test_extract_ocaml_file_integration() {
        let mut file = NamedTempFile::with_suffix(".ml").unwrap();
        write!(
            file,
            r#"(** Add two numbers *)
let add x y = x + y

let mul x y = x * y
"#
        )
        .unwrap();

        let info = extract_file(file.path(), None).unwrap();
        assert_eq!(info.language, Language::Ocaml);
        assert!(info.functions.len() >= 2);
        assert!(info.functions.iter().any(|f| f.name == "add"));
        assert!(info.functions.iter().any(|f| f.name == "mul"));
    }

    #[test]
    fn test_extract_ocaml_no_classes() {
        let mut file = NamedTempFile::with_suffix(".ml").unwrap();
        writeln!(
            file,
            r#"let add x y = x + y"#
        )
        .unwrap();

        let info = extract_file(file.path(), None).unwrap();
        assert!(info.classes.is_empty(), "OCaml should have no classes");
    }

    #[test]
    fn test_extract_lua_no_classes() {
        let mut file = NamedTempFile::with_suffix(".lua").unwrap();
        writeln!(
            file,
            r#"function foo() end"#
        )
        .unwrap();

        let info = extract_file(file.path(), None).unwrap();
        assert!(info.classes.is_empty(), "Lua should have no classes");
    }

    #[test]
    fn test_extract_from_tree_matches_extract_file() {
        // Verify that extract_from_tree produces the same result as extract_file
        let mut file = NamedTempFile::with_suffix(".py").unwrap();
        write!(
            file,
            r#"
"""Module docstring."""

def foo():
    """Function docstring."""
    bar()

def bar():
    pass
"#
        )
        .unwrap();

        // Extract using extract_file
        let info_from_file = extract_file(file.path(), None).unwrap();

        // Extract using extract_from_tree (parse manually first)
        let (tree, source, language) = parse_file(file.path()).unwrap();
        let info_from_tree =
            extract_from_tree(&tree, &source, language, file.path(), None).unwrap();

        // Both should produce identical results
        assert_eq!(info_from_file.language, info_from_tree.language);
        assert_eq!(info_from_file.docstring, info_from_tree.docstring);
        assert_eq!(
            info_from_file.functions.len(),
            info_from_tree.functions.len()
        );
        assert_eq!(info_from_file.classes.len(), info_from_tree.classes.len());
        assert_eq!(info_from_file.imports.len(), info_from_tree.imports.len());

        // Verify function names match
        for func in &info_from_file.functions {
            assert!(info_from_tree.functions.iter().any(|f| f.name == func.name));
        }
    }

    // =========================================================================
    // Module-level constants extraction tests for missing languages
    // =========================================================================

    #[test]
    fn test_extract_c_module_constants() {
        use crate::ast::parser::parse;

        let source = r#"
#define MAX_SIZE 1024
#define VERSION "1.0.0"

const int BUFFER_LEN = 256;
int mutable_var = 42;
"#;
        let tree = parse(source, Language::C).unwrap();
        let constants = extract_module_constants(&tree, source, Language::C);

        assert!(
            constants
                .iter()
                .any(|c| c.name == "MAX_SIZE" && c.is_constant),
            "Should extract #define MAX_SIZE. Got: {:?}",
            constants
        );
        assert!(
            constants
                .iter()
                .any(|c| c.name == "VERSION" && c.is_constant),
            "Should extract #define VERSION. Got: {:?}",
            constants
        );
        assert!(
            constants
                .iter()
                .any(|c| c.name == "BUFFER_LEN" && c.is_constant),
            "Should extract const int BUFFER_LEN. Got: {:?}",
            constants
        );
        // mutable_var should NOT be extracted (not const, not UPPER_CASE define)
        assert!(
            !constants.iter().any(|c| c.name == "mutable_var"),
            "Should not extract non-const mutable_var"
        );
    }

    #[test]
    fn test_extract_cpp_module_constants() {
        use crate::ast::parser::parse;

        let source = r#"
#define MAX_THREADS 8

const int BUFFER_SIZE = 4096;
constexpr int CACHE_LINE = 64;
int global_var = 0;
"#;
        let tree = parse(source, Language::Cpp).unwrap();
        let constants = extract_module_constants(&tree, source, Language::Cpp);

        assert!(
            constants
                .iter()
                .any(|c| c.name == "MAX_THREADS" && c.is_constant),
            "Should extract #define MAX_THREADS. Got: {:?}",
            constants
        );
        assert!(
            constants
                .iter()
                .any(|c| c.name == "BUFFER_SIZE" && c.is_constant),
            "Should extract const int BUFFER_SIZE. Got: {:?}",
            constants
        );
        assert!(
            constants
                .iter()
                .any(|c| c.name == "CACHE_LINE" && c.is_constant),
            "Should extract constexpr int CACHE_LINE. Got: {:?}",
            constants
        );
        assert!(
            !constants.iter().any(|c| c.name == "global_var"),
            "Should not extract non-const global_var"
        );
    }

    #[test]
    fn test_extract_ruby_module_constants() {
        use crate::ast::parser::parse;

        let source = r#"
MAX_RETRIES = 3
DEFAULT_TIMEOUT = 30
local_var = "hello"
"#;
        let tree = parse(source, Language::Ruby).unwrap();
        let constants = extract_module_constants(&tree, source, Language::Ruby);

        assert!(
            constants
                .iter()
                .any(|c| c.name == "MAX_RETRIES" && c.is_constant),
            "Should extract UPPER_CASE constant MAX_RETRIES. Got: {:?}",
            constants
        );
        assert!(
            constants
                .iter()
                .any(|c| c.name == "DEFAULT_TIMEOUT" && c.is_constant),
            "Should extract UPPER_CASE constant DEFAULT_TIMEOUT. Got: {:?}",
            constants
        );
        assert!(
            !constants.iter().any(|c| c.name == "local_var"),
            "Should not extract lowercase local_var"
        );
    }

    #[test]
    fn test_extract_kotlin_module_constants() {
        use crate::ast::parser::parse;

        let source = r#"
const val MAX_SIZE = 1024
val DEFAULT_NAME = "hello"
var mutableVar = 42
"#;
        let tree = parse(source, Language::Kotlin).unwrap();
        let constants = extract_module_constants(&tree, source, Language::Kotlin);

        assert!(
            constants
                .iter()
                .any(|c| c.name == "MAX_SIZE" && c.is_constant),
            "Should extract const val MAX_SIZE. Got: {:?}",
            constants
        );
        assert!(
            constants
                .iter()
                .any(|c| c.name == "DEFAULT_NAME" && c.is_constant),
            "Should extract val DEFAULT_NAME (UPPER_CASE). Got: {:?}",
            constants
        );
        assert!(
            !constants.iter().any(|c| c.name == "mutableVar"),
            "Should not extract var mutableVar"
        );
    }

    #[test]
    fn test_extract_php_module_constants() {
        use crate::ast::parser::parse;

        let source = r#"<?php
const MAX_CONNECTIONS = 100;
define('API_VERSION', '2.0');
$regular_var = "hello";
"#;
        let tree = parse(source, Language::Php).unwrap();
        let constants = extract_module_constants(&tree, source, Language::Php);

        assert!(
            constants
                .iter()
                .any(|c| c.name == "MAX_CONNECTIONS" && c.is_constant),
            "Should extract const MAX_CONNECTIONS. Got: {:?}",
            constants
        );
        assert!(
            constants
                .iter()
                .any(|c| c.name == "API_VERSION" && c.is_constant),
            "Should extract define('API_VERSION', ...). Got: {:?}",
            constants
        );
    }

    #[test]
    fn test_extract_lua_module_constants() {
        use crate::ast::parser::parse;

        let source = r#"
MAX_RETRIES = 5
DEFAULT_TIMEOUT = 30
local lower_case = "not a constant"
"#;
        let tree = parse(source, Language::Lua).unwrap();
        let constants = extract_module_constants(&tree, source, Language::Lua);

        assert!(
            constants
                .iter()
                .any(|c| c.name == "MAX_RETRIES" && c.is_constant),
            "Should extract UPPER_CASE assignment MAX_RETRIES. Got: {:?}",
            constants
        );
        assert!(
            constants
                .iter()
                .any(|c| c.name == "DEFAULT_TIMEOUT" && c.is_constant),
            "Should extract UPPER_CASE assignment DEFAULT_TIMEOUT. Got: {:?}",
            constants
        );
        assert!(
            !constants.iter().any(|c| c.name == "lower_case"),
            "Should not extract lowercase variable"
        );
    }

    #[test]
    fn test_extract_luau_module_constants() {
        use crate::ast::parser::parse;

        let source = r#"
local MAX_SIZE = 100
local DEFAULT_NAME = "world"
local mutable_value = 42
"#;
        let tree = parse(source, Language::Luau).unwrap();
        let constants = extract_module_constants(&tree, source, Language::Luau);

        assert!(
            constants
                .iter()
                .any(|c| c.name == "MAX_SIZE" && c.is_constant),
            "Should extract UPPER_CASE local MAX_SIZE. Got: {:?}",
            constants
        );
        assert!(
            constants
                .iter()
                .any(|c| c.name == "DEFAULT_NAME" && c.is_constant),
            "Should extract UPPER_CASE local DEFAULT_NAME. Got: {:?}",
            constants
        );
        assert!(
            !constants.iter().any(|c| c.name == "mutable_value"),
            "Should not extract lowercase mutable_value"
        );
    }

    #[test]
    fn test_extract_elixir_module_constants() {
        use crate::ast::parser::parse;

        // Elixir module attributes use @ prefix. UPPER_CASE names at top level
        // are parsed as unary_operator(@) with alias operand.
        let source = r#"
@MAX_RETRIES 3
@DEFAULT_TIMEOUT 30
"#;
        let tree = parse(source, Language::Elixir).unwrap();
        let constants = extract_module_constants(&tree, source, Language::Elixir);

        assert!(
            constants
                .iter()
                .any(|c| c.name == "MAX_RETRIES" && c.is_constant),
            "Should extract @MAX_RETRIES module attribute. Got: {:?}",
            constants
        );
        assert!(
            constants
                .iter()
                .any(|c| c.name == "DEFAULT_TIMEOUT" && c.is_constant),
            "Should extract @DEFAULT_TIMEOUT module attribute. Got: {:?}",
            constants
        );
    }

    #[test]
    fn test_extract_scala_module_constants() {
        use crate::ast::parser::parse;

        let source = r#"
val MAX_SIZE = 1024
val DEFAULT_NAME = "hello"
var mutableVar = 42
"#;
        let tree = parse(source, Language::Scala).unwrap();
        let constants = extract_module_constants(&tree, source, Language::Scala);

        assert!(
            constants
                .iter()
                .any(|c| c.name == "MAX_SIZE" && c.is_constant),
            "Should extract val MAX_SIZE (UPPER_CASE). Got: {:?}",
            constants
        );
        assert!(
            constants
                .iter()
                .any(|c| c.name == "DEFAULT_NAME" && c.is_constant),
            "Should extract val DEFAULT_NAME (UPPER_CASE). Got: {:?}",
            constants
        );
        assert!(
            !constants.iter().any(|c| c.name == "mutableVar"),
            "Should not extract var mutableVar"
        );
    }

    #[test]
    fn test_extract_csharp_module_constants() {
        use crate::ast::parser::parse;

        let source = r#"
const int MAX_SIZE = 1024;
"#;
        let tree = parse(source, Language::CSharp).unwrap();
        let constants = extract_module_constants(&tree, source, Language::CSharp);

        // C# rarely has top-level constants outside classes, but when present they should be extracted
        // If the tree-sitter grammar puts this inside an implicit compilation_unit, it may still work
        assert!(
            constants
                .iter()
                .any(|c| c.name == "MAX_SIZE" && c.is_constant),
            "Should extract const int MAX_SIZE. Got: {:?}",
            constants
        );
    }

    #[test]
    fn test_extract_ocaml_module_constants() {
        use crate::ast::parser::parse;

        let source = r#"
let MAX_SIZE = 1024
let DEFAULT_NAME = "hello"
let lowercase_val = 42
"#;
        let tree = parse(source, Language::Ocaml).unwrap();
        let constants = extract_module_constants(&tree, source, Language::Ocaml);

        assert!(
            constants
                .iter()
                .any(|c| c.name == "MAX_SIZE" && c.is_constant),
            "Should extract let MAX_SIZE (UPPER_CASE). Got: {:?}",
            constants
        );
        assert!(
            constants
                .iter()
                .any(|c| c.name == "DEFAULT_NAME" && c.is_constant),
            "Should extract let DEFAULT_NAME (UPPER_CASE). Got: {:?}",
            constants
        );
        assert!(
            !constants.iter().any(|c| c.name == "lowercase_val"),
            "Should not extract lowercase lowercase_val"
        );
    }

    #[test]
    fn test_extract_swift_module_constants() {
        use crate::ast::parser::parse;

        let source = r#"
let MAX_SIZE = 1024
let DEFAULT_NAME = "hello"
var mutableVar = 42
"#;
        let tree = parse(source, Language::Swift).unwrap();
        let constants = extract_module_constants(&tree, source, Language::Swift);

        assert!(
            constants
                .iter()
                .any(|c| c.name == "MAX_SIZE" && c.is_constant),
            "Should extract let MAX_SIZE (UPPER_CASE). Got: {:?}",
            constants
        );
        assert!(
            constants
                .iter()
                .any(|c| c.name == "DEFAULT_NAME" && c.is_constant),
            "Should extract let DEFAULT_NAME (UPPER_CASE). Got: {:?}",
            constants
        );
        assert!(
            !constants.iter().any(|c| c.name == "mutableVar"),
            "Should not extract var mutableVar"
        );
    }

    #[test]
    fn test_go_var_declaration_extraction() {
        use crate::ast::parser::parse;

        let source = "package mypkg\n\nimport \"errors\"\n\nvar (\n\tErrTimeout  = errors.New(\"timeout\")\n\tErrCanceled = errors.New(\"canceled\")\n)\n";
        let tree = parse(source, Language::Go).unwrap();

        let constants = extract_module_constants(&tree, source, Language::Go);
        let names: Vec<&str> = constants.iter().map(|c| c.name.as_str()).collect();
        assert!(
            names.contains(&"ErrTimeout"),
            "Should extract ErrTimeout from grouped var block, got: {:?}",
            names
        );
        assert!(
            names.contains(&"ErrCanceled"),
            "Should extract ErrCanceled from grouped var block, got: {:?}",
            names
        );
        // Verify they're not marked as constants
        for c in &constants {
            assert!(!c.is_constant, "var entries should have is_constant=false");
        }
    }
}
