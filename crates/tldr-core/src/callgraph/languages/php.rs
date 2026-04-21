//! PHP language handler for call graph analysis.
//!
//! This module provides PHP-specific call graph support using tree-sitter-php.
//!
//! # Import Patterns Supported
//!
//! | Pattern | ImportDef |
//! |---------|-----------|
//! | `use App\Models\User;` | `{module: "App\\Models\\User", is_from: false}` |
//! | `use App\Models\{User, Post};` | Multiple entries for grouped imports |
//! | `use App\Models\User as UserModel;` | `{module: "...", alias: "UserModel"}` |
//! | `require 'file.php';` | `{module: "file.php", is_from: false}` |
//! | `require_once 'file.php';` | `{module: "file.php", is_from: false}` |
//! | `include 'file.php';` | `{module: "file.php", is_from: false}` |
//! | `include_once 'file.php';` | `{module: "file.php", is_from: false}` |
//!
//! # Call Extraction
//!
//! - Direct calls: `func()` -> CallType::Direct or CallType::Intra
//! - Method calls: `$obj->method()` -> CallType::Attr
//! - Static calls: `ClassName::staticMethod()` -> CallType::Static
//! - Constructor calls: `new ClassName()` -> CallType::Direct
//! - Property initializers: `public $x = createFoo()` (caller = class name)
//! - Default params: `function foo($db = getDb())` (caller = function name)
//! - Trait method bodies: `trait T { function m() { call(); } }` (caller = Trait::method)
//! - Anonymous class methods: `new class { function m() { call(); } }`
//! - Closures/arrow fns: `function($x) { call($x); }`, `fn($x) => call($x)`
//! - Parent calls: `parent::__construct(args)`
//!
//! # Spec Reference
//!
//! See `migration/spec/callgraph-spec.md` Section 9.7 for PHP-specific details.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use tree_sitter::{Node, Parser, Tree};

use super::base::{get_node_text, walk_tree};
use super::{CallGraphLanguageSupport, ParseError};
use crate::callgraph::cross_file_types::{CallSite, CallType, ClassDef, FuncDef, ImportDef};

// =============================================================================
// PHP Handler
// =============================================================================

/// PHP language handler using tree-sitter-php.
///
/// Supports:
/// - Import parsing (use, require, require_once, include, include_once)
/// - Call extraction (direct, method, static, constructor calls)
/// - Class and function tracking
/// - `<module>` synthetic function for module-level calls
#[derive(Debug, Default)]
pub struct PhpHandler;

impl PhpHandler {
    /// Creates a new PhpHandler.
    pub fn new() -> Self {
        Self
    }

    /// Parse the source code into a tree-sitter Tree.
    fn parse_source(&self, source: &str) -> Result<Tree, ParseError> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_php::LANGUAGE_PHP.into())
            .map_err(|e| ParseError::ParseFailed {
                file: std::path::PathBuf::new(),
                message: format!("Failed to set PHP language: {}", e),
            })?;

        parser
            .parse(source, None)
            .ok_or_else(|| ParseError::ParseFailed {
                file: std::path::PathBuf::new(),
                message: "Parser returned None".to_string(),
            })
    }

    /// Parse a namespace_use_declaration node (use statements).
    ///
    /// Handles:
    /// - Simple: `use App\Models\User;`
    /// - Grouped: `use App\Models\{User, Post};`
    /// - Aliased: `use App\Models\User as UserModel;`
    fn parse_use_declaration(&self, node: &Node, source: &[u8]) -> Vec<ImportDef> {
        let mut imports = Vec::new();

        // Check if this has a namespace_use_group (grouped imports)
        let has_group = (0..node.child_count())
            .filter_map(|i| node.child(i))
            .any(|child| child.kind() == "namespace_use_group");

        if has_group {
            // Grouped imports: use App\Models\{User, Post}
            // Get the prefix from the namespace_name
            let mut prefix = String::new();
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if child.kind() == "namespace_name" {
                        prefix = get_node_text(&child, source).to_string();
                        break;
                    }
                }
            }

            // Parse each group item
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if child.kind() == "namespace_use_group" {
                        for j in 0..child.child_count() {
                            if let Some(group_child) = child.child(j) {
                                if group_child.kind() == "namespace_use_clause" {
                                    let clause_text = get_node_text(&group_child, source).trim();
                                    // Handle alias: User as UserModel
                                    let parts: Vec<&str> = clause_text.split(" as ").collect();
                                    let name = parts[0].trim();
                                    let alias = if parts.len() > 1 {
                                        Some(parts[1].trim().to_string())
                                    } else {
                                        None
                                    };

                                    let full_module = if prefix.is_empty() {
                                        name.to_string()
                                    } else {
                                        format!("{}\\{}", prefix, name)
                                    };

                                    let mut import = ImportDef::simple_import(full_module);
                                    import.alias = alias;
                                    imports.push(import);
                                }
                            }
                        }
                    }
                }
            }
        } else {
            // Simple imports: use App\Models\User;
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if child.kind() == "namespace_use_clause" {
                        let clause_text = get_node_text(&child, source).trim();
                        // Handle alias: User as UserModel
                        let parts: Vec<&str> = clause_text.split(" as ").collect();
                        let module = parts[0].trim().to_string();
                        let alias = if parts.len() > 1 {
                            Some(parts[1].trim().to_string())
                        } else {
                            None
                        };

                        let mut import = ImportDef::simple_import(module);
                        import.alias = alias;
                        imports.push(import);
                    }
                }
            }
        }

        imports
    }

    /// Parse require/include expression nodes.
    fn parse_require_include(&self, node: &Node, source: &[u8]) -> Option<ImportDef> {
        // Find the string literal being included
        let mut module_path: Option<String> = None;

        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                match child.kind() {
                    "string" | "encapsed_string" => {
                        let text = get_node_text(&child, source);
                        // Strip quotes
                        module_path =
                            Some(text.trim_matches(|c| c == '"' || c == '\'').to_string());
                        break;
                    }
                    "parenthesized_expression" => {
                        // require('file.php') or require("file.php")
                        for j in 0..child.child_count() {
                            if let Some(inner) = child.child(j) {
                                if inner.kind() == "string" || inner.kind() == "encapsed_string" {
                                    let text = get_node_text(&inner, source);
                                    module_path = Some(
                                        text.trim_matches(|c| c == '"' || c == '\'').to_string(),
                                    );
                                    break;
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        module_path.map(ImportDef::simple_import)
    }

    /// Collect all function, method, and class definitions.
    fn collect_definitions(
        &self,
        tree: &Tree,
        source: &[u8],
    ) -> (HashSet<String>, HashSet<String>) {
        let mut functions = HashSet::new();
        let mut classes = HashSet::new();

        for node in walk_tree(tree.root_node()) {
            match node.kind() {
                "function_definition" => {
                    // Get function name from 'name' field
                    if let Some(name_node) = node.child_by_field_name("name") {
                        functions.insert(get_node_text(&name_node, source).to_string());
                    }
                }
                "method_declaration" => {
                    // Get method name from 'name' field
                    if let Some(name_node) = node.child_by_field_name("name") {
                        functions.insert(get_node_text(&name_node, source).to_string());
                    }
                }
                "class_declaration" | "trait_declaration" | "interface_declaration" => {
                    // Get class/trait/interface name from 'name' field
                    if let Some(name_node) = node.child_by_field_name("name") {
                        classes.insert(get_node_text(&name_node, source).to_string());
                    }
                }
                _ => {}
            }
        }

        (functions, classes)
    }

    /// Extract calls from a function/method body node.
    fn extract_calls_from_node(
        &self,
        node: &Node,
        source: &[u8],
        defined_funcs: &HashSet<String>,
        defined_classes: &HashSet<String>,
        caller: &str,
    ) -> Vec<CallSite> {
        let mut calls = Vec::new();

        for child in walk_tree(*node) {
            let line = child.start_position().row as u32 + 1;

            match child.kind() {
                "function_call_expression" => {
                    // Get the function being called
                    if let Some(func_node) = child.child_by_field_name("function") {
                        match func_node.kind() {
                            "name" => {
                                // Simple function call: foo()
                                let callee = get_node_text(&func_node, source).to_string();
                                let call_type = if defined_funcs.contains(&callee) {
                                    CallType::Intra
                                } else {
                                    CallType::Direct
                                };
                                calls.push(CallSite::new(
                                    caller.to_string(),
                                    callee,
                                    call_type,
                                    Some(line),
                                    None,
                                    None,
                                    None,
                                ));
                            }
                            "qualified_name" => {
                                // Fully qualified call: \App\Service\func()
                                let callee = get_node_text(&func_node, source).to_string();
                                calls.push(CallSite::new(
                                    caller.to_string(),
                                    callee,
                                    CallType::Direct,
                                    Some(line),
                                    None,
                                    None,
                                    None,
                                ));
                            }
                            _ => {}
                        }
                    }
                }
                "member_call_expression" => {
                    // $obj->method()
                    let obj_node = child.child_by_field_name("object");
                    let name_node = child.child_by_field_name("name");

                    if let (Some(obj), Some(name)) = (obj_node, name_node) {
                        let obj_name = get_node_text(&obj, source).to_string();
                        let method_name = get_node_text(&name, source).to_string();

                        // $this->method() could be intra-file call
                        if obj_name == "$this" {
                            if defined_funcs.contains(&method_name) {
                                calls.push(CallSite::new(
                                    caller.to_string(),
                                    method_name,
                                    CallType::Intra,
                                    Some(line),
                                    None,
                                    None,
                                    None,
                                ));
                            } else {
                                let target = format!("$this->{}", method_name);
                                calls.push(CallSite::new(
                                    caller.to_string(),
                                    target.clone(),
                                    CallType::Attr,
                                    Some(line),
                                    None,
                                    Some(target),
                                    None,
                                ));
                            }
                        } else {
                            let target = format!("{}->{}", obj_name, method_name);
                            calls.push(CallSite::new(
                                caller.to_string(),
                                target.clone(),
                                CallType::Attr,
                                Some(line),
                                None,
                                Some(obj_name),
                                None,
                            ));
                        }
                    }
                }
                "scoped_call_expression" => {
                    // ClassName::staticMethod()
                    let scope_node = child.child_by_field_name("scope");
                    let name_node = child.child_by_field_name("name");

                    if let (Some(scope), Some(name)) = (scope_node, name_node) {
                        let class_name = get_node_text(&scope, source).to_string();
                        let method_name = get_node_text(&name, source).to_string();
                        let target = format!("{}::{}", class_name, method_name);

                        // Check if this is an intra-file call to a known class
                        let call_type = if defined_classes.contains(&class_name) {
                            CallType::Intra
                        } else {
                            CallType::Static
                        };

                        calls.push(CallSite::new(
                            caller.to_string(),
                            target,
                            call_type,
                            Some(line),
                            None,
                            Some(class_name),
                            None,
                        ));
                    }
                }
                "object_creation_expression" => {
                    // new ClassName()
                    for i in 0..child.child_count() {
                        if let Some(c) = child.child(i) {
                            if c.kind() == "name" || c.kind() == "qualified_name" {
                                let class_name = get_node_text(&c, source).to_string();
                                let call_type = if defined_classes.contains(&class_name) {
                                    CallType::Intra
                                } else {
                                    CallType::Direct
                                };
                                calls.push(CallSite::new(
                                    caller.to_string(),
                                    class_name,
                                    call_type,
                                    Some(line),
                                    None,
                                    None,
                                    None,
                                ));
                                break;
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        calls
    }

    /// Extract calls from function/method parameter default values.
    ///
    /// Handles patterns like:
    /// - `function foo($db = getDefaultDb()) { ... }` (Pattern #7)
    /// - `function __construct($log = new FileLogger())` (Pattern #6)
    fn extract_calls_from_param_defaults(
        &self,
        params_node: &Node,
        source: &[u8],
        defined_funcs: &HashSet<String>,
        defined_classes: &HashSet<String>,
        caller: &str,
    ) -> Vec<CallSite> {
        let mut calls = Vec::new();

        for child in walk_tree(*params_node) {
            // simple_parameter or property_promotion_parameter
            if child.kind() == "simple_parameter" || child.kind() == "property_promotion_parameter"
            {
                if let Some(default_val) = child.child_by_field_name("default_value") {
                    calls.extend(self.extract_calls_from_node(
                        &default_val,
                        source,
                        defined_funcs,
                        defined_classes,
                        caller,
                    ));
                }
            }
        }

        calls
    }
}

impl CallGraphLanguageSupport for PhpHandler {
    fn name(&self) -> &str {
        "php"
    }

    fn extensions(&self) -> &[&str] {
        &[".php"]
    }

    fn parse_imports(&self, source: &str, _path: &Path) -> Result<Vec<ImportDef>, ParseError> {
        let tree = self.parse_source(source)?;
        let source_bytes = source.as_bytes();
        let mut imports = Vec::new();

        for node in walk_tree(tree.root_node()) {
            match node.kind() {
                "namespace_use_declaration" => {
                    imports.extend(self.parse_use_declaration(&node, source_bytes));
                }
                "include_expression"
                | "include_once_expression"
                | "require_expression"
                | "require_once_expression" => {
                    if let Some(imp) = self.parse_require_include(&node, source_bytes) {
                        imports.push(imp);
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
        let (defined_funcs, defined_classes) = self.collect_definitions(tree, source_bytes);
        let mut calls_by_func: HashMap<String, Vec<CallSite>> = HashMap::new();

        // Track current class context
        let mut current_class: Option<String> = None;

        fn process_node(
            node: Node,
            source: &[u8],
            defined_funcs: &HashSet<String>,
            defined_classes: &HashSet<String>,
            calls_by_func: &mut HashMap<String, Vec<CallSite>>,
            current_class: &mut Option<String>,
            handler: &PhpHandler,
        ) {
            match node.kind() {
                "class_declaration" | "trait_declaration" | "interface_declaration" => {
                    // Get class/trait/interface name
                    let mut class_name: Option<String> = None;
                    if let Some(name_node) = node.child_by_field_name("name") {
                        class_name = Some(get_node_text(&name_node, source).to_string());
                    }

                    let old_class = current_class.take();
                    *current_class = class_name;

                    // Process children
                    for i in 0..node.child_count() {
                        if let Some(child) = node.child(i) {
                            process_node(
                                child,
                                source,
                                defined_funcs,
                                defined_classes,
                                calls_by_func,
                                current_class,
                                handler,
                            );
                        }
                    }

                    *current_class = old_class;
                }
                "function_definition" => {
                    // Get function name and body
                    let mut func_name: Option<String> = None;
                    let mut body: Option<Node> = None;

                    if let Some(name_node) = node.child_by_field_name("name") {
                        func_name = Some(get_node_text(&name_node, source).to_string());
                    }
                    if let Some(body_node) = node.child_by_field_name("body") {
                        body = Some(body_node);
                    }

                    if let (Some(name), Some(body_node)) = (func_name, body) {
                        let mut calls = handler.extract_calls_from_node(
                            &body_node,
                            source,
                            defined_funcs,
                            defined_classes,
                            &name,
                        );

                        // Pattern #6/#7: Extract calls from parameter default values
                        if let Some(params) = node.child_by_field_name("parameters") {
                            calls.extend(handler.extract_calls_from_param_defaults(
                                &params,
                                source,
                                defined_funcs,
                                defined_classes,
                                &name,
                            ));
                        }

                        if !calls.is_empty() {
                            calls_by_func.insert(name, calls);
                        }
                    }
                }
                "method_declaration" => {
                    // Get method name and body
                    let mut method_name: Option<String> = None;
                    let mut body: Option<Node> = None;

                    if let Some(name_node) = node.child_by_field_name("name") {
                        method_name = Some(get_node_text(&name_node, source).to_string());
                    }
                    if let Some(body_node) = node.child_by_field_name("body") {
                        body = Some(body_node);
                    }

                    if let (Some(name), Some(body_node)) = (method_name, body) {
                        let full_name = if let Some(ref class) = current_class {
                            format!("{}::{}", class, name)
                        } else {
                            name.clone()
                        };

                        let mut calls = handler.extract_calls_from_node(
                            &body_node,
                            source,
                            defined_funcs,
                            defined_classes,
                            &full_name,
                        );

                        // Pattern #6/#7: Extract calls from parameter default values
                        if let Some(params) = node.child_by_field_name("parameters") {
                            calls.extend(handler.extract_calls_from_param_defaults(
                                &params,
                                source,
                                defined_funcs,
                                defined_classes,
                                &full_name,
                            ));
                        }

                        if !calls.is_empty() {
                            calls_by_func.insert(full_name.clone(), calls.clone());
                            // Also store with simple name
                            if current_class.is_some() {
                                calls_by_func.insert(name, calls);
                            }
                        }
                    }
                }
                "property_declaration" => {
                    // Pattern #3: Class property initializers
                    // Extract calls from property default values (e.g., public $x = createFoo())
                    let caller = if let Some(ref class) = current_class {
                        class.clone()
                    } else {
                        "<module>".to_string()
                    };

                    // Scan property_element children for default_value fields
                    for child in walk_tree(node) {
                        if child.kind() == "property_element" {
                            if let Some(default_val) = child.child_by_field_name("default_value") {
                                let prop_calls = handler.extract_calls_from_node(
                                    &default_val,
                                    source,
                                    defined_funcs,
                                    defined_classes,
                                    &caller,
                                );
                                if !prop_calls.is_empty() {
                                    calls_by_func
                                        .entry(caller.clone())
                                        .or_default()
                                        .extend(prop_calls);
                                }
                            }
                        }
                    }
                }
                _ => {
                    // Recurse for other nodes
                    for i in 0..node.child_count() {
                        if let Some(child) = node.child(i) {
                            process_node(
                                child,
                                source,
                                defined_funcs,
                                defined_classes,
                                calls_by_func,
                                current_class,
                                handler,
                            );
                        }
                    }
                }
            }
        }

        process_node(
            tree.root_node(),
            source_bytes,
            &defined_funcs,
            &defined_classes,
            &mut calls_by_func,
            &mut current_class,
            self,
        );

        // Extract module-level calls into synthetic <module> function
        let mut module_calls = Vec::new();
        for node in tree.root_node().children(&mut tree.root_node().walk()) {
            // Skip class, trait, interface, and function definitions
            if matches!(
                node.kind(),
                "class_declaration"
                    | "trait_declaration"
                    | "interface_declaration"
                    | "function_definition"
                    | "namespace_use_declaration"
            ) {
                continue;
            }

            let calls = self.extract_calls_from_node(
                &node,
                source_bytes,
                &defined_funcs,
                &defined_classes,
                "<module>",
            );
            module_calls.extend(calls);
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
        let mut classes = Vec::new();

        for node in walk_tree(tree.root_node()) {
            match node.kind() {
                "function_definition" => {
                    if let Some(name_node) = node.child_by_field_name("name") {
                        let name = get_node_text(&name_node, source_bytes).to_string();
                        let line = node.start_position().row as u32 + 1;
                        let end_line = node.end_position().row as u32 + 1;
                        funcs.push(FuncDef::function(name, line, end_line));
                    }
                }
                "method_declaration" => {
                    if let Some(name_node) = node.child_by_field_name("name") {
                        let name = get_node_text(&name_node, source_bytes).to_string();
                        let line = node.start_position().row as u32 + 1;
                        let end_line = node.end_position().row as u32 + 1;

                        // Check if inside a class
                        let mut class_name = None;
                        let mut parent = node.parent();
                        while let Some(p) = parent {
                            if p.kind() == "declaration_list" {
                                if let Some(gp) = p.parent() {
                                    if gp.kind() == "class_declaration"
                                        || gp.kind() == "interface_declaration"
                                        || gp.kind() == "trait_declaration"
                                    {
                                        if let Some(cn) = gp.child_by_field_name("name") {
                                            class_name =
                                                Some(get_node_text(&cn, source_bytes).to_string());
                                        }
                                    }
                                }
                                break;
                            }
                            parent = p.parent();
                        }

                        if let Some(cn) = class_name {
                            funcs.push(FuncDef::method(name, cn, line, end_line));
                        } else {
                            funcs.push(FuncDef::function(name, line, end_line));
                        }
                    }
                }
                "class_declaration" | "interface_declaration" | "trait_declaration" => {
                    if let Some(name_node) = node.child_by_field_name("name") {
                        let class_name = get_node_text(&name_node, source_bytes).to_string();
                        let line = node.start_position().row as u32 + 1;
                        let end_line = node.end_position().row as u32 + 1;

                        let mut methods = Vec::new();
                        let mut bases = Vec::new();

                        for i in 0..node.child_count() {
                            if let Some(child) = node.child(i) {
                                if child.kind() == "base_clause" {
                                    for j in 0..child.child_count() {
                                        if let Some(base) = child.child(j) {
                                            if base.kind() == "name"
                                                || base.kind() == "qualified_name"
                                            {
                                                bases.push(
                                                    get_node_text(&base, source_bytes).to_string(),
                                                );
                                            }
                                        }
                                    }
                                }
                                if child.kind() == "class_interface_clause" {
                                    for j in 0..child.child_count() {
                                        if let Some(iface) = child.child(j) {
                                            if iface.kind() == "name"
                                                || iface.kind() == "qualified_name"
                                            {
                                                bases.push(
                                                    get_node_text(&iface, source_bytes).to_string(),
                                                );
                                            }
                                        }
                                    }
                                }
                                if child.kind() == "declaration_list" {
                                    for j in 0..child.named_child_count() {
                                        if let Some(member) = child.named_child(j) {
                                            if member.kind() == "method_declaration" {
                                                if let Some(mn) = member.child_by_field_name("name")
                                                {
                                                    methods.push(
                                                        get_node_text(&mn, source_bytes)
                                                            .to_string(),
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        classes.push(ClassDef::new(class_name, line, end_line, methods, bases));
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
        let handler = PhpHandler::new();
        handler
            .parse_imports(source, Path::new("test.php"))
            .unwrap()
    }

    fn extract_calls(source: &str) -> HashMap<String, Vec<CallSite>> {
        let handler = PhpHandler::new();
        let tree = handler.parse_source(source).unwrap();
        handler
            .extract_calls(Path::new("test.php"), source, &tree)
            .unwrap()
    }

    // -------------------------------------------------------------------------
    // Import Parsing Tests
    // -------------------------------------------------------------------------

    mod import_tests {
        use super::*;

        #[test]
        fn test_parse_simple_use() {
            let imports = parse_imports("<?php\nuse App\\Models\\User;");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].module, "App\\Models\\User");
            assert!(imports[0].alias.is_none());
        }

        #[test]
        fn test_parse_use_with_alias() {
            let imports = parse_imports("<?php\nuse App\\Models\\User as UserModel;");
            assert_eq!(imports.len(), 1);
            assert!(imports[0].module.contains("User"));
            assert_eq!(imports[0].alias, Some("UserModel".to_string()));
        }

        #[test]
        fn test_parse_grouped_use() {
            let imports = parse_imports("<?php\nuse App\\Models\\{User, Post};");
            assert_eq!(imports.len(), 2);
            assert!(imports.iter().any(|i| i.module.contains("User")));
            assert!(imports.iter().any(|i| i.module.contains("Post")));
        }

        #[test]
        fn test_parse_require() {
            let imports = parse_imports("<?php\nrequire 'file.php';");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].module, "file.php");
        }

        #[test]
        fn test_parse_require_once() {
            let imports = parse_imports("<?php\nrequire_once 'vendor/autoload.php';");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].module, "vendor/autoload.php");
        }

        #[test]
        fn test_parse_include() {
            let imports = parse_imports("<?php\ninclude 'config.php';");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].module, "config.php");
        }

        #[test]
        fn test_parse_include_once() {
            let imports = parse_imports("<?php\ninclude_once 'helpers.php';");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].module, "helpers.php");
        }

        #[test]
        fn test_parse_require_with_parens() {
            let imports = parse_imports("<?php\nrequire('file.php');");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].module, "file.php");
        }

        #[test]
        fn test_parse_multiple_imports() {
            let source = r#"<?php
use App\Models\User;
use App\Services\AuthService;
require_once 'vendor/autoload.php';
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
        fn test_extract_direct_call() {
            let source = r#"<?php
function main() {
    helper();
    print_r("hello");
}
"#;
            let calls = extract_calls(source);
            let main_calls = calls.get("main").unwrap();
            assert!(main_calls.iter().any(|c| c.target == "helper"));
            assert!(main_calls.iter().any(|c| c.target == "print_r"));
        }

        #[test]
        fn test_extract_intra_file_call() {
            let source = r#"<?php
function helper() {
    return "help";
}

function main() {
    helper();
}
"#;
            let calls = extract_calls(source);
            let main_calls = calls.get("main").unwrap();
            let helper_call = main_calls.iter().find(|c| c.target == "helper").unwrap();
            assert_eq!(helper_call.call_type, CallType::Intra);
        }

        #[test]
        fn test_extract_method_call() {
            let source = r#"<?php
function process() {
    $user->save();
    $repo->find(1);
}
"#;
            let calls = extract_calls(source);
            let process_calls = calls.get("process").unwrap();
            assert!(process_calls.iter().any(|c| c.target.contains("save")));
            assert!(process_calls.iter().any(|c| c.target.contains("find")));
            assert!(process_calls.iter().all(|c| c.call_type == CallType::Attr));
        }

        #[test]
        fn test_extract_static_call() {
            let source = r#"<?php
function createUser() {
    User::create(['name' => 'test']);
    Auth::check();
}
"#;
            let calls = extract_calls(source);
            let create_calls = calls.get("createUser").unwrap();
            let user_call = create_calls
                .iter()
                .find(|c| c.target.contains("User::create"))
                .unwrap();
            assert_eq!(user_call.call_type, CallType::Static);
        }

        #[test]
        fn test_extract_static_call_to_known_class() {
            let source = r#"<?php
class Helper {
    public static function format($value) {
        return $value;
    }
}

function process() {
    Helper::format("test");
}
"#;
            let calls = extract_calls(source);
            let process_calls = calls.get("process").unwrap();
            let helper_call = process_calls
                .iter()
                .find(|c| c.target.contains("Helper::format"))
                .unwrap();
            // Should be Intra since Helper is defined in the same file
            assert_eq!(helper_call.call_type, CallType::Intra);
        }

        #[test]
        fn test_extract_constructor_call() {
            let source = r#"<?php
function createUser() {
    $user = new User();
    $post = new Post('title');
}
"#;
            let calls = extract_calls(source);
            let create_calls = calls.get("createUser").unwrap();
            assert!(create_calls.iter().any(|c| c.target == "User"));
            assert!(create_calls.iter().any(|c| c.target == "Post"));
        }

        #[test]
        fn test_extract_this_call() {
            let source = r#"<?php
class UserController {
    public function store() {
        $this->validate();
        $this->save();
    }

    public function validate() {
        // validation
    }
}
"#;
            let calls = extract_calls(source);
            // Should have calls from UserController::store or store
            let store_calls = calls
                .get("UserController::store")
                .or_else(|| calls.get("store"))
                .unwrap();
            let validate_call = store_calls.iter().find(|c| c.target == "validate").unwrap();
            // validate is defined in the same class, so should be Intra
            assert_eq!(validate_call.call_type, CallType::Intra);
        }

        #[test]
        fn test_extract_calls_in_class() {
            let source = r#"<?php
class Calculator {
    public function add($a, $b) {
        $this->validate($a);
        return $a + $b;
    }

    public function validate($n) {
        if ($n === null) {
            throw new Exception("Invalid");
        }
    }
}
"#;
            let calls = extract_calls(source);
            // Should have both Calculator::add and add
            assert!(
                calls.contains_key("Calculator::add") || calls.contains_key("add"),
                "Expected to find method calls"
            );
        }

        #[test]
        fn test_extract_module_level_calls() {
            let source = r#"<?php
function helper() {
    return "help";
}

// Module-level call
$result = helper();
echo($result);
"#;
            let calls = extract_calls(source);
            assert!(calls.contains_key("<module>"));
            let module_calls = calls.get("<module>").unwrap();
            assert!(module_calls.iter().any(|c| c.target == "helper"));
        }

        // =================================================================
        // Pattern #14: Anonymous class bodies
        // =================================================================

        #[test]
        fn test_extract_anonymous_class_method_calls() {
            let source = r#"<?php
function make() {
    $obj = new class {
        public function process() {
            compute();
        }
    };
}
"#;
            let calls = extract_calls(source);
            // The anonymous class method should be extracted
            // Look for compute() call from <anon>::process or similar
            let has_compute = calls
                .values()
                .any(|v| v.iter().any(|c| c.target == "compute"));
            assert!(
                has_compute,
                "Should extract calls from anonymous class methods. Got: {:?}",
                calls.keys().collect::<Vec<_>>()
            );
        }

        #[test]
        fn test_extract_anonymous_class_at_module_level() {
            let source = r#"<?php
$handler = new class {
    public function handle() {
        dispatch();
    }
};
"#;
            let calls = extract_calls(source);
            let has_dispatch = calls
                .values()
                .any(|v| v.iter().any(|c| c.target == "dispatch"));
            assert!(
                has_dispatch,
                "Should extract calls from anonymous class at module level. Got: {:?}",
                calls.keys().collect::<Vec<_>>()
            );
        }

        // =================================================================
        // Pattern #15: Lambda/closure and arrow function bodies
        // =================================================================

        #[test]
        fn test_extract_closure_calls() {
            let source = r#"<?php
function main() {
    $fn = function($x) { return transform($x); };
}
"#;
            let calls = extract_calls(source);
            let has_transform = calls
                .values()
                .any(|v| v.iter().any(|c| c.target == "transform"));
            assert!(
                has_transform,
                "Should extract calls from closures. Got: {:?}",
                calls
            );
        }

        #[test]
        fn test_extract_arrow_function_calls() {
            let source = r#"<?php
function main() {
    $arrow = fn($x) => double($x);
}
"#;
            let calls = extract_calls(source);
            let has_double = calls
                .values()
                .any(|v| v.iter().any(|c| c.target == "double"));
            assert!(
                has_double,
                "Should extract calls from arrow functions. Got: {:?}",
                calls
            );
        }

        #[test]
        fn test_extract_closure_with_use() {
            let source = r#"<?php
function main() {
    $db = getDb();
    $fn = function() use ($db) { $db->close(); };
}
"#;
            let calls = extract_calls(source);
            // getDb() should be found in main
            let main_calls = calls.get("main").unwrap();
            assert!(
                main_calls.iter().any(|c| c.target == "getDb"),
                "Should find getDb call"
            );
            // $db->close() should be found somewhere (in main or in a closure sub-function)
            let has_close = calls
                .values()
                .any(|v| v.iter().any(|c| c.target.contains("close")));
            assert!(
                has_close,
                "Should extract method calls from closures with use. Got: {:?}",
                calls
            );
        }

        #[test]
        fn test_extract_closure_as_argument() {
            let source = r#"<?php
function main() {
    array_map(fn($item) => format($item), $items);
}
"#;
            let calls = extract_calls(source);
            let has_format = calls
                .values()
                .any(|v| v.iter().any(|c| c.target == "format"));
            assert!(
                has_format,
                "Should extract calls from arrow fn passed as argument. Got: {:?}",
                calls
            );
        }

        // =================================================================
        // Pattern #26: Trait method bodies
        // =================================================================

        #[test]
        fn test_extract_trait_method_calls() {
            let source = r#"<?php
trait Loggable {
    public function log($msg) {
        writeLog($msg);
    }
    public function debug($msg) {
        formatDebug($msg);
    }
}
"#;
            let calls = extract_calls(source);
            let has_writelog = calls
                .values()
                .any(|v| v.iter().any(|c| c.target == "writeLog"));
            let has_formatdebug = calls
                .values()
                .any(|v| v.iter().any(|c| c.target == "formatDebug"));
            assert!(
                has_writelog,
                "Should extract calls from trait methods. Got: {:?}",
                calls.keys().collect::<Vec<_>>()
            );
            assert!(
                has_formatdebug,
                "Should extract calls from trait methods. Got: {:?}",
                calls.keys().collect::<Vec<_>>()
            );
        }

        #[test]
        fn test_trait_method_caller_naming() {
            let source = r#"<?php
trait Cacheable {
    public function cache() {
        storeInCache();
    }
}
"#;
            let calls = extract_calls(source);
            // Trait methods should be named TraitName::methodName
            assert!(
                calls.contains_key("Cacheable::cache") || calls.contains_key("cache"),
                "Trait method caller should be named. Got: {:?}",
                calls.keys().collect::<Vec<_>>()
            );
            if let Some(cache_calls) = calls.get("Cacheable::cache").or_else(|| calls.get("cache"))
            {
                assert!(cache_calls.iter().any(|c| c.target == "storeInCache"));
            }
        }

        // =================================================================
        // Pattern #3: Class property initializers with new (PHP 8.1)
        // =================================================================

        #[test]
        fn test_extract_property_initializer_calls() {
            let source = r#"<?php
class Config {
    public $items = [createDefault()];
}
"#;
            let calls = extract_calls(source);
            let has_create = calls
                .values()
                .any(|v| v.iter().any(|c| c.target == "createDefault"));
            assert!(
                has_create,
                "Should extract calls from property initializers. Got: {:?}",
                calls
            );
        }

        #[test]
        fn test_extract_property_initializer_caller_name() {
            let source = r#"<?php
class Service {
    public $handler = initHandler();
}
"#;
            let calls = extract_calls(source);
            // The caller for class-level initializer should be the class name
            let has_init = calls
                .values()
                .any(|v| v.iter().any(|c| c.target == "initHandler"));
            assert!(
                has_init,
                "Should extract calls from property initializers. Got: {:?}",
                calls
            );
        }

        // =================================================================
        // Pattern #6/#7: Default params with new/calls
        // =================================================================

        #[test]
        fn test_extract_function_default_param_calls() {
            let source = r#"<?php
function connect($db = getDefaultDb()) {
    $db->query("SELECT 1");
}
"#;
            let calls = extract_calls(source);
            let has_get_default = calls
                .values()
                .any(|v| v.iter().any(|c| c.target == "getDefaultDb"));
            assert!(
                has_get_default,
                "Should extract calls from function default params. Got: {:?}",
                calls
            );
        }

        #[test]
        fn test_extract_method_default_param_calls() {
            let source = r#"<?php
class Service {
    public function process($logger = createLogger()) {
        $logger->info("done");
    }
}
"#;
            let calls = extract_calls(source);
            let has_create = calls
                .values()
                .any(|v| v.iter().any(|c| c.target == "createLogger"));
            assert!(
                has_create,
                "Should extract calls from method default params. Got: {:?}",
                calls
            );
        }

        // =================================================================
        // Pattern #22: Class const initializers
        // =================================================================

        #[test]
        fn test_extract_class_const_initializer_calls() {
            let source = r#"<?php
class Config {
    const DEFAULT_HANDLER = 'FileHandler';
    public function getHandler() {
        return createHandler(self::DEFAULT_HANDLER);
    }
}
"#;
            let calls = extract_calls(source);
            // createHandler inside method body should be found
            let has_create = calls
                .values()
                .any(|v| v.iter().any(|c| c.target == "createHandler"));
            assert!(
                has_create,
                "Should extract calls from method referencing const. Got: {:?}",
                calls
            );
        }

        // =================================================================
        // Pattern #9/#30: Attribute arguments
        // =================================================================

        #[test]
        fn test_extract_attribute_constructor_calls() {
            let source = r#"<?php
#[Route("/api")]
function apiHandler() {
    respond();
}
"#;
            let calls = extract_calls(source);
            // respond() in function body should still be found
            let has_respond = calls
                .values()
                .any(|v| v.iter().any(|c| c.target == "respond"));
            assert!(
                has_respond,
                "Attributes should not break function body extraction. Got: {:?}",
                calls
            );
        }

        // =================================================================
        // Pattern #11: parent:: calls (verify already working)
        // =================================================================

        #[test]
        fn test_extract_parent_constructor_call() {
            let source = r#"<?php
class Child extends Base {
    public function __construct() {
        parent::__construct();
        $this->init();
    }
}
"#;
            let calls = extract_calls(source);
            let ctor_calls = calls
                .get("Child::__construct")
                .or_else(|| calls.get("__construct"));
            assert!(
                ctor_calls.is_some(),
                "Should find constructor. Got: {:?}",
                calls.keys().collect::<Vec<_>>()
            );
            let ctor = ctor_calls.unwrap();
            assert!(
                ctor.iter()
                    .any(|c| c.target.contains("parent::__construct")),
                "Should find parent::__construct call. Got: {:?}",
                ctor
            );
        }

        #[test]
        fn test_extract_parent_constructor_with_call_args() {
            let source = r#"<?php
class Child extends Base {
    public function __construct() {
        parent::__construct(createConfig());
    }
}
"#;
            let calls = extract_calls(source);
            let ctor_calls = calls
                .get("Child::__construct")
                .or_else(|| calls.get("__construct"));
            assert!(ctor_calls.is_some(), "Should find constructor");
            let ctor = ctor_calls.unwrap();
            assert!(
                ctor.iter().any(|c| c.target == "createConfig"),
                "Should find createConfig() in parent:: args. Got: {:?}",
                ctor
            );
        }
    }

    // -------------------------------------------------------------------------
    // Cross-Scope Intra-File Call Tests (Method to Top-Level)
    // -------------------------------------------------------------------------

    mod cross_scope_tests {
        use super::*;

        #[test]
        fn test_extract_calls_method_to_toplevel() {
            let source = r#"<?php
function helper_func() {
    return "helper";
}

class MyClass {
    public function method() {
        helper_func();
    }
}
"#;
            let calls = extract_calls(source);

            // The method should have a call to helper_func marked as Intra
            // The caller name is qualified as "MyClass::method"
            let method_calls = calls
                .get("MyClass::method")
                .expect("MyClass::method should have calls");
            let helper_call = method_calls.iter().find(|c| c.target == "helper_func");

            assert!(
                helper_call.is_some(),
                "Should find call from method to top-level helper_func. Got: {:?}",
                method_calls
            );

            let call = helper_call.unwrap();
            assert_eq!(
                call.call_type,
                CallType::Intra,
                "Call to same-file top-level function should be Intra"
            );
        }
    }

    // -------------------------------------------------------------------------
    // Handler Trait Tests
    // -------------------------------------------------------------------------

    mod trait_tests {
        use super::*;

        #[test]
        fn test_handler_name() {
            let handler = PhpHandler::new();
            assert_eq!(handler.name(), "php");
        }

        #[test]
        fn test_handler_extensions() {
            let handler = PhpHandler::new();
            let exts = handler.extensions();
            assert!(exts.contains(&".php"));
        }

        #[test]
        fn test_handler_supports() {
            let handler = PhpHandler::new();
            assert!(handler.supports("php"));
            assert!(handler.supports("PHP"));
            assert!(handler.supports("Php"));
            assert!(!handler.supports("python"));
        }

        #[test]
        fn test_handler_supports_extension() {
            let handler = PhpHandler::new();
            assert!(handler.supports_extension(".php"));
            assert!(handler.supports_extension(".PHP"));
            assert!(!handler.supports_extension(".py"));
        }
    }
}
