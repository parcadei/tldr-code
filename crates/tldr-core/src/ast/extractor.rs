//! Code structure extraction (spec Section 2.1.2)
//!
//! Extracts functions, classes, methods, and imports from source files.

use std::collections::HashSet;
use std::path::Path;

use tree_sitter::{Node, Tree};

use crate::fs::tree::{collect_files, get_file_tree};
use crate::types::{CodeStructure, DefinitionInfo, FileStructure, IgnoreSpec, Language};
use crate::TldrResult;

use super::extract::is_upper_case_name;
use super::imports::extract_imports_from_tree;
use super::parser::parse_file;

/// Extract code structure from all files in a directory.
///
/// # Arguments
/// * `root` - Root directory to scan
/// * `language` - Programming language to extract
/// * `max_results` - Maximum number of files (0 = unlimited)
/// * `ignore_spec` - Optional ignore patterns
///
/// # Edge Cases (per spec)
/// - Syntax error in file: Skip file, continue with others
/// - Binary file: Skip silently
/// - Encoding error: UTF-8 lossy fallback
/// - Empty file: Include with empty lists
pub fn get_code_structure(
    root: &Path,
    language: Language,
    max_results: usize,
    ignore_spec: Option<&IgnoreSpec>,
) -> TldrResult<CodeStructure> {
    // Handle single file case: extract structure directly
    if root.is_file() {
        let parent = root.parent().unwrap_or(root);
        match extract_file_structure(root, parent, language) {
            Ok(structure) => {
                return Ok(CodeStructure {
                    root: root.to_path_buf(),
                    language,
                    files: vec![structure],
                });
            }
            Err(e) => {
                return Err(e);
            }
        }
    }

    // Get file tree filtered by language extensions
    let extensions: HashSet<String> = language
        .extensions()
        .iter()
        .map(|s| s.to_string())
        .collect();

    let tree = get_file_tree(root, Some(&extensions), true, ignore_spec)?;
    let files = collect_files(&tree, root);

    let mut file_structures = Vec::new();

    for file_path in files {
        // Apply max_results limit
        if max_results > 0 && file_structures.len() >= max_results {
            break;
        }

        // Try to extract structure, skip on error (per spec edge case handling)
        match extract_file_structure(&file_path, root, language) {
            Ok(structure) => file_structures.push(structure),
            Err(e) => {
                // Log error but continue - recoverable errors per spec
                if e.is_recoverable() {
                    eprintln!("Warning: Skipping {} - {}", file_path.display(), e);
                } else {
                    return Err(e);
                }
            }
        }
    }

    Ok(CodeStructure {
        root: root.to_path_buf(),
        language,
        files: file_structures,
    })
}

/// Extract structure from a single file
fn extract_file_structure(
    path: &Path,
    root: &Path,
    language: Language,
) -> TldrResult<FileStructure> {
    let (tree, source, _) = parse_file(path)?;

    let relative_path = path.strip_prefix(root).unwrap_or(path).to_path_buf();

    let functions = extract_functions(&tree, &source, language);
    let classes = extract_classes(&tree, &source, language);
    let methods = extract_methods(&tree, &source, language);
    let imports = extract_imports_from_tree(&tree, &source, language)?;
    let definitions = extract_definitions(&tree, &source, language);

    Ok(FileStructure {
        path: relative_path,
        functions,
        classes,
        methods,
        imports,
        definitions,
    })
}

/// Extract function names from a syntax tree
pub fn extract_functions(tree: &Tree, source: &str, language: Language) -> Vec<String> {
    let mut functions = Vec::new();
    let root = tree.root_node();

    match language {
        Language::Python => extract_python_functions(&root, source, &mut functions, false),
        Language::TypeScript | Language::JavaScript => {
            extract_ts_functions(&root, source, &mut functions, false)
        }
        Language::Go => extract_go_functions(&root, source, &mut functions),
        Language::Rust => extract_rust_functions(&root, source, &mut functions),
        Language::Java => extract_java_functions(&root, source, &mut functions, false),
        Language::C => extract_c_functions(&root, source, &mut functions),
        Language::Cpp => extract_cpp_functions(&root, source, &mut functions),
        Language::Ruby => extract_ruby_functions(&root, source, &mut functions, false),
        Language::Scala => extract_scala_functions(&root, source, &mut functions),
        Language::Kotlin => extract_kotlin_functions(&root, source, &mut functions, false),
        Language::Ocaml => extract_ocaml_functions(&root, source, &mut functions),
        Language::Php => extract_php_functions(&root, source, &mut functions, false),
        Language::Swift => extract_swift_functions(&root, source, &mut functions),
        Language::CSharp => {} // C# has no free functions
        Language::Elixir => extract_elixir_functions(&root, source, &mut functions),
        Language::Lua => extract_lua_functions(&root, source, &mut functions),
        Language::Luau => extract_luau_functions(&root, source, &mut functions),
    }

    functions
}

/// Extract class names from a syntax tree
pub fn extract_classes(tree: &Tree, source: &str, language: Language) -> Vec<String> {
    let mut classes = Vec::new();
    let root = tree.root_node();

    match language {
        Language::Python => extract_python_classes(&root, source, &mut classes),
        Language::TypeScript | Language::JavaScript => {
            extract_ts_classes(&root, source, &mut classes)
        }
        Language::Go => extract_go_structs(&root, source, &mut classes),
        Language::Rust => extract_rust_structs(&root, source, &mut classes),
        Language::Java => extract_java_classes(&root, source, &mut classes),
        Language::C => extract_c_structs(&root, source, &mut classes),
        Language::Cpp => extract_cpp_classes(&root, source, &mut classes),
        Language::Ruby => extract_ruby_classes(&root, source, &mut classes),
        Language::Scala => extract_scala_classes(&root, source, &mut classes),
        Language::Kotlin => extract_kotlin_classes(&root, source, &mut classes),
        Language::Php => extract_php_classes(&root, source, &mut classes),
        Language::Swift => extract_swift_classes(&root, source, &mut classes),
        Language::CSharp => extract_csharp_classes(&root, source, &mut classes),
        Language::Elixir => extract_elixir_classes(&root, source, &mut classes),
        Language::Lua => {}  // Lua has no native classes
        Language::Luau => {} // Luau has no native classes
        _ => {}
    }

    classes
}

/// Extract method names (methods inside classes) from a syntax tree
pub fn extract_methods(tree: &Tree, source: &str, language: Language) -> Vec<String> {
    let mut methods = Vec::new();
    let root = tree.root_node();

    match language {
        Language::Python => extract_python_functions(&root, source, &mut methods, true),
        Language::TypeScript | Language::JavaScript => {
            extract_ts_functions(&root, source, &mut methods, true)
        }
        Language::Go => {} // Go methods are extracted as functions with receivers
        Language::Rust => extract_rust_impl_methods(&root, source, &mut methods),
        Language::Java => extract_java_functions(&root, source, &mut methods, true),
        Language::C => {} // C has no methods
        Language::Cpp => extract_cpp_methods(&root, source, &mut methods),
        Language::Ruby => extract_ruby_functions(&root, source, &mut methods, true),
        Language::Scala => extract_scala_methods(&root, source, &mut methods),
        Language::Kotlin => extract_kotlin_functions(&root, source, &mut methods, true),
        Language::Php => extract_php_functions(&root, source, &mut methods, true),
        Language::Swift => extract_swift_methods(&root, source, &mut methods),
        Language::CSharp => extract_csharp_methods(&root, source, &mut methods),
        Language::Elixir => {} // Elixir has no methods (modules are not OOP classes)
        Language::Lua => {}    // Lua has no methods
        Language::Luau => {}   // Luau has no methods
        _ => {}
    }

    methods
}

// =============================================================================
// Python extraction
// =============================================================================

fn extract_python_functions(
    node: &Node,
    source: &str,
    functions: &mut Vec<String>,
    methods_only: bool,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                // Check if inside a class
                let is_method = is_inside_class(&child);

                if methods_only == is_method {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let name = get_node_text(&name_node, source);
                        functions.push(name);
                    }
                }
            }
            "class_definition" => {
                // Recurse into class body for methods
                if let Some(body) = child.child_by_field_name("body") {
                    extract_python_functions(&body, source, functions, methods_only);
                }
            }
            _ => {
                // Recurse into other nodes
                extract_python_functions(&child, source, functions, methods_only);
            }
        }
    }
}

fn extract_python_classes(node: &Node, source: &str, classes: &mut Vec<String>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "class_definition" {
            if let Some(name_node) = child.child_by_field_name("name") {
                let name = get_node_text(&name_node, source);
                classes.push(name);
            }
        }
        extract_python_classes(&child, source, classes);
    }
}

// =============================================================================
// TypeScript/JavaScript extraction
// =============================================================================

fn extract_ts_functions(
    node: &Node,
    source: &str,
    functions: &mut Vec<String>,
    methods_only: bool,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration"
            | "function"
            | "generator_function_declaration"
            | "generator_function" => {
                if !methods_only {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let name = get_node_text(&name_node, source);
                        functions.push(name);
                    }
                }
            }
            "function_expression" => {
                // Named function expression: const x = function name() {}
                // Use variable name if inside a variable_declarator, else the function's own name
                if !methods_only {
                    let mut extracted = false;
                    if let Some(parent) = child.parent() {
                        if parent.kind() == "variable_declarator" {
                            if let Some(name_node) = parent.child_by_field_name("name") {
                                let name = get_node_text(&name_node, source);
                                functions.push(name);
                                extracted = true;
                            }
                        }
                    }
                    if !extracted {
                        if let Some(name_node) = child.child_by_field_name("name") {
                            let name = get_node_text(&name_node, source);
                            functions.push(name);
                        }
                    }
                }
            }
            "method_definition" => {
                if methods_only {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let name = get_node_text(&name_node, source);
                        functions.push(name);
                    }
                }
            }
            "arrow_function" => {
                // Arrow functions assigned to variables
                if !methods_only {
                    if let Some(parent) = child.parent() {
                        if parent.kind() == "variable_declarator" {
                            if let Some(name_node) = parent.child_by_field_name("name") {
                                let name = get_node_text(&name_node, source);
                                functions.push(name);
                            }
                        }
                    }
                }
            }
            "class_declaration" | "class" => {
                // Recurse into class body for methods
                if let Some(body) = child.child_by_field_name("body") {
                    extract_ts_functions(&body, source, functions, methods_only);
                }
            }
            _ => {
                extract_ts_functions(&child, source, functions, methods_only);
            }
        }
    }
}

fn extract_ts_classes(node: &Node, source: &str, classes: &mut Vec<String>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "class_declaration" || child.kind() == "class" {
            if let Some(name_node) = child.child_by_field_name("name") {
                let name = get_node_text(&name_node, source);
                classes.push(name);
            }
        }
        extract_ts_classes(&child, source, classes);
    }
}

// =============================================================================
// Go extraction
// =============================================================================

fn extract_go_functions(node: &Node, source: &str, functions: &mut Vec<String>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "function_declaration" || child.kind() == "method_declaration" {
            if let Some(name_node) = child.child_by_field_name("name") {
                let name = get_node_text(&name_node, source);
                functions.push(name);
            }
        }
        extract_go_functions(&child, source, functions);
    }
}

/// Extract Go struct types as classes.
/// Go uses `type_declaration` containing `type_spec` with a `struct_type` body.
fn extract_go_structs(node: &Node, source: &str, structs: &mut Vec<String>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "type_declaration" {
            // type_declaration contains one or more type_spec children
            let mut inner_cursor = child.walk();
            for inner in child.children(&mut inner_cursor) {
                if inner.kind() == "type_spec" {
                    // Check if it's a struct type (has struct_type child)
                    let mut has_struct = false;
                    let mut type_name = None;
                    let mut spec_cursor = inner.walk();
                    for spec_child in inner.children(&mut spec_cursor) {
                        if spec_child.kind() == "type_identifier" {
                            type_name = Some(get_node_text(&spec_child, source));
                        }
                        if spec_child.kind() == "struct_type"
                            || spec_child.kind() == "interface_type"
                        {
                            has_struct = true;
                        }
                    }
                    if has_struct {
                        if let Some(name) = type_name {
                            structs.push(name);
                        }
                    }
                }
            }
        }
        extract_go_structs(&child, source, structs);
    }
}

// =============================================================================
// Rust extraction
// =============================================================================

fn extract_rust_functions(node: &Node, source: &str, functions: &mut Vec<String>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "function_item" {
            // Only top-level functions (not inside impl blocks)
            if !is_inside_impl(&child) {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = get_node_text(&name_node, source);
                    functions.push(name);
                }
            }
        }
        extract_rust_functions(&child, source, functions);
    }
}

fn extract_rust_structs(node: &Node, source: &str, structs: &mut Vec<String>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "struct_item"
            || child.kind() == "enum_item"
            || child.kind() == "trait_item"
        {
            if let Some(name_node) = child.child_by_field_name("name") {
                let name = get_node_text(&name_node, source);
                structs.push(name);
            }
        }
        extract_rust_structs(&child, source, structs);
    }
}

fn extract_rust_impl_methods(node: &Node, source: &str, methods: &mut Vec<String>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "impl_item" {
            // Get methods from impl block
            if let Some(body) = child.child_by_field_name("body") {
                let mut body_cursor = body.walk();
                for item in body.children(&mut body_cursor) {
                    if item.kind() == "function_item" {
                        if let Some(name_node) = item.child_by_field_name("name") {
                            let name = get_node_text(&name_node, source);
                            methods.push(name);
                        }
                    }
                }
            }
        } else if child.kind() == "trait_item" {
            // Get methods from trait definition (declaration_list body)
            let mut trait_cursor = child.walk();
            for trait_child in child.children(&mut trait_cursor) {
                if trait_child.kind() == "declaration_list" {
                    let mut body_cursor = trait_child.walk();
                    for item in trait_child.children(&mut body_cursor) {
                        if item.kind() == "function_item"
                            || item.kind() == "function_signature_item"
                        {
                            if let Some(name_node) = item.child_by_field_name("name") {
                                let name = get_node_text(&name_node, source);
                                methods.push(name);
                            }
                        }
                    }
                }
            }
        }
        extract_rust_impl_methods(&child, source, methods);
    }
}

// =============================================================================
// Java extraction
// =============================================================================

fn extract_java_functions(
    node: &Node,
    source: &str,
    functions: &mut Vec<String>,
    methods_only: bool,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "method_declaration" => {
                let is_in_class = is_inside_class(&child);
                if methods_only == is_in_class {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let name = get_node_text(&name_node, source);
                        functions.push(name);
                    }
                }
            }
            "constructor_declaration" => {
                // Constructors are always inside a class, so they are methods
                if methods_only {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let name = get_node_text(&name_node, source);
                        functions.push(name);
                    }
                }
            }
            _ => {}
        }
        extract_java_functions(&child, source, functions, methods_only);
    }
}

fn extract_java_classes(node: &Node, source: &str, classes: &mut Vec<String>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "class_declaration" || child.kind() == "interface_declaration" {
            if let Some(name_node) = child.child_by_field_name("name") {
                let name = get_node_text(&name_node, source);
                classes.push(name);
            }
        }
        extract_java_classes(&child, source, classes);
    }
}

// =============================================================================
// C extraction
// =============================================================================

fn extract_c_functions(node: &Node, source: &str, functions: &mut Vec<String>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "function_definition" {
            // Get the declarator which contains the function name
            if let Some(declarator) = child.child_by_field_name("declarator") {
                if let Some(name) = extract_c_function_name(&declarator, source) {
                    functions.push(name);
                }
            }
        }
        extract_c_functions(&child, source, functions);
    }
}

/// Extract function name from a C declarator node
/// Handles: function_declarator, pointer_declarator wrapping function_declarator
fn extract_c_function_name(node: &Node, source: &str) -> Option<String> {
    match node.kind() {
        "function_declarator" => {
            // Direct function declarator - get the declarator field which is the identifier
            if let Some(declarator) = node.child_by_field_name("declarator") {
                if declarator.kind() == "identifier" {
                    return Some(get_node_text(&declarator, source));
                } else if declarator.kind() == "parenthesized_declarator" {
                    // Handle (*func_ptr)(args) style
                    return extract_c_function_name(&declarator, source);
                }
            }
        }
        "pointer_declarator" => {
            // int *foo() - recurse into the declarator
            if let Some(declarator) = node.child_by_field_name("declarator") {
                return extract_c_function_name(&declarator, source);
            }
        }
        "identifier" => {
            return Some(get_node_text(node, source));
        }
        _ => {
            // Try children
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if let Some(name) = extract_c_function_name(&child, source) {
                    return Some(name);
                }
            }
        }
    }
    None
}

fn extract_c_structs(node: &Node, source: &str, structs: &mut Vec<String>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "struct_specifier" | "enum_specifier" => {
                // Named struct/enum: struct Foo { ... }
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = get_node_text(&name_node, source);
                    structs.push(name);
                }
            }
            "type_definition" => {
                // typedef struct { ... } Name;
                // The type_definition contains a struct_specifier (anonymous) and a type_identifier
                let mut has_struct_or_enum = false;
                let mut typedef_name = None;
                let mut inner_cursor = child.walk();
                for inner in child.children(&mut inner_cursor) {
                    if inner.kind() == "struct_specifier" || inner.kind() == "enum_specifier" {
                        has_struct_or_enum = true;
                    }
                    if inner.kind() == "type_identifier" {
                        typedef_name = Some(get_node_text(&inner, source));
                    }
                }
                if has_struct_or_enum {
                    if let Some(name) = typedef_name {
                        structs.push(name);
                    }
                }
            }
            _ => {}
        }
        extract_c_structs(&child, source, structs);
    }
}

// =============================================================================
// C++ extraction
// =============================================================================

fn extract_cpp_functions(node: &Node, source: &str, functions: &mut Vec<String>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "function_definition" {
            // Only count as free function if NOT inside a class/struct body
            if !is_inside_cpp_class(&child) {
                if let Some(declarator) = child.child_by_field_name("declarator") {
                    if let Some(name) = extract_cpp_function_name(&declarator, source) {
                        functions.push(name);
                    }
                }
            }
        }
        extract_cpp_functions(&child, source, functions);
    }
}

/// Extract function name from a C++ declarator node
/// Handles: function_declarator, qualified_identifier, reference_declarator
fn extract_cpp_function_name(node: &Node, source: &str) -> Option<String> {
    match node.kind() {
        "function_declarator" => {
            if let Some(declarator) = node.child_by_field_name("declarator") {
                return extract_cpp_function_name(&declarator, source);
            }
        }
        "qualified_identifier" | "scoped_identifier" => {
            // namespace::function - get the name part
            if let Some(name) = node.child_by_field_name("name") {
                return Some(get_node_text(&name, source));
            }
            // Fallback: get full qualified name
            return Some(get_node_text(node, source));
        }
        "pointer_declarator" | "reference_declarator" => {
            if let Some(declarator) = node.child_by_field_name("declarator") {
                return extract_cpp_function_name(&declarator, source);
            }
        }
        "identifier" | "field_identifier" | "destructor_name" => {
            return Some(get_node_text(node, source));
        }
        "operator_name" => {
            // operator+ etc.
            return Some(get_node_text(node, source));
        }
        _ => {
            // Try children
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if let Some(name) = extract_cpp_function_name(&child, source) {
                    return Some(name);
                }
            }
        }
    }
    None
}

fn extract_cpp_classes(node: &Node, source: &str, classes: &mut Vec<String>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_specifier" | "struct_specifier" | "enum_specifier" => {
                // Get the name field
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = get_node_text(&name_node, source);
                    classes.push(name);
                }
            }
            _ => {}
        }
        extract_cpp_classes(&child, source, classes);
    }
}

/// Extract C++ methods: function_definition nodes inside class/struct bodies.
/// Only counts methods with actual bodies (function_definition), not forward
/// declarations or `= default`/`= delete` stubs.
fn extract_cpp_methods(node: &Node, source: &str, methods: &mut Vec<String>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_specifier" | "struct_specifier" => {
                // Look inside the class/struct body (field_declaration_list)
                if let Some(body) = child.child_by_field_name("body") {
                    let mut body_cursor = body.walk();
                    for body_child in body.children(&mut body_cursor) {
                        if body_child.kind() == "function_definition" {
                            // Skip `= default` and `= delete` methods (no real body)
                            if cpp_has_default_or_delete(&body_child) {
                                continue;
                            }
                            // Inline method definition with body: void greet() { ... }
                            if let Some(declarator) = body_child.child_by_field_name("declarator") {
                                if let Some(name) = extract_cpp_function_name(&declarator, source) {
                                    methods.push(name);
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        extract_cpp_methods(&child, source, methods);
    }
}

/// Check if a C++ function_definition has `= default` or `= delete` (no real body)
fn cpp_has_default_or_delete(node: &Node) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "default_method_clause" || child.kind() == "delete_method_clause" {
            return true;
        }
    }
    false
}

/// Check if a C++ node is inside a class or struct specifier body
fn is_inside_cpp_class(node: &Node) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "class_specifier" | "struct_specifier" => return true,
            "field_declaration_list" => {
                // field_declaration_list is the body of a class/struct
                if let Some(grandparent) = parent.parent() {
                    if matches!(grandparent.kind(), "class_specifier" | "struct_specifier") {
                        return true;
                    }
                }
            }
            "translation_unit" => return false, // Top-level
            _ => {}
        }
        current = parent.parent();
    }
    false
}

// =============================================================================
// Ruby extraction
// =============================================================================

/// Extract Ruby functions/methods from AST
/// Ruby uses `method` nodes for both top-level functions and class methods.
/// `methods_only` = false: top-level methods only
/// `methods_only` = true: methods inside classes/modules only
fn extract_ruby_functions(
    node: &Node,
    source: &str,
    functions: &mut Vec<String>,
    methods_only: bool,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "method" => {
                // Check if this method is inside a class or module
                let is_method = is_inside_ruby_class_or_module(&child);

                if methods_only == is_method {
                    // Get the method name from the identifier child
                    let mut method_cursor = child.walk();
                    for method_child in child.children(&mut method_cursor) {
                        if method_child.kind() == "identifier" {
                            let name = get_node_text(&method_child, source);
                            functions.push(name);
                            break;
                        }
                    }
                }
            }
            "singleton_method" => {
                // def self.method_name or def object.method_name
                // These are class methods (module functions)
                if methods_only {
                    let mut method_cursor = child.walk();
                    for method_child in child.children(&mut method_cursor) {
                        // The name is the second identifier (after "self" or object)
                        if method_child.kind() == "identifier" {
                            let name = get_node_text(&method_child, source);
                            // Skip "self" - we want the method name
                            if name != "self" {
                                functions.push(name);
                                break;
                            }
                        }
                    }
                }
            }
            "class" | "module" => {
                // Recurse into class/module body for methods.
                // In Ruby's tree-sitter grammar, `child_by_field_name("body")`
                // returns the `body_statement` node. We must use only ONE
                // recursion path to avoid double-counting methods.
                let body_found = child.child_by_field_name("body");
                if let Some(body) = body_found {
                    extract_ruby_functions(&body, source, functions, methods_only);
                } else {
                    // Fallback: iterate children for body_statement
                    let mut class_cursor = child.walk();
                    for class_child in child.children(&mut class_cursor) {
                        if class_child.kind() == "body_statement" {
                            extract_ruby_functions(&class_child, source, functions, methods_only);
                        }
                    }
                }
            }
            _ => {
                // Recurse into other nodes
                extract_ruby_functions(&child, source, functions, methods_only);
            }
        }
    }
}

/// Extract Ruby class and module names
fn extract_ruby_classes(node: &Node, source: &str, classes: &mut Vec<String>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "class" | "module" => {
                // Get the class/module name from the constant child
                let mut class_cursor = child.walk();
                for class_child in child.children(&mut class_cursor) {
                    if class_child.kind() == "constant" || class_child.kind() == "scope_resolution"
                    {
                        let name = get_node_text(&class_child, source);
                        classes.push(name);
                        break;
                    }
                }
            }
            _ => {}
        }
        // Recurse to find nested classes/modules
        extract_ruby_classes(&child, source, classes);
    }
}

/// Check if a Ruby node is inside a class or module definition
fn is_inside_ruby_class_or_module(node: &Node) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "class" | "module" | "body_statement" => {
                // body_statement alone isn't enough - check if its parent is class/module
                if parent.kind() == "body_statement" {
                    if let Some(grandparent) = parent.parent() {
                        if grandparent.kind() == "class" || grandparent.kind() == "module" {
                            return true;
                        }
                    }
                } else {
                    return true;
                }
            }
            "program" => return false, // Top-level
            _ => {}
        }
        current = parent.parent();
    }
    false
}

// =============================================================================
// Scala extraction
// =============================================================================

/// Extract Scala top-level functions (def inside object, or standalone)
/// Scala uses: function_definition for def foo(...) = ...
fn extract_scala_functions(node: &Node, source: &str, functions: &mut Vec<String>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                // Only top-level functions (in objects or at package level)
                // Skip methods inside class/trait definitions
                if !is_inside_scala_class_or_trait(&child) {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let name = get_node_text(&name_node, source);
                        functions.push(name);
                    }
                }
            }
            "object_definition" => {
                // Recurse into object body for functions
                if let Some(body) = child.child_by_field_name("body") {
                    extract_scala_functions(&body, source, functions);
                }
            }
            "template_body" => {
                // Recurse into template body
                extract_scala_functions(&child, source, functions);
            }
            _ => {
                // Recurse into other nodes
                extract_scala_functions(&child, source, functions);
            }
        }
    }
}

/// Extract Scala class and trait names (not objects -- objects are singletons, not classes)
fn extract_scala_classes(node: &Node, source: &str, classes: &mut Vec<String>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_definition" | "trait_definition" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = get_node_text(&name_node, source);
                    classes.push(name);
                }
            }
            _ => {}
        }
        // Recurse to find nested classes
        extract_scala_classes(&child, source, classes);
    }
}

/// Extract Scala methods (functions inside class/trait)
fn extract_scala_methods(node: &Node, source: &str, methods: &mut Vec<String>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                // Only methods inside class/trait definitions
                if is_inside_scala_class_or_trait(&child) {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let name = get_node_text(&name_node, source);
                        methods.push(name);
                    }
                }
            }
            "class_definition" | "trait_definition" => {
                // Recurse into class/trait body for methods
                if let Some(body) = child.child_by_field_name("body") {
                    extract_scala_methods(&body, source, methods);
                }
            }
            "template_body" => {
                // Recurse into template body
                extract_scala_methods(&child, source, methods);
            }
            _ => {
                // Recurse into other nodes
                extract_scala_methods(&child, source, methods);
            }
        }
    }
}

/// Check if a Scala node is inside a class or trait definition (but not object)
fn is_inside_scala_class_or_trait(node: &Node) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "class_definition" | "trait_definition" => return true,
            "object_definition" => return false, // Object methods are considered functions
            "compilation_unit" => return false,  // Top-level
            _ => {}
        }
        current = parent.parent();
    }
    false
}

// =============================================================================
// Kotlin extraction
// =============================================================================

/// Extract Kotlin functions/methods from AST
/// Kotlin uses `function_declaration` for both top-level functions and class methods.
/// `methods_only` = false: top-level functions only (not inside class/object/interface)
/// `methods_only` = true: methods inside class/object/interface only
fn extract_kotlin_functions(
    node: &Node,
    source: &str,
    functions: &mut Vec<String>,
    methods_only: bool,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                // Check if inside a class, object, or interface
                let is_method = is_inside_kotlin_class_or_object(&child);

                if methods_only == is_method {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let name = get_node_text(&name_node, source);
                        functions.push(name);
                    }
                }
            }
            "class_declaration" | "object_declaration" | "companion_object" => {
                // Recurse into class/object/companion body for methods
                let mut class_cursor = child.walk();
                for class_child in child.children(&mut class_cursor) {
                    if class_child.kind() == "class_body" {
                        extract_kotlin_functions(&class_child, source, functions, methods_only);
                    }
                }
            }
            _ => {
                extract_kotlin_functions(&child, source, functions, methods_only);
            }
        }
    }
}

/// Extract Kotlin class, object, and interface names
fn extract_kotlin_classes(node: &Node, source: &str, classes: &mut Vec<String>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_declaration" | "object_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = get_node_text(&name_node, source);
                    classes.push(name);
                }
            }
            _ => {}
        }
        extract_kotlin_classes(&child, source, classes);
    }
}

/// Check if a Kotlin node is inside a class, object, or interface definition
fn is_inside_kotlin_class_or_object(node: &Node) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "class_declaration" | "object_declaration" | "companion_object" => return true,
            "class_body" => {
                // class_body is a container -- check if its parent is a class-like node
                if let Some(grandparent) = parent.parent() {
                    if matches!(
                        grandparent.kind(),
                        "class_declaration" | "object_declaration" | "companion_object"
                    ) {
                        return true;
                    }
                }
            }
            "source_file" => return false, // Top-level
            _ => {}
        }
        current = parent.parent();
    }
    false
}

// =============================================================================
// OCaml extraction
// =============================================================================

/// Extract OCaml function names from AST.
/// OCaml functions are `value_definition` nodes containing `let_binding` children
/// that have `parameter` children (distinguishing functions from value bindings).
/// The function name is in the `pattern` field of the `let_binding`.
fn extract_ocaml_functions(node: &Node, source: &str, functions: &mut Vec<String>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "value_definition" {
            let mut inner_cursor = child.walk();
            for inner in child.children(&mut inner_cursor) {
                if inner.kind() == "let_binding" {
                    // Only extract if it has parameters (i.e., is a function, not a value binding)
                    if ocaml_binding_has_params_simple(&inner) {
                        if let Some(pattern_node) = inner.child_by_field_name("pattern") {
                            let name = get_node_text(&pattern_node, source);
                            // Skip anonymous bindings like `let () = ...`
                            if name != "()" && !name.is_empty() {
                                functions.push(name);
                            }
                        }
                    }
                }
            }
        }
        extract_ocaml_functions(&child, source, functions);
    }
}

/// Check if an OCaml let_binding has parameter children (i.e., is a function definition).
fn ocaml_binding_has_params_simple(node: &Node) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "parameter" {
            return true;
        }
    }
    false
}

// =============================================================================
// PHP extraction
// =============================================================================

/// Extract PHP functions/methods from AST
/// PHP uses `function_definition` for standalone functions and `method_declaration` for class methods.
/// `methods_only` = false: top-level functions only
/// `methods_only` = true: methods inside classes only
fn extract_php_functions(
    node: &Node,
    source: &str,
    functions: &mut Vec<String>,
    methods_only: bool,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                // Standalone function: function hello() {}
                // Check if inside a class
                let is_method = is_inside_php_class(&child);

                if methods_only == is_method {
                    // Get the function name
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let name = get_node_text(&name_node, source);
                        functions.push(name);
                    }
                }
            }
            "method_declaration" => {
                // Class method: public function greet() {}
                if methods_only {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let name = get_node_text(&name_node, source);
                        functions.push(name);
                    }
                }
            }
            "class_declaration" | "interface_declaration" | "trait_declaration" => {
                // Recurse into class/interface/trait body for methods.
                // In PHP's tree-sitter grammar, the body field IS the declaration_list.
                // Only recurse via one path to avoid double-counting.
                if let Some(body) = child.child_by_field_name("body") {
                    extract_php_functions(&body, source, functions, methods_only);
                } else {
                    // Fallback: look for declaration_list directly
                    let mut class_cursor = child.walk();
                    for class_child in child.children(&mut class_cursor) {
                        if class_child.kind() == "declaration_list" {
                            extract_php_functions(&class_child, source, functions, methods_only);
                        }
                    }
                }
            }
            _ => {
                // Recurse into other nodes
                extract_php_functions(&child, source, functions, methods_only);
            }
        }
    }
}

/// Extract PHP class, interface, and trait names
fn extract_php_classes(node: &Node, source: &str, classes: &mut Vec<String>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_declaration" | "interface_declaration" | "trait_declaration" => {
                // Get the class/interface/trait name
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = get_node_text(&name_node, source);
                    classes.push(name);
                }
            }
            _ => {}
        }
        // Recurse to find nested classes (though PHP doesn't support truly nested classes,
        // we still recurse for completeness)
        extract_php_classes(&child, source, classes);
    }
}

/// Check if a PHP node is inside a class, interface, or trait definition
fn is_inside_php_class(node: &Node) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "class_declaration"
            | "interface_declaration"
            | "trait_declaration"
            | "declaration_list" => {
                // declaration_list alone isn't enough - check if its parent is a class type
                if parent.kind() == "declaration_list" {
                    if let Some(grandparent) = parent.parent() {
                        if matches!(
                            grandparent.kind(),
                            "class_declaration" | "interface_declaration" | "trait_declaration"
                        ) {
                            return true;
                        }
                    }
                } else {
                    return true;
                }
            }
            "program" => return false, // Top-level
            _ => {}
        }
        current = parent.parent();
    }
    false
}

// =============================================================================
// Swift extraction (stubs - pending full implementation)
// =============================================================================

fn extract_swift_functions(node: &Node, source: &str, functions: &mut Vec<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "function_declaration" && !is_inside_class(&child) {
            if let Some(name_node) = child.child_by_field_name("name") {
                let name = get_node_text(&name_node, source);
                functions.push(name);
            }
        }
        extract_swift_functions(&child, source, functions);
    }
}

fn extract_swift_classes(node: &Node, source: &str, classes: &mut Vec<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "class_declaration" || child.kind() == "protocol_declaration" {
            if let Some(name_node) = child.child_by_field_name("name") {
                let name = get_node_text(&name_node, source);
                classes.push(name);
            }
        }
        extract_swift_classes(&child, source, classes);
    }
}

fn extract_swift_methods(node: &Node, source: &str, methods: &mut Vec<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                if is_inside_class(&child) {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let name = get_node_text(&name_node, source);
                        methods.push(name);
                    }
                }
            }
            "init_declaration" => {
                // Swift init() constructors inside classes
                if is_inside_class(&child) {
                    methods.push("init".to_string());
                }
            }
            _ => {}
        }
        extract_swift_methods(&child, source, methods);
    }
}

// =============================================================================
// C# extraction (stubs - pending full implementation)
// =============================================================================

fn extract_csharp_classes(node: &Node, source: &str, classes: &mut Vec<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_declaration" | "interface_declaration" | "struct_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = get_node_text(&name_node, source);
                    classes.push(name);
                }
            }
            _ => {}
        }
        extract_csharp_classes(&child, source, classes);
    }
}

fn extract_csharp_methods(node: &Node, source: &str, methods: &mut Vec<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "method_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = get_node_text(&name_node, source);
                    methods.push(name);
                }
            }
            "constructor_declaration" => {
                // C# constructors: public Animal(string name) { }
                // The constructor name is an identifier child (same as the class name)
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = get_node_text(&name_node, source);
                    methods.push(name);
                } else {
                    // Fallback: find the first identifier child
                    let mut inner_cursor = child.walk();
                    for inner in child.children(&mut inner_cursor) {
                        if inner.kind() == "identifier" {
                            let name = get_node_text(&inner, source);
                            methods.push(name);
                            break;
                        }
                    }
                }
            }
            _ => {}
        }
        extract_csharp_methods(&child, source, methods);
    }
}

// =============================================================================
// Elixir extraction (stubs - pending full implementation)
// =============================================================================

fn extract_elixir_functions(node: &Node, source: &str, functions: &mut Vec<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call" {
            // def/defp in Elixir are calls
            let mut inner_cursor = child.walk();
            for inner in child.children(&mut inner_cursor) {
                if inner.kind() == "identifier" {
                    let text = get_node_text(&inner, source);
                    if text == "def" || text == "defp" {
                        // Next sibling should be the function call with name
                        if let Some(args) = inner.next_sibling() {
                            if args.kind() == "arguments" || args.kind() == "call" {
                                if let Some(name_node) = args.child(0) {
                                    if name_node.kind() == "identifier"
                                        || name_node.kind() == "call"
                                    {
                                        let fname = if name_node.kind() == "call" {
                                            if let Some(n) = name_node.child(0) {
                                                get_node_text(&n, source)
                                            } else {
                                                get_node_text(&name_node, source)
                                            }
                                        } else {
                                            get_node_text(&name_node, source)
                                        };
                                        if !functions.contains(&fname) {
                                            functions.push(fname);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        extract_elixir_functions(&child, source, functions);
    }
}

fn extract_elixir_classes(node: &Node, source: &str, classes: &mut Vec<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call" {
            let mut inner_cursor = child.walk();
            for inner in child.children(&mut inner_cursor) {
                if inner.kind() == "identifier" {
                    let text = get_node_text(&inner, source);
                    if text == "defmodule" {
                        if let Some(args) = inner.next_sibling() {
                            if let Some(name_node) = args.child(0) {
                                let name = get_node_text(&name_node, source);
                                classes.push(name);
                            }
                        }
                    }
                }
            }
        }
        extract_elixir_classes(&child, source, classes);
    }
}

// =============================================================================
// Lua extraction (stubs - pending full implementation)
// =============================================================================

fn extract_lua_functions(node: &Node, source: &str, functions: &mut Vec<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                // Handles: function foo(), local function foo(),
                //          function Table.method(), function Table:method()
                if let Some(name) = extract_lua_function_name(&child, source) {
                    functions.push(name);
                }
                // Don't recurse into function_declaration children (no nested functions to find)
                continue;
            }
            "variable_declaration" => {
                // Handles: local foo = function() end
                // Structure: variable_declaration > [local] assignment_statement >
                //   variable_list > identifier + expression_list > function_definition
                extract_lua_variable_function(&child, source, functions);
                // Don't recurse -- we already looked inside via extract_lua_variable_function
                continue;
            }
            "assignment_statement" => {
                // Handles: foo = function() end (without local, at top level)
                extract_lua_assignment_function(&child, source, functions);
                continue;
            }
            _ => {}
        }
        extract_lua_functions(&child, source, functions);
    }
}

/// Extract function name from a Lua function_declaration node.
/// Handles: identifier (simple name), dot_index_expression (Table.method),
/// method_index_expression (Table:method)
fn extract_lua_function_name(node: &Node, source: &str) -> Option<String> {
    // Try field name first
    if let Some(name_node) = node.child_by_field_name("name") {
        let name = get_node_text(&name_node, source);
        return Some(name);
    }
    // Fallback: iterate children for identifier/dot_index_expression/method_index_expression
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                let name = get_node_text(&child, source);
                // Skip keywords
                if name != "function" && name != "local" && name != "end" {
                    return Some(name);
                }
            }
            "dot_index_expression" | "method_index_expression" => {
                // Table.method or Table:method -- extract the last identifier (method name)
                if let Some(field) = child.child_by_field_name("field") {
                    return Some(get_node_text(&field, source));
                }
                // Fallback: get the last identifier child
                let mut inner_cursor = child.walk();
                let mut last_ident = None;
                for inner in child.children(&mut inner_cursor) {
                    if inner.kind() == "identifier" {
                        last_ident = Some(get_node_text(&inner, source));
                    }
                }
                return last_ident;
            }
            _ => {}
        }
    }
    None
}

/// Extract function from a variable_declaration like: local foo = function() end
fn extract_lua_variable_function(node: &Node, source: &str, functions: &mut Vec<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "assignment_statement" {
            extract_lua_assignment_function(&child, source, functions);
        }
    }
}

/// Extract function from an assignment_statement if RHS is function_definition
fn extract_lua_assignment_function(node: &Node, source: &str, functions: &mut Vec<String>) {
    // Check if the expression_list contains a function_definition
    let mut has_function_def = false;
    let mut inner_cursor = node.walk();
    for inner in node.children(&mut inner_cursor) {
        if inner.kind() == "expression_list" {
            let mut expr_cursor = inner.walk();
            for expr in inner.children(&mut expr_cursor) {
                if expr.kind() == "function_definition" {
                    has_function_def = true;
                    break;
                }
            }
        }
    }
    if !has_function_def {
        return;
    }
    // Get the variable name from variable_list
    let mut inner_cursor2 = node.walk();
    for inner in node.children(&mut inner_cursor2) {
        if inner.kind() == "variable_list" {
            if let Some(name_node) = inner.child(0) {
                if name_node.kind() == "identifier" {
                    functions.push(get_node_text(&name_node, source));
                    return;
                }
            }
        }
    }
}

// =============================================================================
// Luau extraction (stubs - pending full implementation)
// =============================================================================

fn extract_luau_functions(node: &Node, source: &str, functions: &mut Vec<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration" | "local_function" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = get_node_text(&name_node, source);
                    functions.push(name);
                }
            }
            _ => {}
        }
        extract_luau_functions(&child, source, functions);
    }
}

// =============================================================================
// Definition extraction (DefinitionInfo population)
// =============================================================================

/// Extract all definitions (functions, classes, structs, methods) from a syntax tree
/// as `DefinitionInfo` entries with line ranges and signatures.
///
/// This walks the tree-sitter AST recursively and classifies nodes as function-like
/// or class-like, mirroring the logic in `search/enriched.rs::classify_node`.
fn extract_definitions(tree: &Tree, source: &str, language: Language) -> Vec<DefinitionInfo> {
    let mut definitions = Vec::new();
    let root = tree.root_node();
    collect_definitions(root, source, language, &mut definitions);
    definitions
}

/// Recursively collect definition nodes from a tree-sitter AST.
fn collect_definitions(
    node: Node,
    source: &str,
    language: Language,
    definitions: &mut Vec<DefinitionInfo>,
) {
    let kind = node.kind();

    // Elixir: def/defp/defmodule are macro calls parsed as "call" nodes.
    // Handle them specially before the generic path.
    if language == Language::Elixir && kind == "call" {
        if let Some(def_info) = try_elixir_call_definition(node, source) {
            definitions.push(def_info);
        }
    }

    // Constants: detect const/static/UPPER_CASE assignments across languages.
    if let Some(const_def) = try_constant_definition(node, source, language) {
        definitions.push(const_def);
    }

    let (is_func, is_class) = classify_definition_node(kind, language);

    if is_func || is_class {
        if let Some(name) = get_definition_node_name(node, source) {
            let line_start = node.start_position().row as u32 + 1; // 1-indexed
            let line_end = node.end_position().row as u32 + 1;

            // Extract signature: skip doc comments/attributes, use actual def line
            let signature = extract_def_signature(node, source);

            let entry_kind = if is_class {
                match kind {
                    "struct_item" | "struct_definition" | "struct_specifier" => "struct",
                    "enum_item" => "enum",
                    "trait_item" => "trait",
                    "interface_declaration" => "interface",
                    "module" => "module",
                    _ => "class",
                }
            } else {
                // Check if inside a class/impl => method
                if is_inside_class_or_impl(&node, language) {
                    "method"
                } else {
                    "function"
                }
            };

            definitions.push(DefinitionInfo {
                name,
                kind: entry_kind.to_string(),
                line_start,
                line_end,
                signature,
            });
        }
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_definitions(child, source, language, definitions);
    }
}

/// Try to classify a tree-sitter node as a constant definition.
///
/// Returns `Some(DefinitionInfo)` with `kind: "constant"` if the node represents a
/// module-level constant. Uses explicit `const`/`static`/`final` keywords for languages
/// that have them, and UPPER_CASE naming convention for Python/JS/TS/Ruby/C/C++.
fn try_constant_definition(
    node: Node,
    source: &str,
    language: Language,
) -> Option<DefinitionInfo> {
    let kind = node.kind();

    match language {
        Language::Python => {
            // UPPER_CASE = value (at module level: expression_statement → assignment)
            if kind != "expression_statement" {
                return None;
            }
            let inner = node.child(0)?;
            if inner.kind() != "assignment" {
                return None;
            }
            let left = inner.child_by_field_name("left")?;
            if left.kind() != "identifier" {
                return None;
            }
            let name = get_node_text(&left, source);
            if !is_upper_case_name(&name) {
                return None;
            }
            Some(make_constant_def(node, name, source))
        }

        Language::Rust => {
            // const_item: `const NAME: Type = value;`
            // static_item: `static NAME: Type = value;`
            if kind != "const_item" && kind != "static_item" {
                return None;
            }
            let name = node
                .child_by_field_name("name")
                .map(|n| get_node_text(&n, source))?;
            Some(make_constant_def(node, name, source))
        }

        Language::Go => {
            // const_spec inside const_declaration (individual constant in a group)
            if kind != "const_spec" {
                return None;
            }
            let name = node
                .child_by_field_name("name")
                .map(|n| get_node_text(&n, source))?;
            Some(make_constant_def(node, name, source))
        }

        Language::TypeScript | Language::JavaScript => {
            // `const UPPER_CASE = ...` (also found inside `export const ...` via recursion)
            if kind != "lexical_declaration" {
                return None;
            }
            let decl_text = get_node_text(&node, source);
            if !decl_text.starts_with("const ") {
                return None;
            }
            let mut cursor = node.walk();
            let declarator = node
                .children(&mut cursor)
                .find(|c| c.kind() == "variable_declarator")?;
            let name = declarator
                .child_by_field_name("name")
                .map(|n| get_node_text(&n, source))?;
            if !is_upper_case_name(&name) {
                return None;
            }
            Some(make_constant_def(node, name, source))
        }

        Language::C | Language::Cpp => {
            match kind {
                "preproc_def" => {
                    // #define UPPER_CASE value
                    let mut cursor = node.walk();
                    let ident = node
                        .children(&mut cursor)
                        .find(|c| c.kind() == "identifier")?;
                    let name = get_node_text(&ident, source);
                    if !is_upper_case_name(&name) {
                        return None;
                    }
                    Some(make_constant_def(node, name, source))
                }
                "declaration" => {
                    // const/constexpr TYPE UPPER_CASE = value;
                    let mut cursor = node.walk();
                    let has_const = node.children(&mut cursor).any(|c| {
                        if c.kind() != "type_qualifier" {
                            return false;
                        }
                        let text = get_node_text(&c, source);
                        text == "const"
                            || (language == Language::Cpp && text == "constexpr")
                    });
                    if !has_const {
                        return None;
                    }
                    let mut cursor2 = node.walk();
                    let init_decl = node
                        .children(&mut cursor2)
                        .find(|c| c.kind() == "init_declarator")?;
                    let decl = init_decl.child_by_field_name("declarator")?;
                    let name = get_node_text(&decl, source);
                    if !is_upper_case_name(&name) {
                        return None;
                    }
                    Some(make_constant_def(node, name, source))
                }
                _ => None,
            }
        }

        Language::Ruby => {
            // CONSTANT = value (LHS is a `constant` node in tree-sitter-ruby)
            if kind != "assignment" {
                return None;
            }
            let mut cursor = node.walk();
            let const_node = node
                .children(&mut cursor)
                .find(|c| c.kind() == "constant")?;
            let name = get_node_text(&const_node, source);
            Some(make_constant_def(node, name, source))
        }

        Language::Java => {
            // static final fields: `public static final TYPE NAME = value;`
            if kind != "field_declaration" {
                return None;
            }
            let mut cursor = node.walk();
            let modifiers = node
                .children(&mut cursor)
                .find(|c| c.kind() == "modifiers")?;
            let mod_text = get_node_text(&modifiers, source);
            if !mod_text.contains("static") || !mod_text.contains("final") {
                return None;
            }
            let mut cursor2 = node.walk();
            let declarator = node
                .children(&mut cursor2)
                .find(|c| c.kind() == "variable_declarator")?;
            let name = declarator
                .child_by_field_name("name")
                .map(|n| get_node_text(&n, source))?;
            Some(make_constant_def(node, name, source))
        }

        Language::Kotlin => {
            // top-level val/const val declarations
            // tree-sitter-kotlin-ng: property_declaration
            if kind != "property_declaration" {
                return None;
            }
            let text = get_node_text(&node, source);
            if !text.starts_with("val ") && !text.starts_with("const val ") {
                return None;
            }
            // Only UPPER_CASE or const val
            let mut cursor = node.walk();
            let var_decl = node
                .children(&mut cursor)
                .find(|c| c.kind() == "variable_declaration")?;
            let name_node = if let Some(n) = var_decl.child_by_field_name("name") {
                Some(n)
            } else {
                let mut vc = var_decl.walk();
                let found = var_decl
                    .children(&mut vc)
                    .find(|c| c.kind() == "simple_identifier" || c.kind() == "identifier");
                found
            };
            let name = name_node.map(|n| get_node_text(&n, source))?;
            if !text.starts_with("const val ") && !is_upper_case_name(&name) {
                return None;
            }
            Some(make_constant_def(node, name, source))
        }

        Language::Swift => {
            // `let` declarations at module level
            if kind != "property_declaration" {
                return None;
            }
            let text = get_node_text(&node, source);
            if !text.starts_with("let ") {
                return None;
            }
            let name_node = if let Some(n) = node.child_by_field_name("name") {
                Some(n)
            } else {
                let mut cursor = node.walk();
                let found = node.children(&mut cursor).find(|c| {
                    c.kind() == "pattern"
                        || c.kind() == "simple_identifier"
                        || c.kind() == "identifier"
                });
                found
            };
            let name_node = name_node?;
            let name = get_node_text(&name_node, source);
            // Skip if name looks like a destructuring pattern
            if name.contains('(') || name.contains('{') {
                return None;
            }
            Some(make_constant_def(node, name, source))
        }

        Language::CSharp => {
            // const fields: `public const TYPE NAME = value;`
            if kind != "field_declaration" {
                return None;
            }
            let mut cursor = node.walk();
            let has_const = node.children(&mut cursor).any(|c| {
                c.kind() == "modifier" && get_node_text(&c, source) == "const"
            });
            if !has_const {
                return None;
            }
            let mut cursor2 = node.walk();
            let var_decl = node
                .children(&mut cursor2)
                .find(|c| c.kind() == "variable_declaration")?;
            let mut cursor3 = var_decl.walk();
            let declarator = var_decl
                .children(&mut cursor3)
                .find(|c| c.kind() == "variable_declarator")?;
            let name = declarator
                .child_by_field_name("name")
                .or_else(|| declarator.child(0))
                .map(|n| get_node_text(&n, source))?;
            Some(make_constant_def(node, name, source))
        }

        Language::Scala => {
            // val UPPER_CASE = value
            if kind != "val_definition" {
                return None;
            }
            let name_node = if let Some(n) = node.child_by_field_name("pattern") {
                Some(n)
            } else {
                let mut cursor = node.walk();
                let found = node
                    .children(&mut cursor)
                    .find(|c| c.kind() == "identifier");
                found
            };
            let name_node = name_node?;
            let name = get_node_text(&name_node, source);
            if !is_upper_case_name(&name) {
                return None;
            }
            Some(make_constant_def(node, name, source))
        }

        Language::Php => {
            // const NAME = value; or define('NAME', value);
            if kind != "const_declaration" {
                return None;
            }
            let mut cursor = node.walk();
            let element = node
                .children(&mut cursor)
                .find(|c| c.kind() == "const_element")?;
            let name = element
                .child_by_field_name("name")
                .or_else(|| element.child(0))
                .map(|n| get_node_text(&n, source))?;
            Some(make_constant_def(node, name, source))
        }

        Language::Elixir => {
            // @module_attribute: `@attr value`
            if kind != "unary_operator" {
                return None;
            }
            let text = get_node_text(&node, source);
            if !text.starts_with('@') {
                return None;
            }
            let mut cursor = node.walk();
            let ident = node
                .children(&mut cursor)
                .find(|c| c.kind() == "identifier")?;
            let name = format!("@{}", get_node_text(&ident, source));
            // Skip common non-constant attributes
            if name == "@doc" || name == "@moduledoc" || name == "@spec" || name == "@type" {
                return None;
            }
            Some(make_constant_def(node, name, source))
        }

        Language::Lua | Language::Luau | Language::Ocaml => None,
    }
}

/// Build a `DefinitionInfo` with `kind: "constant"` from a node and its name.
/// Uses the full node span for `line_start`/`line_end` and the first line for `signature`.
fn make_constant_def(node: Node, name: String, source: &str) -> DefinitionInfo {
    let line_start = node.start_position().row as u32 + 1;
    let line_end = node.end_position().row as u32 + 1;
    let sig_start = node.start_byte();
    let signature = source[sig_start..]
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    DefinitionInfo {
        name,
        kind: "constant".to_string(),
        line_start,
        line_end,
        signature,
    }
}

/// Try to extract a definition from an Elixir `call` node.
/// In Elixir, `def`/`defp`/`defmodule` are macro calls that tree-sitter parses as `call` nodes.
fn try_elixir_call_definition(node: Node, source: &str) -> Option<DefinitionInfo> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "identifier" {
            continue;
        }
        let keyword = child.utf8_text(source.as_bytes()).ok()?;
        let args = child.next_sibling()?;

        match keyword {
            "def" | "defp" => {
                let first_arg = args.child(0)?;
                let name = if first_arg.kind() == "call" {
                    // def process(data) → call node wrapping name + args
                    first_arg.child(0)?.utf8_text(source.as_bytes()).ok()?
                } else {
                    first_arg.utf8_text(source.as_bytes()).ok()?
                };
                let line_start = node.start_position().row as u32 + 1;
                let line_end = node.end_position().row as u32 + 1;
                let signature = extract_def_signature(node, source);
                return Some(DefinitionInfo {
                    name: name.to_string(),
                    kind: "function".to_string(),
                    line_start,
                    line_end,
                    signature,
                });
            }
            "defmodule" => {
                let first_arg = args.child(0)?;
                let name = first_arg.utf8_text(source.as_bytes()).ok()?;
                let line_start = node.start_position().row as u32 + 1;
                let line_end = node.end_position().row as u32 + 1;
                let signature = extract_def_signature(node, source);
                return Some(DefinitionInfo {
                    name: name.to_string(),
                    kind: "module".to_string(),
                    line_start,
                    line_end,
                    signature,
                });
            }
            _ => {}
        }
    }
    None
}

/// Classify a tree-sitter node kind as function-like or class-like.
/// Mirrors `search/enriched.rs::classify_node`.
fn classify_definition_node(kind: &str, _language: Language) -> (bool, bool) {
    let is_func = matches!(
        kind,
        "function_definition"
            | "function_declaration"
            | "function_item"     // Rust
            | "method_definition"
            | "method_declaration"
            | "method"            // Ruby
            | "singleton_method"  // Ruby class methods
            | "arrow_function"
            | "function_expression"
            | "function"           // JS/TS
            | "func_literal"       // Go
            | "function_type"
            | "value_definition"   // OCaml top-level let binding (functions and values)
    );

    let is_class = matches!(
        kind,
        "class_definition"
            | "class_declaration"
            | "class_specifier"   // C++
            | "class"             // Ruby
            | "module"            // Ruby
            | "struct_item"        // Rust
            | "struct_definition"  // C/C++
            | "struct_specifier"   // C
            | "enum_item"          // Rust
            | "trait_item"         // Rust
            | "type_spec"          // Go struct
            | "interface_declaration"
            | "type_definition"    // OCaml type definition
            | "module_definition"  // OCaml module definition
            | "companion_object"   // Kotlin companion object (name: "Companion" by convention)
    );

    (is_func, is_class)
}

/// Extract the name from a function/class definition node.
/// Mirrors `search/enriched.rs::get_definition_name`.
fn get_definition_node_name(node: Node, source: &str) -> Option<String> {
    // Most languages use a "name" field
    if let Some(name_node) = node.child_by_field_name("name") {
        let text = name_node.utf8_text(source.as_bytes()).ok()?;
        return Some(text.to_string());
    }

    // C/C++ function_definition uses "declarator" instead of "name"
    if node.kind() == "function_definition" {
        if let Some(declarator) = node.child_by_field_name("declarator") {
            return extract_name_from_declarator(declarator, source);
        }
    }

    // For arrow functions assigned to variables, check parent
    if node.kind() == "arrow_function" || node.kind() == "function_expression" {
        if let Some(parent) = node.parent() {
            if parent.kind() == "variable_declarator" {
                if let Some(name_node) = parent.child_by_field_name("name") {
                    let text = name_node.utf8_text(source.as_bytes()).ok()?;
                    return Some(text.to_string());
                }
            }
        }
    }

    // OCaml: value_definition contains a let_binding child with a "pattern" field.
    // The pattern field holds the function/value name (e.g. `let top_level x = ...`
    // has pattern="top_level"). Skip anonymous bindings like `let () = ...`.
    if node.kind() == "value_definition" {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "let_binding" {
                if let Some(pattern_node) = child.child_by_field_name("pattern") {
                    let text = pattern_node.utf8_text(source.as_bytes()).ok()?;
                    if text != "()" && !text.is_empty() {
                        return Some(text.to_string());
                    }
                }
            }
        }
        return None;
    }

    // Kotlin: companion_object has no identifier child; use "Companion" by convention.
    if node.kind() == "companion_object" {
        return Some("Companion".to_string());
    }

    None
}

/// Extract a function name from a C/C++ declarator chain.
/// Handles function_declarator, pointer_declarator, reference_declarator, etc.
fn extract_name_from_declarator(node: Node, source: &str) -> Option<String> {
    match node.kind() {
        "identifier" | "field_identifier" | "destructor_name" => {
            Some(get_node_text(&node, source))
        }
        "function_declarator" | "pointer_declarator" | "reference_declarator" => {
            let inner = node.child_by_field_name("declarator")?;
            extract_name_from_declarator(inner, source)
        }
        "qualified_identifier" | "scoped_identifier" => {
            if let Some(name) = node.child_by_field_name("name") {
                return Some(get_node_text(&name, source));
            }
            Some(get_node_text(&node, source))
        }
        _ => {
            // Fallback: try children
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if let Some(name) = extract_name_from_declarator(child, source) {
                    return Some(name);
                }
            }
            None
        }
    }
}

/// Check if a node is inside a class/struct body or impl block.
///
/// The `language` parameter is used to disambiguate node kinds that are shared
/// across tree-sitter grammars but have different semantics. For example,
/// `"module"` is the root node in tree-sitter-python (not a class scope) but
/// represents a Ruby module definition (a class scope) in tree-sitter-ruby.
fn is_inside_class_or_impl(node: &Node, language: Language) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        let kind = parent.kind();
        // "module" is a class-scope node in Ruby but the root node in Python.
        // Only treat it as a class scope when the language is Ruby.
        let module_is_class = !matches!(language, Language::Python);
        if matches!(
            kind,
            "class_definition"
                | "class_declaration"
                | "class_specifier"   // C++
                | "class"             // Ruby
                | "class_body"
                | "impl_item"
                | "struct_item"
                | "trait_item"
                | "companion_object"  // Kotlin
                | "object_declaration" // Kotlin
        ) || (kind == "module" && module_is_class) // Ruby module
        {
            return true;
        }
        current = parent.parent();
    }
    false
}

/// Extract the actual definition signature from a tree-sitter node,
/// skipping doc comments, attributes, and decorators.
/// Mirrors `search/enriched.rs::extract_definition_signature`.
fn extract_def_signature(node: Node, source: &str) -> String {
    // Strategy: find the first child node that isn't a comment or attribute,
    // then use its start position as the beginning of the actual definition.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let ckind = child.kind();
        // Skip doc comments and attributes/decorators
        if ckind == "line_comment"
            || ckind == "block_comment"
            || ckind == "comment"
            || ckind == "attribute_item"    // Rust #[...]
            || ckind == "attribute"         // Rust #[...]
            || ckind == "decorator"         // Python @decorator
            || ckind == "decorator_list"
        // Python
        {
            continue;
        }
        // Found the first non-comment child -- extract its line as signature
        let start_byte = child.start_byte();
        let line_from_start = &source[start_byte..];
        let sig = line_from_start
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        if !sig.is_empty() {
            return sig;
        }
    }

    // Fallback: find the first non-comment line in the node's text
    let node_text = &source[node.start_byte()..node.end_byte()];
    for line in node_text.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty()
            && !trimmed.starts_with("///")
            && !trimmed.starts_with("//!")
            && !trimmed.starts_with("//")
            && !trimmed.starts_with("/*")
            && !trimmed.starts_with("*")
            && !trimmed.starts_with("#[")
            && !trimmed.starts_with("@")
            && !trimmed.starts_with("#")
        {
            return trimmed.to_string();
        }
    }

    // Last resort: use the first line
    source[node.start_byte()..]
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

// =============================================================================
// Helper functions
// =============================================================================

/// Get text content of a node
fn get_node_text(node: &Node, source: &str) -> String {
    source[node.byte_range()].to_string()
}

/// Check if a node is inside a class definition
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

/// Check if a node is inside an impl block or trait definition (Rust)
fn is_inside_impl(node: &Node) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "impl_item" || parent.kind() == "trait_item" {
            return true;
        }
        current = parent.parent();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::parser::parse;

    #[test]
    fn test_extract_python_functions() {
        let source = r#"
def foo():
    pass

def bar(x):
    return x

class MyClass:
    def method(self):
        pass
"#;
        let tree = parse(source, Language::Python).unwrap();
        let functions = extract_functions(&tree, source, Language::Python);

        assert!(functions.contains(&"foo".to_string()));
        assert!(functions.contains(&"bar".to_string()));
        // method should not be in functions (it's a method)
        assert!(!functions.contains(&"method".to_string()));
    }

    #[test]
    fn test_extract_python_classes() {
        let source = r#"
class MyClass:
    pass

class AnotherClass:
    def method(self):
        pass
"#;
        let tree = parse(source, Language::Python).unwrap();
        let classes = extract_classes(&tree, source, Language::Python);

        assert!(classes.contains(&"MyClass".to_string()));
        assert!(classes.contains(&"AnotherClass".to_string()));
    }

    #[test]
    fn test_extract_python_methods() {
        let source = r#"
class MyClass:
    def method1(self):
        pass

    def method2(self, x):
        return x
"#;
        let tree = parse(source, Language::Python).unwrap();
        let methods = extract_methods(&tree, source, Language::Python);

        assert!(methods.contains(&"method1".to_string()));
        assert!(methods.contains(&"method2".to_string()));
    }

    #[test]
    fn test_extract_typescript_functions() {
        let source = r#"
function foo() {}

const bar = () => {};

class MyClass {
    method() {}
}
"#;
        let tree = parse(source, Language::TypeScript).unwrap();
        let functions = extract_functions(&tree, source, Language::TypeScript);

        assert!(functions.contains(&"foo".to_string()));
        // Arrow function detection depends on variable declarator
    }

    #[test]
    fn test_extract_go_functions() {
        let source = r#"
package main

func foo() {}

func (r *Receiver) bar() {}
"#;
        let tree = parse(source, Language::Go).unwrap();
        let functions = extract_functions(&tree, source, Language::Go);

        assert!(functions.contains(&"foo".to_string()));
        assert!(functions.contains(&"bar".to_string()));
    }

    #[test]
    fn test_extract_c_functions() {
        let source = r#"
void hello(void) {
}

int main(int argc, char** argv) {
    return 0;
}

static void helper(int x) {
}
"#;
        let tree = parse(source, Language::C).unwrap();
        let functions = extract_functions(&tree, source, Language::C);

        assert!(
            functions.contains(&"hello".to_string()),
            "Should find hello function"
        );
        assert!(
            functions.contains(&"main".to_string()),
            "Should find main function"
        );
        assert!(
            functions.contains(&"helper".to_string()),
            "Should find helper function"
        );
    }

    #[test]
    fn test_extract_c_structs() {
        let source = r#"
struct Point {
    int x;
    int y;
};

enum Color {
    RED,
    GREEN,
    BLUE
};
"#;
        let tree = parse(source, Language::C).unwrap();
        let classes = extract_classes(&tree, source, Language::C);

        assert!(
            classes.contains(&"Point".to_string()),
            "Should find Point struct"
        );
        assert!(
            classes.contains(&"Color".to_string()),
            "Should find Color enum"
        );
    }

    #[test]
    fn test_extract_cpp_functions() {
        let source = r#"
void hello() {
}

int main() {
    return 0;
}

namespace greeting {
    void greet(const std::string& name) {
    }
}
"#;
        let tree = parse(source, Language::Cpp).unwrap();
        let functions = extract_functions(&tree, source, Language::Cpp);

        assert!(
            functions.contains(&"hello".to_string()),
            "Should find hello function"
        );
        assert!(
            functions.contains(&"main".to_string()),
            "Should find main function"
        );
        // Namespace functions should also be found
        assert!(
            functions.contains(&"greet".to_string()),
            "Should find greet function"
        );
    }

    #[test]
    fn test_extract_cpp_classes() {
        let source = r#"
class Greeter {
public:
    void greet();
};

struct Point {
    int x, y;
};
"#;
        let tree = parse(source, Language::Cpp).unwrap();
        let classes = extract_classes(&tree, source, Language::Cpp);

        assert!(
            classes.contains(&"Greeter".to_string()),
            "Should find Greeter class"
        );
        assert!(
            classes.contains(&"Point".to_string()),
            "Should find Point struct"
        );
    }

    #[test]
    fn test_extract_ruby_functions() {
        let source = r##"
def top_level_function
  puts "I'm a function"
end

def another_function(x)
  x * 2
end

class MyClass
  def method_in_class
    puts "method"
  end
end
"##;
        let tree = parse(source, Language::Ruby).unwrap();
        let functions = extract_functions(&tree, source, Language::Ruby);

        assert!(
            functions.contains(&"top_level_function".to_string()),
            "Should find top_level_function"
        );
        assert!(
            functions.contains(&"another_function".to_string()),
            "Should find another_function"
        );
        // Method inside class should NOT be in functions
        assert!(
            !functions.contains(&"method_in_class".to_string()),
            "Should not find method_in_class in functions"
        );
    }

    #[test]
    fn test_extract_ruby_methods() {
        let source = r##"
def top_level_function
  puts "I'm a function"
end

class MyClass
  def initialize(name)
    @name = name
  end

  def greet
    puts "Hello"
  end
end

module MyModule
  def module_method
    puts "module method"
  end
end
"##;
        let tree = parse(source, Language::Ruby).unwrap();
        let methods = extract_methods(&tree, source, Language::Ruby);

        // Methods inside class should be found
        assert!(
            methods.contains(&"initialize".to_string()),
            "Should find initialize method"
        );
        assert!(
            methods.contains(&"greet".to_string()),
            "Should find greet method"
        );
        assert!(
            methods.contains(&"module_method".to_string()),
            "Should find module_method"
        );
        // Top-level function should NOT be in methods
        assert!(
            !methods.contains(&"top_level_function".to_string()),
            "Should not find top_level_function in methods"
        );
    }

    #[test]
    fn test_extract_ruby_classes() {
        let source = r##"
class MyClass
  def initialize
  end
end

class AnotherClass < BaseClass
  def method
  end
end

module MyModule
  class NestedClass
  end
end
"##;
        let tree = parse(source, Language::Ruby).unwrap();
        let classes = extract_classes(&tree, source, Language::Ruby);

        assert!(
            classes.contains(&"MyClass".to_string()),
            "Should find MyClass"
        );
        assert!(
            classes.contains(&"AnotherClass".to_string()),
            "Should find AnotherClass"
        );
        assert!(
            classes.contains(&"MyModule".to_string()),
            "Should find MyModule"
        );
        assert!(
            classes.contains(&"NestedClass".to_string()),
            "Should find NestedClass"
        );
    }

    #[test]
    fn test_extract_kotlin_functions() {
        let source = r#"
fun topLevel() {
    println("hello")
}

fun anotherTopLevel(x: Int): Int {
    return x * 2
}

class MyClass {
    fun classMethod() {}
}
"#;
        let tree = parse(source, Language::Kotlin).unwrap();
        let functions = extract_functions(&tree, source, Language::Kotlin);

        assert!(
            functions.contains(&"topLevel".to_string()),
            "Should find topLevel function"
        );
        assert!(
            functions.contains(&"anotherTopLevel".to_string()),
            "Should find anotherTopLevel function"
        );
        // Methods inside classes should NOT be in top-level functions
        assert!(
            !functions.contains(&"classMethod".to_string()),
            "Should not find classMethod in functions"
        );
    }

    #[test]
    fn test_extract_kotlin_methods() {
        let source = r#"
fun topLevel() {}

class MyClass {
    fun method1() {}
    fun method2(x: Int): String { return x.toString() }
}

object Singleton {
    fun singletonMethod() {}
}
"#;
        let tree = parse(source, Language::Kotlin).unwrap();
        let methods = extract_methods(&tree, source, Language::Kotlin);

        assert!(
            methods.contains(&"method1".to_string()),
            "Should find method1"
        );
        assert!(
            methods.contains(&"method2".to_string()),
            "Should find method2"
        );
        // Top-level functions should NOT be in methods
        assert!(
            !methods.contains(&"topLevel".to_string()),
            "Should not find topLevel in methods"
        );
    }

    #[test]
    fn test_extract_kotlin_classes() {
        let source = r#"
class HttpClient(val engine: Engine) {
    fun config() {}
}

object Singleton {
    fun method() {}
}

interface MyInterface {
    fun abstractMethod()
}
"#;
        let tree = parse(source, Language::Kotlin).unwrap();
        let classes = extract_classes(&tree, source, Language::Kotlin);

        assert!(
            classes.contains(&"HttpClient".to_string()),
            "Should find HttpClient class"
        );
        assert!(
            classes.contains(&"Singleton".to_string()),
            "Should find Singleton object"
        );
        assert!(
            classes.contains(&"MyInterface".to_string()),
            "Should find MyInterface interface"
        );
    }

    #[test]
    fn test_extract_ocaml_functions() {
        let source = r#"
let greet name =
  Printf.printf "Hello, %s!\n" name

let add x y = x + y

let value = 42

let rec factorial n =
  if n <= 1 then 1
  else n * factorial (n - 1)

let () = greet "world"
"#;
        let tree = parse(source, Language::Ocaml).unwrap();
        let functions = extract_functions(&tree, source, Language::Ocaml);

        assert!(
            functions.contains(&"greet".to_string()),
            "Should find greet function"
        );
        assert!(
            functions.contains(&"add".to_string()),
            "Should find add function"
        );
        assert!(
            functions.contains(&"factorial".to_string()),
            "Should find factorial function"
        );
        // 'value' has no parameters, it is a value binding not a function
        assert!(
            !functions.contains(&"value".to_string()),
            "Should not find value binding as function"
        );
        // let () = ... is not a named function
        assert!(
            !functions.contains(&"()".to_string()),
            "Should not find anonymous let () binding"
        );
    }

    // =========================================================================
    // Rust extraction tests -- traits, impl blocks, and struct/enum
    // =========================================================================

    #[test]
    fn test_extract_rust_classes_includes_traits() {
        let source = r#"
pub struct Config {
    pub name: String,
}

pub enum Color {
    Red,
    Green,
    Blue,
}

pub trait Serialize {
    fn serialize(&self) -> String;
}

trait Deserialize {
    fn deserialize(input: &str) -> Self;
}
"#;
        let tree = parse(source, Language::Rust).unwrap();
        let classes = extract_classes(&tree, source, Language::Rust);

        assert!(
            classes.contains(&"Config".to_string()),
            "Should find Config struct"
        );
        assert!(
            classes.contains(&"Color".to_string()),
            "Should find Color enum"
        );
        assert!(
            classes.contains(&"Serialize".to_string()),
            "Should find Serialize trait"
        );
        assert!(
            classes.contains(&"Deserialize".to_string()),
            "Should find Deserialize trait"
        );
    }

    #[test]
    fn test_extract_rust_functions_excludes_trait_methods() {
        let source = r#"
pub fn top_level() -> bool {
    true
}

pub trait Visitor {
    fn visit_bool(&self, v: bool) {}
    fn visit_i32(&self, v: i32) {}
}

impl Config {
    pub fn new() -> Self {
        Config {}
    }
}
"#;
        let tree = parse(source, Language::Rust).unwrap();
        let functions = extract_functions(&tree, source, Language::Rust);

        assert!(
            functions.contains(&"top_level".to_string()),
            "Should find top_level function"
        );
        // Trait methods should NOT appear as top-level functions
        assert!(
            !functions.contains(&"visit_bool".to_string()),
            "Trait method visit_bool should not be a top-level function"
        );
        assert!(
            !functions.contains(&"visit_i32".to_string()),
            "Trait method visit_i32 should not be a top-level function"
        );
        // impl methods should NOT appear as top-level functions
        assert!(
            !functions.contains(&"new".to_string()),
            "Impl method new should not be a top-level function"
        );
    }

    #[test]
    fn test_extract_rust_methods_includes_trait_methods() {
        let source = r#"
pub trait Visitor {
    fn visit_bool(&self, v: bool) {}
    fn visit_i32(&self, v: i32) {}
}

impl Config {
    pub fn new() -> Self {
        Config {}
    }
    pub fn name(&self) -> &str {
        &self.name
    }
}

fn top_level() {}
"#;
        let tree = parse(source, Language::Rust).unwrap();
        let methods = extract_methods(&tree, source, Language::Rust);

        // Trait default methods should be in methods
        assert!(
            methods.contains(&"visit_bool".to_string()),
            "Should find visit_bool trait method"
        );
        assert!(
            methods.contains(&"visit_i32".to_string()),
            "Should find visit_i32 trait method"
        );
        // Impl methods should be in methods
        assert!(
            methods.contains(&"new".to_string()),
            "Should find new impl method"
        );
        assert!(
            methods.contains(&"name".to_string()),
            "Should find name impl method"
        );
        // Top-level functions should NOT be in methods
        assert!(
            !methods.contains(&"top_level".to_string()),
            "top_level should not be in methods"
        );
    }

    // =========================================================================
    // Fixture-based extraction accuracy tests (18 languages)
    // =========================================================================
    //
    // Each test reads a fixture file, parses it, and asserts the expected
    // counts for functions, classes, and methods. These document the current
    // extraction coverage and identify gaps for unimplemented languages.

    /// Helper: run all three extractors on a source string for a given language
    /// and return (functions, classes, methods) counts.
    fn extract_counts(source: &str, lang: Language) -> (usize, usize, usize) {
        let tree = parse(source, lang).expect("parsing should succeed");
        let functions = extract_functions(&tree, source, lang);
        let classes = extract_classes(&tree, source, lang);
        let methods = extract_methods(&tree, source, lang);
        (functions.len(), classes.len(), methods.len())
    }

    #[test]
    fn test_extractor_python() {
        let source = include_str!("../../tests/fixtures/extractor/test_python.py");
        let (f, c, m) = extract_counts(source, Language::Python);
        assert_eq!(f, 3, "Python: expected 3 functions, got {}", f);
        assert_eq!(c, 2, "Python: expected 2 classes, got {}", c);
        assert_eq!(m, 5, "Python: expected 5 methods, got {}", m);
    }

    #[test]
    fn test_extractor_go() {
        let source = include_str!("../../tests/fixtures/extractor/test_go.go");
        let (f, c, m) = extract_counts(source, Language::Go);
        assert_eq!(f, 4, "Go: expected 4 functions, got {}", f);
        assert_eq!(c, 2, "Go: expected 2 structs, got {}", c);
        assert_eq!(m, 0, "Go: expected 0 methods, got {}", m);
    }

    #[test]
    fn test_extractor_rust() {
        let source = include_str!("../../tests/fixtures/extractor/test_rust.rs");
        let (f, c, m) = extract_counts(source, Language::Rust);
        assert_eq!(f, 3, "Rust: expected 3 functions, got {}", f);
        assert_eq!(c, 2, "Rust: expected 2 structs, got {}", c);
        assert_eq!(m, 4, "Rust: expected 4 methods, got {}", m);
    }

    #[test]
    fn test_extractor_java() {
        let source = include_str!("../../tests/fixtures/extractor/test_java.java");
        let (f, c, m) = extract_counts(source, Language::Java);
        assert_eq!(f, 0, "Java: expected 0 functions, got {}", f);
        assert_eq!(c, 3, "Java: expected 3 classes, got {}", c);
        assert_eq!(m, 6, "Java: expected 6 methods, got {}", m);
    }

    #[test]
    fn test_extractor_c() {
        let source = include_str!("../../tests/fixtures/extractor/test_c.c");
        let (f, c, m) = extract_counts(source, Language::C);
        assert_eq!(f, 4, "C: expected 4 functions, got {}", f);
        assert_eq!(c, 2, "C: expected 2 structs, got {}", c);
        assert_eq!(m, 0, "C: expected 0 methods, got {}", m);
    }

    #[test]
    fn test_extractor_cpp() {
        let source = include_str!("../../tests/fixtures/extractor/test_cpp.cpp");
        let (f, c, m) = extract_counts(source, Language::Cpp);
        assert_eq!(f, 2, "C++: expected 2 functions, got {}", f);
        assert_eq!(c, 2, "C++: expected 2 classes, got {}", c);
        assert_eq!(m, 5, "C++: expected 5 methods, got {}", m);
    }

    #[test]
    fn test_extractor_typescript() {
        let source = include_str!("../../tests/fixtures/extractor/test_typescript.ts");
        let (f, c, m) = extract_counts(source, Language::TypeScript);
        assert_eq!(f, 3, "TypeScript: expected 3 functions, got {}", f);
        assert_eq!(c, 2, "TypeScript: expected 2 classes, got {}", c);
        assert_eq!(m, 4, "TypeScript: expected 4 methods, got {}", m);
    }

    #[test]
    fn test_extractor_javascript() {
        let source = include_str!("../../tests/fixtures/extractor/test_javascript.js");
        // Note: JS uses the TypeScript parser in this crate
        let (f, c, m) = extract_counts(source, Language::JavaScript);
        assert_eq!(f, 5, "JavaScript: expected 5 functions, got {}", f);
        assert_eq!(c, 2, "JavaScript: expected 2 classes, got {}", c);
        assert_eq!(m, 4, "JavaScript: expected 4 methods, got {}", m);
    }

    #[test]
    fn test_extractor_ruby() {
        let source = include_str!("../../tests/fixtures/extractor/test_ruby.rb");
        let (f, c, m) = extract_counts(source, Language::Ruby);
        assert_eq!(f, 2, "Ruby: expected 2 functions, got {}", f);
        assert_eq!(c, 2, "Ruby: expected 2 classes, got {}", c);
        assert_eq!(m, 5, "Ruby: expected 5 methods, got {}", m);
    }

    #[test]
    fn test_extractor_php() {
        let source = include_str!("../../tests/fixtures/extractor/test_php.php");
        let (f, c, m) = extract_counts(source, Language::Php);
        assert_eq!(f, 2, "PHP: expected 2 functions, got {}", f);
        assert_eq!(c, 2, "PHP: expected 2 classes, got {}", c);
        assert_eq!(m, 5, "PHP: expected 5 methods, got {}", m);
    }

    #[test]
    fn test_extractor_kotlin() {
        let source = include_str!("../../tests/fixtures/extractor/test_kotlin.kt");
        let (f, c, m) = extract_counts(source, Language::Kotlin);
        assert_eq!(f, 2, "Kotlin: expected 2 functions, got {}", f);
        assert_eq!(c, 2, "Kotlin: expected 2 classes, got {}", c);
        assert_eq!(m, 5, "Kotlin: expected 5 methods, got {}", m);
    }

    #[test]
    fn test_extractor_swift() {
        let source = include_str!("../../tests/fixtures/extractor/test_swift.swift");
        let (f, c, m) = extract_counts(source, Language::Swift);
        assert_eq!(f, 3, "Swift: expected 3 functions, got {}", f);
        assert_eq!(c, 2, "Swift: expected 2 classes, got {}", c);
        assert_eq!(m, 5, "Swift: expected 5 methods, got {}", m);
    }

    #[test]
    fn test_extractor_csharp() {
        let source = include_str!("../../tests/fixtures/extractor/test_csharp.cs");
        let (f, c, m) = extract_counts(source, Language::CSharp);
        assert_eq!(f, 0, "C#: expected 0 functions, got {}", f);
        assert_eq!(c, 3, "C#: expected 3 classes, got {}", c);
        assert_eq!(
            m, 7,
            "C#: expected 7 methods (including constructors), got {}",
            m
        );
    }

    #[test]
    fn test_extractor_scala() {
        let source = include_str!("../../tests/fixtures/extractor/test_scala.scala");
        let (f, c, m) = extract_counts(source, Language::Scala);
        assert_eq!(f, 2, "Scala: expected 2 functions, got {}", f);
        assert_eq!(c, 2, "Scala: expected 2 classes, got {}", c);
        assert_eq!(m, 4, "Scala: expected 4 methods, got {}", m);
    }

    #[test]
    fn test_extractor_ocaml() {
        let source = include_str!("../../tests/fixtures/extractor/test_ocaml.ml");
        let (f, c, m) = extract_counts(source, Language::Ocaml);
        assert_eq!(f, 3, "OCaml: expected 3 functions, got {}", f);
        assert_eq!(c, 0, "OCaml: expected 0 classes, got {}", c);
        assert_eq!(m, 0, "OCaml: expected 0 methods, got {}", m);
    }

    #[test]
    fn test_extractor_elixir() {
        let source = include_str!("../../tests/fixtures/extractor/test_elixir.ex");
        let (f, c, m) = extract_counts(source, Language::Elixir);
        assert_eq!(f, 3, "Elixir: expected 3 functions, got {}", f);
        assert_eq!(c, 1, "Elixir: expected 1 class (module), got {}", c);
        assert_eq!(m, 0, "Elixir: expected 0 methods, got {}", m);
    }

    #[test]
    fn test_extractor_lua() {
        let source = include_str!("../../tests/fixtures/extractor/test_lua.lua");
        let (f, c, m) = extract_counts(source, Language::Lua);
        assert_eq!(f, 5, "Lua: expected 5 functions, got {}", f);
        assert_eq!(c, 0, "Lua: expected 0 classes, got {}", c);
        assert_eq!(m, 0, "Lua: expected 0 methods, got {}", m);
    }

    #[test]
    fn test_extractor_luau() {
        let source = include_str!("../../tests/fixtures/extractor/test_luau.luau");
        let (f, c, m) = extract_counts(source, Language::Luau);
        assert_eq!(f, 3, "Luau: expected 3 functions, got {}", f);
        assert_eq!(c, 0, "Luau: expected 0 classes, got {}", c);
        assert_eq!(m, 0, "Luau: expected 0 methods, got {}", m);
    }

    // ── constant definitions ──────────────────────────────────────────────

    fn get_constants(source: &str, language: Language) -> Vec<DefinitionInfo> {
        let tree = parse(source, language).unwrap();
        let defs = extract_definitions(&tree, source, language);
        defs.into_iter().filter(|d| d.kind == "constant").collect()
    }

    #[test]
    fn test_python_constant_definitions() {
        let source = "MAX_RETRIES = 3\n\nEXTERNAL_FUNCTIONS = {\n    \"foo\": bar,\n    \"baz\": qux,\n}\n\nlower_case = 42\n";
        let consts = get_constants(source, Language::Python);
        assert_eq!(consts.len(), 2);
        assert_eq!(consts[0].name, "MAX_RETRIES");
        assert_eq!(consts[0].line_start, 1);
        assert_eq!(consts[0].line_end, 1);
        assert_eq!(consts[0].signature, "MAX_RETRIES = 3");
        assert_eq!(consts[1].name, "EXTERNAL_FUNCTIONS");
        assert_eq!(consts[1].line_start, 3);
        assert_eq!(consts[1].line_end, 6);
        assert_eq!(consts[1].signature, "EXTERNAL_FUNCTIONS = {");
    }

    #[test]
    fn test_rust_constant_definitions() {
        let source = "const MAX_SIZE: usize = 100;\npub static GLOBAL: &str = \"hello\";\nlet x = 5;\n";
        let consts = get_constants(source, Language::Rust);
        assert_eq!(consts.len(), 2);
        assert_eq!(consts[0].name, "MAX_SIZE");
        assert_eq!(consts[0].kind, "constant");
        assert_eq!(consts[1].name, "GLOBAL");
    }

    #[test]
    fn test_go_constant_definitions() {
        let source = "package main\n\nconst MaxRetries = 3\n\nconst (\n\tA = 1\n\tB = 2\n)\n";
        let consts = get_constants(source, Language::Go);
        assert_eq!(consts.len(), 3);
        let names: Vec<&str> = consts.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"MaxRetries"));
        assert!(names.contains(&"A"));
        assert!(names.contains(&"B"));
    }

    #[test]
    fn test_typescript_constant_definitions() {
        let source = "const MAX_RETRIES = 3;\nexport const API_URL = \"https://example.com\";\nconst lower = 42;\n";
        let consts = get_constants(source, Language::TypeScript);
        assert_eq!(consts.len(), 2);
        assert_eq!(consts[0].name, "MAX_RETRIES");
        assert_eq!(consts[1].name, "API_URL");
    }

    #[test]
    fn test_javascript_multiline_constant() {
        let source = "const EXTERNAL_FUNCTIONS = {\n  foo: 1,\n  bar: 2,\n};\n";
        let consts = get_constants(source, Language::JavaScript);
        assert_eq!(consts.len(), 1);
        assert_eq!(consts[0].name, "EXTERNAL_FUNCTIONS");
        assert_eq!(consts[0].line_start, 1);
        assert_eq!(consts[0].line_end, 4);
        assert_eq!(consts[0].signature, "const EXTERNAL_FUNCTIONS = {");
    }

    #[test]
    fn test_c_constant_definitions() {
        let source = "#define MAX_SIZE 100\nconst int BUFFER_LEN = 256;\nint x = 5;\n";
        let consts = get_constants(source, Language::C);
        assert_eq!(consts.len(), 2);
        assert_eq!(consts[0].name, "MAX_SIZE");
        assert_eq!(consts[1].name, "BUFFER_LEN");
    }

    #[test]
    fn test_java_constant_definitions() {
        let source = "class Config {\n    public static final int MAX_RETRIES = 3;\n    public static final String API_URL = \"https://example.com\";\n    private int x = 5;\n}\n";
        let consts = get_constants(source, Language::Java);
        assert_eq!(consts.len(), 2);
        assert_eq!(consts[0].name, "MAX_RETRIES");
        assert_eq!(consts[1].name, "API_URL");
    }

    #[test]
    fn test_ruby_constant_definitions() {
        let source = "MAX_RETRIES = 3\nAPI_URL = \"https://example.com\"\nlower = 42\n";
        let consts = get_constants(source, Language::Ruby);
        assert_eq!(consts.len(), 2);
        assert_eq!(consts[0].name, "MAX_RETRIES");
        assert_eq!(consts[1].name, "API_URL");
    }

    // ── Bug fix: Python top-level def must be "function" not "method" ─────

    #[test]
    fn test_python_toplevel_function_kind_is_function() {
        let source = r#"
def top_level():
    pass

class MyClass:
    def method(self):
        pass
"#;
        let tree = parse(source, Language::Python).unwrap();
        let defs = extract_definitions(&tree, source, Language::Python);
        let top_level = defs.iter().find(|d| d.name == "top_level");
        assert!(top_level.is_some(), "top_level definition not found");
        assert_eq!(
            top_level.unwrap().kind,
            "function",
            "top-level Python def must have kind 'function', not 'method'"
        );
        let method = defs.iter().find(|d| d.name == "method");
        assert!(method.is_some(), "method definition not found");
        assert_eq!(
            method.unwrap().kind,
            "method",
            "Python def inside class must have kind 'method'"
        );
    }

    // ── Bug fix: OCaml definitions array must be populated ────────────────

    #[test]
    fn test_ocaml_definitions_non_empty() {
        let source = r#"
let top_level x = x * 2

let another_func x y = x + y

let rec factorial n =
  if n <= 1 then 1
  else n * factorial (n - 1)

let () =
  let result = factorial 5 in
  Printf.printf "%d\n" result
"#;
        let tree = parse(source, Language::Ocaml).unwrap();
        let defs = extract_definitions(&tree, source, Language::Ocaml);
        assert!(
            !defs.is_empty(),
            "OCaml definitions array must be non-empty; got 0"
        );
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert!(
            names.contains(&"top_level"),
            "OCaml: expected 'top_level' in definitions, got {:?}",
            names
        );
        assert!(
            names.contains(&"another_func"),
            "OCaml: expected 'another_func' in definitions, got {:?}",
            names
        );
        assert!(
            names.contains(&"factorial"),
            "OCaml: expected 'factorial' in definitions, got {:?}",
            names
        );
        // Anonymous entry-point let () = ... must not appear
        assert!(
            !names.contains(&"()"),
            "OCaml: '()' binding must not appear in definitions"
        );
    }

    // ── Bug fix: Kotlin companion object must produce a named definition ──

    #[test]
    fn test_kotlin_companion_object_definition() {
        let source = r#"
class Animal(val name: String) {
    fun speak(): String = "..."

    companion object {
        fun create(name: String): Animal = Animal(name)
    }
}
"#;
        let tree = parse(source, Language::Kotlin).unwrap();
        let defs = extract_definitions(&tree, source, Language::Kotlin);
        let companion = defs.iter().find(|d| d.name == "Companion");
        assert!(
            companion.is_some(),
            "Kotlin: companion object must produce a 'Companion' definition; definitions: {:?}",
            defs.iter().map(|d| (&d.name, &d.kind)).collect::<Vec<_>>()
        );
        assert_eq!(
            companion.unwrap().kind,
            "class",
            "Kotlin companion object kind must be 'class'"
        );
    }
}
