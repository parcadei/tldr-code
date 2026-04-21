//! Swift language handler for call graph analysis.
//!
//! This module provides Swift call graph support using tree-sitter-swift (v0.7.1, ABI v15).
//!
//! # Import Patterns Supported
//!
//! | Pattern | ImportDef |
//! |---------|-----------|
//! | `import Foundation` | `{module: "Foundation"}` |
//! | `import struct Foundation.Date` | `{module: "Foundation", names: ["Date"], kind: "struct"}` |
//! | `import class UIKit.UIView` | `{module: "UIKit", names: ["UIView"], kind: "class"}` |
//! | `import func Foundation.strcmp` | `{module: "Foundation", names: ["strcmp"], kind: "func"}` |
//! | `@testable import MyApp` | `{module: "MyApp", is_testable: true}` |
//!
//! # Call Extraction
//!
//! - Direct calls: `func()` -> CallType::Direct or CallType::Intra
//! - Method calls: `obj.method()` -> CallType::Attr
//! - Static calls: `Type.staticMethod()` -> CallType::Attr
//! - Property initializer calls: `let x = Foo()` at class/struct scope
//! - Default parameter calls: `func f(x: Int = defaultVal())`
//! - Global/file-level calls: Calls at file scope use `<module>` as caller
//! - Lazy property calls: `lazy var x = computeValue()`
//! - Closure calls: `{ compute() }` inside function bodies
//! - Guard let/if let calls: `guard let x = tryFoo() else { ... }`
//! - Switch case calls: Calls inside switch patterns
//! - Protocol extension calls: Calls in extension methods
//!
//! # Spec Reference
//!
//! See `migration/spec/callgraph-spec.md` Section 9.11 for Swift-specific details.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use regex::Regex;
use tree_sitter::{Node, Parser, Tree};

use super::base::{get_node_text, walk_tree};
use super::{CallGraphLanguageSupport, ParseError};
use crate::callgraph::cross_file_types::{CallSite, CallType, ClassDef, FuncDef, ImportDef};

// =============================================================================
// Regex Patterns (kept for import parsing)
// =============================================================================

lazy_static::lazy_static! {
    // Import patterns:
    // - import Foundation
    // - import struct Foundation.Date
    // - @testable import MyApp
    static ref IMPORT_RE: Regex = Regex::new(
        r"(?m)^[ \t]*(@testable\s+)?import\s+(struct|class|enum|protocol|func|var|let|typealias)?\s*(\S+)"
    ).unwrap();
}

fn parse_swift_bases(line: &str) -> Vec<String> {
    let mut bases = Vec::new();
    let colon_pos = match line.find(':') {
        Some(pos) => pos,
        None => return bases,
    };

    let mut after = &line[colon_pos + 1..];
    if let Some(before_brace) = after.split('{').next() {
        after = before_brace;
    }
    if let Some(before_where) = after.split(" where ").next() {
        after = before_where;
    }

    for part in after.split(',') {
        let name = part.trim();
        if name.is_empty() {
            continue;
        }
        let name = name.split('<').next().unwrap_or(name).trim();
        if !name.is_empty() {
            bases.push(name.to_string());
        }
    }

    bases
}

// =============================================================================
// Swift Handler
// =============================================================================

/// Swift language handler using tree-sitter-swift for AST-based call extraction.
///
/// Supports:
/// - Import declaration parsing (simple, selective, @testable) via regex
/// - Function/method declaration extraction via tree-sitter
/// - Type (class/struct/enum/protocol) declaration extraction via tree-sitter
/// - Call extraction (direct, method, static, property init, default params,
///   closures, guard/if-let, switch cases) via tree-sitter
#[derive(Debug, Default)]
pub struct SwiftHandler;

impl SwiftHandler {
    /// Creates a new SwiftHandler.
    pub fn new() -> Self {
        Self
    }

    /// Parse the source code into a tree-sitter Tree using the Swift grammar.
    fn parse_source(&self, source: &str) -> Result<Tree, ParseError> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_swift::LANGUAGE.into())
            .map_err(|e| ParseError::ParseFailed {
                file: std::path::PathBuf::new(),
                message: format!("Failed to set Swift language: {}", e),
            })?;

        parser
            .parse(source, None)
            .ok_or_else(|| ParseError::ParseFailed {
                file: std::path::PathBuf::new(),
                message: "Parser returned None".to_string(),
            })
    }

    /// Parse imports from source code using regex.
    fn parse_imports_regex(&self, source: &str) -> Vec<ImportDef> {
        let mut imports = Vec::new();

        for cap in IMPORT_RE.captures_iter(source) {
            let is_testable = cap.get(1).is_some();
            let kind = cap.get(2).map(|m| m.as_str().to_string());
            let module_path = cap.get(3).map(|m| m.as_str()).unwrap_or("");

            if module_path.is_empty() {
                continue;
            }

            // Parse module path: Foundation.Date -> module=Foundation, name=Date
            let (module, names) = if let Some(_kind_str) = &kind {
                // Selective import: import struct Foundation.Date
                if let Some(last_dot) = module_path.rfind('.') {
                    let module = module_path[..last_dot].to_string();
                    let name = module_path[last_dot + 1..].to_string();
                    (module, vec![name])
                } else {
                    // No dot, entire thing is the name from implicit module
                    // This is unusual but handle it
                    (module_path.to_string(), vec![])
                }
            } else {
                // Simple import: import Foundation
                (module_path.to_string(), vec![])
            };

            let mut import_def = if names.is_empty() {
                ImportDef::simple_import(module)
            } else {
                ImportDef::from_import(module, names)
            };

            // Store kind and testable flag in resolved_module for now
            // since ImportDef doesn't have dedicated fields for these
            if is_testable {
                import_def.resolved_module = Some("@testable".to_string());
            }

            imports.push(import_def);
        }

        imports
    }

    /// Collect all function names defined in the file using tree-sitter.
    fn collect_function_definitions_ts(&self, tree: &Tree, source: &[u8]) -> HashSet<String> {
        let mut functions = HashSet::new();

        for node in walk_tree(tree.root_node()) {
            match node.kind() {
                "function_declaration" => {
                    if let Some(name_node) = node.child_by_field_name("name") {
                        functions.insert(get_node_text(&name_node, source).to_string());
                    }
                }
                "protocol_function_declaration" => {
                    // Protocol requirement: func required()
                    if let Some(name_node) = node.child_by_field_name("name") {
                        functions.insert(get_node_text(&name_node, source).to_string());
                    }
                }
                "init_declaration" => {
                    functions.insert("init".to_string());
                }
                "class_declaration" => {
                    // Type names are callable (as initializers)
                    if let Some(name_node) = node.child_by_field_name("name") {
                        let text = get_node_text(&name_node, source);
                        // name_node might be a type_identifier or user_type
                        // For classes: type_identifier directly
                        // For extensions: user_type containing type_identifier
                        let type_name = if name_node.kind() == "user_type" {
                            // Extract the type_identifier from user_type
                            name_node
                                .child(0)
                                .map(|c| get_node_text(&c, source))
                                .unwrap_or(text)
                        } else {
                            text
                        };
                        functions.insert(type_name.to_string());
                    }
                    // Also check declaration_kind to see if it's a struct/enum/protocol
                }
                "protocol_declaration" => {
                    if let Some(name_node) = node.child_by_field_name("name") {
                        functions.insert(get_node_text(&name_node, source).to_string());
                    }
                }
                _ => {}
            }
        }

        functions
    }

    /// Collect function definitions using regex (kept for backward compat with tests).
    pub fn collect_function_definitions(&self, source: &str) -> HashSet<String> {
        let tree = match self.parse_source(source) {
            Ok(t) => t,
            Err(_) => return HashSet::new(),
        };
        self.collect_function_definitions_ts(&tree, source.as_bytes())
    }

    /// Resolve the target and receiver from a call_expression node.
    ///
    /// Returns (target_string, receiver_option, call_type_hint).
    fn resolve_call_target(
        &self,
        node: &Node,
        source: &[u8],
        defined_funcs: &HashSet<String>,
    ) -> Option<(String, Option<String>, CallType)> {
        // The first child of call_expression is the callable
        let func_node = node.child(0)?;

        match func_node.kind() {
            "simple_identifier" => {
                let target = get_node_text(&func_node, source).to_string();
                if is_swift_keyword(&target) {
                    return None;
                }
                let call_type = if defined_funcs.contains(&target) {
                    CallType::Intra
                } else {
                    CallType::Direct
                };
                Some((target, None, call_type))
            }
            "navigation_expression" => self.resolve_navigation_call(&func_node, source),
            _ => None,
        }
    }

    /// Resolve a navigation_expression (receiver.method) into call target info.
    fn resolve_navigation_call(
        &self,
        nav_node: &Node,
        source: &[u8],
    ) -> Option<(String, Option<String>, CallType)> {
        let target_node = nav_node.child_by_field_name("target")?;
        let suffix_node = nav_node.child_by_field_name("suffix")?;

        // Extract the method name from the suffix
        let method = self.extract_suffix_name(&suffix_node, source)?;

        // Extract the receiver
        let receiver = self.extract_target_name(&target_node, source)?;

        let target = format!("{}.{}", receiver, method);
        Some((target, Some(receiver), CallType::Attr))
    }

    /// Extract the method/property name from a navigation_suffix node.
    fn extract_suffix_name(&self, node: &Node, source: &[u8]) -> Option<String> {
        // navigation_suffix has children: "." and simple_identifier
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if child.kind() == "simple_identifier" {
                    return Some(get_node_text(&child, source).to_string());
                }
            }
        }
        None
    }

    /// Extract the receiver name from the target of a navigation_expression.
    fn extract_target_name(&self, node: &Node, source: &[u8]) -> Option<String> {
        match node.kind() {
            "simple_identifier" => Some(get_node_text(node, source).to_string()),
            "self_expression" => Some("self".to_string()),
            "super_expression" => Some("super".to_string()),
            "call_expression" => {
                // Chained call: receiver is the full call expression text, but
                // for our purposes we want the outermost simple_identifier
                // e.g., data.filter { ... }.map { ... } -> "data.filter"
                // Actually, let's use the full text for chained expressions
                let text = get_node_text(node, source).to_string();
                // Extract just the first identifier for a cleaner receiver
                if let Some(first_child) = node.child(0) {
                    if first_child.kind() == "navigation_expression" {
                        if let Some(t) = first_child.child_by_field_name("target") {
                            return self.extract_target_name(&t, source);
                        }
                    } else if first_child.kind() == "simple_identifier" {
                        return Some(get_node_text(&first_child, source).to_string());
                    }
                }
                Some(text)
            }
            "navigation_expression" => {
                // Nested navigation: a.b.c - use the full text
                Some(get_node_text(node, source).to_string())
            }
            _ => Some(get_node_text(node, source).to_string()),
        }
    }

    /// Extract all call_expression nodes from a subtree, returning CallSites.
    fn extract_calls_from_subtree(
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

                if let Some((target, receiver, call_type)) =
                    self.resolve_call_target(&child, source, defined_funcs)
                {
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
        }

        calls
    }

    /// Extract calls from the entire source using tree-sitter AST walking.
    ///
    /// This walks the AST looking for:
    /// 1. function_declaration / init_declaration nodes -> extract calls from their bodies
    /// 2. Property initializer calls at class/struct scope
    /// 3. Default parameter calls
    /// 4. Global/file-level calls
    fn extract_calls_from_source_ts(
        &self,
        tree: &Tree,
        source: &[u8],
        defined_funcs: &HashSet<String>,
    ) -> HashMap<String, Vec<CallSite>> {
        let mut calls_by_func: HashMap<String, Vec<CallSite>> = HashMap::new();

        // Walk the tree looking for class/extension and function declarations
        self.walk_for_calls(
            &tree.root_node(),
            source,
            defined_funcs,
            &mut calls_by_func,
            None, // no enclosing type at top level
        );

        // Remove empty entries
        calls_by_func.retain(|_, v| !v.is_empty());

        calls_by_func
    }

    /// Recursively walk AST nodes to find function declarations and extract calls.
    fn walk_for_calls(
        &self,
        node: &Node,
        source: &[u8],
        defined_funcs: &HashSet<String>,
        calls_by_func: &mut HashMap<String, Vec<CallSite>>,
        current_type: Option<&str>,
    ) {
        match node.kind() {
            "class_declaration" => {
                // Get the type name
                let type_name = node.child_by_field_name("name").map(|n| {
                    let text = get_node_text(&n, source);
                    if n.kind() == "user_type" {
                        // For extensions, name is user_type > type_identifier
                        n.child(0)
                            .map(|c| get_node_text(&c, source))
                            .unwrap_or(text)
                            .to_string()
                    } else {
                        text.to_string()
                    }
                });

                // Walk the body with the type context
                if let Some(body) = node.child_by_field_name("body") {
                    for i in 0..body.child_count() {
                        if let Some(child) = body.child(i) {
                            self.walk_for_calls(
                                &child,
                                source,
                                defined_funcs,
                                calls_by_func,
                                type_name.as_deref(),
                            );
                        }
                    }
                }
            }
            "function_declaration" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let func_name = get_node_text(&name_node, source).to_string();
                    let qualified_name = if let Some(type_name) = current_type {
                        format!("{}.{}", type_name, func_name)
                    } else {
                        func_name.clone()
                    };

                    let mut all_calls = Vec::new();

                    // Extract calls from the function body
                    if let Some(body) = node.child_by_field_name("body") {
                        let body_calls = self.extract_calls_from_subtree(
                            &body,
                            source,
                            defined_funcs,
                            &qualified_name,
                        );
                        all_calls.extend(body_calls);
                    }

                    // Extract calls from default parameter values
                    let default_calls = self.extract_default_param_calls(
                        node,
                        source,
                        defined_funcs,
                        &qualified_name,
                    );
                    all_calls.extend(default_calls);

                    if !all_calls.is_empty() {
                        // Insert under qualified name
                        calls_by_func
                            .entry(qualified_name.clone())
                            .or_default()
                            .extend(all_calls.iter().cloned());

                        // Also insert under unqualified name if different
                        if qualified_name != func_name {
                            calls_by_func
                                .entry(func_name)
                                .or_default()
                                .extend(all_calls);
                        }
                    }
                }
            }
            "init_declaration" => {
                let func_name = "init".to_string();
                let qualified_name = if let Some(type_name) = current_type {
                    format!("{}.{}", type_name, func_name)
                } else {
                    func_name.clone()
                };

                if let Some(body) = node.child_by_field_name("body") {
                    let calls = self.extract_calls_from_subtree(
                        &body,
                        source,
                        defined_funcs,
                        &qualified_name,
                    );
                    if !calls.is_empty() {
                        calls_by_func
                            .entry(qualified_name.clone())
                            .or_default()
                            .extend(calls.iter().cloned());

                        if qualified_name != func_name {
                            calls_by_func.entry(func_name).or_default().extend(calls);
                        }
                    }
                }
            }
            "property_declaration" => {
                // Property initializer calls at class/struct scope
                // e.g., `let x = Foo()` or `lazy var y = computeValue()`
                // These are attributed to the enclosing type or <module>
                if let Some(value_node) = node.child_by_field_name("value") {
                    if value_node.kind() == "call_expression" {
                        let caller_name = current_type
                            .map(|t| t.to_string())
                            .unwrap_or_else(|| "<module>".to_string());

                        if let Some((target, receiver, call_type)) =
                            self.resolve_call_target(&value_node, source, defined_funcs)
                        {
                            let line = value_node.start_position().row as u32 + 1;
                            let call = CallSite::new(
                                caller_name.clone(),
                                target,
                                call_type,
                                Some(line),
                                None,
                                receiver,
                                None,
                            );
                            calls_by_func.entry(caller_name).or_default().push(call);
                        }
                    }
                }
            }
            _ => {
                // For other nodes (e.g., source_file), recurse into children
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        self.walk_for_calls(
                            &child,
                            source,
                            defined_funcs,
                            calls_by_func,
                            current_type,
                        );
                    }
                }
            }
        }
    }

    /// Extract calls from default parameter values in a function declaration.
    fn extract_default_param_calls(
        &self,
        func_node: &Node,
        source: &[u8],
        defined_funcs: &HashSet<String>,
        caller: &str,
    ) -> Vec<CallSite> {
        let mut calls = Vec::new();

        // Look for `default_value` field children which are call_expressions
        for i in 0..func_node.child_count() {
            if let Some(child) = func_node.child(i) {
                if child.kind() == "parameter" {
                    // Parameters may have default values
                    // In the AST, defaults appear as siblings of the parameter
                    // Actually, from exploration: default_value is a field on function_declaration directly
                    // as `= call_expression` after the parameter
                    continue;
                }
                // default_value field: call_expression with field "default_value"
                if let Some(field_name) = func_node.field_name_for_child(i as u32) {
                    if field_name == "default_value" && child.kind() == "call_expression" {
                        let line = child.start_position().row as u32 + 1;
                        if let Some((target, receiver, call_type)) =
                            self.resolve_call_target(&child, source, defined_funcs)
                        {
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
                }
            }
        }

        calls
    }

    /// Backward-compatible method for tests that call extract_calls_from_source directly.
    pub fn extract_calls_from_source(
        &self,
        source: &str,
        _defined_funcs: &HashSet<String>,
    ) -> HashMap<String, Vec<CallSite>> {
        let tree = match self.parse_source(source) {
            Ok(t) => t,
            Err(_) => return HashMap::new(),
        };
        let source_bytes = source.as_bytes();
        let defined_funcs = self.collect_function_definitions_ts(&tree, source_bytes);
        self.extract_calls_from_source_ts(&tree, source_bytes, &defined_funcs)
    }
}

/// Check if a string is a Swift keyword (to avoid false call detections).
fn is_swift_keyword(s: &str) -> bool {
    matches!(
        s,
        "if" | "else"
            | "for"
            | "while"
            | "switch"
            | "case"
            | "default"
            | "do"
            | "try"
            | "catch"
            | "throw"
            | "throws"
            | "return"
            | "break"
            | "continue"
            | "fallthrough"
            | "in"
            | "where"
            | "guard"
            | "defer"
            | "self"
            | "super"
            | "nil"
            | "true"
            | "false"
            | "let"
            | "var"
            | "func"
            | "class"
            | "struct"
            | "enum"
            | "protocol"
            | "extension"
            | "import"
            | "typealias"
            | "associatedtype"
            | "static"
            | "override"
            | "final"
            | "mutating"
            | "nonmutating"
            | "public"
            | "private"
            | "internal"
            | "fileprivate"
            | "open"
            | "init"
            | "deinit"
            | "subscript"
            | "convenience"
            | "required"
            | "get"
            | "set"
            | "willSet"
            | "didSet"
            | "is"
            | "as"
            | "Any"
            | "Self"
            | "Type"
            | "async"
            | "await"
            | "actor"
    )
}

impl CallGraphLanguageSupport for SwiftHandler {
    fn name(&self) -> &str {
        "swift"
    }

    fn extensions(&self) -> &[&str] {
        &[".swift"]
    }

    fn parse_imports(&self, source: &str, _path: &Path) -> Result<Vec<ImportDef>, ParseError> {
        Ok(self.parse_imports_regex(source))
    }

    fn extract_calls(
        &self,
        _path: &Path,
        source: &str,
        _tree: &Tree,
    ) -> Result<HashMap<String, Vec<CallSite>>, ParseError> {
        // Parse with the real Swift grammar (ignore the passed tree which may be a dummy)
        let tree = self.parse_source(source)?;
        let source_bytes = source.as_bytes();
        let defined_funcs = self.collect_function_definitions_ts(&tree, source_bytes);
        Ok(self.extract_calls_from_source_ts(&tree, source_bytes, &defined_funcs))
    }

    fn extract_definitions(
        &self,
        source: &str,
        _path: &Path,
        _tree: &Tree,
    ) -> Result<(Vec<FuncDef>, Vec<ClassDef>), super::ParseError> {
        // Parse with the real Swift grammar
        let tree = self.parse_source(source)?;
        let source_bytes = source.as_bytes();
        let mut funcs = Vec::new();
        let mut classes = Vec::new();
        let mut class_lines: HashMap<String, (u32, u32)> = HashMap::new();
        let mut class_bases: HashMap<String, Vec<String>> = HashMap::new();
        let mut methods_by_type: HashMap<String, Vec<String>> = HashMap::new();

        // Use tree-sitter for definitions with regex fallback for base parsing
        let lines: Vec<&str> = source.lines().collect();

        for node in walk_tree(tree.root_node()) {
            match node.kind() {
                "class_declaration" => {
                    let line_number = node.start_position().row as u32 + 1;
                    let end_line = node.end_position().row as u32 + 1;

                    // Determine if this is a class, struct, enum, or extension
                    let _decl_kind = node
                        .child_by_field_name("declaration_kind")
                        .map(|dk| get_node_text(&dk, source_bytes).to_string());

                    let type_name = node.child_by_field_name("name").map(|n| {
                        let text = get_node_text(&n, source_bytes);
                        if n.kind() == "user_type" {
                            n.child(0)
                                .map(|c| get_node_text(&c, source_bytes))
                                .unwrap_or(text)
                                .to_string()
                        } else {
                            text.to_string()
                        }
                    });

                    if let Some(type_name) = type_name {
                        class_lines
                            .entry(type_name.clone())
                            .and_modify(|(_, end)| *end = (*end).max(end_line))
                            .or_insert((line_number, end_line));

                        // Parse base types using regex on the declaration line
                        if let Some(decl_line) = lines.get(line_number as usize - 1) {
                            let bases = parse_swift_bases(decl_line);
                            if !bases.is_empty() {
                                let entry = class_bases.entry(type_name.clone()).or_default();
                                for base in bases {
                                    if !entry.contains(&base) {
                                        entry.push(base);
                                    }
                                }
                            }
                        }

                        // Walk the class body for function declarations
                        if let Some(body) = node.child_by_field_name("body") {
                            for i in 0..body.child_count() {
                                if let Some(child) = body.child(i) {
                                    match child.kind() {
                                        "function_declaration" => {
                                            if let Some(name_node) =
                                                child.child_by_field_name("name")
                                            {
                                                let func_name =
                                                    get_node_text(&name_node, source_bytes)
                                                        .to_string();
                                                let fn_line = child.start_position().row as u32 + 1;
                                                let fn_end = child.end_position().row as u32 + 1;
                                                funcs.push(FuncDef::method(
                                                    func_name.clone(),
                                                    type_name.clone(),
                                                    fn_line,
                                                    fn_end,
                                                ));
                                                methods_by_type
                                                    .entry(type_name.clone())
                                                    .or_default()
                                                    .push(func_name);
                                            }
                                        }
                                        "init_declaration" => {
                                            let func_name = "init".to_string();
                                            let fn_line = child.start_position().row as u32 + 1;
                                            let fn_end = child.end_position().row as u32 + 1;
                                            funcs.push(FuncDef::method(
                                                func_name.clone(),
                                                type_name.clone(),
                                                fn_line,
                                                fn_end,
                                            ));
                                            methods_by_type
                                                .entry(type_name.clone())
                                                .or_default()
                                                .push(func_name);
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
                }
                "protocol_declaration" => {
                    let line_number = node.start_position().row as u32 + 1;
                    let end_line = node.end_position().row as u32 + 1;

                    if let Some(name_node) = node.child_by_field_name("name") {
                        let type_name = get_node_text(&name_node, source_bytes).to_string();
                        class_lines
                            .entry(type_name.clone())
                            .and_modify(|(_, end)| *end = (*end).max(end_line))
                            .or_insert((line_number, end_line));

                        // Parse base protocols
                        if let Some(decl_line) = lines.get(line_number as usize - 1) {
                            let bases = parse_swift_bases(decl_line);
                            if !bases.is_empty() {
                                let entry = class_bases.entry(type_name.clone()).or_default();
                                for base in bases {
                                    if !entry.contains(&base) {
                                        entry.push(base);
                                    }
                                }
                            }
                        }

                        // Walk for required function declarations
                        // Protocol body is "protocol_body", not "class_body"
                        if let Some(body) = node.child_by_field_name("body") {
                            for i in 0..body.child_count() {
                                if let Some(child) = body.child(i) {
                                    // Protocol functions are protocol_function_declaration
                                    if child.kind() == "function_declaration"
                                        || child.kind() == "protocol_function_declaration"
                                    {
                                        if let Some(fn_name_node) =
                                            child.child_by_field_name("name")
                                        {
                                            let func_name =
                                                get_node_text(&fn_name_node, source_bytes)
                                                    .to_string();
                                            let fn_line = child.start_position().row as u32 + 1;
                                            let fn_end = child.end_position().row as u32 + 1;
                                            funcs.push(FuncDef::method(
                                                func_name.clone(),
                                                type_name.clone(),
                                                fn_line,
                                                fn_end,
                                            ));
                                            methods_by_type
                                                .entry(type_name.clone())
                                                .or_default()
                                                .push(func_name);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                "function_declaration" => {
                    // Top-level functions (not inside a class/extension)
                    // Only process if parent is source_file
                    if let Some(parent) = node.parent() {
                        if parent.kind() == "source_file" {
                            if let Some(name_node) = node.child_by_field_name("name") {
                                let func_name = get_node_text(&name_node, source_bytes).to_string();
                                let fn_line = node.start_position().row as u32 + 1;
                                let fn_end = node.end_position().row as u32 + 1;
                                funcs.push(FuncDef::function(func_name, fn_line, fn_end));
                            }
                        }
                    }
                }
                "init_declaration" => {
                    // Top-level init (unusual but handle it)
                    if let Some(parent) = node.parent() {
                        if parent.kind() == "source_file" {
                            let func_name = "init".to_string();
                            let fn_line = node.start_position().row as u32 + 1;
                            let fn_end = node.end_position().row as u32 + 1;
                            funcs.push(FuncDef::function(func_name, fn_line, fn_end));
                        }
                    }
                }
                _ => {}
            }
        }

        for (name, (line, end_line)) in class_lines {
            let methods = methods_by_type.remove(&name).unwrap_or_default();
            let bases = class_bases.remove(&name).unwrap_or_default();
            classes.push(ClassDef::new(name, line, end_line, methods, bases));
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
        let handler = SwiftHandler::new();
        handler
            .parse_imports(source, Path::new("test.swift"))
            .unwrap()
    }

    fn extract_calls(source: &str) -> HashMap<String, Vec<CallSite>> {
        let handler = SwiftHandler::new();
        // Create a dummy tree (the handler parses with its own Swift parser)
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse("", None).unwrap();

        handler
            .extract_calls(Path::new("test.swift"), source, &tree)
            .unwrap()
    }

    // -------------------------------------------------------------------------
    // Import Parsing Tests
    // -------------------------------------------------------------------------

    mod import_tests {
        use super::*;

        #[test]
        fn test_parse_import_simple() {
            let imports = parse_imports("import Foundation");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].module, "Foundation");
            assert!(imports[0].names.is_empty());
        }

        #[test]
        fn test_parse_import_uikit() {
            let imports = parse_imports("import UIKit");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].module, "UIKit");
        }

        #[test]
        fn test_parse_import_selective_struct() {
            let imports = parse_imports("import struct Foundation.Date");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].module, "Foundation");
            assert!(imports[0].names.contains(&"Date".to_string()));
        }

        #[test]
        fn test_parse_import_selective_class() {
            let imports = parse_imports("import class UIKit.UIView");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].module, "UIKit");
            assert!(imports[0].names.contains(&"UIView".to_string()));
        }

        #[test]
        fn test_parse_import_selective_func() {
            let imports = parse_imports("import func Foundation.strcmp");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].module, "Foundation");
            assert!(imports[0].names.contains(&"strcmp".to_string()));
        }

        #[test]
        fn test_parse_import_testable() {
            let imports = parse_imports("@testable import MyApp");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].module, "MyApp");
            assert_eq!(imports[0].resolved_module, Some("@testable".to_string()));
        }

        #[test]
        fn test_parse_import_testable_with_whitespace() {
            let imports = parse_imports("@testable   import   MyModule");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].module, "MyModule");
            assert_eq!(imports[0].resolved_module, Some("@testable".to_string()));
        }

        #[test]
        fn test_parse_import_nested_module() {
            let imports = parse_imports("import struct Foundation.URLSession.DataTask");
            assert_eq!(imports.len(), 1);
            // Foundation.URLSession is the module, DataTask is the imported name
            assert_eq!(imports[0].module, "Foundation.URLSession");
            assert!(imports[0].names.contains(&"DataTask".to_string()));
        }

        #[test]
        fn test_parse_multiple_imports() {
            let source = r#"
import Foundation
import UIKit
@testable import MyApp
import struct Foundation.Date
"#;
            let imports = parse_imports(source);
            assert_eq!(imports.len(), 4);

            assert_eq!(imports[0].module, "Foundation");
            assert_eq!(imports[1].module, "UIKit");
            assert_eq!(imports[2].module, "MyApp");
            assert_eq!(imports[3].module, "Foundation");
        }
    }

    // -------------------------------------------------------------------------
    // Function Definition Tests
    // -------------------------------------------------------------------------

    mod definition_tests {
        use super::*;

        #[test]
        fn test_collect_function_definitions() {
            let source = r#"
func helper() {
    print("hello")
}

public func publicHelper() -> Int {
    return 42
}

private func privateHelper() {
}
"#;
            let handler = SwiftHandler::new();
            let funcs = handler.collect_function_definitions(source);

            assert!(funcs.contains("helper"));
            assert!(funcs.contains("publicHelper"));
            assert!(funcs.contains("privateHelper"));
        }

        #[test]
        fn test_collect_type_definitions() {
            let source = r#"
class MyClass {
    func method() {}
}

struct MyStruct {
    func method() {}
}

enum MyEnum {
    case one
}

protocol MyProtocol {
    func required()
}
"#;
            let handler = SwiftHandler::new();
            let funcs = handler.collect_function_definitions(source);

            // Types are callable (initializers)
            assert!(funcs.contains("MyClass"));
            assert!(funcs.contains("MyStruct"));
            assert!(funcs.contains("MyEnum"));
            assert!(funcs.contains("MyProtocol"));

            // Methods
            assert!(funcs.contains("method"));
            assert!(funcs.contains("required"));
        }

        #[test]
        fn test_collect_static_functions() {
            let source = r#"
class MyClass {
    static func staticMethod() {}
    class func classMethod() {}
}
"#;
            let handler = SwiftHandler::new();
            let funcs = handler.collect_function_definitions(source);

            assert!(funcs.contains("staticMethod"));
            assert!(funcs.contains("classMethod"));
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
func main() {
    print("hello")
    someFunction()
}
"#;
            let calls = extract_calls(source);
            let main_calls = calls.get("main").unwrap();

            assert!(main_calls.iter().any(|c| c.target == "print"));
            assert!(main_calls.iter().any(|c| c.target == "someFunction"));
        }

        #[test]
        fn test_extract_calls_intra_file() {
            let source = r#"
func helper() -> Int {
    return 42
}

func main() {
    let x = helper()
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
func main() {
    let arr = [1, 2, 3]
    arr.append(4)
    arr.map({ $0 * 2 })
}
"#;
            let calls = extract_calls(source);
            let main_calls = calls.get("main").unwrap();

            let append_call = main_calls
                .iter()
                .find(|c| c.target == "arr.append")
                .unwrap();
            assert_eq!(append_call.call_type, CallType::Attr);
            assert_eq!(append_call.receiver, Some("arr".to_string()));

            let map_call = main_calls.iter().find(|c| c.target == "arr.map").unwrap();
            assert_eq!(map_call.call_type, CallType::Attr);
        }

        #[test]
        fn test_trailing_closure_not_detected() {
            // With tree-sitter, trailing closure syntax `arr.map { }` IS detected
            // as a call_expression (unlike the old regex parser). Update test
            // to verify this improvement.
            let source = r#"
func main() {
    let arr = [1, 2, 3]
    arr.map { $0 * 2 }
}
"#;
            let calls = extract_calls(source);
            let main_calls = calls.get("main");

            // With tree-sitter, trailing closures ARE detected as calls
            // This is an improvement over the regex parser
            if let Some(mc) = main_calls {
                // Tree-sitter correctly identifies arr.map { } as a call
                assert!(mc.iter().any(|c| c.target == "arr.map"));
            }
        }

        #[test]
        fn test_extract_calls_static() {
            let source = r#"
func main() {
    let date = Date()
    let url = URL(string: "https://example.com")
    let result = MyClass.staticMethod()
}
"#;
            let calls = extract_calls(source);
            let main_calls = calls.get("main").unwrap();

            // Date() is a direct call (initializer)
            assert!(main_calls.iter().any(|c| c.target == "Date"));

            // URL(string:) is a direct call (initializer)
            assert!(main_calls.iter().any(|c| c.target == "URL"));

            // MyClass.staticMethod() is an attr call
            let static_call = main_calls
                .iter()
                .find(|c| c.target == "MyClass.staticMethod")
                .unwrap();
            assert_eq!(static_call.call_type, CallType::Attr);
            assert_eq!(static_call.receiver, Some("MyClass".to_string()));
        }

        #[test]
        fn test_extract_calls_in_class_method() {
            let source = r#"
class MyService {
    func process() {
        helper()
        self.validate()
        OtherService.fetch()
    }

    func helper() {}
    func validate() {}
}
"#;
            let calls = extract_calls(source);

            // Should have calls from MyService.process (qualified name for class method)
            let process_calls = calls.get("MyService.process");
            assert!(
                process_calls.is_some(),
                "Expected calls from MyService.process, got keys: {:?}",
                calls.keys().collect::<Vec<_>>()
            );
            let process_calls = process_calls.unwrap();

            assert!(process_calls.iter().any(|c| c.target == "helper"));
            assert!(process_calls.iter().any(|c| c.target == "self.validate"));
            assert!(process_calls
                .iter()
                .any(|c| c.target == "OtherService.fetch"));
        }

        #[test]
        fn test_extract_calls_method_to_toplevel() {
            let source = r#"
class Service {
    func process() {
        helper()
    }
}

func helper() {}
"#;
            let calls = extract_calls(source);
            // Should have calls from Service.process (qualified name)
            let process_calls = calls.get("Service.process");
            assert!(
                process_calls.is_some(),
                "Expected calls from Service.process, got keys: {:?}",
                calls.keys().collect::<Vec<_>>()
            );
            let process_calls = process_calls.unwrap();
            // Should call helper() as intra-file call
            let helper_call = process_calls.iter().find(|c| c.target == "helper");
            assert!(
                helper_call.is_some(),
                "Expected call to helper from Service.process, got: {:?}",
                process_calls.iter().map(|c| &c.target).collect::<Vec<_>>()
            );
            assert_eq!(helper_call.unwrap().call_type, CallType::Intra);
            assert_eq!(helper_call.unwrap().caller, "Service.process");
        }

        #[test]
        fn test_extract_calls_in_extension() {
            let source = r#"
extension String {
    func customMethod() {
        let len = self.count
        helper()
    }
}

func helper() {}
"#;
            let calls = extract_calls(source);

            // Should have calls from String.customMethod
            let method_calls = calls.get("String.customMethod").unwrap();

            // self.count is a property access, not detected as call
            // helper() should be detected
            assert!(method_calls.iter().any(|c| c.target == "helper"));
        }

        #[test]
        fn test_extract_calls_chained() {
            let source = r#"
func main() {
    let result = data.filter { $0 > 0 }.map { $0 * 2 }.reduce(0, +)
}
"#;
            let calls = extract_calls(source);
            let main_calls = calls.get("main").unwrap();

            // Chained calls - tree-sitter handles these
            assert!(!main_calls.is_empty());
        }

        #[test]
        fn test_skip_keywords() {
            let source = r#"
func main() {
    if condition {
        for item in items {
            switch value {
            case .one:
                break
            }
        }
    }
}
"#;
            let calls = extract_calls(source);
            let main_calls = calls.get("main");

            // Keywords like if, for, switch should not be detected as calls
            if let Some(mc) = main_calls {
                assert!(!mc.iter().any(|c| c.target == "if"));
                assert!(!mc.iter().any(|c| c.target == "for"));
                assert!(!mc.iter().any(|c| c.target == "switch"));
            }
        }

        #[test]
        fn test_line_numbers() {
            let source = r#"func main() {
    helper()
    other()
}"#;
            let calls = extract_calls(source);
            let main_calls = calls.get("main").unwrap();

            // helper() is on line 2
            let helper_call = main_calls.iter().find(|c| c.target == "helper").unwrap();
            assert_eq!(helper_call.line, Some(2));

            // other() is on line 3
            let other_call = main_calls.iter().find(|c| c.target == "other").unwrap();
            assert_eq!(other_call.line, Some(3));
        }
    }

    // -------------------------------------------------------------------------
    // Handler Trait Tests
    // -------------------------------------------------------------------------

    mod trait_tests {
        use super::*;

        #[test]
        fn test_handler_name() {
            let handler = SwiftHandler::new();
            assert_eq!(handler.name(), "swift");
        }

        #[test]
        fn test_handler_extensions() {
            let handler = SwiftHandler::new();
            assert_eq!(handler.extensions(), &[".swift"]);
        }

        #[test]
        fn test_handler_supports() {
            let handler = SwiftHandler::new();
            assert!(handler.supports("swift"));
            assert!(handler.supports("Swift"));
            assert!(handler.supports("SWIFT"));
            assert!(!handler.supports("rust"));
        }

        #[test]
        fn test_handler_supports_extension() {
            let handler = SwiftHandler::new();
            assert!(handler.supports_extension(".swift"));
            assert!(handler.supports_extension(".SWIFT"));
            assert!(!handler.supports_extension(".rs"));
        }
    }

    // -------------------------------------------------------------------------
    // Integration Tests
    // -------------------------------------------------------------------------

    mod integration_tests {
        use super::*;

        #[test]
        fn test_complete_swift_file() {
            let source = r#"
import Foundation
import UIKit
@testable import MyApp

class ViewController: UIViewController {
    private let service = MyService()

    override func viewDidLoad() {
        super.viewDidLoad()
        setupUI()
        service.fetchData()
    }

    private func setupUI() {
        let label = UILabel()
        label.text = "Hello"
        view.addSubview(label)
    }
}

extension ViewController {
    func handleAction() {
        print("Action handled")
        dismiss(animated: true)
    }
}
"#;
            let handler = SwiftHandler::new();

            // Test imports
            let imports = handler
                .parse_imports(source, Path::new("ViewController.swift"))
                .unwrap();
            assert_eq!(imports.len(), 3);

            // Test calls
            let defined_funcs = handler.collect_function_definitions(source);
            assert!(defined_funcs.contains("viewDidLoad"));
            assert!(defined_funcs.contains("setupUI"));
            assert!(defined_funcs.contains("handleAction"));
            assert!(defined_funcs.contains("ViewController"));

            let calls = handler.extract_calls_from_source(source, &defined_funcs);

            // viewDidLoad should call setupUI
            let view_did_load_calls = calls.get("ViewController.viewDidLoad").unwrap();
            assert!(view_did_load_calls.iter().any(|c| c.target == "setupUI"));
            assert!(view_did_load_calls
                .iter()
                .any(|c| c.target == "service.fetchData"));

            // handleAction should call print and dismiss
            let handle_action_calls = calls.get("ViewController.handleAction").unwrap();
            assert!(handle_action_calls.iter().any(|c| c.target == "print"));
            assert!(handle_action_calls.iter().any(|c| c.target == "dismiss"));
        }
    }

    // -------------------------------------------------------------------------
    // Public API Tests
    // -------------------------------------------------------------------------

    mod public_api_tests {
        use super::*;
        use crate::callgraph::languages::{extract_calls_for_language, parse_imports_for_language};

        #[test]
        fn test_extract_calls_via_public_api() {
            let source = r#"
func main() {
    print("hello")
    helper()
}

func helper() {}
"#;
            let calls =
                extract_calls_for_language("swift", Path::new("test.swift"), source).unwrap();

            let main_calls = calls.get("main").unwrap();
            assert!(main_calls.iter().any(|c| c.target == "print"));
            assert!(main_calls.iter().any(|c| c.target == "helper"));
        }

        #[test]
        fn test_parse_imports_via_public_api() {
            let source = r#"
import Foundation
import UIKit
@testable import MyApp
"#;
            let imports =
                parse_imports_for_language("swift", source, Path::new("test.swift")).unwrap();

            assert_eq!(imports.len(), 3);
            assert!(imports.iter().any(|i| i.module == "Foundation"));
            assert!(imports.iter().any(|i| i.module == "UIKit"));
            assert!(imports.iter().any(|i| i.module == "MyApp"));
        }
    }

    // -------------------------------------------------------------------------
    // New Tree-Sitter Feature Tests
    // -------------------------------------------------------------------------

    mod treesitter_tests {
        use super::*;

        #[test]
        fn test_property_initializer_calls() {
            let source = r#"
class MyClass {
    let x = Foo()
    var y = Bar.create()
}
"#;
            let calls = extract_calls(source);

            // Property initializer calls should be attributed to the class
            let class_calls = calls.get("MyClass").unwrap();
            assert!(class_calls.iter().any(|c| c.target == "Foo"));
            assert!(class_calls.iter().any(|c| c.target == "Bar.create"));
        }

        #[test]
        fn test_default_parameter_calls() {
            let source = r#"
func doStuff(x: Int = defaultVal()) {
    print("hello")
}
"#;
            let calls = extract_calls(source);
            let do_stuff_calls = calls.get("doStuff").unwrap();
            assert!(do_stuff_calls.iter().any(|c| c.target == "defaultVal"));
            assert!(do_stuff_calls.iter().any(|c| c.target == "print"));
        }

        #[test]
        fn test_global_file_level_calls() {
            let source = r#"
let globalInit = makeGlobal()
"#;
            let calls = extract_calls(source);
            let module_calls = calls.get("<module>").unwrap();
            assert!(module_calls.iter().any(|c| c.target == "makeGlobal"));
        }

        #[test]
        fn test_lazy_property_calls() {
            let source = r#"
class MyClass {
    lazy var computed = computeValue()
}
"#;
            let calls = extract_calls(source);
            let class_calls = calls.get("MyClass").unwrap();
            assert!(class_calls.iter().any(|c| c.target == "computeValue"));
        }

        #[test]
        fn test_closure_calls() {
            let source = r#"
func test() {
    let closure = { compute() }
}
"#;
            let calls = extract_calls(source);
            let test_calls = calls.get("test").unwrap();
            assert!(test_calls.iter().any(|c| c.target == "compute"));
        }

        #[test]
        fn test_guard_let_calls() {
            let source = r#"
func test() {
    guard let val = tryFoo() else { return }
    process(val)
}
"#;
            let calls = extract_calls(source);
            let test_calls = calls.get("test").unwrap();
            assert!(test_calls.iter().any(|c| c.target == "tryFoo"));
            assert!(test_calls.iter().any(|c| c.target == "process"));
        }

        #[test]
        fn test_if_let_calls() {
            let source = r#"
func test() {
    if let opt = tryBar() {
        handle()
    }
}
"#;
            let calls = extract_calls(source);
            let test_calls = calls.get("test").unwrap();
            assert!(test_calls.iter().any(|c| c.target == "tryBar"));
            assert!(test_calls.iter().any(|c| c.target == "handle"));
        }

        #[test]
        fn test_switch_case_calls() {
            let source = r#"
func test() {
    switch value {
    case .one:
        caseCall()
    default:
        defaultCall()
    }
}
"#;
            let calls = extract_calls(source);
            let test_calls = calls.get("test").unwrap();
            assert!(test_calls.iter().any(|c| c.target == "caseCall"));
            assert!(test_calls.iter().any(|c| c.target == "defaultCall"));
        }

        #[test]
        fn test_protocol_extension_calls() {
            let source = r#"
extension Array {
    func customFilter() {
        extCall()
        self.forEach { _ in }
    }
}
"#;
            let calls = extract_calls(source);
            let ext_calls = calls.get("Array.customFilter").unwrap();
            assert!(ext_calls.iter().any(|c| c.target == "extCall"));
        }

        #[test]
        fn test_trailing_closure_detected() {
            // Tree-sitter correctly detects trailing closure calls
            let source = r#"
func test() {
    arr.map { $0 * 2 }
    arr.filter { $0 > 0 }
}
"#;
            let calls = extract_calls(source);
            let test_calls = calls.get("test").unwrap();
            assert!(test_calls.iter().any(|c| c.target == "arr.map"));
            assert!(test_calls.iter().any(|c| c.target == "arr.filter"));
        }

        #[test]
        fn test_super_calls() {
            let source = r#"
class Child: Parent {
    override func viewDidLoad() {
        super.viewDidLoad()
        setup()
    }
}
"#;
            let calls = extract_calls(source);
            let vdl_calls = calls.get("Child.viewDidLoad").unwrap();
            assert!(vdl_calls.iter().any(|c| c.target == "super.viewDidLoad"));
            assert!(vdl_calls.iter().any(|c| c.target == "setup"));
        }

        #[test]
        fn test_self_method_calls() {
            let source = r#"
class Foo {
    func bar() {
        self.baz()
    }
    func baz() {}
}
"#;
            let calls = extract_calls(source);
            let bar_calls = calls.get("Foo.bar").unwrap();
            assert!(bar_calls.iter().any(|c| c.target == "self.baz"));
            assert_eq!(
                bar_calls
                    .iter()
                    .find(|c| c.target == "self.baz")
                    .unwrap()
                    .receiver,
                Some("self".to_string())
            );
        }

        #[test]
        fn test_init_declaration_calls() {
            let source = r#"
class Foo {
    convenience init(x: Int) {
        self.init()
        helper()
    }
    func helper() {}
}
"#;
            let calls = extract_calls(source);
            let init_calls = calls.get("Foo.init").unwrap();
            assert!(init_calls.iter().any(|c| c.target == "self.init"));
            assert!(init_calls.iter().any(|c| c.target == "helper"));
        }
    }
}
