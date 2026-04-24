//! Scala language handler for call graph analysis.
//!
//! This module provides Scala-specific call graph support using tree-sitter-scala.
//!
//! # Import Patterns Supported
//!
//! | Pattern | ImportDef |
//! |---------|-----------|
//! | `import scala.collection.mutable` | `{module: "scala.collection.mutable"}` |
//! | `import scala.collection.mutable._` | `{module: "scala.collection.mutable", names: ["*"], is_namespace: true}` |
//! | `import scala.collection.{mutable, immutable}` | Multiple imports for each selector |
//! | `import java.util.{List => JList}` | `{module: "java.util.List", alias: "JList"}` |
//!
//! # Call Extraction
//!
//! - Direct calls: `method()` -> CallType::Direct or CallType::Intra
//! - Method calls: `obj.method()` -> CallType::Attr
//! - Object method calls: `Object.method()` -> CallType::Attr
//! - Apply method: `obj()` -> CallType::Attr (implicit apply)
//! - Infix notation: `a method b` -> CallType::Attr
//! - Constructor calls: `new Class()` -> CallType::Direct
//! - Val/var initializers: `val x = compute()` -> attributed to `Container.<init>`
//! - Constructor default params: `class Foo(x: Int = default())` -> `Foo.<init>`
//! - Super constructor args: `class Child extends Parent(init())` -> `Child.<init>`
//! - Function default params: `def foo(x: Int = default())` -> attributed to `foo`
//! - For-comprehension bodies, guard clauses, if-expressions, lambdas
//! - String interpolation: `s"${compute()}"`, collection literals: `List(a(), b())`
//! - Anonymous class bodies: `new Trait { def m = call() }`
//! - Trait default method bodies: `trait T { def m: Int = call() }`
//!
//! # Spec Reference
//!
//! See `migration/spec/callgraph-spec.md` Section 9.13 for Scala-specific details.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use tree_sitter::{Node, Parser, Tree};

use super::base::{get_node_text, walk_tree};
use super::{CallGraphLanguageSupport, ParseError};
use crate::callgraph::cross_file_types::{CallSite, CallType, ClassDef, FuncDef, ImportDef};

// =============================================================================
// Scala Handler
// =============================================================================

/// Scala language handler using tree-sitter-scala.
///
/// Supports:
/// - Import parsing (standard, wildcard, selective, renamed imports)
/// - Call extraction (direct, method, constructor, infix notation)
/// - Class, object, and trait method tracking
/// - Companion object support
#[derive(Debug, Default)]
pub struct ScalaHandler;

impl ScalaHandler {
    /// Creates a new ScalaHandler.
    pub fn new() -> Self {
        Self
    }

    /// Parse the source code into a tree-sitter Tree.
    fn parse_source(&self, source: &str) -> Result<Tree, ParseError> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_scala::LANGUAGE.into())
            .map_err(|e| ParseError::ParseFailed {
                file: std::path::PathBuf::new(),
                message: format!("Failed to set Scala language: {}", e),
            })?;

        parser
            .parse(source, None)
            .ok_or_else(|| ParseError::ParseFailed {
                file: std::path::PathBuf::new(),
                message: "Parser returned None".to_string(),
            })
    }

    /// Parse an import declaration node.
    ///
    /// Scala imports can have multiple forms:
    /// - `import scala.collection.mutable` (simple)
    /// - `import scala.collection.mutable._` (wildcard)
    /// - `import scala.collection.{mutable, immutable}` (selective)
    /// - `import java.util.{List => JList}` (renamed)
    fn parse_import_node(&self, node: &Node, source: &[u8]) -> Vec<ImportDef> {
        if node.kind() != "import_declaration" {
            return vec![];
        }

        let mut results = Vec::new();
        let mut path_parts: Vec<String> = Vec::new();
        let mut has_wildcard = false;
        let mut selectors: Vec<(String, Option<String>)> = Vec::new(); // (name, alias)

        // Walk children to extract import components
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                match child.kind() {
                    "identifier" | "type_identifier" => {
                        let text = get_node_text(&child, source).to_string();
                        path_parts.push(text);
                    }
                    "stable_identifier" => {
                        // Qualified path like scala.collection.mutable
                        self.extract_path_parts(&child, source, &mut path_parts);
                    }
                    "namespace_wildcard" | "wildcard" | "_" => {
                        has_wildcard = true;
                    }
                    "namespace_selectors" => {
                        // Parse selective imports: {mutable, immutable} or {List => JList}
                        self.parse_namespace_selectors(
                            &child,
                            source,
                            &mut selectors,
                            &mut has_wildcard,
                        );
                    }
                    _ => {}
                }
            }
        }

        let base_path = path_parts.join(".");

        if has_wildcard {
            // Wildcard import: import scala.collection.mutable._
            let mut imp = ImportDef::wildcard_import(&base_path);
            imp.is_namespace = true;
            results.push(imp);
        } else if !selectors.is_empty() {
            // Selective imports: import scala.collection.{mutable, immutable}
            for (name, alias) in selectors {
                let full_module = if base_path.is_empty() {
                    name.clone()
                } else {
                    format!("{}.{}", base_path, name)
                };

                let mut imp = ImportDef::simple_import(&full_module);
                if let Some(alias_name) = alias {
                    imp.alias = Some(alias_name);
                }
                results.push(imp);
            }
        } else if !base_path.is_empty() {
            // Simple import: import scala.collection.mutable
            results.push(ImportDef::simple_import(&base_path));
        }

        results
    }

    /// Extract path parts from a stable_identifier node.
    fn extract_path_parts(&self, node: &Node, source: &[u8], parts: &mut Vec<String>) {
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                match child.kind() {
                    "identifier" | "type_identifier" => {
                        parts.push(get_node_text(&child, source).to_string());
                    }
                    "stable_identifier" => {
                        self.extract_path_parts(&child, source, parts);
                    }
                    _ => {}
                }
            }
        }
    }

    /// Parse namespace selectors: {mutable, immutable} or {List => JList, _}
    fn parse_namespace_selectors(
        &self,
        node: &Node,
        source: &[u8],
        selectors: &mut Vec<(String, Option<String>)>,
        has_wildcard: &mut bool,
    ) {
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                match child.kind() {
                    "identifier" | "type_identifier" => {
                        let name = get_node_text(&child, source).to_string();
                        selectors.push((name, None));
                    }
                    "arrow_renamed_identifier" | "renamed_identifier" | "import_selector" => {
                        // Handle renamed imports: List => JList
                        let (orig, alias) = self.parse_renamed_identifier(&child, source);
                        if let Some(name) = orig {
                            selectors.push((name, alias));
                        }
                    }
                    "namespace_wildcard" | "wildcard" | "_" => {
                        *has_wildcard = true;
                    }
                    _ => {}
                }
            }
        }
    }

    /// Parse a renamed identifier: Foo => Bar
    fn parse_renamed_identifier(
        &self,
        node: &Node,
        source: &[u8],
    ) -> (Option<String>, Option<String>) {
        let mut orig_name: Option<String> = None;
        let mut alias_name: Option<String> = None;

        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if child.kind() == "identifier" || child.kind() == "type_identifier" {
                    let text = get_node_text(&child, source).to_string();
                    if orig_name.is_none() {
                        orig_name = Some(text);
                    } else {
                        alias_name = Some(text);
                    }
                }
            }
        }

        (orig_name, alias_name)
    }

    /// Collect all class, object, trait, and method definitions.
    fn collect_definitions(
        &self,
        tree: &Tree,
        source: &[u8],
    ) -> (HashSet<String>, HashSet<String>) {
        let mut methods = HashSet::new();
        let mut classes = HashSet::new();

        for node in walk_tree(tree.root_node()) {
            match node.kind() {
                "function_definition" | "function_declaration" => {
                    // Get method name
                    if let Some(name) = self.get_identifier_from_node(&node, source) {
                        methods.insert(name);
                    }
                }
                "class_definition" | "object_definition" | "trait_definition" => {
                    if let Some(name) = self.get_identifier_from_node(&node, source) {
                        classes.insert(name.clone());
                        // Object companion can be called like a function (apply)
                        methods.insert(name);
                    }
                }
                _ => {}
            }
        }

        (methods, classes)
    }

    /// Get the identifier (name) from a declaration node.
    fn get_identifier_from_node(&self, node: &Node, source: &[u8]) -> Option<String> {
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if child.kind() == "identifier" || child.kind() == "type_identifier" {
                    return Some(get_node_text(&child, source).to_string());
                }
            }
        }
        None
    }

    /// Extract calls from a method/function body.
    fn extract_calls_from_node(
        &self,
        node: &Node,
        source: &[u8],
        defined_methods: &HashSet<String>,
        defined_classes: &HashSet<String>,
        caller: &str,
    ) -> Vec<CallSite> {
        let mut calls = Vec::new();

        for child in walk_tree(*node) {
            match child.kind() {
                "call_expression" => {
                    let line = child.start_position().row as u32 + 1;

                    // Parse call_expression: function(args) or obj.method(args)
                    let mut func_node: Option<Node> = None;

                    for i in 0..child.child_count() {
                        if let Some(c) = child.child(i) {
                            // Skip argument lists
                            if c.kind() != "arguments" && c.kind() != "block" {
                                func_node = Some(c);
                                break;
                            }
                        }
                    }

                    if let Some(func) = func_node {
                        match func.kind() {
                            "identifier" => {
                                // Simple call: helper()
                                let call_name = get_node_text(&func, source).to_string();
                                let call_type = if defined_methods.contains(&call_name) {
                                    CallType::Intra
                                } else {
                                    CallType::Direct
                                };
                                calls.push(CallSite::new(
                                    caller.to_string(),
                                    call_name,
                                    call_type,
                                    Some(line),
                                    None,
                                    None,
                                    None,
                                ));
                            }
                            "field_expression" | "select_expression" => {
                                // Method call: obj.method()
                                let (receiver, method) = self.parse_field_expression(&func, source);
                                if let Some(method_name) = method {
                                    let target = if let Some(ref recv) = receiver {
                                        format!("{}.{}", recv, method_name)
                                    } else {
                                        method_name.clone()
                                    };
                                    calls.push(CallSite::new(
                                        caller.to_string(),
                                        target,
                                        CallType::Attr,
                                        Some(line),
                                        None,
                                        receiver,
                                        None,
                                    ));
                                }
                            }
                            "generic_function" => {
                                // Generic call: method[Type](args)
                                if let Some(name) = self.get_identifier_from_node(&func, source) {
                                    let call_type = if defined_methods.contains(&name) {
                                        CallType::Intra
                                    } else {
                                        CallType::Direct
                                    };
                                    calls.push(CallSite::new(
                                        caller.to_string(),
                                        name,
                                        call_type,
                                        Some(line),
                                        None,
                                        None,
                                        None,
                                    ));
                                }
                            }
                            _ => {}
                        }
                    }
                }
                "infix_expression" => {
                    // Infix notation: a method b
                    let line = child.start_position().row as u32 + 1;

                    let (receiver, method) = self.parse_infix_expression(&child, source);
                    if let Some(method_name) = method {
                        let target = if let Some(ref recv) = receiver {
                            format!("{}.{}", recv, method_name)
                        } else {
                            method_name.clone()
                        };
                        calls.push(CallSite::new(
                            caller.to_string(),
                            target,
                            CallType::Attr,
                            Some(line),
                            None,
                            receiver,
                            None,
                        ));
                    }
                }
                "instance_expression" | "object_creation" => {
                    // new ClassName()
                    let line = child.start_position().row as u32 + 1;

                    if let Some(class_name) = self.extract_type_from_instance(&child, source) {
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
                    }
                }
                _ => {}
            }
        }

        calls
    }

    /// Parse field/select expression: obj.method
    fn parse_field_expression(
        &self,
        node: &Node,
        source: &[u8],
    ) -> (Option<String>, Option<String>) {
        let mut receiver: Option<String> = None;
        let mut method: Option<String> = None;

        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                match child.kind() {
                    "identifier" | "type_identifier" => {
                        let text = get_node_text(&child, source).to_string();
                        if receiver.is_none() {
                            receiver = Some(text);
                        } else {
                            method = Some(text);
                        }
                    }
                    "field_expression" | "select_expression" => {
                        // Nested: a.b.c
                        let nested_text = get_node_text(&child, source).to_string();
                        receiver = Some(nested_text);
                    }
                    "this" => {
                        receiver = Some("this".to_string());
                    }
                    "super" => {
                        receiver = Some("super".to_string());
                    }
                    _ => {}
                }
            }
        }

        (receiver, method)
    }

    /// Parse infix expression: a method b
    fn parse_infix_expression(
        &self,
        node: &Node,
        source: &[u8],
    ) -> (Option<String>, Option<String>) {
        let mut parts: Vec<String> = Vec::new();

        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if child.kind() == "identifier" || child.kind() == "type_identifier" {
                    parts.push(get_node_text(&child, source).to_string());
                }
            }
        }

        // In `a method b`, typically: parts[0]=a, parts[1]=method (or parts[1]=b and method is an operator)
        if parts.len() >= 2 {
            // First identifier is receiver, second is method
            (Some(parts[0].clone()), Some(parts[1].clone()))
        } else if parts.len() == 1 {
            (None, Some(parts[0].clone()))
        } else {
            (None, None)
        }
    }

    /// Extract type name from instance/new expression.
    fn extract_type_from_instance(&self, node: &Node, source: &[u8]) -> Option<String> {
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                match child.kind() {
                    "type_identifier" | "identifier" => {
                        return Some(get_node_text(&child, source).to_string());
                    }
                    "generic_type" | "simple_type" => {
                        // Get the base type from generic: List[Int]
                        return self.get_identifier_from_node(&child, source);
                    }
                    _ => {}
                }
            }
        }
        None
    }
}

impl CallGraphLanguageSupport for ScalaHandler {
    fn name(&self) -> &str {
        "scala"
    }

    fn extensions(&self) -> &[&str] {
        &[".scala", ".sc"]
    }

    fn parse_imports(&self, source: &str, _path: &Path) -> Result<Vec<ImportDef>, ParseError> {
        let tree = self.parse_source(source)?;
        let source_bytes = source.as_bytes();
        let mut imports = Vec::new();

        for node in walk_tree(tree.root_node()) {
            if node.kind() == "import_declaration" {
                let parsed = self.parse_import_node(&node, source_bytes);
                imports.extend(parsed);
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
        let (defined_methods, defined_classes) = self.collect_definitions(tree, source_bytes);
        let mut calls_by_func: HashMap<String, Vec<CallSite>> = HashMap::new();

        // Track current container (class/object/trait)
        let mut current_container: Option<String> = None;

        fn process_node(
            node: Node,
            source: &[u8],
            defined_methods: &HashSet<String>,
            defined_classes: &HashSet<String>,
            calls_by_func: &mut HashMap<String, Vec<CallSite>>,
            current_container: &mut Option<String>,
            handler: &ScalaHandler,
        ) {
            match node.kind() {
                "class_definition" | "object_definition" | "trait_definition" => {
                    let mut container_name: Option<String> = None;
                    for i in 0..node.child_count() {
                        if let Some(child) = node.child(i) {
                            if child.kind() == "identifier" || child.kind() == "type_identifier" {
                                container_name = Some(get_node_text(&child, source).to_string());
                                break;
                            }
                        }
                    }

                    let old_container = current_container.take();
                    *current_container = container_name;

                    // Pattern #6/#11/#12: Extract calls from class_parameters and extends_clause
                    let init_caller = if let Some(ref cname) = current_container {
                        format!("{}.<init>", cname)
                    } else {
                        "<init>".to_string()
                    };

                    // Pattern #6/#11/#12: Extract calls from constructor defaults
                    // and super constructor args
                    for i in 0..node.child_count() {
                        if let Some(child) = node.child(i) {
                            if child.kind() == "class_parameters"
                                || child.kind() == "extends_clause"
                            {
                                let calls = handler.extract_calls_from_node(
                                    &child,
                                    source,
                                    defined_methods,
                                    defined_classes,
                                    &init_caller,
                                );
                                if !calls.is_empty() {
                                    calls_by_func
                                        .entry(init_caller.clone())
                                        .or_default()
                                        .extend(calls);
                                }
                            }
                        }
                    }

                    // Process children (recurse into template_body, etc.)
                    for i in 0..node.child_count() {
                        if let Some(child) = node.child(i) {
                            process_node(
                                child,
                                source,
                                defined_methods,
                                defined_classes,
                                calls_by_func,
                                current_container,
                                handler,
                            );
                        }
                    }

                    *current_container = old_container;
                }
                "function_definition" | "function_declaration" => {
                    let mut method_name: Option<String> = None;
                    let mut body: Option<Node> = None;
                    let mut params_node: Option<Node> = None;

                    for i in 0..node.child_count() {
                        if let Some(child) = node.child(i) {
                            match child.kind() {
                                "identifier" => {
                                    if method_name.is_none() {
                                        method_name =
                                            Some(get_node_text(&child, source).to_string());
                                    }
                                }
                                "parameters" => {
                                    params_node = Some(child);
                                }
                                "block" | "indented_block" | "template_body" => {
                                    body = Some(child);
                                }
                                // In Scala, the function body can be a direct expression
                                "call_expression" | "field_expression" | "infix_expression"
                                | "literal" => {
                                    if body.is_none() && method_name.is_some() {
                                        body = Some(child);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }

                    // If no explicit body found, use the entire function node
                    let body_node = body.unwrap_or(node);

                    if let Some(name) = method_name {
                        let full_name = if let Some(ref container) = current_container {
                            format!("{}.{}", container, name)
                        } else {
                            name.clone()
                        };

                        let mut calls = handler.extract_calls_from_node(
                            &body_node,
                            source,
                            defined_methods,
                            defined_classes,
                            &full_name,
                        );

                        // Pattern #7: Extract calls from default parameter values
                        if let Some(params) = params_node {
                            let param_calls = handler.extract_calls_from_node(
                                &params,
                                source,
                                defined_methods,
                                defined_classes,
                                &full_name,
                            );
                            calls.extend(param_calls);
                        }

                        if !calls.is_empty() {
                            calls_by_func.insert(full_name.clone(), calls.clone());
                            // Also store with simple name
                            calls_by_func.insert(name, calls);
                        }
                    }
                }
                // Pattern #3/#22: val/var initializer calls
                "val_definition" | "var_definition" => {
                    let caller = if let Some(ref container) = current_container {
                        format!("{}.<init>", container)
                    } else {
                        "<module>".to_string()
                    };

                    let calls = handler.extract_calls_from_node(
                        &node,
                        source,
                        defined_methods,
                        defined_classes,
                        &caller,
                    );

                    if !calls.is_empty() {
                        calls_by_func.entry(caller).or_default().extend(calls);
                    }
                }
                _ => {
                    // Recurse for other nodes
                    for i in 0..node.child_count() {
                        if let Some(child) = node.child(i) {
                            process_node(
                                child,
                                source,
                                defined_methods,
                                defined_classes,
                                calls_by_func,
                                current_container,
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
            &defined_methods,
            &defined_classes,
            &mut calls_by_func,
            &mut current_container,
            self,
        );

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
                "function_definition" | "function_declaration" => {
                    if let Some(name) = self.get_identifier_from_node(&node, source_bytes) {
                        let line = node.start_position().row as u32 + 1;
                        let end_line = node.end_position().row as u32 + 1;

                        // Check if inside a class/object/trait
                        let mut class_name = None;
                        let mut parent = node.parent();
                        while let Some(p) = parent {
                            if p.kind() == "template_body" {
                                if let Some(gp) = p.parent() {
                                    if gp.kind() == "class_definition"
                                        || gp.kind() == "object_definition"
                                        || gp.kind() == "trait_definition"
                                    {
                                        class_name =
                                            self.get_identifier_from_node(&gp, source_bytes);
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
                "class_definition" | "trait_definition" => {
                    if let Some(name) = self.get_identifier_from_node(&node, source_bytes) {
                        let line = node.start_position().row as u32 + 1;
                        let end_line = node.end_position().row as u32 + 1;

                        let mut methods = Vec::new();
                        let mut bases = Vec::new();

                        for i in 0..node.child_count() {
                            if let Some(child) = node.child(i) {
                                if child.kind() == "extends_clause" {
                                    for j in 0..child.child_count() {
                                        if let Some(base) = child.child(j) {
                                            if base.kind() == "type_identifier"
                                                || base.kind() == "identifier"
                                            {
                                                bases.push(
                                                    get_node_text(&base, source_bytes).to_string(),
                                                );
                                            }
                                        }
                                    }
                                }
                                if child.kind() == "template_body" {
                                    for j in 0..child.named_child_count() {
                                        if let Some(member) = child.named_child(j) {
                                            if member.kind() == "function_definition"
                                                || member.kind() == "function_declaration"
                                            {
                                                if let Some(mn) = self
                                                    .get_identifier_from_node(&member, source_bytes)
                                                {
                                                    methods.push(mn);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        classes.push(ClassDef::new(name, line, end_line, methods, bases));
                    }
                }
                "object_definition" => {
                    if let Some(name) = self.get_identifier_from_node(&node, source_bytes) {
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
        let handler = ScalaHandler::new();
        handler
            .parse_imports(source, Path::new("Test.scala"))
            .unwrap()
    }

    fn extract_calls(source: &str) -> HashMap<String, Vec<CallSite>> {
        let handler = ScalaHandler::new();
        let tree = handler.parse_source(source).unwrap();
        handler
            .extract_calls(Path::new("Test.scala"), source, &tree)
            .unwrap()
    }

    // -------------------------------------------------------------------------
    // Import Parsing Tests
    // -------------------------------------------------------------------------

    mod import_tests {
        use super::*;

        #[test]
        fn test_parse_simple_import() {
            let imports = parse_imports("import scala.collection.mutable");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].module, "scala.collection.mutable");
            assert!(!imports[0].is_wildcard());
        }

        #[test]
        fn test_parse_wildcard_import() {
            let imports = parse_imports("import scala.collection.mutable._");
            assert_eq!(imports.len(), 1);
            assert!(imports[0].module.contains("scala.collection.mutable"));
            assert!(imports[0].is_wildcard() || imports[0].is_namespace);
        }

        #[test]
        fn test_parse_selective_import() {
            let imports = parse_imports("import scala.collection.{mutable, immutable}");
            assert_eq!(imports.len(), 2);
            assert!(imports.iter().any(|i| i.module.contains("mutable")));
            assert!(imports.iter().any(|i| i.module.contains("immutable")));
        }

        #[test]
        fn test_parse_renamed_import() {
            let imports = parse_imports("import java.util.{List => JList}");
            assert_eq!(imports.len(), 1);
            assert!(imports[0].module.contains("List"));
            assert_eq!(imports[0].alias.as_deref(), Some("JList"));
        }

        #[test]
        fn test_parse_multiple_imports() {
            let source = r#"
import scala.collection.mutable
import java.util.List
import scala.concurrent._
"#;
            let imports = parse_imports(source);
            assert_eq!(imports.len(), 3);
        }

        #[test]
        fn test_parse_java_import() {
            let imports = parse_imports("import java.util.ArrayList");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].module, "java.util.ArrayList");
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
object Main {
    def main(args: Array[String]): Unit = {
        println("hello")
        helper()
    }
}
"#;
            let calls = extract_calls(source);
            let main_calls = calls.get("main").or(calls.get("Main.main"));
            assert!(main_calls.is_some(), "Expected calls from main method");
            let main_calls = main_calls.unwrap();
            assert!(main_calls.iter().any(|c| c.target.contains("println")));
            assert!(main_calls.iter().any(|c| c.target == "helper"));
        }

        #[test]
        fn test_extract_calls_intra_file() {
            let source = r#"
object Calculator {
    def add(a: Int, b: Int): Int = a + b

    def calculate(): Int = {
        add(1, 2)
    }
}
"#;
            let calls = extract_calls(source);
            let calc_calls = calls.get("Calculator.calculate");
            assert!(
                calc_calls.is_some(),
                "Expected calls from Calculator.calculate, got keys: {:?}",
                calls.keys().collect::<Vec<_>>()
            );
            let calc_calls = calc_calls.unwrap();
            let add_call = calc_calls.iter().find(|c| c.target == "add");
            assert!(add_call.is_some(), "Expected call to add");
            assert_eq!(add_call.unwrap().call_type, CallType::Intra);
        }

        #[test]
        fn test_extract_calls_method_to_toplevel() {
            let source = r#"
class Service {
    def process(): Int = {
        helper()
    }
}

def helper(): Int = 42
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
        fn test_extract_calls_method_on_object() {
            let source = r#"
class Service {
    val repo: Repository = new Repository()

    def process(): Unit = {
        repo.save(data)
    }
}
"#;
            let calls = extract_calls(source);
            let process_calls = calls.get("process").or(calls.get("Service.process"));
            assert!(
                process_calls.is_some(),
                "Expected calls from process method"
            );
            let process_calls = process_calls.unwrap();
            assert!(process_calls.iter().any(|c| c.target.contains("save")));
        }

        #[test]
        fn test_extract_calls_constructor() {
            let source = r#"
class Factory {
    def createUser(): User = {
        new User()
    }
}
"#;
            let calls = extract_calls(source);
            let create_calls = calls.get("createUser").or(calls.get("Factory.createUser"));
            assert!(
                create_calls.is_some(),
                "Expected calls from createUser method"
            );
            let create_calls = create_calls.unwrap();
            assert!(create_calls.iter().any(|c| c.target == "User"));
        }

        #[test]
        fn test_extract_calls_with_line_numbers() {
            let source = r#"object Test {
    def test(): Unit = {
        first()
        second()
    }
}"#;
            let calls = extract_calls(source);
            let test_calls = calls.get("test").or(calls.get("Test.test"));
            assert!(test_calls.is_some(), "Expected calls from test method");
            let test_calls = test_calls.unwrap();

            let first = test_calls.iter().find(|c| c.target == "first");
            let second = test_calls.iter().find(|c| c.target == "second");

            assert!(first.is_some() && second.is_some());
            assert!(first.unwrap().line.is_some());
            assert!(second.unwrap().line.is_some());
        }

        #[test]
        fn test_extract_calls_from_trait() {
            let source = r#"
trait Logger {
    def log(msg: String): Unit = {
        println(msg)
    }
}
"#;
            let calls = extract_calls(source);
            let log_calls = calls.get("log").or(calls.get("Logger.log"));
            assert!(log_calls.is_some(), "Expected calls from log method");
        }

        #[test]
        fn test_extract_calls_from_class() {
            let source = r#"
class Calculator {
    def calculate(): Int = {
        helper()
    }

    def helper(): Int = 42
}
"#;
            let calls = extract_calls(source);
            let calc_calls = calls.get("calculate").or(calls.get("Calculator.calculate"));
            assert!(calc_calls.is_some(), "Expected calls from calculate method");
        }
    }

    // -------------------------------------------------------------------------
    // Pattern Coverage Tests (Patterns #3, #6, #7, #11, #12, #14, #15, #16,
    //   #17, #19, #20, #22, #24, #25, #26, #27)
    // -------------------------------------------------------------------------

    mod pattern_tests {
        use super::*;

        // Pattern #3: Class val/var initializers
        #[test]
        fn test_val_initializer_in_object() {
            let source = r#"
object Config {
    val timeout = computeTimeout()
    var retries: Int = getRetries()
}
"#;
            let calls = extract_calls(source);
            // val/var initializers should be attributed to the container
            let all_targets: Vec<String> = calls
                .values()
                .flat_map(|v| v.iter().map(|c| c.target.clone()))
                .collect();
            assert!(
                all_targets.iter().any(|t| t == "computeTimeout"),
                "Expected call to computeTimeout from val initializer, got: {:?}",
                all_targets
            );
            assert!(
                all_targets.iter().any(|t| t == "getRetries"),
                "Expected call to getRetries from var initializer, got: {:?}",
                all_targets
            );
        }

        // Pattern #3: Class field initializer
        #[test]
        fn test_val_initializer_in_class() {
            let source = r#"
class Service {
    val repo = createRepo()

    def run(): Unit = {}
}
"#;
            let calls = extract_calls(source);
            let all_targets: Vec<String> = calls
                .values()
                .flat_map(|v| v.iter().map(|c| c.target.clone()))
                .collect();
            assert!(
                all_targets.iter().any(|t| t == "createRepo"),
                "Expected call to createRepo from class val initializer, got: {:?}",
                all_targets
            );
        }

        // Pattern #6: Constructor default params
        #[test]
        fn test_constructor_default_params() {
            let source = r#"
class Config(timeout: Int = defaultTimeout(), retries: Int = 3)
"#;
            let calls = extract_calls(source);
            let all_targets: Vec<String> = calls
                .values()
                .flat_map(|v| v.iter().map(|c| c.target.clone()))
                .collect();
            assert!(
                all_targets.iter().any(|t| t == "defaultTimeout"),
                "Expected call to defaultTimeout from constructor default param, got: {:?}",
                all_targets
            );
        }

        // Pattern #7: Function default params
        #[test]
        fn test_function_default_params() {
            let source = r#"
object Utils {
    def process(x: Int = computeDefault()): Unit = {
        println(x)
    }
}
"#;
            let calls = extract_calls(source);
            let process_calls = calls.get("process").or(calls.get("Utils.process"));
            assert!(
                process_calls.is_some(),
                "Expected calls from process, got keys: {:?}",
                calls.keys().collect::<Vec<_>>()
            );
            let process_calls = process_calls.unwrap();
            assert!(
                process_calls.iter().any(|c| c.target == "computeDefault"),
                "Expected call to computeDefault from default param, got: {:?}",
                process_calls.iter().map(|c| &c.target).collect::<Vec<_>>()
            );
        }

        // Pattern #11: Super constructor args in extends clause
        #[test]
        fn test_super_constructor_args() {
            let source = r#"
class Child extends Parent(init()) {
    def method(): Unit = {}
}
"#;
            let calls = extract_calls(source);
            let all_targets: Vec<String> = calls
                .values()
                .flat_map(|v| v.iter().map(|c| c.target.clone()))
                .collect();
            assert!(
                all_targets.iter().any(|t| t == "init"),
                "Expected call to init from super constructor args, got: {:?}",
                all_targets
            );
        }

        // Pattern #12: Enum/case class constructor args with defaults
        #[test]
        fn test_case_class_default_params() {
            let source = r#"
case class Data(x: Int = computeX(), y: String = "hello")
"#;
            let calls = extract_calls(source);
            let all_targets: Vec<String> = calls
                .values()
                .flat_map(|v| v.iter().map(|c| c.target.clone()))
                .collect();
            assert!(
                all_targets.iter().any(|t| t == "computeX"),
                "Expected call to computeX from case class default param, got: {:?}",
                all_targets
            );
        }

        // Pattern #14: Anonymous class/trait body
        #[test]
        fn test_anonymous_class_body() {
            let source = r#"
object Main {
    def create(): Unit = {
        val listener = new Listener {
            def onEvent(): Unit = handleEvent()
        }
    }
}
"#;
            let calls = extract_calls(source);
            let create_calls = calls.get("create").or(calls.get("Main.create"));
            assert!(create_calls.is_some(), "Expected calls from create");
            let _create_calls = create_calls.unwrap();
            // The anonymous class body calls should be visible somewhere
            let all_targets: Vec<String> = calls
                .values()
                .flat_map(|v| v.iter().map(|c| c.target.clone()))
                .collect();
            assert!(
                all_targets.iter().any(|t| t == "handleEvent"),
                "Expected call to handleEvent from anonymous class body, got: {:?}",
                all_targets
            );
        }

        // Pattern #15: Lambda/closure bodies
        #[test]
        fn test_lambda_body_calls() {
            let source = r#"
object Processor {
    def run(): Unit = {
        items.map(x => transform(x))
    }
}
"#;
            let calls = extract_calls(source);
            let run_calls = calls.get("run").or(calls.get("Processor.run"));
            assert!(run_calls.is_some(), "Expected calls from run");
            let run_calls = run_calls.unwrap();
            assert!(
                run_calls.iter().any(|c| c.target == "transform"),
                "Expected call to transform from lambda body, got: {:?}",
                run_calls.iter().map(|c| &c.target).collect::<Vec<_>>()
            );
        }

        // Pattern #16: For-comprehension bodies
        #[test]
        fn test_for_comprehension_calls() {
            let source = r#"
object Processor {
    def process(): Unit = {
        for { x <- getItems() } yield transform(x)
    }
}
"#;
            let calls = extract_calls(source);
            let process_calls = calls.get("process").or(calls.get("Processor.process"));
            assert!(process_calls.is_some(), "Expected calls from process");
            let process_calls = process_calls.unwrap();
            assert!(
                process_calls.iter().any(|c| c.target == "getItems"),
                "Expected call to getItems in for-comprehension, got: {:?}",
                process_calls.iter().map(|c| &c.target).collect::<Vec<_>>()
            );
            assert!(
                process_calls.iter().any(|c| c.target == "transform"),
                "Expected call to transform in for-comprehension yield, got: {:?}",
                process_calls.iter().map(|c| &c.target).collect::<Vec<_>>()
            );
        }

        // Pattern #17: Conditional expression calls
        #[test]
        fn test_if_expression_calls() {
            let source = r#"
object Logic {
    def decide(): Unit = {
        if (check()) doA() else doB()
    }
}
"#;
            let calls = extract_calls(source);
            let decide_calls = calls.get("decide").or(calls.get("Logic.decide"));
            assert!(decide_calls.is_some(), "Expected calls from decide");
            let decide_calls = decide_calls.unwrap();
            assert!(
                decide_calls.iter().any(|c| c.target == "check"),
                "Expected call to check in if-expression, got: {:?}",
                decide_calls.iter().map(|c| &c.target).collect::<Vec<_>>()
            );
            assert!(
                decide_calls.iter().any(|c| c.target == "doA"),
                "Expected call to doA in if-expression, got: {:?}",
                decide_calls.iter().map(|c| &c.target).collect::<Vec<_>>()
            );
            assert!(
                decide_calls.iter().any(|c| c.target == "doB"),
                "Expected call to doB in if-expression, got: {:?}",
                decide_calls.iter().map(|c| &c.target).collect::<Vec<_>>()
            );
        }

        // Pattern #22: Companion object val initializers
        #[test]
        fn test_companion_object_val() {
            let source = r#"
object Companion {
    val shared = initialize()
    val config: Config = loadConfig()
}
"#;
            let calls = extract_calls(source);
            let all_targets: Vec<String> = calls
                .values()
                .flat_map(|v| v.iter().map(|c| c.target.clone()))
                .collect();
            assert!(
                all_targets.iter().any(|t| t == "initialize"),
                "Expected call to initialize from companion object val, got: {:?}",
                all_targets
            );
            assert!(
                all_targets.iter().any(|t| t == "loadConfig"),
                "Expected call to loadConfig from companion object val, got: {:?}",
                all_targets
            );
        }

        // Pattern #24: String interpolation
        #[test]
        fn test_string_interpolation_calls() {
            let source = r#"
object Formatter {
    def format(): String = {
        s"Result: ${compute()}"
    }
}
"#;
            let calls = extract_calls(source);
            let format_calls = calls.get("format").or(calls.get("Formatter.format"));
            assert!(format_calls.is_some(), "Expected calls from format");
            let format_calls = format_calls.unwrap();
            assert!(
                format_calls.iter().any(|c| c.target == "compute"),
                "Expected call to compute in string interpolation, got: {:?}",
                format_calls.iter().map(|c| &c.target).collect::<Vec<_>>()
            );
        }

        // Pattern #25: Collection literals with calls
        #[test]
        fn test_collection_literal_calls() {
            let source = r#"
object Builder {
    def build(): Unit = {
        List(createA(), createB())
    }
}
"#;
            let calls = extract_calls(source);
            let build_calls = calls.get("build").or(calls.get("Builder.build"));
            assert!(build_calls.is_some(), "Expected calls from build");
            let build_calls = build_calls.unwrap();
            assert!(
                build_calls.iter().any(|c| c.target == "createA"),
                "Expected call to createA in collection literal, got: {:?}",
                build_calls.iter().map(|c| &c.target).collect::<Vec<_>>()
            );
            assert!(
                build_calls.iter().any(|c| c.target == "createB"),
                "Expected call to createB in collection literal, got: {:?}",
                build_calls.iter().map(|c| &c.target).collect::<Vec<_>>()
            );
        }

        // Pattern #26: Trait default method bodies
        #[test]
        fn test_trait_default_method() {
            let source = r#"
trait Defaults {
    def computed(): Int = calculate()
    def withArg(x: Int): Int = transform(x)
}
"#;
            let calls = extract_calls(source);
            let computed_calls = calls.get("computed").or(calls.get("Defaults.computed"));
            assert!(
                computed_calls.is_some(),
                "Expected calls from trait default method computed, got keys: {:?}",
                calls.keys().collect::<Vec<_>>()
            );
            let computed_calls = computed_calls.unwrap();
            assert!(
                computed_calls.iter().any(|c| c.target == "calculate"),
                "Expected call to calculate from trait default method, got: {:?}",
                computed_calls.iter().map(|c| &c.target).collect::<Vec<_>>()
            );
        }

        // Pattern #27: Guard clauses in pattern matching
        #[test]
        fn test_guard_clause_calls() {
            let source = r#"
object Matcher {
    def matchIt(x: Any): Unit = {
        x match {
            case y if isValid(y) => handle(y)
        }
    }
}
"#;
            let calls = extract_calls(source);
            let match_calls = calls.get("matchIt").or(calls.get("Matcher.matchIt"));
            assert!(match_calls.is_some(), "Expected calls from matchIt");
            let match_calls = match_calls.unwrap();
            assert!(
                match_calls.iter().any(|c| c.target == "isValid"),
                "Expected call to isValid from guard clause, got: {:?}",
                match_calls.iter().map(|c| &c.target).collect::<Vec<_>>()
            );
            assert!(
                match_calls.iter().any(|c| c.target == "handle"),
                "Expected call to handle from case body, got: {:?}",
                match_calls.iter().map(|c| &c.target).collect::<Vec<_>>()
            );
        }

        // Pattern #2: Top-level / package object calls
        #[test]
        fn test_top_level_val_initializer() {
            let source = r#"
val globalConfig = loadGlobal()
"#;
            let calls = extract_calls(source);
            let all_targets: Vec<String> = calls
                .values()
                .flat_map(|v| v.iter().map(|c| c.target.clone()))
                .collect();
            assert!(
                all_targets.iter().any(|t| t == "loadGlobal"),
                "Expected call to loadGlobal from top-level val, got: {:?}",
                all_targets
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
            let handler = ScalaHandler::new();
            assert_eq!(handler.name(), "scala");
        }

        #[test]
        fn test_handler_extensions() {
            let handler = ScalaHandler::new();
            let exts = handler.extensions();
            assert!(exts.contains(&".scala"));
            assert!(exts.contains(&".sc"));
            assert_eq!(exts.len(), 2);
        }

        #[test]
        fn test_handler_supports() {
            let handler = ScalaHandler::new();
            assert!(handler.supports("scala"));
            assert!(handler.supports("Scala"));
            assert!(handler.supports("SCALA"));
            assert!(!handler.supports("java"));
        }

        #[test]
        fn test_handler_supports_extension() {
            let handler = ScalaHandler::new();
            assert!(handler.supports_extension(".scala"));
            assert!(handler.supports_extension(".SCALA"));
            assert!(handler.supports_extension(".sc"));
            assert!(!handler.supports_extension(".java"));
        }
    }
}
