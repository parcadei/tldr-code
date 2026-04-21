//! Rust language handler for call graph analysis.
//!
//! This module provides Rust call graph support using tree-sitter-rust.
//!
//! # Import Patterns Supported
//!
//! | Pattern | ImportDef |
//! |---------|-----------|
//! | `use crate::mod::item` | `{module: "crate::mod", names: ["item"]}` |
//! | `use self::item` | `{module: "self", names: ["item"]}` |
//! | `use super::item` | `{module: "super", names: ["item"]}` |
//! | `use mod::{a, b}` | `{module: "mod", names: ["a", "b"]}` |
//! | `use mod::*` | `{module: "mod", names: ["*"]}` |
//! | `use mod::item as alias` | `{module: "mod", names: ["item"], aliases: {"alias": "item"}}` |
//! | `mod foo;` | `{module: "foo", is_mod: true}` |
//! | `extern crate foo;` | `{module: "foo"}` |
//!
//! # Call Extraction
//!
//! - Direct calls: `func()` -> CallType::Direct or CallType::Intra
//! - Method calls: `obj.method()` -> CallType::Attr
//! - Static/associated calls: `Type::method()` -> CallType::Attr
//! - Trait method calls: Resolved via trait bounds (limited support)
//! - Macro calls: Limited support (name extraction only)
//!
//! # Spec Reference
//!
//! See `migration/spec/callgraph-spec.md` Section 9.2 for Rust-specific details.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use regex::Regex;
use tree_sitter::{Node, Parser, Tree};

use super::base::{get_node_text, walk_tree};
use super::common::{extend_calls_if_any, insert_calls_if_any};
use super::{CallGraphLanguageSupport, ParseError};
use crate::callgraph::cross_file_types::{CallSite, CallType, ClassDef, FuncDef, ImportDef};

// =============================================================================
// Rust Handler
// =============================================================================

/// Rust language handler using tree-sitter-rust.
///
/// Supports:
/// - Use declaration parsing (simple, grouped, glob)
/// - Mod declarations
/// - Call extraction (direct, method, associated functions)
/// - Struct/enum/trait definitions
#[derive(Debug, Default)]
pub struct RustLangHandler;

impl RustLangHandler {
    /// Creates a new RustLangHandler.
    pub fn new() -> Self {
        Self
    }

    /// Parse the source code into a tree-sitter Tree.
    fn parse_source(&self, source: &str) -> Result<Tree, ParseError> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .map_err(|e| ParseError::ParseFailed {
                file: std::path::PathBuf::new(),
                message: format!("Failed to set Rust language: {}", e),
            })?;

        parser
            .parse(source, None)
            .ok_or_else(|| ParseError::ParseFailed {
                file: std::path::PathBuf::new(),
                message: "Parser returned None".to_string(),
            })
    }

    /// Parse a use declaration node.
    fn parse_use_declaration(&self, node: &Node, source: &[u8]) -> Vec<ImportDef> {
        let mut imports = Vec::new();

        // Get the full use path text
        let text = get_node_text(node, source);

        // Strip "use " prefix, "pub use " prefix, and trailing semicolon
        let text = text
            .strip_prefix("pub use ")
            .or_else(|| text.strip_prefix("use "))
            .unwrap_or(text)
            .trim_end_matches(';')
            .trim();

        // Parse the use tree
        self.parse_use_tree(text, &mut imports);

        imports
    }

    /// Parse a use tree (handles grouped imports, glob, aliases).
    fn parse_use_tree(&self, text: &str, imports: &mut Vec<ImportDef>) {
        // Handle grouped imports: use mod::{a, b, c}
        if let Some(brace_start) = text.find('{') {
            let module = text[..brace_start].trim_end_matches("::").to_string();
            let brace_end = text.rfind('}').unwrap_or(text.len());
            let items = &text[brace_start + 1..brace_end];

            // Split by comma, handling nested groups
            let mut names = Vec::new();
            let mut aliases = HashMap::new();
            let mut depth = 0;
            let mut current = String::new();

            for c in items.chars() {
                match c {
                    '{' => {
                        depth += 1;
                        current.push(c);
                    }
                    '}' => {
                        depth -= 1;
                        current.push(c);
                    }
                    ',' if depth == 0 => {
                        self.parse_single_import(current.trim(), &mut names, &mut aliases);
                        current.clear();
                    }
                    _ => {
                        current.push(c);
                    }
                }
            }
            if !current.trim().is_empty() {
                self.parse_single_import(current.trim(), &mut names, &mut aliases);
            }

            let mut imp = ImportDef::from_import(module, names);
            if !aliases.is_empty() {
                imp.aliases = Some(aliases);
            }
            imports.push(imp);
        } else if text.ends_with("::*") {
            // Glob import: use mod::*
            let module = text.trim_end_matches("::*").to_string();
            imports.push(ImportDef::wildcard_import(module));
        } else if text.contains("::") {
            // Simple path import: use mod::item
            let parts: Vec<&str> = text.rsplitn(2, "::").collect();
            if parts.len() == 2 {
                let item = parts[0].trim();
                let module = parts[1].trim().to_string();

                // Check for alias: use mod::item as alias
                if item.contains(" as ") {
                    let alias_parts: Vec<&str> = item.splitn(2, " as ").collect();
                    let name = alias_parts[0].trim().to_string();
                    let alias = alias_parts[1].trim().to_string();

                    let mut imp = ImportDef::from_import(module, vec![name.clone()]);
                    let mut aliases = HashMap::new();
                    aliases.insert(alias, name);
                    imp.aliases = Some(aliases);
                    imports.push(imp);
                } else {
                    imports.push(ImportDef::from_import(module, vec![item.to_string()]));
                }
            } else {
                // Just a module path without trailing item
                imports.push(ImportDef::simple_import(text.to_string()));
            }
        } else {
            // Simple import: use item
            imports.push(ImportDef::simple_import(text.to_string()));
        }
    }

    /// Parse a single import item (possibly with alias).
    fn parse_single_import(
        &self,
        item: &str,
        names: &mut Vec<String>,
        aliases: &mut HashMap<String, String>,
    ) {
        let item = item.trim();
        if item.is_empty() {
            return;
        }

        if item.contains(" as ") {
            let parts: Vec<&str> = item.splitn(2, " as ").collect();
            let name = parts[0].trim().to_string();
            let alias = parts[1].trim().to_string();
            names.push(name.clone());
            aliases.insert(alias, name);
        } else if item == "self" {
            names.push("self".to_string());
        } else if item == "*" {
            names.push("*".to_string());
        } else {
            names.push(item.to_string());
        }
    }

    /// Parse a mod declaration node.
    fn parse_mod_declaration(&self, node: &Node, source: &[u8]) -> Option<ImportDef> {
        // Check if it's a mod declaration (not an inline module)
        let mut has_body = false;
        let mut name = None;

        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                match child.kind() {
                    "identifier" => {
                        name = Some(get_node_text(&child, source).to_string());
                    }
                    "declaration_list" => {
                        has_body = true;
                    }
                    _ => {}
                }
            }
        }

        // Only return for mod declarations without body (mod foo;)
        if !has_body {
            if let Some(n) = name {
                let mut imp = ImportDef::simple_import(n);
                imp.is_mod = true;
                return Some(imp);
            }
        }

        None
    }

    /// Collect all function, struct, enum, and trait definitions.
    fn collect_definitions(
        &self,
        tree: &Tree,
        source: &[u8],
    ) -> (HashSet<String>, HashSet<String>) {
        let mut functions = HashSet::new();
        let mut types = HashSet::new();

        for node in walk_tree(tree.root_node()) {
            match node.kind() {
                "function_item" => {
                    if let Some(name_node) = node.child_by_field_name("name") {
                        functions.insert(get_node_text(&name_node, source).to_string());
                    }
                }
                "struct_item" => {
                    if let Some(name_node) = node.child_by_field_name("name") {
                        let name = get_node_text(&name_node, source).to_string();
                        types.insert(name.clone());
                        functions.insert(name);
                    }
                }
                "enum_item" => {
                    if let Some(name_node) = node.child_by_field_name("name") {
                        let name = get_node_text(&name_node, source).to_string();
                        types.insert(name.clone());
                        functions.insert(name);
                    }
                }
                "trait_item" => {
                    if let Some(name_node) = node.child_by_field_name("name") {
                        let name = get_node_text(&name_node, source).to_string();
                        types.insert(name.clone());
                        functions.insert(name);
                    }
                }
                "impl_item" => {
                    // Collect methods from impl blocks
                    let mut type_name: Option<String> = None;

                    for i in 0..node.child_count() {
                        if let Some(child) = node.child(i) {
                            match child.kind() {
                                "type_identifier" | "generic_type" => {
                                    // Get the base type name
                                    if child.kind() == "generic_type" {
                                        for j in 0..child.child_count() {
                                            if let Some(tc) = child.child(j) {
                                                if tc.kind() == "type_identifier" {
                                                    type_name = Some(
                                                        get_node_text(&tc, source).to_string(),
                                                    );
                                                    break;
                                                }
                                            }
                                        }
                                    } else {
                                        type_name = Some(get_node_text(&child, source).to_string());
                                    }
                                }
                                "declaration_list" => {
                                    // Index methods
                                    for j in 0..child.named_child_count() {
                                        if let Some(item) = child.named_child(j) {
                                            if item.kind() == "function_item" {
                                                if let Some(name_node) =
                                                    item.child_by_field_name("name")
                                                {
                                                    let method_name =
                                                        get_node_text(&name_node, source)
                                                            .to_string();
                                                    functions.insert(method_name.clone());

                                                    // Also add as Type::method
                                                    if let Some(ref tn) = type_name {
                                                        functions.insert(format!(
                                                            "{}::{}",
                                                            tn, method_name
                                                        ));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        (functions, types)
    }

    /// Heuristically extract function calls from macro invocation token trees.
    ///
    /// Since tree-sitter treats macro bodies as opaque token_tree nodes,
    /// we use regex to find patterns that look like function calls:
    /// `identifier(` or `path::identifier(`.
    ///
    /// We are conservative: only extract names that look like real function calls,
    /// excluding Rust keywords and common false positives.
    fn extract_calls_from_macro_body(
        &self,
        text: &str,
        defined_funcs: &HashSet<String>,
        caller: &str,
        line: u32,
    ) -> Vec<CallSite> {
        // Keywords that should never be treated as function calls
        static KEYWORDS: &[&str] = &[
            "if", "else", "for", "while", "loop", "match", "fn", "let", "mut", "ref", "type",
            "pub", "use", "mod", "impl", "trait", "struct", "enum", "where", "as", "in", "return",
            "break", "continue", "move", "async", "await", "unsafe", "extern", "crate", "self",
            "super", "Self", "true", "false", "const", "static", "dyn", "box", "yield",
        ];

        let keyword_set: HashSet<&str> = KEYWORDS.iter().copied().collect();

        let mut calls = Vec::new();

        // Match patterns: `identifier(` or `path::identifier(`
        // The regex captures: (optional_path::)identifier followed by (
        let re = Regex::new(r"([a-zA-Z_][\w]*(?:::[a-zA-Z_][\w]*)*)\s*\(").unwrap();

        for cap in re.captures_iter(text) {
            let full_match = cap.get(1).unwrap().as_str();

            // Extract the bare function name (last segment for path calls)
            let bare_name = full_match.rsplit("::").next().unwrap_or(full_match);

            // Skip keywords
            if keyword_set.contains(bare_name) {
                continue;
            }

            // Skip if it looks like a macro name (contains !)
            // (shouldn't happen with our regex, but be safe)
            if full_match.contains('!') {
                continue;
            }

            // Determine call type
            let call_type = if defined_funcs.contains(full_match) {
                CallType::Intra
            } else if full_match.contains("::") {
                CallType::Attr
            } else {
                CallType::Direct
            };

            let receiver = if full_match.contains("::") {
                full_match.rsplitn(2, "::").last().map(|s| s.to_string())
            } else {
                None
            };

            calls.push(CallSite::new(
                caller.to_string(),
                full_match.to_string(),
                call_type,
                Some(line),
                None,
                receiver,
                None,
            ));
        }

        calls
    }

    /// Find the enclosing function for a given node.
    ///
    /// Walks up the AST parent chain to find the nearest enclosing function_item.
    /// If inside an impl block, returns "Type::method". If inside a trait default
    /// method, returns "Trait::method". If at module level, returns "<module>".
    fn find_enclosing_function(&self, node: &Node, source: &[u8]) -> String {
        let mut current = node.parent();
        while let Some(parent) = current {
            if parent.kind() == "function_item" {
                if let Some(name_node) = parent.child_by_field_name("name") {
                    let func_name = get_node_text(&name_node, source).to_string();

                    // Check if this function is inside an impl or trait block
                    if let Some(decl_list) = parent.parent() {
                        if decl_list.kind() == "declaration_list" {
                            if let Some(container) = decl_list.parent() {
                                match container.kind() {
                                    "impl_item" => {
                                        // Find the type name
                                        for i in 0..container.child_count() {
                                            if let Some(child) = container.child(i) {
                                                match child.kind() {
                                                    "type_identifier" => {
                                                        return format!(
                                                            "{}::{}",
                                                            get_node_text(&child, source),
                                                            func_name
                                                        );
                                                    }
                                                    "generic_type" => {
                                                        for j in 0..child.child_count() {
                                                            if let Some(tc) = child.child(j) {
                                                                if tc.kind() == "type_identifier" {
                                                                    return format!(
                                                                        "{}::{}",
                                                                        get_node_text(&tc, source),
                                                                        func_name
                                                                    );
                                                                }
                                                            }
                                                        }
                                                    }
                                                    _ => {}
                                                }
                                            }
                                        }
                                    }
                                    "trait_item" => {
                                        // Find the trait name
                                        for i in 0..container.child_count() {
                                            if let Some(child) = container.child(i) {
                                                if child.kind() == "type_identifier" {
                                                    return format!(
                                                        "{}::{}",
                                                        get_node_text(&child, source),
                                                        func_name
                                                    );
                                                }
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }

                    return func_name;
                }
            }
            current = parent.parent();
        }
        "<module>".to_string()
    }

    /// Extract calls from a function body.
    fn extract_calls_from_node(
        &self,
        node: &Node,
        source: &[u8],
        defined_funcs: &HashSet<String>,
        caller: &str,
    ) -> Vec<CallSite> {
        let mut calls = Vec::new();

        for child in walk_tree(*node) {
            if child.kind() == "call_expression" {
                let line = child.start_position().row as u32 + 1;

                // Get the function being called
                if let Some(func_node) = child.child(0) {
                    match func_node.kind() {
                        "identifier" => {
                            // Direct call: func()
                            let target = get_node_text(&func_node, source).to_string();
                            let call_type = if defined_funcs.contains(&target) {
                                CallType::Intra
                            } else {
                                CallType::Direct
                            };
                            calls.push(CallSite::new(
                                caller.to_string(),
                                target,
                                call_type,
                                Some(line),
                                None,
                                None,
                                None,
                            ));
                        }
                        "scoped_identifier" => {
                            // Path call: module::func() or Type::method()
                            let target = get_node_text(&func_node, source).to_string();

                            // Extract receiver (the part before ::)
                            let receiver = if target.contains("::") {
                                target.rsplitn(2, "::").last().map(|s| s.to_string())
                            } else {
                                None
                            };

                            // Check if it's a local associated function
                            let call_type = if defined_funcs.contains(&target) {
                                CallType::Intra
                            } else if receiver.is_some() {
                                CallType::Attr
                            } else {
                                CallType::Direct
                            };

                            calls.push(CallSite::new(
                                caller.to_string(),
                                target,
                                call_type,
                                Some(line),
                                None,
                                receiver,
                                None,
                            ));
                        }
                        "field_expression" => {
                            // Method call: obj.method()
                            let mut method_name: Option<String> = None;
                            let mut receiver: Option<String> = None;

                            for i in 0..func_node.child_count() {
                                if let Some(fc) = func_node.child(i) {
                                    match fc.kind() {
                                        "field_identifier" => {
                                            method_name =
                                                Some(get_node_text(&fc, source).to_string());
                                        }
                                        "identifier" => {
                                            receiver = Some(get_node_text(&fc, source).to_string());
                                        }
                                        "self" => {
                                            receiver = Some("self".to_string());
                                        }
                                        _ => {}
                                    }
                                }
                            }

                            if let Some(method) = method_name {
                                if receiver.is_none() {
                                    if let Some(value_node) = func_node.child_by_field_name("value")
                                    {
                                        let value_text =
                                            get_node_text(&value_node, source).to_string();
                                        if !value_text.is_empty() {
                                            receiver = Some(value_text);
                                        }
                                    }
                                }

                                let target = if let Some(ref recv) = receiver {
                                    format!("{}.{}", recv, method)
                                } else {
                                    method.clone()
                                };

                                let call_type = if defined_funcs.contains(&method) {
                                    CallType::Intra
                                } else if receiver.is_some() {
                                    CallType::Attr
                                } else {
                                    CallType::Direct
                                };

                                calls.push(CallSite::new(
                                    caller.to_string(),
                                    target,
                                    call_type,
                                    Some(line),
                                    None,
                                    receiver,
                                    None,
                                ));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        calls
    }

    fn process_function_item_calls(
        &self,
        node: &Node,
        source: &[u8],
        defined_funcs: &HashSet<String>,
        calls_by_func: &mut HashMap<String, Vec<CallSite>>,
    ) {
        let inside_container = node
            .parent()
            .is_some_and(|parent| parent.kind() == "declaration_list")
            && node
                .parent()
                .and_then(|parent| parent.parent())
                .is_some_and(|grand_parent| {
                    grand_parent.kind() == "impl_item" || grand_parent.kind() == "trait_item"
                });
        if inside_container {
            return;
        }

        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let func_name = get_node_text(&name_node, source).to_string();
        let Some(body) = node.child_by_field_name("body") else {
            return;
        };

        let calls = self.extract_calls_from_node(&body, source, defined_funcs, &func_name);
        insert_calls_if_any(calls_by_func, func_name, calls);
    }

    fn process_impl_item_calls(
        &self,
        node: &Node,
        source: &[u8],
        defined_funcs: &HashSet<String>,
        calls_by_func: &mut HashMap<String, Vec<CallSite>>,
    ) {
        let mut type_name: Option<String> = None;

        for i in 0..node.child_count() {
            let Some(child) = node.child(i) else {
                continue;
            };
            match child.kind() {
                "type_identifier" | "generic_type" => {
                    if child.kind() == "generic_type" {
                        for j in 0..child.child_count() {
                            if let Some(type_child) = child.child(j) {
                                if type_child.kind() == "type_identifier" {
                                    type_name =
                                        Some(get_node_text(&type_child, source).to_string());
                                    break;
                                }
                            }
                        }
                    } else {
                        type_name = Some(get_node_text(&child, source).to_string());
                    }
                }
                "declaration_list" => {
                    for j in 0..child.named_child_count() {
                        let Some(item) = child.named_child(j) else {
                            continue;
                        };
                        if item.kind() != "function_item" {
                            continue;
                        }
                        let Some(name_node) = item.child_by_field_name("name") else {
                            continue;
                        };
                        let method_name = get_node_text(&name_node, source).to_string();
                        let full_name = if let Some(type_name) = type_name.as_deref() {
                            format!("{type_name}::{method_name}")
                        } else {
                            method_name
                        };
                        let Some(body) = item.child_by_field_name("body") else {
                            continue;
                        };
                        let calls =
                            self.extract_calls_from_node(&body, source, defined_funcs, &full_name);
                        insert_calls_if_any(calls_by_func, full_name, calls);
                    }
                }
                _ => {}
            }
        }
    }

    fn process_trait_item_calls(
        &self,
        node: &Node,
        source: &[u8],
        defined_funcs: &HashSet<String>,
        calls_by_func: &mut HashMap<String, Vec<CallSite>>,
    ) {
        let mut trait_name: Option<String> = None;

        for i in 0..node.child_count() {
            let Some(child) = node.child(i) else {
                continue;
            };
            match child.kind() {
                "type_identifier" => {
                    trait_name = Some(get_node_text(&child, source).to_string());
                }
                "declaration_list" => {
                    for j in 0..child.named_child_count() {
                        let Some(item) = child.named_child(j) else {
                            continue;
                        };
                        if item.kind() != "function_item" {
                            continue;
                        }
                        let Some(name_node) = item.child_by_field_name("name") else {
                            continue;
                        };
                        let method_name = get_node_text(&name_node, source).to_string();
                        let full_name = if let Some(trait_name) = trait_name.as_deref() {
                            format!("{trait_name}::{method_name}")
                        } else {
                            method_name
                        };
                        let Some(body) = item.child_by_field_name("body") else {
                            continue;
                        };
                        let calls =
                            self.extract_calls_from_node(&body, source, defined_funcs, &full_name);
                        insert_calls_if_any(calls_by_func, full_name, calls);
                    }
                }
                _ => {}
            }
        }
    }

    fn process_const_or_static_calls(
        &self,
        node: &Node,
        source: &[u8],
        defined_funcs: &HashSet<String>,
        calls_by_func: &mut HashMap<String, Vec<CallSite>>,
    ) {
        let Some(value) = node.child_by_field_name("value") else {
            return;
        };
        let calls = self.extract_calls_from_node(&value, source, defined_funcs, "<module>");
        extend_calls_if_any(calls_by_func, "<module>".to_string(), calls);
    }

    fn process_macro_invocation_calls(
        &self,
        node: &Node,
        source: &[u8],
        defined_funcs: &HashSet<String>,
        calls_by_func: &mut HashMap<String, Vec<CallSite>>,
    ) {
        let line = node.start_position().row as u32 + 1;
        let caller = self.find_enclosing_function(node, source);

        for i in 0..node.child_count() {
            let Some(child) = node.child(i) else {
                continue;
            };
            if child.kind() != "token_tree" {
                continue;
            }

            let token_text = get_node_text(&child, source);
            let inner = if (token_text.starts_with('(') && token_text.ends_with(')'))
                || (token_text.starts_with('{') && token_text.ends_with('}'))
                || (token_text.starts_with('[') && token_text.ends_with(']'))
            {
                &token_text[1..token_text.len() - 1]
            } else {
                token_text
            };

            let macro_calls =
                self.extract_calls_from_macro_body(inner, defined_funcs, &caller, line);
            extend_calls_if_any(calls_by_func, caller.clone(), macro_calls);
            break;
        }
    }
}

impl CallGraphLanguageSupport for RustLangHandler {
    fn name(&self) -> &str {
        "rust"
    }

    fn extensions(&self) -> &[&str] {
        &[".rs"]
    }

    fn parse_imports(&self, source: &str, _path: &Path) -> Result<Vec<ImportDef>, ParseError> {
        let tree = self.parse_source(source)?;
        let source_bytes = source.as_bytes();
        let mut imports = Vec::new();

        for node in walk_tree(tree.root_node()) {
            match node.kind() {
                "use_declaration" => {
                    imports.extend(self.parse_use_declaration(&node, source_bytes));
                }
                "mod_item" => {
                    if let Some(imp) = self.parse_mod_declaration(&node, source_bytes) {
                        imports.push(imp);
                    }
                }
                "extern_crate_declaration" => {
                    // extern crate foo;
                    for i in 0..node.child_count() {
                        if let Some(child) = node.child(i) {
                            if child.kind() == "identifier" {
                                imports.push(ImportDef::simple_import(
                                    get_node_text(&child, source_bytes).to_string(),
                                ));
                                break;
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        Ok(imports)
    }

    fn extract_calls(
        &self,
        _path: &Path,
        source: &str,
        tree: &Tree,
    ) -> Result<HashMap<String, Vec<CallSite>>, ParseError> {
        let source_bytes = source.as_bytes();
        let (defined_funcs, _defined_types) = self.collect_definitions(tree, source_bytes);
        let mut calls_by_func: HashMap<String, Vec<CallSite>> = HashMap::new();

        for node in walk_tree(tree.root_node()) {
            match node.kind() {
                "function_item" => {
                    self.process_function_item_calls(
                        &node,
                        source_bytes,
                        &defined_funcs,
                        &mut calls_by_func,
                    );
                }
                "impl_item" => {
                    self.process_impl_item_calls(
                        &node,
                        source_bytes,
                        &defined_funcs,
                        &mut calls_by_func,
                    );
                }
                "trait_item" => {
                    self.process_trait_item_calls(
                        &node,
                        source_bytes,
                        &defined_funcs,
                        &mut calls_by_func,
                    );
                }
                "const_item" | "static_item" => {
                    self.process_const_or_static_calls(
                        &node,
                        source_bytes,
                        &defined_funcs,
                        &mut calls_by_func,
                    );
                }
                "macro_invocation" => {
                    self.process_macro_invocation_calls(
                        &node,
                        source_bytes,
                        &defined_funcs,
                        &mut calls_by_func,
                    );
                }
                _ => {}
            }
        }

        Ok(calls_by_func)
    }

    fn extract_definitions(
        &self,
        source: &str,
        _path: &Path,
        tree: &Tree,
    ) -> Result<(Vec<FuncDef>, Vec<ClassDef>), super::ParseError> {
        let source_bytes = source.as_bytes();
        let mut funcs = Vec::new();
        let mut classes = Vec::new();

        for node in walk_tree(tree.root_node()) {
            match node.kind() {
                "function_item" => {
                    if let Some(name_node) = node.child_by_field_name("name") {
                        let name = get_node_text(&name_node, source_bytes).to_string();
                        let line = node.start_position().row as u32 + 1;
                        let end_line = node.end_position().row as u32 + 1;

                        // Check if inside an impl block
                        let mut impl_type = None;
                        let mut parent = node.parent();
                        while let Some(p) = parent {
                            if p.kind() == "declaration_list" {
                                if let Some(gp) = p.parent() {
                                    if gp.kind() == "impl_item" {
                                        // Find the type name
                                        for i in 0..gp.child_count() {
                                            if let Some(child) = gp.child(i) {
                                                match child.kind() {
                                                    "type_identifier" => {
                                                        impl_type = Some(
                                                            get_node_text(&child, source_bytes)
                                                                .to_string(),
                                                        );
                                                    }
                                                    "generic_type" => {
                                                        for j in 0..child.child_count() {
                                                            if let Some(tc) = child.child(j) {
                                                                if tc.kind() == "type_identifier" {
                                                                    impl_type = Some(
                                                                        get_node_text(
                                                                            &tc,
                                                                            source_bytes,
                                                                        )
                                                                        .to_string(),
                                                                    );
                                                                    break;
                                                                }
                                                            }
                                                        }
                                                    }
                                                    _ => {}
                                                }
                                                if impl_type.is_some() {
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                }
                                break;
                            }
                            parent = p.parent();
                        }

                        if let Some(type_name) = impl_type {
                            funcs.push(FuncDef::method(name, type_name, line, end_line));
                        } else {
                            funcs.push(FuncDef::function(name, line, end_line));
                        }
                    }
                }
                "struct_item" => {
                    if let Some(name_node) = node.child_by_field_name("name") {
                        let name = get_node_text(&name_node, source_bytes).to_string();
                        let line = node.start_position().row as u32 + 1;
                        let end_line = node.end_position().row as u32 + 1;
                        classes.push(ClassDef::simple(name, line, end_line));
                    }
                }
                "enum_item" => {
                    if let Some(name_node) = node.child_by_field_name("name") {
                        let name = get_node_text(&name_node, source_bytes).to_string();
                        let line = node.start_position().row as u32 + 1;
                        let end_line = node.end_position().row as u32 + 1;
                        classes.push(ClassDef::simple(name, line, end_line));
                    }
                }
                "trait_item" => {
                    if let Some(name_node) = node.child_by_field_name("name") {
                        let name = get_node_text(&name_node, source_bytes).to_string();
                        let line = node.start_position().row as u32 + 1;
                        let end_line = node.end_position().row as u32 + 1;
                        classes.push(ClassDef::simple(name, line, end_line));
                    }
                }
                _ => {}
            }
        }

        Ok((funcs, classes))
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_imports(source: &str) -> Vec<ImportDef> {
        let handler = RustLangHandler::new();
        handler.parse_imports(source, Path::new("test.rs")).unwrap()
    }

    fn extract_calls(source: &str) -> HashMap<String, Vec<CallSite>> {
        let handler = RustLangHandler::new();
        let tree = handler.parse_source(source).unwrap();
        handler
            .extract_calls(Path::new("test.rs"), source, &tree)
            .unwrap()
    }

    // -------------------------------------------------------------------------
    // Import Parsing Tests
    // -------------------------------------------------------------------------

    mod import_tests {
        use super::*;

        #[test]
        fn test_parse_use_simple() {
            let imports = parse_imports("use std::io;");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].module, "std");
            assert!(imports[0].names.contains(&"io".to_string()));
        }

        #[test]
        fn test_parse_use_crate() {
            let imports = parse_imports("use crate::utils::helper;");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].module, "crate::utils");
            assert!(imports[0].names.contains(&"helper".to_string()));
        }

        #[test]
        fn test_parse_use_self() {
            let imports = parse_imports("use self::inner::Item;");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].module, "self::inner");
            assert!(imports[0].names.contains(&"Item".to_string()));
        }

        #[test]
        fn test_parse_use_super() {
            let imports = parse_imports("use super::parent::helper;");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].module, "super::parent");
            assert!(imports[0].names.contains(&"helper".to_string()));
        }

        #[test]
        fn test_parse_use_grouped() {
            let imports = parse_imports("use std::collections::{HashMap, HashSet};");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].module, "std::collections");
            assert!(imports[0].names.contains(&"HashMap".to_string()));
            assert!(imports[0].names.contains(&"HashSet".to_string()));
        }

        #[test]
        fn test_parse_use_glob() {
            let imports = parse_imports("use std::collections::*;");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].module, "std::collections");
            assert!(imports[0].is_wildcard());
        }

        #[test]
        fn test_parse_use_alias() {
            let imports = parse_imports("use std::collections::HashMap as Map;");
            assert_eq!(imports.len(), 1);
            assert!(imports[0].names.contains(&"HashMap".to_string()));
            let aliases = imports[0].aliases.as_ref().unwrap();
            assert_eq!(aliases.get("Map"), Some(&"HashMap".to_string()));
        }

        #[test]
        fn test_parse_mod_declaration() {
            let imports = parse_imports("mod utils;");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].module, "utils");
            assert!(imports[0].is_mod);
        }

        #[test]
        fn test_parse_pub_use() {
            let imports = parse_imports("pub use crate::types::*;");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].module, "crate::types");
            assert!(imports[0].is_wildcard());
        }

        #[test]
        fn test_parse_multiple_imports() {
            let source = r#"
use std::io;
use std::collections::HashMap;
mod utils;
"#;
            let imports = parse_imports(source);
            assert_eq!(imports.len(), 3);
        }
    }

    // -------------------------------------------------------------------------
    // Call Extraction Tests
    // -------------------------------------------------------------------------

    mod call_tests {
        use super::*;

        #[test]
        fn test_extract_calls_direct() {
            let source = r#"
fn main() {
    println!("hello");
    some_func();
}
"#;
            let calls = extract_calls(source);
            let main_calls = calls.get("main").unwrap();
            assert!(main_calls.iter().any(|c| c.target == "some_func"));
        }

        #[test]
        fn test_extract_calls_intra_file() {
            let source = r#"
fn helper() -> i32 {
    42
}

fn main() {
    helper();
}
"#;
            let calls = extract_calls(source);
            let main_calls = calls.get("main").unwrap();
            let helper_call = main_calls.iter().find(|c| c.target == "helper").unwrap();
            assert_eq!(helper_call.call_type, CallType::Intra);
        }

        #[test]
        fn test_extract_calls_method() {
            let source = r#"
fn main() {
    let v = Vec::new();
    v.push(1);
}
"#;
            let calls = extract_calls(source);
            let main_calls = calls.get("main").unwrap();

            let new_call = main_calls.iter().find(|c| c.target == "Vec::new").unwrap();
            assert_eq!(new_call.call_type, CallType::Attr);

            let push_call = main_calls.iter().find(|c| c.target == "v.push").unwrap();
            assert_eq!(push_call.call_type, CallType::Attr);
            assert_eq!(push_call.receiver, Some("v".to_string()));
        }

        #[test]
        fn test_extract_calls_associated_function() {
            let source = r#"
struct MyStruct;

impl MyStruct {
    fn new() -> Self {
        MyStruct
    }

    fn helper(&self) -> i32 {
        42
    }
}

fn main() {
    let obj = MyStruct::new();
}
"#;
            let calls = extract_calls(source);
            let main_calls = calls.get("main").unwrap();

            let new_call = main_calls
                .iter()
                .find(|c| c.target == "MyStruct::new")
                .unwrap();
            assert_eq!(new_call.call_type, CallType::Intra);
        }

        #[test]
        fn test_extract_calls_self_method() {
            let source = r#"
struct MyStruct;

impl MyStruct {
    fn helper(&self) -> i32 {
        42
    }

    fn main(&self) {
        self.helper();
    }
}
"#;
            let calls = extract_calls(source);
            let main_calls = calls.get("MyStruct::main").unwrap();

            let helper_call = main_calls
                .iter()
                .find(|c| c.target == "self.helper")
                .unwrap();
            assert_eq!(helper_call.call_type, CallType::Intra);
            assert_eq!(helper_call.receiver, Some("self".to_string()));
        }

        #[test]
        fn test_extract_calls_with_line_numbers() {
            let source = r#"fn main() {
    first();
    second();
}
"#;
            let calls = extract_calls(source);
            let main_calls = calls.get("main").unwrap();

            let first = main_calls.iter().find(|c| c.target == "first").unwrap();
            let second = main_calls.iter().find(|c| c.target == "second").unwrap();

            assert!(first.line.is_some());
            assert!(second.line.is_some());
            assert!(second.line.unwrap() > first.line.unwrap());
        }

        /// Test that methods in different impl blocks calling the same top-level function
        /// are recorded separately with qualified names like "TypeA::method" and "TypeB::method".
        #[test]
        fn test_extract_calls_method_to_toplevel() {
            let source = r#"
fn top_level_helper() -> i32 {
    42
}

struct TypeA;

impl TypeA {
    fn method_a(&self) -> i32 {
        top_level_helper()
    }
}

struct TypeB;

impl TypeB {
    fn method_b(&self) -> i32 {
        top_level_helper()
    }
}
"#;
            let calls = extract_calls(source);

            // Verify that calls are recorded with qualified caller names
            assert!(
                calls.contains_key("TypeA::method_a"),
                "Should have TypeA::method_a as caller (qualified with impl type)"
            );
            assert!(
                calls.contains_key("TypeB::method_b"),
                "Should have TypeB::method_b as caller (qualified with impl type)"
            );

            // Verify that TypeA::method_a calls top_level_helper
            let type_a_calls = calls
                .get("TypeA::method_a")
                .expect("TypeA::method_a should have calls");
            assert!(
                type_a_calls.iter().any(|c| c.target == "top_level_helper"),
                "TypeA::method_a should call top_level_helper"
            );
            assert!(
                type_a_calls.iter().any(|c| c.call_type == CallType::Intra),
                "Call to top_level_helper should be Intra (same file)"
            );

            // Verify that TypeB::method_b calls top_level_helper
            let type_b_calls = calls
                .get("TypeB::method_b")
                .expect("TypeB::method_b should have calls");
            assert!(
                type_b_calls.iter().any(|c| c.target == "top_level_helper"),
                "TypeB::method_b should call top_level_helper"
            );
            assert!(
                type_b_calls.iter().any(|c| c.call_type == CallType::Intra),
                "Call to top_level_helper should be Intra (same file)"
            );

            // Verify that the calls are recorded separately (different callers)
            assert_ne!(
                type_a_calls, type_b_calls,
                "Calls from different impl methods should be separate entries"
            );
        }
    }

    // -------------------------------------------------------------------------
    // Trait Default Method Body Tests (#26)
    // -------------------------------------------------------------------------

    mod trait_default_method_tests {
        use super::*;

        #[test]
        fn test_trait_default_method_extracts_direct_call() {
            let source = r#"
trait Processor {
    fn process(&self) {
        validate();
    }
}
"#;
            let calls = extract_calls(source);
            let proc_calls = calls.get("Processor::process").expect(
                "Should extract calls from trait default method with caller Processor::process",
            );
            assert!(
                proc_calls.iter().any(|c| c.target == "validate"),
                "Should find validate() call inside trait default method body"
            );
        }

        #[test]
        fn test_trait_default_method_extracts_self_call() {
            let source = r#"
trait Processor {
    fn transform(&self) -> i32 { 42 }
    fn process(&self) {
        self.transform();
    }
}
"#;
            let calls = extract_calls(source);
            let proc_calls = calls
                .get("Processor::process")
                .expect("Should extract calls from trait default method");
            assert!(
                proc_calls.iter().any(|c| c.target == "self.transform"),
                "Should find self.transform() call inside trait default method body"
            );
        }

        #[test]
        fn test_trait_default_method_extracts_associated_call() {
            let source = r#"
trait Builder {
    fn build(&self) {
        Config::new();
        Self::default_config();
    }
}
"#;
            let calls = extract_calls(source);
            let build_calls = calls
                .get("Builder::build")
                .expect("Should extract calls from trait default method");
            assert!(
                build_calls.iter().any(|c| c.target == "Config::new"),
                "Should find Config::new() inside trait default method body"
            );
        }

        #[test]
        fn test_trait_abstract_method_not_extracted() {
            // Abstract methods (no body) should NOT produce call entries
            let source = r#"
trait Processor {
    fn process(&self);
    fn validate(&self);
}
"#;
            let calls = extract_calls(source);
            assert!(
                !calls.contains_key("Processor::process"),
                "Abstract trait method without body should not be in call map"
            );
            assert!(
                !calls.contains_key("Processor::validate"),
                "Abstract trait method without body should not be in call map"
            );
        }

        #[test]
        fn test_trait_mixed_methods() {
            // Mix of abstract and default methods
            let source = r#"
trait Handler {
    fn abstract_method(&self);
    fn default_method(&self) {
        do_something();
    }
}
"#;
            let calls = extract_calls(source);
            assert!(
                !calls.contains_key("Handler::abstract_method"),
                "Abstract method should not be in call map"
            );
            let default_calls = calls
                .get("Handler::default_method")
                .expect("Default method with body should be extracted");
            assert!(
                default_calls.iter().any(|c| c.target == "do_something"),
                "Should find do_something() in default method body"
            );
        }
    }

    // -------------------------------------------------------------------------
    // Const/Static Initializer Tests (#22/#10)
    // -------------------------------------------------------------------------

    mod const_static_tests {
        use super::*;

        #[test]
        fn test_const_initializer_call() {
            let source = r#"
const HANDLER: Handler = create_handler();
"#;
            let calls = extract_calls(source);
            let module_calls = calls
                .get("<module>")
                .expect("Should extract calls from const initializer with caller <module>");
            assert!(
                module_calls.iter().any(|c| c.target == "create_handler"),
                "Should find create_handler() in const initializer"
            );
        }

        #[test]
        fn test_static_initializer_call() {
            let source = r#"
static CONFIG: Config = Config::new();
"#;
            let calls = extract_calls(source);
            let module_calls = calls
                .get("<module>")
                .expect("Should extract calls from static initializer with caller <module>");
            assert!(
                module_calls.iter().any(|c| c.target == "Config::new"),
                "Should find Config::new() in static initializer"
            );
        }

        #[test]
        fn test_static_mut_initializer_call() {
            let source = r#"
static mut COUNTER: u32 = compute_initial();
"#;
            let calls = extract_calls(source);
            let module_calls = calls
                .get("<module>")
                .expect("Should extract calls from static mut initializer with caller <module>");
            assert!(
                module_calls.iter().any(|c| c.target == "compute_initial"),
                "Should find compute_initial() in static mut initializer"
            );
        }

        #[test]
        fn test_const_literal_no_calls() {
            // Literal initializers should NOT produce entries
            let source = r#"
const MAX_SIZE: usize = 100;
static NAME: &str = "hello";
"#;
            let calls = extract_calls(source);
            assert!(
                !calls.contains_key("<module>"),
                "Literal const/static should not produce call entries"
            );
        }

        #[test]
        fn test_const_with_associated_function() {
            let source = r#"
const DEFAULT: Vec<i32> = Vec::new();
"#;
            let calls = extract_calls(source);
            let module_calls = calls
                .get("<module>")
                .expect("Should extract associated function call from const initializer");
            assert!(
                module_calls.iter().any(|c| c.target == "Vec::new"),
                "Should find Vec::new() in const initializer"
            );
        }

        #[test]
        fn test_multiple_const_static_initializers() {
            let source = r#"
const A: i32 = compute_a();
static B: i32 = compute_b();
const C: i32 = 42;
"#;
            let calls = extract_calls(source);
            let module_calls = calls
                .get("<module>")
                .expect("Should collect calls from multiple const/static items");
            assert!(
                module_calls.iter().any(|c| c.target == "compute_a"),
                "Should find compute_a()"
            );
            assert!(
                module_calls.iter().any(|c| c.target == "compute_b"),
                "Should find compute_b()"
            );
            assert_eq!(
                module_calls.len(),
                2,
                "Should only have 2 calls (not the literal 42)"
            );
        }
    }

    // -------------------------------------------------------------------------
    // Closure Body Tests (#15) — Verify already handled by walk_tree
    // -------------------------------------------------------------------------

    mod closure_tests {
        use super::*;

        #[test]
        fn test_closure_calls_extracted() {
            let source = r#"
fn main() {
    let f = |x| transform(x);
}
"#;
            let calls = extract_calls(source);
            let main_calls = calls
                .get("main")
                .expect("Should extract calls from closure body within function");
            assert!(
                main_calls.iter().any(|c| c.target == "transform"),
                "Should find transform() call inside closure body"
            );
        }

        #[test]
        fn test_closure_in_iterator_chain() {
            let source = r#"
fn process_items() {
    let items = vec![1, 2, 3];
    items.iter().map(|x| compute(x));
}
"#;
            let calls = extract_calls(source);
            let calls = calls
                .get("process_items")
                .expect("Should extract calls from closure in iterator chain");
            assert!(
                calls.iter().any(|c| c.target == "compute"),
                "Should find compute() call inside closure in iterator chain"
            );
        }

        #[test]
        fn test_move_closure_calls() {
            let source = r#"
fn setup() {
    let g = move || { initialize(); };
}
"#;
            let calls = extract_calls(source);
            let setup_calls = calls
                .get("setup")
                .expect("Should extract calls from move closure body");
            assert!(
                setup_calls.iter().any(|c| c.target == "initialize"),
                "Should find initialize() call inside move closure body"
            );
        }
    }

    // -------------------------------------------------------------------------
    // Macro Invocation Heuristic Tests (#18)
    // -------------------------------------------------------------------------

    mod macro_heuristic_tests {
        use super::*;

        #[test]
        fn test_macro_with_function_call_arg() {
            let source = r#"
fn main() {
    assert_eq!(compute(), 42);
}
"#;
            let calls = extract_calls(source);
            let main_calls = calls
                .get("main")
                .expect("Should extract calls from macro arguments");
            assert!(
                main_calls.iter().any(|c| c.target == "compute"),
                "Should find compute() call inside assert_eq! macro"
            );
        }

        #[test]
        fn test_macro_vec_with_calls() {
            let source = r#"
fn build() {
    let v = vec![init_value(), other()];
}
"#;
            let calls = extract_calls(source);
            let build_calls = calls
                .get("build")
                .expect("Should extract calls from vec! macro arguments");
            assert!(
                build_calls.iter().any(|c| c.target == "init_value"),
                "Should find init_value() inside vec! macro"
            );
            assert!(
                build_calls.iter().any(|c| c.target == "other"),
                "Should find other() inside vec! macro"
            );
        }

        #[test]
        fn test_macro_println_with_call() {
            let source = r#"
fn display() {
    println!("{}", format_data());
}
"#;
            let calls = extract_calls(source);
            let display_calls = calls
                .get("display")
                .expect("Should extract calls from println! macro arguments");
            assert!(
                display_calls.iter().any(|c| c.target == "format_data"),
                "Should find format_data() inside println! macro"
            );
        }

        #[test]
        fn test_macro_does_not_extract_keywords() {
            // Keywords like if, for, while should NOT be extracted as calls
            let source = r#"
fn check() {
    assert!(if true { valid() } else { false });
}
"#;
            let calls = extract_calls(source);
            if let Some(check_calls) = calls.get("check") {
                assert!(
                    !check_calls.iter().any(|c| c.target == "if"),
                    "Should NOT extract 'if' keyword as a call"
                );
                // valid() might or might not be extracted depending on tree-sitter parsing
                // but 'if' definitely should NOT be
            }
        }

        #[test]
        fn test_macro_with_path_call() {
            let source = r#"
fn configure() {
    assert!(Config::validate());
}
"#;
            let calls = extract_calls(source);
            let cfg_calls = calls
                .get("configure")
                .expect("Should extract path calls from macro arguments");
            assert!(
                cfg_calls.iter().any(|c| c.target == "Config::validate"),
                "Should find Config::validate() inside assert! macro"
            );
        }

        #[test]
        fn test_module_level_macro_call() {
            let source = r#"
lazy_static! {
    static ref INSTANCE: Config = Config::create();
}

fn main() {
    use_instance();
}
"#;
            let calls = extract_calls(source);
            let module_calls = calls.get("<module>");
            // Module-level macro should extract calls with <module> as caller
            if let Some(mc) = module_calls {
                assert!(
                    mc.iter().any(|c| c.target == "Config::create"),
                    "Should find Config::create() inside module-level lazy_static! macro"
                );
            } else {
                panic!("Should extract Config::create() from module-level lazy_static! macro with caller <module>");
            }
        }

        #[test]
        fn test_macro_no_false_positive_on_macro_name() {
            // The macro name itself (e.g., println!) should NOT be extracted as a call
            let source = r#"
fn greet() {
    println!("hello");
}
"#;
            let calls = extract_calls(source);
            // greet might or might not have calls (println has no function call args)
            if let Some(greet_calls) = calls.get("greet") {
                assert!(
                    !greet_calls.iter().any(|c| c.target == "println"),
                    "Should NOT extract the macro name 'println' as a function call"
                );
            }
        }
    }

    // -------------------------------------------------------------------------
    // Handler Trait Tests
    // -------------------------------------------------------------------------

    mod trait_tests {
        use super::*;

        #[test]
        fn test_handler_name() {
            let handler = RustLangHandler::new();
            assert_eq!(handler.name(), "rust");
        }

        #[test]
        fn test_handler_extensions() {
            let handler = RustLangHandler::new();
            let exts = handler.extensions();
            assert!(exts.contains(&".rs"));
        }

        #[test]
        fn test_handler_supports() {
            let handler = RustLangHandler::new();
            assert!(handler.supports("rust"));
            assert!(handler.supports("Rust"));
            assert!(!handler.supports("python"));
        }

        #[test]
        fn test_handler_supports_extension() {
            let handler = RustLangHandler::new();
            assert!(handler.supports_extension(".rs"));
            assert!(!handler.supports_extension(".py"));
        }
    }
}
