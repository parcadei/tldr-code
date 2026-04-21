//! Language-specific import parsing (spec Section 2.1.4)
//!
//! Parses import statements from source files for various languages:
//! - Python: import X, from X import Y, relative imports
//! - TypeScript: import, require, dynamic import
//! - Go: import "pkg", import alias
//! - Rust: use, mod, extern crate

use std::path::Path;

use tree_sitter::{Node, Tree};

use crate::types::{ImportInfo, Language};
use crate::TldrResult;

use super::parser::parse_file;

/// Parse imports from a source file.
///
/// # Arguments
/// * `file_path` - Path to source file
/// * `language` - Programming language
///
/// # Returns
/// * `Ok(Vec<ImportInfo>)` - List of imports
/// * `Err(TldrError::PathNotFound)` - File doesn't exist
pub fn get_imports(file_path: &Path, language: Language) -> TldrResult<Vec<ImportInfo>> {
    let (tree, source, _) = parse_file(file_path)?;
    extract_imports_from_tree(&tree, &source, language)
}

/// Extract imports from a parsed tree
pub fn extract_imports_from_tree(
    tree: &Tree,
    source: &str,
    language: Language,
) -> TldrResult<Vec<ImportInfo>> {
    let root = tree.root_node();

    let imports = match language {
        Language::Python => extract_python_imports(&root, source),
        Language::TypeScript | Language::JavaScript => extract_ts_imports(&root, source),
        Language::Go => extract_go_imports(&root, source),
        Language::Rust => extract_rust_imports(&root, source),
        Language::Java => extract_java_imports(&root, source),
        Language::C => extract_c_imports(&root, source),
        Language::Cpp => extract_cpp_imports(&root, source),
        Language::Ruby => extract_ruby_imports(&root, source),
        Language::CSharp => extract_csharp_imports(&root, source),
        Language::Scala => extract_scala_imports(&root, source),
        Language::Elixir => extract_elixir_imports(&root, source),
        Language::Ocaml => extract_ocaml_imports(&root, source),
        Language::Php => extract_php_imports(&root, source),
        Language::Lua | Language::Luau => extract_lua_imports(&root, source),
        // Languages with import extraction not yet implemented -- return empty
        Language::Kotlin | Language::Swift => Vec::new(),
    };

    Ok(imports)
}

// =============================================================================
// Python imports
// =============================================================================

fn extract_python_imports(node: &Node, source: &str) -> Vec<ImportInfo> {
    let mut imports = Vec::new();
    extract_python_imports_recursive(node, source, &mut imports);
    imports
}

fn extract_python_imports_recursive(node: &Node, source: &str, imports: &mut Vec<ImportInfo>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_statement" => {
                // import X, Y, Z
                let mut import_cursor = child.walk();
                for import_child in child.children(&mut import_cursor) {
                    if import_child.kind() == "dotted_name" {
                        let module = get_node_text(&import_child, source);
                        imports.push(ImportInfo {
                            module,
                            names: Vec::new(),
                            is_from: false,
                            alias: None,
                        });
                    } else if import_child.kind() == "aliased_import" {
                        let module = import_child
                            .child_by_field_name("name")
                            .map(|n| get_node_text(&n, source))
                            .unwrap_or_default();
                        let alias = import_child
                            .child_by_field_name("alias")
                            .map(|n| get_node_text(&n, source));
                        imports.push(ImportInfo {
                            module,
                            names: Vec::new(),
                            is_from: false,
                            alias,
                        });
                    }
                }
            }
            "import_from_statement" => {
                // from X import Y, Z
                let module = child
                    .child_by_field_name("module_name")
                    .map(|n| get_node_text(&n, source))
                    .unwrap_or_else(|| {
                        // Handle relative imports (from . import X)
                        let mut module_parts = Vec::new();
                        let mut c = child.walk();
                        for part in child.children(&mut c) {
                            if part.kind() == "." || part.kind() == "relative_import" {
                                module_parts.push(".".to_string());
                            } else if part.kind() == "dotted_name" {
                                module_parts.push(get_node_text(&part, source));
                            }
                        }
                        module_parts.join("")
                    });

                let mut names = Vec::new();
                let mut import_cursor = child.walk();

                for import_child in child.children(&mut import_cursor) {
                    match import_child.kind() {
                        "dotted_name" | "identifier" => {
                            // Skip if this is the module name
                            if import_child.start_byte()
                                > child
                                    .child_by_field_name("module_name")
                                    .map(|n| n.end_byte())
                                    .unwrap_or(0)
                            {
                                names.push(get_node_text(&import_child, source));
                            }
                        }
                        "aliased_import" => {
                            // For aliased imports, create a separate ImportInfo entry
                            let name = import_child
                                .child_by_field_name("name")
                                .map(|n| get_node_text(&n, source))
                                .unwrap_or_default();
                            let alias = import_child
                                .child_by_field_name("alias")
                                .map(|n| get_node_text(&n, source));

                            imports.push(ImportInfo {
                                module: module.clone(),
                                names: vec![name],
                                is_from: true,
                                alias,
                            });
                        }
                        "wildcard_import" => {
                            names.push("*".to_string());
                        }
                        _ => {}
                    }
                }

                // Only push a general import if we collected non-aliased names
                if !names.is_empty() {
                    imports.push(ImportInfo {
                        module,
                        names,
                        is_from: true,
                        alias: None,
                    });
                }
            }
            _ => {
                extract_python_imports_recursive(&child, source, imports);
            }
        }
    }
}

// =============================================================================
// TypeScript/JavaScript imports
// =============================================================================

fn extract_ts_imports(node: &Node, source: &str) -> Vec<ImportInfo> {
    let mut imports = Vec::new();
    extract_ts_imports_recursive(node, source, &mut imports);
    imports
}

fn extract_ts_imports_recursive(node: &Node, source: &str, imports: &mut Vec<ImportInfo>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_statement" => {
                let module = child
                    .child_by_field_name("source")
                    .map(|n| get_string_content(&n, source))
                    .unwrap_or_default();

                let mut names = Vec::new();
                let mut is_default = false;

                // Parse import clause
                if let Some(clause) = child
                    .children(&mut child.walk())
                    .find(|c| c.kind() == "import_clause")
                {
                    let mut clause_cursor = clause.walk();
                    for clause_child in clause.children(&mut clause_cursor) {
                        match clause_child.kind() {
                            "identifier" => {
                                // Default import
                                is_default = true;
                                names.push(get_node_text(&clause_child, source));
                            }
                            "named_imports" => {
                                // { a, b, c }
                                let mut named_cursor = clause_child.walk();
                                for named in clause_child.children(&mut named_cursor) {
                                    if named.kind() == "import_specifier" {
                                        if let Some(name) = named.child_by_field_name("name") {
                                            names.push(get_node_text(&name, source));
                                        }
                                    }
                                }
                            }
                            "namespace_import" => {
                                // import * as X — extract X as alias
                                names.push("*".to_string());
                                // Find the identifier after "as" in namespace_import
                                let mut ns_cursor = clause_child.walk();
                                for ns_child in clause_child.children(&mut ns_cursor) {
                                    if ns_child.kind() == "identifier" {
                                        is_default = false; // mark as namespace, not default
                                                            // Store alias name temporarily — will be set below
                                        names.push(get_node_text(&ns_child, source));
                                        break;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }

                imports.push(ImportInfo {
                    module,
                    names,
                    is_from: !is_default,
                    alias: None,
                });
            }
            "export_statement" => {
                // export { x } from 'module' - re-exports
                if let Some(source_node) = child.child_by_field_name("source") {
                    let module = get_string_content(&source_node, source);
                    imports.push(ImportInfo {
                        module,
                        names: Vec::new(),
                        is_from: true,
                        alias: None,
                    });
                }
            }
            _ => {
                extract_ts_imports_recursive(&child, source, imports);
            }
        }
    }
}

// =============================================================================
// Go imports
// =============================================================================

fn extract_go_imports(node: &Node, source: &str) -> Vec<ImportInfo> {
    let mut imports = Vec::new();
    extract_go_imports_recursive(node, source, &mut imports);
    imports
}

fn extract_go_imports_recursive(node: &Node, source: &str, imports: &mut Vec<ImportInfo>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_declaration" => {
                let mut decl_cursor = child.walk();
                for decl_child in child.children(&mut decl_cursor) {
                    match decl_child.kind() {
                        "import_spec" => {
                            let module = decl_child
                                .child_by_field_name("path")
                                .map(|n| get_string_content(&n, source))
                                .unwrap_or_default();

                            let alias = decl_child
                                .child_by_field_name("name")
                                .map(|n| get_node_text(&n, source));

                            imports.push(ImportInfo {
                                module,
                                names: Vec::new(),
                                is_from: false,
                                alias,
                            });
                        }
                        "import_spec_list" => {
                            let mut list_cursor = decl_child.walk();
                            for spec in decl_child.children(&mut list_cursor) {
                                if spec.kind() == "import_spec" {
                                    let module = spec
                                        .child_by_field_name("path")
                                        .map(|n| get_string_content(&n, source))
                                        .unwrap_or_default();

                                    let alias = spec
                                        .child_by_field_name("name")
                                        .map(|n| get_node_text(&n, source));

                                    imports.push(ImportInfo {
                                        module,
                                        names: Vec::new(),
                                        is_from: false,
                                        alias,
                                    });
                                }
                            }
                        }
                        "interpreted_string_literal" => {
                            // Single import without parentheses
                            let module = get_string_content(&decl_child, source);
                            imports.push(ImportInfo {
                                module,
                                names: Vec::new(),
                                is_from: false,
                                alias: None,
                            });
                        }
                        _ => {}
                    }
                }
            }
            _ => {
                extract_go_imports_recursive(&child, source, imports);
            }
        }
    }
}

// =============================================================================
// Rust imports
// =============================================================================

fn extract_rust_imports(node: &Node, source: &str) -> Vec<ImportInfo> {
    let mut imports = Vec::new();
    extract_rust_imports_recursive(node, source, &mut imports);
    imports
}

fn extract_rust_imports_recursive(node: &Node, source: &str, imports: &mut Vec<ImportInfo>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "use_declaration" => {
                // use std::collections::HashMap;
                // use crate::module::{A, B};
                // Extract the path and names
                if let Some(arg) = child.child_by_field_name("argument") {
                    let (module, names) = parse_rust_use_path(&arg, source);
                    imports.push(ImportInfo {
                        module,
                        names,
                        is_from: true,
                        alias: None,
                    });
                }
            }
            "mod_item" => {
                // mod module_name;
                if let Some(name) = child.child_by_field_name("name") {
                    let module = get_node_text(&name, source);
                    imports.push(ImportInfo {
                        module,
                        names: Vec::new(),
                        is_from: false,
                        alias: None,
                    });
                }
            }
            "extern_crate_declaration" => {
                // extern crate foo;
                if let Some(name) = child.child_by_field_name("name") {
                    let module = get_node_text(&name, source);
                    let alias = child
                        .child_by_field_name("alias")
                        .map(|n| get_node_text(&n, source));
                    imports.push(ImportInfo {
                        module,
                        names: Vec::new(),
                        is_from: false,
                        alias,
                    });
                }
            }
            _ => {
                extract_rust_imports_recursive(&child, source, imports);
            }
        }
    }
}

fn parse_rust_use_path(node: &Node, source: &str) -> (String, Vec<String>) {
    // Use proper AST traversal for complex use statements
    let mut imports = Vec::new();
    collect_rust_use_paths(node, source, String::new(), &mut imports);

    // If we collected imports, use the first one's module and all names
    if !imports.is_empty() {
        // Find the common module prefix
        let first_module = imports[0].0.clone();
        let names: Vec<String> = imports.into_iter().map(|(_, name)| name).collect();
        return (first_module, names);
    }

    // Fallback to simple text parsing for edge cases
    let text = get_node_text(node, source);

    // Simple heuristic: split on :: and handle {a, b}
    if let Some(brace_pos) = text.find('{') {
        let module = text[..brace_pos].trim_end_matches("::").to_string();
        let names_part = &text[brace_pos..];
        let names: Vec<String> = names_part
            .trim_matches(|c| c == '{' || c == '}')
            .split(',')
            .map(|s| {
                // Handle "self" and aliases like "HashMap as Map"
                let s = s.trim();
                if let Some(as_pos) = s.find(" as ") {
                    s[..as_pos].trim().to_string()
                } else {
                    s.to_string()
                }
            })
            .filter(|s| !s.is_empty())
            .collect();
        (module, names)
    } else {
        // No braces - extract last segment as name
        let parts: Vec<&str> = text.split("::").collect();
        if parts.len() > 1 {
            let module = parts[..parts.len() - 1].join("::");
            let name = parts.last().unwrap().to_string();
            (module, vec![name])
        } else {
            (text, Vec::new())
        }
    }
}

/// Recursively collect all imports from a Rust use tree
/// Handles nested use groups like `use std::{io::{self, Read}, collections::HashMap}`
fn collect_rust_use_paths(
    node: &Node,
    source: &str,
    prefix: String,
    imports: &mut Vec<(String, String)>,
) {
    match node.kind() {
        "scoped_identifier" | "identifier" => {
            // Simple path like `std::collections::HashMap`
            let text = get_node_text(node, source);
            let full_path = if prefix.is_empty() {
                text.clone()
            } else {
                format!("{}::{}", prefix, text)
            };

            // Extract the module and name parts
            let parts: Vec<&str> = full_path.split("::").collect();
            if parts.len() > 1 {
                let module = parts[..parts.len() - 1].join("::");
                let name = parts.last().unwrap().to_string();
                imports.push((module, name));
            } else {
                imports.push((String::new(), full_path));
            }
        }
        "scoped_use_list" => {
            // Handle `std::io::{Read, Write}`
            // First child is the path, second is the use_list
            if let Some(path_node) = node.child_by_field_name("path") {
                let path_text = get_node_text(&path_node, source);
                let new_prefix = if prefix.is_empty() {
                    path_text
                } else {
                    format!("{}::{}", prefix, path_text)
                };

                // Find the use_list child
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "use_list" {
                        collect_rust_use_paths(&child, source, new_prefix.clone(), imports);
                    }
                }
            }
        }
        "use_list" => {
            // Handle `{Read, Write, self}`
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_rust_use_paths(&child, source, prefix.clone(), imports);
            }
        }
        "use_as_clause" => {
            // Handle `HashMap as Map`
            if let Some(path_node) = node.child_by_field_name("path") {
                collect_rust_use_paths(&path_node, source, prefix, imports);
            }
        }
        "use_wildcard" => {
            // Handle `use foo::*`
            imports.push((prefix, "*".to_string()));
        }
        "self" => {
            // Handle `{self, Read}` - self imports the module itself
            imports.push((prefix, "self".to_string()));
        }
        _ => {
            // Recursively check children
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_rust_use_paths(&child, source, prefix.clone(), imports);
            }
        }
    }
}

// =============================================================================
// Java imports
// =============================================================================

fn extract_java_imports(node: &Node, source: &str) -> Vec<ImportInfo> {
    let mut imports = Vec::new();
    extract_java_imports_recursive(node, source, &mut imports);
    imports
}

fn extract_java_imports_recursive(node: &Node, source: &str, imports: &mut Vec<ImportInfo>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "import_declaration" {
            let mut is_static = false;
            let mut is_wildcard = false;
            let mut module = String::new();

            let mut import_cursor = child.walk();
            for import_child in child.children(&mut import_cursor) {
                match import_child.kind() {
                    "static" => is_static = true,
                    "scoped_identifier" | "identifier" => {
                        module = get_node_text(&import_child, source);
                    }
                    "asterisk" => is_wildcard = true,
                    _ => {}
                }
            }

            // Handle wildcard
            if is_wildcard {
                module = format!("{}.*", module);
            }

            imports.push(ImportInfo {
                module,
                names: Vec::new(),
                is_from: is_static,
                alias: None,
            });
        } else {
            extract_java_imports_recursive(&child, source, imports);
        }
    }
}

// =============================================================================
// C imports (#include directives)
// =============================================================================

fn extract_c_imports(node: &Node, source: &str) -> Vec<ImportInfo> {
    let mut imports = Vec::new();
    extract_c_imports_recursive(node, source, &mut imports);
    imports
}

fn extract_c_imports_recursive(node: &Node, source: &str, imports: &mut Vec<ImportInfo>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "preproc_include" {
            // #include <header.h> or #include "header.h"
            if let Some(path_node) = child.child_by_field_name("path") {
                let path_kind = path_node.kind();
                let raw_text = get_node_text(&path_node, source);

                // Extract the header name, stripping quotes or angle brackets
                let module = match path_kind {
                    "system_lib_string" => {
                        // <stdio.h> -> strip < and >
                        raw_text.trim_matches(|c| c == '<' || c == '>').to_string()
                    }
                    "string_literal" => {
                        // "local.h" -> strip quotes
                        raw_text.trim_matches('"').to_string()
                    }
                    _ => raw_text,
                };

                // is_from = true for system headers (<>), false for local headers ("")
                let is_system = path_kind == "system_lib_string";

                imports.push(ImportInfo {
                    module,
                    names: Vec::new(),
                    is_from: is_system, // We use is_from to indicate system vs local
                    alias: None,
                });
            }
        } else {
            // Recurse into other nodes
            extract_c_imports_recursive(&child, source, imports);
        }
    }
}

// =============================================================================
// C++ imports (#include directives)
// =============================================================================

fn extract_cpp_imports(node: &Node, source: &str) -> Vec<ImportInfo> {
    // C++ uses the same #include syntax as C
    // The tree-sitter-cpp grammar also uses preproc_include
    let mut imports = Vec::new();
    extract_cpp_imports_recursive(node, source, &mut imports);
    imports
}

fn extract_cpp_imports_recursive(node: &Node, source: &str, imports: &mut Vec<ImportInfo>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "preproc_include" {
            // #include <header> or #include "header"
            if let Some(path_node) = child.child_by_field_name("path") {
                let path_kind = path_node.kind();
                let raw_text = get_node_text(&path_node, source);

                let module = match path_kind {
                    "system_lib_string" => {
                        raw_text.trim_matches(|c| c == '<' || c == '>').to_string()
                    }
                    "string_literal" => raw_text.trim_matches('"').to_string(),
                    _ => raw_text,
                };

                let is_system = path_kind == "system_lib_string";

                imports.push(ImportInfo {
                    module,
                    names: Vec::new(),
                    is_from: is_system,
                    alias: None,
                });
            }
        } else {
            extract_cpp_imports_recursive(&child, source, imports);
        }
    }
}

// =============================================================================
// Ruby imports (require/require_relative)
// =============================================================================

fn extract_ruby_imports(node: &Node, source: &str) -> Vec<ImportInfo> {
    let mut imports = Vec::new();
    extract_ruby_imports_recursive(node, source, &mut imports);
    imports
}

fn extract_ruby_imports_recursive(node: &Node, source: &str, imports: &mut Vec<ImportInfo>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "call" {
            // Check if this is a require/require_relative call
            let mut call_cursor = child.walk();
            let mut method_name = String::new();
            let mut arg_value = String::new();

            for call_child in child.children(&mut call_cursor) {
                match call_child.kind() {
                    "identifier" => {
                        method_name = get_node_text(&call_child, source);
                    }
                    "argument_list" => {
                        // Get the string argument
                        let mut arg_cursor = call_child.walk();
                        for arg_child in call_child.children(&mut arg_cursor) {
                            if arg_child.kind() == "string" {
                                // Look for string_content inside the string node
                                let mut str_cursor = arg_child.walk();
                                for str_child in arg_child.children(&mut str_cursor) {
                                    if str_child.kind() == "string_content" {
                                        arg_value = get_node_text(&str_child, source);
                                        break;
                                    }
                                }
                                if arg_value.is_empty() {
                                    // Fallback: use the whole string text with quotes stripped
                                    arg_value = get_string_content(&arg_child, source);
                                }
                                break;
                            }
                        }
                    }
                    _ => {}
                }
            }

            // Handle different require patterns
            match method_name.as_str() {
                "require" => {
                    if !arg_value.is_empty() {
                        // require 'gem' or require './path'
                        // is_from = false for external gems, true for relative requires
                        let is_relative =
                            arg_value.starts_with("./") || arg_value.starts_with("../");
                        imports.push(ImportInfo {
                            module: arg_value,
                            names: Vec::new(),
                            is_from: is_relative, // is_from indicates relative path
                            alias: None,
                        });
                    }
                }
                "require_relative" => {
                    if !arg_value.is_empty() {
                        // require_relative './path' - always relative
                        imports.push(ImportInfo {
                            module: arg_value,
                            names: Vec::new(),
                            is_from: true, // is_from = true for require_relative (relative import)
                            alias: None,
                        });
                    }
                }
                _ => {}
            }
        }

        // Recurse into other nodes
        extract_ruby_imports_recursive(&child, source, imports);
    }
}

// =============================================================================
// C# imports (using directives)
// =============================================================================

fn extract_csharp_imports(node: &Node, source: &str) -> Vec<ImportInfo> {
    let mut imports = Vec::new();
    extract_csharp_imports_recursive(node, source, &mut imports);
    imports
}

fn extract_csharp_imports_recursive(node: &Node, source: &str, imports: &mut Vec<ImportInfo>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "using_directive" {
            // C# using directives:
            // - using System;
            // - using static System.Math;
            // - global using System;
            // - using Alias = System.Collections.Generic;

            let text = get_node_text(&child, source);
            let is_static = text.contains("static");
            let is_global = text.contains("global");

            let mut module = String::new();
            let mut alias: Option<String> = None;

            let mut using_cursor = child.walk();
            for using_child in child.children(&mut using_cursor) {
                match using_child.kind() {
                    // Handle qualified name (e.g., System.Collections.Generic)
                    "qualified_name" | "identifier" | "name" => {
                        // Only set module if not already set (for alias case)
                        if module.is_empty() {
                            module = get_node_text(&using_child, source);
                        }
                    }
                    // Handle alias: using Alias = Namespace;
                    "name_equals" => {
                        // The alias is in the name_equals node
                        let mut name_cursor = using_child.walk();
                        for name_child in using_child.children(&mut name_cursor) {
                            if name_child.kind() == "identifier" {
                                alias = Some(get_node_text(&name_child, source));
                                break;
                            }
                        }
                        // The actual namespace comes after name_equals
                        // Continue iteration to find it
                    }
                    _ => {}
                }
            }

            // If we found a name_equals but module is the alias, we need to find the real module
            // Re-traverse to get the qualified_name after name_equals
            if alias.is_some() {
                let mut found_name_equals = false;
                let mut using_cursor2 = child.walk();
                for using_child in child.children(&mut using_cursor2) {
                    if using_child.kind() == "name_equals" {
                        found_name_equals = true;
                        continue;
                    }
                    if found_name_equals
                        && (using_child.kind() == "qualified_name"
                            || using_child.kind() == "identifier")
                    {
                        module = get_node_text(&using_child, source);
                        break;
                    }
                }
            }

            if !module.is_empty() {
                imports.push(ImportInfo {
                    module,
                    names: Vec::new(),
                    // Use is_from to indicate static imports (similar to Java pattern)
                    is_from: is_static || is_global,
                    alias,
                });
            }
        } else {
            // Recurse into other nodes (e.g., namespace declarations)
            extract_csharp_imports_recursive(&child, source, imports);
        }
    }
}

// =============================================================================
// Scala imports
// =============================================================================

fn extract_scala_imports(node: &Node, source: &str) -> Vec<ImportInfo> {
    let mut imports = Vec::new();
    extract_scala_imports_recursive(node, source, &mut imports);
    imports
}

fn extract_scala_imports_recursive(node: &Node, source: &str, imports: &mut Vec<ImportInfo>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "import_declaration" {
            // Scala import syntax:
            // - import scala.util.Try                    (simple)
            // - import scala.collection._               (wildcard)
            // - import scala.util.{Try, Success}        (selective)
            // - import scala.util.{Try => T}            (rename with =>)
            // - import scala.util.{Try, Success => S, _} (mixed)

            // Get the full import text for parsing
            let import_text = get_node_text(&child, source);

            // Remove "import " prefix
            let text = import_text
                .strip_prefix("import ")
                .unwrap_or(&import_text)
                .trim();

            // Parse the import text
            parse_scala_import_text(text, imports);
        } else {
            // Recurse into other nodes
            extract_scala_imports_recursive(&child, source, imports);
        }
    }
}

/// Parse Scala import text and extract ImportInfo entries
fn parse_scala_import_text(text: &str, imports: &mut Vec<ImportInfo>) {
    // Check for selective imports with braces: import scala.util.{Try, Success}
    if let Some(brace_pos) = text.find('{') {
        let base_path = text[..brace_pos].trim_end_matches('.').to_string();
        let selectors_part = &text[brace_pos..];

        // Extract content between braces
        let selectors_content = selectors_part
            .trim_start_matches('{')
            .trim_end_matches('}')
            .trim();

        // Parse each selector
        for selector in selectors_content.split(',') {
            let selector = selector.trim();
            if selector.is_empty() {
                continue;
            }

            // Check for rename: member => alias (Scala uses => for rename)
            if selector.contains("=>") {
                let parts: Vec<&str> = selector.split("=>").collect();
                if parts.len() == 2 {
                    let orig = parts[0].trim();
                    let alias = parts[1].trim();

                    // Skip if original is "_" (hide import)
                    if orig == "_" {
                        continue;
                    }

                    let full_module = if base_path.is_empty() {
                        orig.to_string()
                    } else {
                        format!("{}.{}", base_path, orig)
                    };

                    imports.push(ImportInfo {
                        module: full_module,
                        names: Vec::new(),
                        is_from: false,
                        // alias is None if it's "_" (hide), otherwise the alias name
                        alias: if alias == "_" {
                            None
                        } else {
                            Some(alias.to_string())
                        },
                    });
                }
            } else if selector == "_" {
                // Wildcard import inside braces: import scala.util.{_, ...}
                imports.push(ImportInfo {
                    module: base_path.clone(),
                    names: vec!["*".to_string()],
                    is_from: true,
                    alias: None,
                });
            } else {
                // Simple selector: import scala.util.{Try}
                let full_module = if base_path.is_empty() {
                    selector.to_string()
                } else {
                    format!("{}.{}", base_path, selector)
                };

                imports.push(ImportInfo {
                    module: full_module,
                    names: Vec::new(),
                    is_from: false,
                    alias: None,
                });
            }
        }
    } else if text.ends_with("._") {
        // Wildcard import: import scala.collection.mutable._
        let base_path = text.strip_suffix("._").unwrap_or(text).to_string();
        imports.push(ImportInfo {
            module: base_path,
            names: vec!["*".to_string()],
            is_from: true,
            alias: None,
        });
    } else {
        // Simple import: import scala.collection.mutable.ListBuffer
        imports.push(ImportInfo {
            module: text.to_string(),
            names: Vec::new(),
            is_from: false,
            alias: None,
        });
    }
}

// =============================================================================
// Elixir imports (import, alias, require, use)
// =============================================================================

/// Extract imports from Elixir source code.
///
/// Handles:
/// - `import Phoenix.Controller` — imports all functions from a module
/// - `alias Phoenix.LiveView` — creates alias using last segment as short name
/// - `alias Phoenix.LiveView, as: LV` — explicit alias
/// - `require Logger` — requires module for macros
/// - `use GenServer` — imports and extends with macros
fn extract_elixir_imports(node: &Node, source: &str) -> Vec<ImportInfo> {
    let mut imports = Vec::new();
    extract_elixir_imports_recursive(node, source, &mut imports);
    imports
}

fn extract_elixir_imports_recursive(node: &Node, source: &str, imports: &mut Vec<ImportInfo>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "call" {
            // Elixir import-like statements are all `call` nodes.
            // Structure: call -> identifier (keyword) + arguments -> alias (module name)
            let mut call_cursor = child.walk();
            let mut keyword = String::new();
            let mut module_name = String::new();
            let mut explicit_alias: Option<String> = None;

            for call_child in child.children(&mut call_cursor) {
                match call_child.kind() {
                    "identifier" => {
                        keyword = get_node_text(&call_child, source);
                    }
                    "arguments" => {
                        // First alias child is the module name
                        let mut args_cursor = call_child.walk();
                        for arg_child in call_child.children(&mut args_cursor) {
                            match arg_child.kind() {
                                "alias" if module_name.is_empty() => {
                                    module_name = get_node_text(&arg_child, source);
                                }
                                "keywords" => {
                                    // Parse `as: ShortName` from keywords -> pair -> keyword + alias
                                    let mut kw_cursor = arg_child.walk();
                                    for kw_child in arg_child.children(&mut kw_cursor) {
                                        if kw_child.kind() == "pair" {
                                            let mut pair_cursor = kw_child.walk();
                                            let mut is_as_pair = false;
                                            for pair_child in kw_child.children(&mut pair_cursor) {
                                                match pair_child.kind() {
                                                    "keyword" => {
                                                        let kw_text =
                                                            get_node_text(&pair_child, source);
                                                        // keyword text includes trailing colon+space: "as: "
                                                        if kw_text.trim().trim_end_matches(':')
                                                            == "as"
                                                        {
                                                            is_as_pair = true;
                                                        }
                                                    }
                                                    "alias" if is_as_pair => {
                                                        explicit_alias = Some(get_node_text(
                                                            &pair_child,
                                                            source,
                                                        ));
                                                    }
                                                    _ => {}
                                                }
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }

            // Only process recognized Elixir import keywords
            match keyword.as_str() {
                "import" => {
                    if !module_name.is_empty() {
                        imports.push(ImportInfo {
                            module: module_name,
                            names: vec!["*".to_string()],
                            is_from: true,
                            alias: None,
                        });
                    }
                }
                "alias" => {
                    if !module_name.is_empty() {
                        // If no explicit alias, Elixir uses the last segment
                        let resolved_alias = explicit_alias
                            .or_else(|| module_name.rsplit('.').next().map(|s| s.to_string()));
                        imports.push(ImportInfo {
                            module: module_name,
                            names: Vec::new(),
                            is_from: false,
                            alias: resolved_alias,
                        });
                    }
                }
                "require" => {
                    if !module_name.is_empty() {
                        imports.push(ImportInfo {
                            module: module_name,
                            names: Vec::new(),
                            is_from: false,
                            alias: None,
                        });
                    }
                }
                "use" => {
                    if !module_name.is_empty() {
                        imports.push(ImportInfo {
                            module: module_name,
                            names: vec!["*".to_string()],
                            is_from: true,
                            alias: None,
                        });
                    }
                }
                _ => {
                    // Non-import call nodes (e.g., defmodule, def, defp) may contain import statements
                    // Recurse into the call node to find nested imports
                    extract_elixir_imports_recursive(&child, source, imports);
                }
            }
        } else {
            // Recurse into other nodes
            extract_elixir_imports_recursive(&child, source, imports);
        }
    }
}

// =============================================================================
// OCaml imports (open, module alias, include)
// =============================================================================

/// Extract imports from OCaml source code.
///
/// Handles:
/// - `open ModuleName` — opens a module (like import *)
/// - `module M = ModuleName` — module alias
/// - `include ModuleName` — includes module contents
fn extract_ocaml_imports(node: &Node, source: &str) -> Vec<ImportInfo> {
    let mut imports = Vec::new();
    extract_ocaml_imports_recursive(node, source, &mut imports);
    imports
}

fn extract_ocaml_imports_recursive(node: &Node, source: &str, imports: &mut Vec<ImportInfo>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "open_module" => {
                // Structure: open_module -> "open" + module_path -> module_name
                if let Some(module) = extract_ocaml_module_path(&child, source) {
                    imports.push(ImportInfo {
                        module,
                        names: vec!["*".to_string()],
                        is_from: true,
                        alias: None,
                    });
                }
            }
            "module_definition" => {
                // Structure: module_definition -> "module" + module_binding
                //   module_binding -> module_name (alias) + "=" + module_path (target)
                let mut def_cursor = child.walk();
                for def_child in child.children(&mut def_cursor) {
                    if def_child.kind() == "module_binding" {
                        let mut alias_name: Option<String> = None;
                        let mut target_module: Option<String> = None;

                        let mut bind_cursor = def_child.walk();
                        for bind_child in def_child.children(&mut bind_cursor) {
                            match bind_child.kind() {
                                "module_name" if alias_name.is_none() => {
                                    alias_name = Some(get_node_text(&bind_child, source));
                                }
                                "module_path" => {
                                    target_module =
                                        Some(extract_ocaml_module_path_text(&bind_child, source));
                                }
                                _ => {}
                            }
                        }

                        if let Some(target) = target_module {
                            imports.push(ImportInfo {
                                module: target,
                                names: Vec::new(),
                                is_from: false,
                                alias: alias_name,
                            });
                        }
                    }
                }
                // Recurse into module_definition body to find nested open/include statements
                // (e.g., module M = struct open List end)
                extract_ocaml_imports_recursive(&child, source, imports);
            }
            "include_module" => {
                // Structure: include_module -> "include" + module_path -> module_name
                if let Some(module) = extract_ocaml_module_path(&child, source) {
                    imports.push(ImportInfo {
                        module,
                        names: vec!["*".to_string()],
                        is_from: true,
                        alias: None,
                    });
                }
            }
            _ => {
                // Recurse into other nodes
                extract_ocaml_imports_recursive(&child, source, imports);
            }
        }
    }
}

/// Extract module path from an OCaml node that contains a module_path child.
/// Returns the dot-separated module path (e.g., "Stdlib.Map").
fn extract_ocaml_module_path(node: &Node, source: &str) -> Option<String> {
    let mut node_cursor = node.walk();
    for child in node.children(&mut node_cursor) {
        if child.kind() == "module_path" {
            return Some(extract_ocaml_module_path_text(&child, source));
        }
    }
    None
}

/// Extract text from a module_path node, joining nested module_name children with dots.
fn extract_ocaml_module_path_text(node: &Node, source: &str) -> String {
    let mut parts = Vec::new();
    let mut path_cursor = node.walk();
    for child in node.children(&mut path_cursor) {
        if child.kind() == "module_name" {
            parts.push(get_node_text(&child, source));
        } else if child.kind() == "module_path" {
            // Nested module_path for dotted names
            parts.push(extract_ocaml_module_path_text(&child, source));
        }
    }
    if parts.is_empty() {
        // Fallback: use the entire node text
        get_node_text(node, source)
    } else {
        parts.join(".")
    }
}

// =============================================================================
// Lua imports (require calls)
// =============================================================================

/// Extract Lua imports from `require()` calls.
///
/// Lua patterns:
/// - `local socket = require("socket")`     -- standard with parentheses
/// - `local dict = require"socket.dict"`    -- no parentheses, direct string
/// - `local mime = require "mime"`          -- space before string
/// - `require("module")`                    -- bare require without local
fn extract_lua_imports(node: &Node, source: &str) -> Vec<ImportInfo> {
    let mut imports = Vec::new();
    extract_lua_imports_recursive(node, source, &mut imports);
    imports
}

fn extract_lua_imports_recursive(node: &Node, source: &str, imports: &mut Vec<ImportInfo>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        // Look for function_call nodes where the function is "require"
        if child.kind() == "function_call" {
            if let Some(import) = extract_lua_require(&child, source) {
                imports.push(import);
                continue;
            }
        }

        // Also check variable_declaration / assignment_statement that may contain require
        // The require call could be nested inside these
        extract_lua_imports_recursive(&child, source, imports);
    }
}

/// Extract a single require() import from a Lua function_call node.
fn extract_lua_require(node: &Node, source: &str) -> Option<ImportInfo> {
    // Structure varies by tree-sitter-lua grammar:
    // function_call -> name: identifier("require") + arguments: (string | arguments(string))
    // OR
    // function_call -> prefix: identifier("require") + arguments(string)

    let mut is_require = false;
    let mut module_name = String::new();

    let mut call_cursor = node.walk();
    for child in node.children(&mut call_cursor) {
        match child.kind() {
            // The function name (could be in "name" field or as first identifier child)
            "identifier" => {
                let text = get_node_text(&child, source);
                if text == "require" {
                    is_require = true;
                }
            }
            // Arguments with parentheses: require("socket")
            "arguments" => {
                if is_require {
                    module_name = extract_string_from_arguments(&child, source);
                }
            }
            // Direct string argument without parens: require"socket.dict" or require "mime"
            "string" => {
                if is_require {
                    module_name = get_string_content(&child, source);
                }
            }
            _ => {}
        }
    }

    if is_require && !module_name.is_empty() {
        Some(ImportInfo {
            module: module_name,
            names: Vec::new(),
            is_from: false,
            alias: None,
        })
    } else {
        None
    }
}

/// Extract a string value from an arguments node.
/// Handles both `(arguments (string "value"))` and nested patterns.
fn extract_string_from_arguments(node: &Node, source: &str) -> String {
    let mut arg_cursor = node.walk();
    for child in node.children(&mut arg_cursor) {
        if child.kind() == "string" {
            return get_string_content(&child, source);
        }
    }
    String::new()
}

// =============================================================================
// PHP imports (use statements, require/include)
// =============================================================================

fn extract_php_imports(node: &Node, source: &str) -> Vec<ImportInfo> {
    let mut imports = Vec::new();
    extract_php_imports_recursive(node, source, &mut imports);
    imports
}

fn extract_php_imports_recursive(node: &Node, source: &str, imports: &mut Vec<ImportInfo>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            // PHP use statements: use App\Models\User;
            "namespace_use_declaration" => {
                extract_php_use_declaration(&child, source, imports);
            }
            // PHP require/include expressions
            "expression_statement" => {
                // Check if this contains a require/include
                let mut expr_cursor = child.walk();
                for expr_child in child.children(&mut expr_cursor) {
                    match expr_child.kind() {
                        "require_expression"
                        | "require_once_expression"
                        | "include_expression"
                        | "include_once_expression" => {
                            if let Some(import_info) =
                                extract_php_require_include(&expr_child, source)
                            {
                                imports.push(import_info);
                            }
                        }
                        _ => {}
                    }
                }
            }
            // Direct require/include at statement level
            "require_expression"
            | "require_once_expression"
            | "include_expression"
            | "include_once_expression" => {
                if let Some(import_info) = extract_php_require_include(&child, source) {
                    imports.push(import_info);
                }
            }
            _ => {
                // Recurse into other nodes
                extract_php_imports_recursive(&child, source, imports);
            }
        }
    }
}

/// Extract PHP use declarations
/// Handles:
/// - Simple: use App\Models\User;
/// - Grouped: use App\Models\{User, Post};
/// - Aliased: use App\Models\User as UserModel;
/// - Function use: use function App\helper;
/// - Const use: use const App\CONSTANT;
fn extract_php_use_declaration(node: &Node, source: &str, imports: &mut Vec<ImportInfo>) {
    let mut use_cursor = node.walk();

    // Check if this is a grouped import by looking for namespace_use_group
    let has_group = node
        .children(&mut use_cursor)
        .any(|c| c.kind() == "namespace_use_group");

    if has_group {
        // Grouped imports: use App\Models\{User, Post};
        let mut prefix = String::new();
        let mut group_cursor = node.walk();

        for use_child in node.children(&mut group_cursor) {
            match use_child.kind() {
                "namespace_name" | "qualified_name" | "name" => {
                    // This is the base namespace prefix
                    prefix = get_node_text(&use_child, source);
                }
                "namespace_use_group" => {
                    // Parse each clause in the group
                    let mut group_items_cursor = use_child.walk();
                    for group_item in use_child.children(&mut group_items_cursor) {
                        if group_item.kind() == "namespace_use_clause" {
                            let clause_text = get_node_text(&group_item, source).trim().to_string();

                            // Handle alias: User as UserModel
                            let (name, alias) = parse_php_use_alias(&clause_text);

                            let full_module = if prefix.is_empty() {
                                name
                            } else {
                                format!("{}\\{}", prefix, name)
                            };

                            imports.push(ImportInfo {
                                module: full_module,
                                names: Vec::new(),
                                is_from: true, // use is similar to "from X import Y"
                                alias,
                            });
                        }
                    }
                }
                _ => {}
            }
        }
    } else {
        // Simple or aliased imports
        let mut simple_cursor = node.walk();
        for use_child in node.children(&mut simple_cursor) {
            if use_child.kind() == "namespace_use_clause" {
                let clause_text = get_node_text(&use_child, source).trim().to_string();

                // Handle alias: App\Models\User as UserModel
                let (module, alias) = parse_php_use_alias(&clause_text);

                imports.push(ImportInfo {
                    module,
                    names: Vec::new(),
                    is_from: true,
                    alias,
                });
            }
        }
    }
}

/// Parse PHP use clause for potential alias
/// Returns (module, Option<alias>)
fn parse_php_use_alias(clause: &str) -> (String, Option<String>) {
    // Check for " as " (case insensitive)
    let lower = clause.to_lowercase();
    if let Some(as_pos) = lower.find(" as ") {
        let module = clause[..as_pos].trim().to_string();
        let alias = clause[as_pos + 4..].trim().to_string();
        (module, Some(alias))
    } else {
        (clause.to_string(), None)
    }
}

/// Extract PHP require/include expressions
/// Handles:
/// - require 'config.php';
/// - require_once __DIR__ . '/file.php';
/// - include 'another.php';
fn extract_php_require_include(node: &Node, source: &str) -> Option<ImportInfo> {
    let node_type = node.kind();

    // Determine import type based on node kind
    let is_require = node_type.starts_with("require");
    let is_once = node_type.contains("_once");

    // Find the path argument
    let mut module = String::new();
    let mut arg_cursor = node.walk();

    for child in node.children(&mut arg_cursor) {
        match child.kind() {
            // String literal: 'file.php' or "file.php"
            "string" | "encapsed_string" => {
                let text = get_node_text(&child, source);
                // Strip quotes
                module = text.trim_matches(|c| c == '"' || c == '\'').to_string();
                break;
            }
            // Binary expression: __DIR__ . '/file.php'
            "binary_expression" => {
                // For complex expressions, capture the whole expression
                module = get_node_text(&child, source);
                break;
            }
            // Parenthesized expression: require('file.php')
            "parenthesized_expression" => {
                // Look inside for string
                let mut paren_cursor = child.walk();
                for paren_child in child.children(&mut paren_cursor) {
                    if paren_child.kind() == "string" || paren_child.kind() == "encapsed_string" {
                        let text = get_node_text(&paren_child, source);
                        module = text.trim_matches(|c| c == '"' || c == '\'').to_string();
                        break;
                    }
                }
                if module.is_empty() {
                    // Fallback to whole expression
                    module = get_node_text(&child, source);
                }
                break;
            }
            _ => {}
        }
    }

    if module.is_empty() {
        // Last resort: extract from the whole node text
        let full_text = get_node_text(node, source);
        // Try to extract path from require 'path' or require('path')
        for pattern in ["require_once", "require", "include_once", "include"] {
            if let Some(pos) = full_text.find(pattern) {
                let rest = full_text[pos + pattern.len()..].trim();
                // Remove parentheses and quotes
                let cleaned = rest
                    .trim_start_matches(['(', ' '])
                    .trim_end_matches([')', ';', ' '])
                    .trim_matches(['"', '\'']);
                if !cleaned.is_empty() {
                    module = cleaned.to_string();
                    break;
                }
            }
        }
    }

    if module.is_empty() {
        return None;
    }

    Some(ImportInfo {
        module,
        names: Vec::new(),
        // Use is_from to distinguish require vs include
        // is_from = true for require (must exist), false for include (optional)
        is_from: is_require,
        // Use alias to track _once variants - store "once" if applicable
        alias: if is_once {
            Some("once".to_string())
        } else {
            None
        },
    })
}

// =============================================================================
// Helper functions
// =============================================================================

/// Get text content of a node
fn get_node_text(node: &Node, source: &str) -> String {
    source[node.byte_range()].to_string()
}

/// Get string content (strips quotes)
fn get_string_content(node: &Node, source: &str) -> String {
    let text = get_node_text(node, source);
    text.trim_matches(|c| c == '"' || c == '\'' || c == '`')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::parser::parse;

    #[test]
    fn test_c_include_system() {
        let source = "#include <stdio.h>";
        let tree = parse(source, Language::C).unwrap();
        let imports = extract_imports_from_tree(&tree, source, Language::C).unwrap();

        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].module, "stdio.h");
        assert!(
            imports[0].is_from,
            "System headers should have is_from=true"
        );
    }

    #[test]
    fn test_c_include_local() {
        let source = r#"#include "local.h""#;
        let tree = parse(source, Language::C).unwrap();
        let imports = extract_imports_from_tree(&tree, source, Language::C).unwrap();

        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].module, "local.h");
        assert!(
            !imports[0].is_from,
            "Local headers should have is_from=false"
        );
    }

    #[test]
    fn test_c_multiple_includes() {
        let source = r#"
#include <stdio.h>
#include <stdlib.h>
#include "myheader.h"
"#;
        let tree = parse(source, Language::C).unwrap();
        let imports = extract_imports_from_tree(&tree, source, Language::C).unwrap();

        assert_eq!(imports.len(), 3);
        let modules: Vec<&str> = imports.iter().map(|i| i.module.as_str()).collect();
        assert!(modules.contains(&"stdio.h"));
        assert!(modules.contains(&"stdlib.h"));
        assert!(modules.contains(&"myheader.h"));
    }

    #[test]
    fn test_cpp_includes() {
        let source = r#"
#include <iostream>
#include <string>
#include "local.hpp"
"#;
        let tree = parse(source, Language::Cpp).unwrap();
        let imports = extract_imports_from_tree(&tree, source, Language::Cpp).unwrap();

        assert_eq!(imports.len(), 3);
        let modules: Vec<&str> = imports.iter().map(|i| i.module.as_str()).collect();
        assert!(modules.contains(&"iostream"));
        assert!(modules.contains(&"string"));
        assert!(modules.contains(&"local.hpp"));
    }

    #[test]
    fn test_python_import() {
        let source = "import os";
        let tree = parse(source, Language::Python).unwrap();
        let imports = extract_imports_from_tree(&tree, source, Language::Python).unwrap();

        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].module, "os");
        assert!(!imports[0].is_from);
    }

    #[test]
    fn test_python_from_import() {
        let source = "from typing import List, Optional";
        let tree = parse(source, Language::Python).unwrap();
        let imports = extract_imports_from_tree(&tree, source, Language::Python).unwrap();

        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].module, "typing");
        assert!(imports[0].is_from);
        assert!(imports[0].names.contains(&"List".to_string()));
        assert!(imports[0].names.contains(&"Optional".to_string()));
    }

    #[test]
    fn test_typescript_import() {
        let source = "import { foo, bar } from './module';";
        let tree = parse(source, Language::TypeScript).unwrap();
        let imports = extract_imports_from_tree(&tree, source, Language::TypeScript).unwrap();

        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].module, "./module");
        assert!(imports[0].names.contains(&"foo".to_string()));
    }

    #[test]
    fn test_go_import() {
        let source = r#"
package main

import "fmt"
"#;
        let tree = parse(source, Language::Go).unwrap();
        let imports = extract_imports_from_tree(&tree, source, Language::Go).unwrap();

        assert!(!imports.is_empty());
        assert!(imports.iter().any(|i| i.module == "fmt"));
    }

    #[test]
    fn test_rust_use() {
        let source = "use std::collections::HashMap;";
        let tree = parse(source, Language::Rust).unwrap();
        let imports = extract_imports_from_tree(&tree, source, Language::Rust).unwrap();

        assert_eq!(imports.len(), 1);
        assert!(imports[0].module.contains("std::collections"));
    }

    #[test]
    fn test_ruby_require_gem() {
        let source = "require 'json'";
        let tree = parse(source, Language::Ruby).unwrap();
        let imports = extract_imports_from_tree(&tree, source, Language::Ruby).unwrap();

        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].module, "json");
        assert!(
            !imports[0].is_from,
            "External gem require should have is_from=false"
        );
    }

    #[test]
    fn test_ruby_require_relative() {
        let source = "require_relative './helper'";
        let tree = parse(source, Language::Ruby).unwrap();
        let imports = extract_imports_from_tree(&tree, source, Language::Ruby).unwrap();

        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].module, "./helper");
        assert!(
            imports[0].is_from,
            "require_relative should have is_from=true"
        );
    }

    #[test]
    fn test_ruby_require_explicit_relative() {
        let source = "require './lib/util'";
        let tree = parse(source, Language::Ruby).unwrap();
        let imports = extract_imports_from_tree(&tree, source, Language::Ruby).unwrap();

        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].module, "./lib/util");
        assert!(
            imports[0].is_from,
            "Explicit relative require should have is_from=true"
        );
    }

    #[test]
    fn test_ruby_multiple_requires() {
        let source = r##"
require 'json'
require 'net/http'
require_relative './local_module'
"##;
        let tree = parse(source, Language::Ruby).unwrap();
        let imports = extract_imports_from_tree(&tree, source, Language::Ruby).unwrap();

        assert_eq!(imports.len(), 3);
        let modules: Vec<&str> = imports.iter().map(|i| i.module.as_str()).collect();
        assert!(modules.contains(&"json"));
        assert!(modules.contains(&"net/http"));
        assert!(modules.contains(&"./local_module"));
    }

    // =========================================================================
    // Elixir import tests
    // =========================================================================

    #[test]
    fn test_elixir_import() {
        let source = "import Phoenix.Controller";
        let tree = parse(source, Language::Elixir).unwrap();
        let imports = extract_imports_from_tree(&tree, source, Language::Elixir).unwrap();

        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].module, "Phoenix.Controller");
        assert!(
            imports[0].is_from,
            "import should have is_from=true (imports all functions)"
        );
    }

    #[test]
    fn test_elixir_alias_simple() {
        let source = "alias Phoenix.LiveView";
        let tree = parse(source, Language::Elixir).unwrap();
        let imports = extract_imports_from_tree(&tree, source, Language::Elixir).unwrap();

        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].module, "Phoenix.LiveView");
        // Simple alias uses last segment as short name
        assert_eq!(imports[0].alias, Some("LiveView".to_string()));
    }

    #[test]
    fn test_elixir_alias_with_as() {
        let source = "alias Phoenix.LiveView, as: LV";
        let tree = parse(source, Language::Elixir).unwrap();
        let imports = extract_imports_from_tree(&tree, source, Language::Elixir).unwrap();

        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].module, "Phoenix.LiveView");
        assert_eq!(imports[0].alias, Some("LV".to_string()));
    }

    #[test]
    fn test_elixir_require() {
        let source = "require Logger";
        let tree = parse(source, Language::Elixir).unwrap();
        let imports = extract_imports_from_tree(&tree, source, Language::Elixir).unwrap();

        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].module, "Logger");
    }

    #[test]
    fn test_elixir_use() {
        let source = "use GenServer";
        let tree = parse(source, Language::Elixir).unwrap();
        let imports = extract_imports_from_tree(&tree, source, Language::Elixir).unwrap();

        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].module, "GenServer");
        assert!(
            imports[0].is_from,
            "use should have is_from=true (imports macros)"
        );
    }

    #[test]
    fn test_elixir_multiple_imports() {
        let source = r#"import Phoenix.Controller
alias Phoenix.LiveView, as: LV
require Logger
use GenServer
"#;
        let tree = parse(source, Language::Elixir).unwrap();
        let imports = extract_imports_from_tree(&tree, source, Language::Elixir).unwrap();

        assert_eq!(imports.len(), 4);
        let modules: Vec<&str> = imports.iter().map(|i| i.module.as_str()).collect();
        assert!(modules.contains(&"Phoenix.Controller"));
        assert!(modules.contains(&"Phoenix.LiveView"));
        assert!(modules.contains(&"Logger"));
        assert!(modules.contains(&"GenServer"));
    }

    #[test]
    fn test_elixir_imports_inside_defmodule() {
        let source = r#"defmodule MyApp.Router do
  alias Phoenix.Socket
  import Plug.Conn
  use Phoenix.Router
  require Logger
end"#;
        let tree = parse(source, Language::Elixir).unwrap();
        let imports = extract_imports_from_tree(&tree, source, Language::Elixir).unwrap();

        assert_eq!(
            imports.len(),
            4,
            "Should find all 4 imports inside defmodule"
        );
        let modules: Vec<&str> = imports.iter().map(|i| i.module.as_str()).collect();
        assert!(
            modules.contains(&"Phoenix.Socket"),
            "Should find alias Phoenix.Socket"
        );
        assert!(
            modules.contains(&"Plug.Conn"),
            "Should find import Plug.Conn"
        );
        assert!(
            modules.contains(&"Phoenix.Router"),
            "Should find use Phoenix.Router"
        );
        assert!(modules.contains(&"Logger"), "Should find require Logger");
    }

    // =========================================================================
    // OCaml import tests
    // =========================================================================

    #[test]
    fn test_ocaml_open() {
        let source = "open List";
        let tree = parse(source, Language::Ocaml).unwrap();
        let imports = extract_imports_from_tree(&tree, source, Language::Ocaml).unwrap();

        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].module, "List");
        assert!(
            imports[0].is_from,
            "open should have is_from=true (like import *)"
        );
    }

    #[test]
    fn test_ocaml_module_alias() {
        let source = "module M = Hashtbl";
        let tree = parse(source, Language::Ocaml).unwrap();
        let imports = extract_imports_from_tree(&tree, source, Language::Ocaml).unwrap();

        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].module, "Hashtbl");
        assert_eq!(imports[0].alias, Some("M".to_string()));
    }

    #[test]
    fn test_ocaml_include() {
        let source = "include Set";
        let tree = parse(source, Language::Ocaml).unwrap();
        let imports = extract_imports_from_tree(&tree, source, Language::Ocaml).unwrap();

        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].module, "Set");
        assert!(imports[0].is_from, "include should have is_from=true");
    }

    #[test]
    fn test_ocaml_multiple_imports() {
        let source = r#"open List
module M = Hashtbl
include Set
"#;
        let tree = parse(source, Language::Ocaml).unwrap();
        let imports = extract_imports_from_tree(&tree, source, Language::Ocaml).unwrap();

        assert_eq!(imports.len(), 3);
        let modules: Vec<&str> = imports.iter().map(|i| i.module.as_str()).collect();
        assert!(modules.contains(&"List"));
        assert!(modules.contains(&"Hashtbl"));
        assert!(modules.contains(&"Set"));
    }

    #[test]
    fn test_ocaml_nested_module() {
        let source = "open Stdlib.Map";
        let tree = parse(source, Language::Ocaml).unwrap();
        let imports = extract_imports_from_tree(&tree, source, Language::Ocaml).unwrap();

        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].module, "Stdlib.Map");
    }

    #[test]
    fn test_ocaml_open_inside_module() {
        let source = r#"module M = struct
  open List
  open Hashtbl
end"#;
        let tree = parse(source, Language::Ocaml).unwrap();
        let imports = extract_imports_from_tree(&tree, source, Language::Ocaml).unwrap();

        assert_eq!(
            imports.len(),
            2,
            "Should find 2 open statements inside module struct"
        );
        let modules: Vec<&str> = imports.iter().map(|i| i.module.as_str()).collect();
        assert!(modules.contains(&"List"), "Should find open List");
        assert!(modules.contains(&"Hashtbl"), "Should find open Hashtbl");
    }

    // =========================================================================
    // PHP import tests
    // =========================================================================

    #[test]
    fn test_php_use_simple() {
        let source = "<?php\nuse App\\Models\\User;";
        let tree = parse(source, Language::Php).unwrap();
        let imports = extract_imports_from_tree(&tree, source, Language::Php).unwrap();

        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].module, "App\\Models\\User");
    }

    #[test]
    fn test_php_use_alias() {
        let source = "<?php\nuse App\\Models\\User as UserModel;";
        let tree = parse(source, Language::Php).unwrap();
        let imports = extract_imports_from_tree(&tree, source, Language::Php).unwrap();

        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].module, "App\\Models\\User");
        assert_eq!(imports[0].alias, Some("UserModel".to_string()));
    }

    #[test]
    fn test_php_require() {
        let source = "<?php\nrequire 'config.php';";
        let tree = parse(source, Language::Php).unwrap();
        let imports = extract_imports_from_tree(&tree, source, Language::Php).unwrap();

        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].module, "config.php");
        assert!(imports[0].is_from, "require should have is_from=true");
    }

    #[test]
    fn test_php_require_once() {
        let source = "<?php\nrequire_once 'autoload.php';";
        let tree = parse(source, Language::Php).unwrap();
        let imports = extract_imports_from_tree(&tree, source, Language::Php).unwrap();

        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].module, "autoload.php");
        assert_eq!(
            imports[0].alias,
            Some("once".to_string()),
            "require_once should have alias='once'"
        );
    }

    #[test]
    fn test_php_multiple_imports() {
        let source = r#"<?php
use App\Models\User;
use App\Models\Post as BlogPost;
require_once 'vendor/autoload.php';
"#;
        let tree = parse(source, Language::Php).unwrap();
        let imports = extract_imports_from_tree(&tree, source, Language::Php).unwrap();

        assert!(
            imports.len() >= 3,
            "Expected at least 3 imports, got {}",
            imports.len()
        );
    }

    // =========================================================================
    // Lua imports
    // =========================================================================

    /// Test: Lua standard require with parentheses
    /// `local socket = require("socket")`
    #[test]
    fn test_lua_require_standard() {
        let source = r#"local socket = require("socket")"#;
        let tree = parse(source, Language::Lua).unwrap();
        let imports = extract_imports_from_tree(&tree, source, Language::Lua).unwrap();

        assert_eq!(imports.len(), 1, "Expected 1 import, got {}", imports.len());
        assert_eq!(imports[0].module, "socket");
    }

    /// Test: Lua require without parentheses
    /// `local dict = require"socket.dict"`
    #[test]
    fn test_lua_require_no_parens() {
        let source = r#"local dict = require"socket.dict""#;
        let tree = parse(source, Language::Lua).unwrap();
        let imports = extract_imports_from_tree(&tree, source, Language::Lua).unwrap();

        assert_eq!(imports.len(), 1, "Expected 1 import, got {}", imports.len());
        assert_eq!(imports[0].module, "socket.dict");
    }

    /// Test: Lua require with space before string (no parens)
    /// `local mime = require "mime"`
    #[test]
    fn test_lua_require_space_string() {
        let source = r#"local mime = require "mime""#;
        let tree = parse(source, Language::Lua).unwrap();
        let imports = extract_imports_from_tree(&tree, source, Language::Lua).unwrap();

        assert_eq!(imports.len(), 1, "Expected 1 import, got {}", imports.len());
        assert_eq!(imports[0].module, "mime");
    }

    /// Test: Lua local require with nested module path
    /// `local http = require("socket.http")`
    #[test]
    fn test_lua_require_local() {
        let source = r#"local http = require("socket.http")"#;
        let tree = parse(source, Language::Lua).unwrap();
        let imports = extract_imports_from_tree(&tree, source, Language::Lua).unwrap();

        assert_eq!(imports.len(), 1, "Expected 1 import, got {}", imports.len());
        assert_eq!(imports[0].module, "socket.http");
    }

    /// Test: Multiple Lua requires in a file
    #[test]
    fn test_lua_multiple_requires() {
        let source = r#"
local socket = require("socket")
local url = require("socket.url")
local ltn12 = require("ltn12")
local mime = require("mime")
"#;
        let tree = parse(source, Language::Lua).unwrap();
        let imports = extract_imports_from_tree(&tree, source, Language::Lua).unwrap();

        assert!(
            imports.len() >= 4,
            "Expected at least 4 imports, got {}",
            imports.len()
        );
        let modules: Vec<&str> = imports.iter().map(|i| i.module.as_str()).collect();
        assert!(modules.contains(&"socket"), "Missing 'socket' import");
        assert!(
            modules.contains(&"socket.url"),
            "Missing 'socket.url' import"
        );
        assert!(modules.contains(&"ltn12"), "Missing 'ltn12' import");
        assert!(modules.contains(&"mime"), "Missing 'mime' import");
    }
}
