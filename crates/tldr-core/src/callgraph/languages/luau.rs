//! Luau (Roblox) language handler for call graph analysis.
//!
//! This module provides Luau-specific call graph support using tree-sitter-luau.
//! Luau is a Lua variant used by Roblox with additional features like type annotations.
//!
//! # Import Patterns Supported
//!
//! | Pattern | ImportDef |
//! |---------|-----------|
//! | `require(script.Module)` | `{module: "script.Module", is_from: false}` |
//! | `require(script.Parent.Module)` | `{module: "script.Parent.Module", level: 1}` |
//! | `require("@pkg/json")` | `{module: "@pkg/json", is_from: false}` |
//! | `game:GetService("Players")` | `{module: "Players", is_namespace: true}` (Roblox service) |
//!
//! # Call Extraction
//!
//! - Direct calls: `func()` -> CallType::Direct or CallType::Intra
//! - Attribute calls: `Module.func()` -> CallType::Attr
//! - Method calls: `obj:method()` -> CallType::Method (colon syntax)
//! - Service calls: `game:GetService("X")` -> Special handling
//!
//! # Roblox-Specific Patterns
//!
//! - `script.Module` - Script-relative requires
//! - `script.Parent.Module` - Parent-relative requires
//! - `game:GetService("ServiceName")` - Roblox service access
//!
//! # Spec Reference
//!
//! See `migration/spec/callgraph-spec.md` for the full specification.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use tree_sitter::{Node, Parser, Tree};

use super::base::{get_node_text, walk_tree};
use super::{CallGraphLanguageSupport, ParseError};
use crate::callgraph::cross_file_types::{CallSite, CallType, ClassDef, FuncDef, ImportDef};

// =============================================================================
// Luau Handler
// =============================================================================

/// Luau (Roblox) language handler using tree-sitter-luau.
///
/// Supports:
/// - Import parsing (require, GetService)
/// - Call extraction (direct, attribute, method calls)
/// - Roblox script-relative paths
/// - Colon method syntax (`obj:method()`)
#[derive(Debug, Default)]
pub struct LuauHandler;

impl LuauHandler {
    /// Creates a new LuauHandler.
    pub fn new() -> Self {
        Self
    }

    /// Parse the source code into a tree-sitter Tree.
    fn parse_source(&self, source: &str) -> Result<Tree, ParseError> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_luau::LANGUAGE.into())
            .map_err(|e| ParseError::ParseFailed {
                file: std::path::PathBuf::new(),
                message: format!("Failed to set Luau language: {}", e),
            })?;

        parser
            .parse(source, None)
            .ok_or_else(|| ParseError::ParseFailed {
                file: std::path::PathBuf::new(),
                message: "Parser returned None".to_string(),
            })
    }

    /// Extract string content from a Luau string node.
    ///
    /// Handles both string_content child nodes and manual quote stripping.
    fn extract_luau_string(&self, node: &Node, source: &[u8]) -> Option<String> {
        // Check for string_content child
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if child.kind() == "string_content" {
                    return Some(get_node_text(&child, source).to_string());
                }
            }
        }

        // Fallback: strip quotes manually
        let text = get_node_text(node, source);
        if (text.starts_with('"') && text.ends_with('"'))
            || (text.starts_with('\'') && text.ends_with('\''))
        {
            Some(text[1..text.len() - 1].to_string())
        } else {
            Some(text.to_string())
        }
    }

    /// Parse a single Luau require or GetService call.
    ///
    /// Handles:
    /// - `require(script.Utils)`
    /// - `require(script.Parent.Module)`
    /// - `require("@pkg/json")`
    /// - `game:GetService("Players")`
    fn parse_luau_import_node(&self, node: &Node, source: &[u8]) -> Option<ImportDef> {
        // Check for method call (GetService pattern)
        let mut method_expr: Option<Node> = None;
        let mut func_name: Option<String> = None;
        let mut arguments: Option<Node> = None;

        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                match child.kind() {
                    "method_index_expression" => {
                        method_expr = Some(child);
                    }
                    "identifier" => {
                        if func_name.is_none() {
                            func_name = Some(get_node_text(&child, source).to_string());
                        }
                    }
                    "arguments" => {
                        arguments = Some(child);
                    }
                    _ => {}
                }
            }
        }

        // Handle GetService pattern: game:GetService("ServiceName")
        if let Some(method_node) = method_expr {
            let mut method_name: Option<String> = None;
            for i in 0..method_node.child_count() {
                if let Some(child) = method_node.child(i) {
                    if child.kind() == "identifier" {
                        method_name = Some(get_node_text(&child, source).to_string());
                    }
                }
            }

            if method_name.as_deref() == Some("GetService") {
                if let Some(args) = arguments {
                    for i in 0..args.child_count() {
                        if let Some(arg_child) = args.child(i) {
                            if arg_child.kind() == "string" {
                                if let Some(service_name) =
                                    self.extract_luau_string(&arg_child, source)
                                {
                                    // Create ImportDef for Roblox service
                                    let mut imp = ImportDef::simple_import(service_name);
                                    imp.is_namespace = true; // Mark as service
                                    return Some(imp);
                                }
                            }
                        }
                    }
                }
            }
            return None;
        }

        // Handle require pattern
        if func_name.as_deref() != Some("require") {
            return None;
        }

        let args = arguments?;

        // Get the module argument - can be dot_index_expression or string
        for i in 0..args.child_count() {
            if let Some(arg_child) = args.child(i) {
                match arg_child.kind() {
                    "dot_index_expression" => {
                        // require(script.Utils) or require(script.Parent.Module)
                        let module_path = get_node_text(&arg_child, source).to_string();

                        // Check if it's a relative require (contains "Parent")
                        let level = if module_path.contains("Parent") {
                            module_path.matches("Parent").count() as u8
                        } else {
                            0
                        };

                        if level > 0 {
                            return Some(ImportDef::relative_import(module_path, vec![], level));
                        } else {
                            return Some(ImportDef::simple_import(module_path));
                        }
                    }
                    "string" => {
                        // require("@pkg/json")
                        if let Some(module_name) = self.extract_luau_string(&arg_child, source) {
                            return Some(ImportDef::simple_import(module_name));
                        }
                    }
                    "identifier" => {
                        // require(someVar) - variable reference
                        let module_name = get_node_text(&arg_child, source).to_string();
                        return Some(ImportDef::simple_import(module_name));
                    }
                    _ => {}
                }
            }
        }

        None
    }

    /// Collect all function definitions from the AST.
    fn collect_definitions(&self, tree: &Tree, source: &[u8]) -> HashSet<String> {
        let mut functions = HashSet::new();

        for node in walk_tree(tree.root_node()) {
            match node.kind() {
                "function_declaration" => {
                    // Get function name from identifier or dot/method expression
                    for i in 0..node.child_count() {
                        if let Some(child) = node.child(i) {
                            match child.kind() {
                                "identifier" => {
                                    functions.insert(get_node_text(&child, source).to_string());
                                    break;
                                }
                                "dot_index_expression" | "method_index_expression" => {
                                    // function M.foo() or function M:foo() -> extract "foo"
                                    let identifiers: Vec<_> = (0..child.child_count())
                                        .filter_map(|j| child.child(j))
                                        .filter(|c| c.kind() == "identifier")
                                        .collect();
                                    if identifiers.len() >= 2 {
                                        let last = identifiers.last().unwrap();
                                        functions.insert(get_node_text(last, source).to_string());
                                    }
                                    break;
                                }
                                _ => {}
                            }
                        }
                    }
                }
                "variable_declaration" => {
                    // Handle variable declarations with function values
                    // local foo = function() end
                    self.collect_func_from_var_decl(&node, source, &mut functions);
                }
                _ => {}
            }
        }

        functions
    }

    /// Extract function name from variable declaration with function value.
    fn collect_func_from_var_decl(
        &self,
        node: &Node,
        source: &[u8],
        functions: &mut HashSet<String>,
    ) {
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if child.kind() == "assignment_statement" {
                    let mut var_list: Option<Node> = None;
                    let mut expr_list: Option<Node> = None;

                    for j in 0..child.child_count() {
                        if let Some(subchild) = child.child(j) {
                            match subchild.kind() {
                                "variable_list" => var_list = Some(subchild),
                                "expression_list" => expr_list = Some(subchild),
                                _ => {}
                            }
                        }
                    }

                    if let (Some(vl), Some(el)) = (var_list, expr_list) {
                        // Check if expression_list contains a function_definition
                        let has_func = (0..el.child_count())
                            .filter_map(|k| el.child(k))
                            .any(|c| c.kind() == "function_definition");

                        if has_func {
                            // Get variable name
                            for k in 0..vl.child_count() {
                                if let Some(var_child) = vl.child(k) {
                                    if var_child.kind() == "identifier" {
                                        functions
                                            .insert(get_node_text(&var_child, source).to_string());
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Extract calls from a function body node.
    fn extract_calls_from_node(
        &self,
        node: &Node,
        source: &[u8],
        defined_funcs: &HashSet<String>,
        caller: &str,
    ) -> Vec<CallSite> {
        let mut calls = Vec::new();

        for child in walk_tree(*node) {
            if child.kind() == "function_call" {
                let line = child.start_position().row as u32 + 1;

                // Determine call type and target
                let mut call_type = CallType::Direct;
                let mut call_target: Option<String> = None;
                let mut receiver: Option<String> = None;

                for i in 0..child.child_count() {
                    if let Some(c) = child.child(i) {
                        match c.kind() {
                            "identifier" => {
                                // Simple call: foo()
                                let name = get_node_text(&c, source).to_string();
                                if defined_funcs.contains(&name) {
                                    call_type = CallType::Intra;
                                } else {
                                    call_type = CallType::Direct;
                                }
                                call_target = Some(name);
                                break;
                            }
                            "dot_index_expression" => {
                                // Module.func() or obj.method()
                                let identifiers: Vec<_> = (0..c.child_count())
                                    .filter_map(|j| c.child(j))
                                    .filter(|n| n.kind() == "identifier")
                                    .collect();

                                if identifiers.len() >= 2 {
                                    let obj_name =
                                        get_node_text(identifiers.first().unwrap(), source);
                                    let method_name =
                                        get_node_text(identifiers.last().unwrap(), source);

                                    receiver = Some(obj_name.to_string());

                                    if defined_funcs.contains(method_name) {
                                        call_type = CallType::Intra;
                                        call_target = Some(method_name.to_string());
                                    } else {
                                        call_type = CallType::Attr;
                                        call_target = Some(format!("{}.{}", obj_name, method_name));
                                    }
                                } else if identifiers.len() == 1 {
                                    call_target = Some(get_node_text(&c, source).to_string());
                                    call_type = CallType::Attr;
                                    receiver = Some(get_node_text(&c, source).to_string());
                                }
                                break;
                            }
                            "method_index_expression" => {
                                // obj:method() - colon syntax for method call
                                // Also handles complex receivers:
                                //   button.Activated:Connect() -> dot_index_expression as first child
                                //   obj:Method1():Method2()    -> function_call as first child
                                let identifiers: Vec<_> = (0..c.child_count())
                                    .filter_map(|j| c.child(j))
                                    .filter(|n| n.kind() == "identifier")
                                    .collect();

                                if identifiers.len() >= 2 {
                                    // Simple case: obj:method() with both obj and method as identifiers
                                    let obj_name =
                                        get_node_text(identifiers.first().unwrap(), source);
                                    let method_name =
                                        get_node_text(identifiers.last().unwrap(), source);

                                    receiver = Some(obj_name.to_string());

                                    // Check for game:GetService pattern
                                    if obj_name == "game" && method_name == "GetService" {
                                        // Extract service name from arguments
                                        if let Some(service_name) =
                                            self.extract_service_name(&child, source)
                                        {
                                            call_type = CallType::Attr;
                                            call_target =
                                                Some(format!("GetService:{}", service_name));
                                            receiver = Some("game".to_string());
                                        } else {
                                            call_type = CallType::Method;
                                            call_target =
                                                Some(format!("{}:{}", obj_name, method_name));
                                        }
                                    } else if defined_funcs.contains(method_name) {
                                        call_type = CallType::Intra;
                                        call_target = Some(method_name.to_string());
                                    } else {
                                        call_type = CallType::Method;
                                        call_target = Some(format!("{}:{}", obj_name, method_name));
                                    }
                                } else if identifiers.len() == 1 {
                                    // Complex receiver: the first child is not an identifier
                                    // e.g. button.Activated:Connect() (dot_index_expression)
                                    //      obj:Method1():Method2()    (function_call)
                                    let method_name =
                                        get_node_text(identifiers.first().unwrap(), source)
                                            .to_string();

                                    // Get the receiver expression (first non-punctuation child)
                                    let receiver_node = (0..c.child_count())
                                        .filter_map(|j| c.child(j))
                                        .find(|n| n.kind() != "identifier" && n.kind() != ":");

                                    let recv_text = receiver_node
                                        .map(|n| get_node_text(&n, source).to_string());
                                    receiver = recv_text;

                                    call_type = CallType::Method;
                                    if let Some(ref recv) = receiver {
                                        call_target = Some(format!("{}:{}", recv, method_name));
                                    } else {
                                        call_target = Some(method_name);
                                    }
                                }
                                break;
                            }
                            _ => {}
                        }
                    }
                }

                // Skip require calls (imports)
                if let Some(ref target) = call_target {
                    if target == "require" {
                        continue;
                    }
                }

                // Create CallSite if we found a target
                if let Some(target) = call_target {
                    match call_type {
                        CallType::Intra | CallType::Direct => {
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
                        CallType::Attr | CallType::Method => {
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
                        _ => {
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

    /// Extract service name from GetService call arguments.
    fn extract_service_name(&self, call_node: &Node, source: &[u8]) -> Option<String> {
        for i in 0..call_node.child_count() {
            if let Some(child) = call_node.child(i) {
                if child.kind() == "arguments" {
                    for j in 0..child.child_count() {
                        if let Some(arg) = child.child(j) {
                            if arg.kind() == "string" {
                                return self.extract_luau_string(&arg, source);
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Get function name from a function declaration node.
    fn get_function_name(&self, node: &Node, source: &[u8]) -> Option<String> {
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                match child.kind() {
                    "identifier" => {
                        return Some(get_node_text(&child, source).to_string());
                    }
                    "dot_index_expression" | "method_index_expression" => {
                        let identifiers: Vec<_> = (0..child.child_count())
                            .filter_map(|j| child.child(j))
                            .filter(|c| c.kind() == "identifier")
                            .collect();
                        if identifiers.len() >= 2 {
                            return Some(
                                get_node_text(identifiers.last().unwrap(), source).to_string(),
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
        None
    }

    /// Get function name from variable declaration with function value.
    fn get_func_name_from_var_decl<'a>(
        &self,
        node: &Node<'a>,
        source: &[u8],
    ) -> Option<(String, Node<'a>)> {
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if child.kind() == "assignment_statement" {
                    let mut var_list: Option<Node> = None;
                    let mut expr_list: Option<Node> = None;

                    for j in 0..child.child_count() {
                        if let Some(subchild) = child.child(j) {
                            match subchild.kind() {
                                "variable_list" => var_list = Some(subchild),
                                "expression_list" => expr_list = Some(subchild),
                                _ => {}
                            }
                        }
                    }

                    if let (Some(vl), Some(el)) = (var_list, expr_list) {
                        // Find function_definition in expression_list
                        let mut func_node: Option<Node> = None;
                        for k in 0..el.child_count() {
                            if let Some(ec) = el.child(k) {
                                if ec.kind() == "function_definition" {
                                    func_node = Some(ec);
                                    break;
                                }
                            }
                        }

                        if let Some(fn_node) = func_node {
                            // Get variable name
                            for k in 0..vl.child_count() {
                                if let Some(var_child) = vl.child(k) {
                                    if var_child.kind() == "identifier" {
                                        return Some((
                                            get_node_text(&var_child, source).to_string(),
                                            fn_node,
                                        ));
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
}

impl CallGraphLanguageSupport for LuauHandler {
    fn name(&self) -> &str {
        "luau"
    }

    fn extensions(&self) -> &[&str] {
        &[".luau", ".lua"]
    }

    fn parse_imports(&self, source: &str, _path: &Path) -> Result<Vec<ImportDef>, ParseError> {
        let tree = self.parse_source(source)?;
        let source_bytes = source.as_bytes();
        let mut imports = Vec::new();

        for node in walk_tree(tree.root_node()) {
            if node.kind() == "function_call" {
                if let Some(imp) = self.parse_luau_import_node(&node, source_bytes) {
                    imports.push(imp);
                }
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
        let defined_funcs = self.collect_definitions(tree, source_bytes);
        let mut calls_by_func: HashMap<String, Vec<CallSite>> = HashMap::new();

        // Process all function declarations and variable declarations with function values
        fn process_node(
            node: Node,
            source: &[u8],
            defined_funcs: &HashSet<String>,
            calls_by_func: &mut HashMap<String, Vec<CallSite>>,
            handler: &LuauHandler,
        ) {
            match node.kind() {
                "function_declaration" => {
                    if let Some(func_name) = handler.get_function_name(&node, source) {
                        let calls = handler.extract_calls_from_node(
                            &node,
                            source,
                            defined_funcs,
                            &func_name,
                        );
                        if !calls.is_empty() {
                            calls_by_func.insert(func_name, calls);
                        }
                    }
                }
                "variable_declaration" => {
                    if let Some((func_name, func_node)) =
                        handler.get_func_name_from_var_decl(&node, source)
                    {
                        let calls = handler.extract_calls_from_node(
                            &func_node,
                            source,
                            defined_funcs,
                            &func_name,
                        );
                        if !calls.is_empty() {
                            calls_by_func.insert(func_name, calls);
                        }
                    }
                }
                _ => {}
            }

            // Recurse into children
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    process_node(child, source, defined_funcs, calls_by_func, handler);
                }
            }
        }

        process_node(
            tree.root_node(),
            source_bytes,
            &defined_funcs,
            &mut calls_by_func,
            self,
        );

        // Extract module-level calls into synthetic <module> function
        let mut module_calls = Vec::new();
        for i in 0..tree.root_node().child_count() {
            if let Some(child) = tree.root_node().child(i) {
                // Skip function declarations and variable declarations with functions
                if child.kind() == "function_declaration" {
                    continue;
                }
                if child.kind() == "variable_declaration" {
                    // Check if it's a function definition
                    let is_func_def =
                        (0..child.child_count())
                            .filter_map(|j| child.child(j))
                            .any(|c| {
                                c.kind() == "assignment_statement"
                                    && (0..c.child_count()).filter_map(|k| c.child(k)).any(|sc| {
                                        sc.kind() == "expression_list"
                                            && (0..sc.child_count())
                                                .filter_map(|l| sc.child(l))
                                                .any(|ec| ec.kind() == "function_definition")
                                    })
                            });
                    if is_func_def {
                        continue;
                    }
                }

                let calls =
                    self.extract_calls_from_node(&child, source_bytes, &defined_funcs, "<module>");
                module_calls.extend(calls);
            }
        }

        if !module_calls.is_empty() {
            calls_by_func.insert("<module>".to_string(), module_calls);
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
        // Luau has no classes

        for node in walk_tree(tree.root_node()) {
            match node.kind() {
                "function_declaration" => {
                    let line = node.start_position().row as u32 + 1;
                    let end_line = node.end_position().row as u32 + 1;

                    for i in 0..node.child_count() {
                        if let Some(child) = node.child(i) {
                            match child.kind() {
                                "identifier" => {
                                    let name = get_node_text(&child, source_bytes).to_string();
                                    funcs.push(FuncDef::function(name, line, end_line));
                                    break;
                                }
                                "dot_index_expression" | "method_index_expression" => {
                                    // function M.foo() or function M:foo() -> extract last identifier
                                    let identifiers: Vec<_> = (0..child.child_count())
                                        .filter_map(|j| child.child(j))
                                        .filter(|c| c.kind() == "identifier")
                                        .collect();
                                    if identifiers.len() >= 2 {
                                        let last = identifiers.last().unwrap();
                                        let name = get_node_text(last, source_bytes).to_string();
                                        funcs.push(FuncDef::function(name, line, end_line));
                                    }
                                    break;
                                }
                                _ => {}
                            }
                        }
                    }
                }
                "variable_declaration" => {
                    // Handle: local foo = function() end
                    let line = node.start_position().row as u32 + 1;
                    let end_line = node.end_position().row as u32 + 1;
                    let mut var_names: Vec<String> = Vec::new();
                    let mut has_function = false;

                    for i in 0..node.child_count() {
                        if let Some(child) = node.child(i) {
                            if child.kind() == "assignment_statement" {
                                for j in 0..child.child_count() {
                                    if let Some(subchild) = child.child(j) {
                                        if subchild.kind() == "variable_list" {
                                            for k in 0..subchild.child_count() {
                                                if let Some(var) = subchild.child(k) {
                                                    if var.kind() == "identifier" {
                                                        var_names.push(
                                                            get_node_text(&var, source_bytes)
                                                                .to_string(),
                                                        );
                                                    }
                                                }
                                            }
                                        }
                                        if subchild.kind() == "expression_list" {
                                            for k in 0..subchild.child_count() {
                                                if let Some(expr) = subchild.child(k) {
                                                    if expr.kind() == "function_definition" {
                                                        has_function = true;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if has_function {
                        for name in var_names {
                            funcs.push(FuncDef::function(name, line, end_line));
                        }
                    }
                }
                _ => {}
            }
        }

        Ok((funcs, Vec::new()))
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_imports(source: &str) -> Vec<ImportDef> {
        let handler = LuauHandler::new();
        handler
            .parse_imports(source, Path::new("test.luau"))
            .unwrap()
    }

    fn extract_calls(source: &str) -> HashMap<String, Vec<CallSite>> {
        let handler = LuauHandler::new();
        let tree = handler.parse_source(source).unwrap();
        handler
            .extract_calls(Path::new("test.luau"), source, &tree)
            .unwrap()
    }

    // -------------------------------------------------------------------------
    // Import Parsing Tests
    // -------------------------------------------------------------------------

    mod import_tests {
        use super::*;

        #[test]
        fn test_parse_require_script() {
            let imports = parse_imports("local Utils = require(script.Utils)");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].module, "script.Utils");
            assert!(!imports[0].is_relative());
        }

        #[test]
        fn test_parse_require_parent() {
            let imports = parse_imports("local Module = require(script.Parent.Module)");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].module, "script.Parent.Module");
            assert!(imports[0].is_relative());
            assert_eq!(imports[0].level, 1);
        }

        #[test]
        fn test_parse_require_parent_parent() {
            let imports = parse_imports("local M = require(script.Parent.Parent.Module)");
            assert_eq!(imports.len(), 1);
            assert!(imports[0].module.contains("Parent.Parent"));
            assert!(imports[0].is_relative());
            assert_eq!(imports[0].level, 2);
        }

        #[test]
        fn test_parse_get_service() {
            let imports = parse_imports("local Players = game:GetService(\"Players\")");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].module, "Players");
            assert!(imports[0].is_namespace); // Marked as service
        }

        #[test]
        fn test_parse_regular_require() {
            let imports = parse_imports("local json = require(\"@pkg/json\")");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].module, "@pkg/json");
            assert!(!imports[0].is_relative());
        }

        #[test]
        fn test_parse_multiple_imports() {
            let source = r#"
local Utils = require(script.Utils)
local Players = game:GetService("Players")
local json = require("@pkg/json")
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
        fn test_extract_calls_simple() {
            let source = r#"
local function main()
    print("hello")
    helper()
end
"#;
            let calls = extract_calls(source);
            let main_calls = calls.get("main").unwrap();
            assert!(main_calls.iter().any(|c| c.target == "print"));
            assert!(main_calls.iter().any(|c| c.target == "helper"));
        }

        #[test]
        fn test_extract_calls_intra_file() {
            let source = r#"
local function helper()
    return "help"
end

local function main()
    helper()
end
"#;
            let calls = extract_calls(source);
            let main_calls = calls.get("main").unwrap();
            let helper_call = main_calls.iter().find(|c| c.target == "helper").unwrap();
            assert_eq!(helper_call.call_type, CallType::Intra);
        }

        #[test]
        fn test_extract_calls_method() {
            let source = r#"
local function process()
    local player = Players:GetPlayerByUserId(userId)
    player:Kick()
end
"#;
            let calls = extract_calls(source);
            let process_calls = calls.get("process").unwrap();
            assert!(process_calls
                .iter()
                .any(|c| c.target.contains("GetPlayerByUserId")));
            assert!(process_calls.iter().any(|c| c.target.contains("Kick")));
        }

        #[test]
        fn test_extract_calls_attr() {
            let source = r#"
local function init()
    Utils.setup()
    Module.configure()
end
"#;
            let calls = extract_calls(source);
            let init_calls = calls.get("init").unwrap();
            assert!(init_calls.iter().any(|c| c.target.contains("Utils.setup")));
            assert!(init_calls
                .iter()
                .any(|c| c.target.contains("Module.configure")));
        }

        #[test]
        fn test_extract_calls_get_service() {
            let source = r#"
local function setup()
    local Players = game:GetService("Players")
end
"#;
            let calls = extract_calls(source);
            let setup_calls = calls.get("setup").unwrap();
            assert!(setup_calls
                .iter()
                .any(|c| c.target.contains("GetService:Players")));
        }

        #[test]
        fn test_extract_calls_module_level() {
            let source = r#"
local function helper()
    return "help"
end

-- Module-level call
local result = helper()
"#;
            let calls = extract_calls(source);
            assert!(calls.contains_key("<module>"));
            let module_calls = calls.get("<module>").unwrap();
            assert!(module_calls.iter().any(|c| c.target == "helper"));
        }

        #[test]
        fn test_extract_calls_var_function() {
            let source = r#"
local process = function()
    doWork()
end
"#;
            let calls = extract_calls(source);
            let process_calls = calls.get("process").unwrap();
            assert!(process_calls.iter().any(|c| c.target == "doWork"));
        }
    }

    // -------------------------------------------------------------------------
    // Handler Trait Tests
    // -------------------------------------------------------------------------

    mod trait_tests {
        use super::*;

        #[test]
        fn test_handler_name() {
            let handler = LuauHandler::new();
            assert_eq!(handler.name(), "luau");
        }

        #[test]
        fn test_handler_extensions() {
            let handler = LuauHandler::new();
            let exts = handler.extensions();
            assert!(exts.contains(&".luau"));
            assert!(exts.contains(&".lua"));
        }

        #[test]
        fn test_handler_supports() {
            let handler = LuauHandler::new();
            assert!(handler.supports("luau"));
            assert!(handler.supports("Luau"));
            assert!(handler.supports("LUAU"));
            assert!(!handler.supports("lua")); // Only "luau" name, not "lua"
            assert!(!handler.supports("python"));
        }

        #[test]
        fn test_handler_supports_extension() {
            let handler = LuauHandler::new();
            assert!(handler.supports_extension(".luau"));
            assert!(handler.supports_extension(".lua"));
            assert!(handler.supports_extension(".LUAU"));
            assert!(!handler.supports_extension(".py"));
        }
    }

    // -------------------------------------------------------------------------
    // Pattern Parity Tests (new)
    // -------------------------------------------------------------------------

    mod parity_tests {
        use super::*;

        #[test]
        fn test_self_method_calls() {
            // self:method() inside function Foo:bar() should be captured
            let source = r#"
function Foo:bar()
    self:method()
    self:other()
end
"#;
            let calls = extract_calls(source);
            let bar_calls = calls.get("bar").unwrap();
            assert!(
                bar_calls.iter().any(|c| c.target.contains("method")),
                "Expected self:method() call, got: {:?}",
                bar_calls
            );
            assert!(
                bar_calls.iter().any(|c| c.target.contains("other")),
                "Expected self:other() call, got: {:?}",
                bar_calls
            );
            // Verify receiver is "self"
            let method_call = bar_calls
                .iter()
                .find(|c| c.target.contains("method"))
                .unwrap();
            assert_eq!(method_call.receiver, Some("self".to_string()));
            assert_eq!(method_call.call_type, CallType::Method);
        }

        #[test]
        fn test_chained_method_calls() {
            // obj:Method1():Method2() should capture both calls
            let source = r#"
function test()
    obj:Method1():Method2()
end
"#;
            let calls = extract_calls(source);
            let test_calls = calls.get("test").unwrap();
            assert!(
                test_calls.iter().any(|c| c.target.contains("Method1")),
                "Expected obj:Method1() call, got: {:?}",
                test_calls
            );
            assert!(
                test_calls.iter().any(|c| c.target.contains("Method2")),
                "Expected :Method2() chained call, got: {:?}",
                test_calls
            );
        }

        #[test]
        fn test_instance_new_pattern() {
            // Instance.new("Part") is a Roblox constructor pattern
            let source = r#"
function test()
    local part = Instance.new("Part")
end
"#;
            let calls = extract_calls(source);
            let test_calls = calls.get("test").unwrap();
            assert!(
                test_calls.iter().any(|c| c.target.contains("Instance.new")),
                "Expected Instance.new() call, got: {:?}",
                test_calls
            );
        }

        #[test]
        fn test_connect_pattern() {
            // button.Activated:Connect(callback) is a Roblox event pattern
            // The method_index_expression has dot_index_expression as first child
            let source = r#"
function test()
    button.Activated:Connect(function()
        print("clicked")
    end)
end
"#;
            let calls = extract_calls(source);
            let test_calls = calls.get("test").unwrap();
            assert!(
                test_calls.iter().any(|c| c.target.contains("Connect")),
                "Expected :Connect() call, got: {:?}",
                test_calls
            );
        }

        #[test]
        fn test_waitforchild_pattern() {
            // player:WaitForChild("PlayerGui") is a common Roblox pattern
            let source = r#"
function test()
    local gui = player:WaitForChild("PlayerGui")
end
"#;
            let calls = extract_calls(source);
            let test_calls = calls.get("test").unwrap();
            assert!(
                test_calls.iter().any(|c| c.target.contains("WaitForChild")),
                "Expected :WaitForChild() call, got: {:?}",
                test_calls
            );
        }

        #[test]
        fn test_table_constructor_calls() {
            // Calls inside table constructors should be captured
            let source = r#"
function test()
    local t = { field = func(), other = bar() }
end
"#;
            let calls = extract_calls(source);
            let test_calls = calls.get("test").unwrap();
            assert!(
                test_calls.iter().any(|c| c.target == "func"),
                "Expected func() call inside table constructor, got: {:?}",
                test_calls
            );
            assert!(
                test_calls.iter().any(|c| c.target == "bar"),
                "Expected bar() call inside table constructor, got: {:?}",
                test_calls
            );
        }

        #[test]
        fn test_nested_function_calls() {
            // foo(bar()) should capture both calls
            let source = r#"
function test()
    foo(bar())
end
"#;
            let calls = extract_calls(source);
            let test_calls = calls.get("test").unwrap();
            assert!(
                test_calls.iter().any(|c| c.target == "foo"),
                "Expected foo() call, got: {:?}",
                test_calls
            );
            assert!(
                test_calls.iter().any(|c| c.target == "bar"),
                "Expected bar() nested call, got: {:?}",
                test_calls
            );
        }

        #[test]
        fn test_type_annotation_calls() {
            // local x: Foo = Foo.new() - the call should still be captured
            let source = r#"
function test()
    local x: Foo = Foo.new()
end
"#;
            let calls = extract_calls(source);
            let test_calls = calls.get("test").unwrap();
            assert!(
                test_calls.iter().any(|c| c.target.contains("Foo.new")),
                "Expected Foo.new() call with type annotation, got: {:?}",
                test_calls
            );
        }

        #[test]
        fn test_method_definition_tracked() {
            // function Foo:bar() should be tracked as a definition named "bar"
            let source = r#"
function Foo:bar()
    print("hello")
end

function Foo:baz()
    bar()
end
"#;
            let handler = LuauHandler::new();
            let tree = handler.parse_source(source).unwrap();
            let (funcs, _classes) = handler
                .extract_definitions(source, Path::new("test.luau"), &tree)
                .unwrap();
            let func_names: Vec<&str> = funcs.iter().map(|f| f.name.as_str()).collect();
            assert!(
                func_names.contains(&"bar"),
                "Expected 'bar' definition from function Foo:bar(), got: {:?}",
                func_names
            );
            assert!(
                func_names.contains(&"baz"),
                "Expected 'baz' definition from function Foo:baz(), got: {:?}",
                func_names
            );
        }

        #[test]
        fn test_complex_roblox_pattern() {
            // Real-world Roblox pattern combining multiple features
            let source = r#"
local Players = game:GetService("Players")
local ReplicatedStorage = game:GetService("ReplicatedStorage")

function PlayerModule:OnPlayerAdded(player)
    local character = player.Character or player.CharacterAdded:Wait()
    local humanoid = character:WaitForChild("Humanoid")
    humanoid.Died:Connect(function()
        self:HandleDeath(player)
    end)
end
"#;
            let calls = extract_calls(source);

            // Module-level GetService calls
            assert!(
                calls.contains_key("<module>"),
                "Expected <module> key for module-level calls"
            );
            let module_calls = calls.get("<module>").unwrap();
            assert!(
                module_calls.iter().any(|c| c.target.contains("GetService")),
                "Expected GetService at module level, got: {:?}",
                module_calls
            );

            // Method body calls
            let method_calls = calls.get("OnPlayerAdded").unwrap();
            assert!(
                method_calls
                    .iter()
                    .any(|c| c.target.contains("WaitForChild")),
                "Expected WaitForChild call, got: {:?}",
                method_calls
            );
        }

        #[test]
        fn test_dot_method_index_connect() {
            // event:Connect where event is accessed via dot notation
            // In AST, method_index_expression has dot_index_expression as first child
            let source = r#"
function test()
    workspace.ChildAdded:Connect(onChildAdded)
    game.Players.PlayerAdded:Connect(onPlayerAdded)
end
"#;
            let calls = extract_calls(source);
            let test_calls = calls
                .get("test")
                .expect("Expected 'test' function to have calls");
            // Should find both Connect calls
            let connect_calls: Vec<_> = test_calls
                .iter()
                .filter(|c| c.target.contains("Connect"))
                .collect();
            assert!(
                connect_calls.len() >= 2,
                "Expected 2 Connect calls, got: {:?}",
                test_calls
            );
        }
    }
}
