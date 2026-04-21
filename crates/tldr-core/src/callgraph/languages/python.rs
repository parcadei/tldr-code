//! Python language handler for call graph analysis.
//!
//! This module provides Python-specific call graph support using tree-sitter-python.
//!
//! # Import Patterns Supported
//!
//! | Pattern | ImportDef |
//! |---------|-----------|
//! | `import os` | `{module: "os", is_from: false}` |
//! | `import os as o` | `{module: "os", alias: "o"}` |
//! | `from os import path` | `{module: "os", is_from: true, names: ["path"]}` |
//! | `from os import path as p` | `{module: "os", names: ["path"], aliases: {"p": "path"}}` |
//! | `from . import types` | `{module: "", is_from: true, names: ["types"], level: 1}` |
//! | `from ..utils import helper` | `{module: "utils", names: ["helper"], level: 2}` |
//! | `from pkg import *` | `{module: "pkg", names: ["*"]}` |
//!
//! # Call Extraction
//!
//! - Direct calls: `func()` -> CallType::Direct or CallType::Intra
//! - Attribute calls: `obj.method()` -> CallType::Attr
//! - Function references: `map(func, ...)` -> CallType::Ref
//!
//! # Spec Reference
//!
//! See `migration/spec/callgraph-spec.md` Section 9.1 for Python-specific details.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use tree_sitter::{Node, Parser, Tree};

use super::base::{get_node_text, walk_tree};
use super::{CallGraphLanguageSupport, ParseError};
use crate::callgraph::cross_file_types::{CallSite, CallType, ClassDef, FuncDef, ImportDef};

// =============================================================================
// Python Handler
// =============================================================================

/// Python language handler using tree-sitter-python.
///
/// Supports:
/// - Import parsing (all Python import styles including relative imports)
/// - Call extraction (direct, method, attribute, references)
/// - TYPE_CHECKING block detection
/// - Nested function tracking via parent_function
/// - `<module>` synthetic function for module-level calls
#[derive(Debug, Default)]
pub struct PythonHandler;

impl PythonHandler {
    /// Creates a new PythonHandler.
    pub fn new() -> Self {
        Self
    }

    /// Parse the source code into a tree-sitter Tree.
    fn parse_source(&self, source: &str) -> Result<Tree, ParseError> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .map_err(|e| ParseError::ParseFailed {
                file: std::path::PathBuf::new(),
                message: format!("Failed to set Python language: {}", e),
            })?;

        parser
            .parse(source, None)
            .ok_or_else(|| ParseError::ParseFailed {
                file: std::path::PathBuf::new(),
                message: "Parser returned None".to_string(),
            })
    }

    /// Check if a node is inside a TYPE_CHECKING block.
    fn is_in_type_checking_block(&self, node: &Node, source: &[u8]) -> bool {
        let mut current = node.parent();
        while let Some(parent) = current {
            if parent.kind() == "if_statement" {
                // Check if the condition is TYPE_CHECKING
                if let Some(condition) = parent.child_by_field_name("condition") {
                    let cond_text = get_node_text(&condition, source);
                    if cond_text == "TYPE_CHECKING"
                        || cond_text == "typing.TYPE_CHECKING"
                        || cond_text.ends_with(".TYPE_CHECKING")
                    {
                        return true;
                    }
                }
            }
            current = parent.parent();
        }
        false
    }

    /// Parse a single import statement node.
    fn parse_import_statement(&self, node: &Node, source: &[u8]) -> Vec<ImportDef> {
        let mut imports = Vec::new();

        match node.kind() {
            "import_statement" => {
                // import X, import X as Y
                for i in 0..node.named_child_count() {
                    if let Some(child) = node.named_child(i) {
                        match child.kind() {
                            "dotted_name" => {
                                let module = get_node_text(&child, source).to_string();
                                imports.push(ImportDef::simple_import(module));
                            }
                            "aliased_import" => {
                                // import X as Y
                                let mut module = String::new();
                                let mut alias = None;
                                for j in 0..child.named_child_count() {
                                    if let Some(gc) = child.named_child(j) {
                                        match gc.kind() {
                                            "dotted_name" => {
                                                module = get_node_text(&gc, source).to_string();
                                            }
                                            "identifier" => {
                                                alias =
                                                    Some(get_node_text(&gc, source).to_string());
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                                if !module.is_empty() {
                                    let mut imp = ImportDef::simple_import(module);
                                    imp.alias = alias;
                                    imports.push(imp);
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            "import_from_statement" => {
                // from X import Y, from . import Y, from ..X import Y
                let mut module = String::new();
                let mut level: u8 = 0;
                let mut names = Vec::new();
                let mut aliases: HashMap<String, String> = HashMap::new();
                let mut is_wildcard = false;

                // Handle relative imports
                // tree-sitter-python uses a "relative_import" node containing dots and module
                // e.g., "from . import X" has relative_import="."
                // e.g., "from ..utils import X" has relative_import="..utils"
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        if child.kind() == "relative_import" {
                            let text = get_node_text(&child, source);
                            // Count leading dots
                            for c in text.chars() {
                                if c == '.' {
                                    level += 1;
                                } else {
                                    break;
                                }
                            }
                            // Extract module name (part after dots)
                            let module_part: String =
                                text.chars().skip_while(|&c| c == '.').collect();
                            if !module_part.is_empty() {
                                module = module_part;
                            }
                            break;
                        }
                    }
                }

                // For non-relative imports, get module name from module_name field
                if level == 0 {
                    if let Some(module_node) = node.child_by_field_name("module_name") {
                        module = get_node_text(&module_node, source).to_string();
                    }
                }

                // Parse imported names
                for i in 0..node.named_child_count() {
                    if let Some(child) = node.named_child(i) {
                        match child.kind() {
                            "dotted_name" | "identifier" => {
                                // Skip the module name itself
                                let text = get_node_text(&child, source);
                                if text != module && !text.is_empty() {
                                    names.push(text.to_string());
                                }
                            }
                            "aliased_import" => {
                                // from X import Y as Z
                                let mut orig_name = String::new();
                                let mut alias_name = None;
                                for j in 0..child.named_child_count() {
                                    if let Some(gc) = child.named_child(j) {
                                        match gc.kind() {
                                            "dotted_name" | "identifier" => {
                                                if orig_name.is_empty() {
                                                    orig_name =
                                                        get_node_text(&gc, source).to_string();
                                                } else {
                                                    alias_name = Some(
                                                        get_node_text(&gc, source).to_string(),
                                                    );
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                                if !orig_name.is_empty() {
                                    names.push(orig_name.clone());
                                    if let Some(alias) = alias_name {
                                        aliases.insert(alias, orig_name);
                                    }
                                }
                            }
                            "wildcard_import" => {
                                is_wildcard = true;
                            }
                            _ => {}
                        }
                    }
                }

                if is_wildcard {
                    names = vec!["*".to_string()];
                }

                // Create the ImportDef
                let mut imp = if level > 0 {
                    ImportDef::relative_import(module, names, level)
                } else {
                    ImportDef::from_import(module, names)
                };

                if !aliases.is_empty() {
                    imp.aliases = Some(aliases);
                }

                // Check if inside TYPE_CHECKING block
                imp.is_type_checking = self.is_in_type_checking_block(node, source);

                imports.push(imp);
            }
            _ => {}
        }

        imports
    }

    /// Collect all function and class names defined in the file.
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
                    if let Some(name_node) = node.child_by_field_name("name") {
                        functions.insert(get_node_text(&name_node, source).to_string());
                    }
                }
                "class_definition" => {
                    if let Some(name_node) = node.child_by_field_name("name") {
                        classes.insert(get_node_text(&name_node, source).to_string());
                    }
                }
                _ => {}
            }
        }

        (functions, classes)
    }

    /// Extract calls from a function body node.
    fn extract_calls_from_node(
        &self,
        node: &Node,
        source: &[u8],
        defined_funcs: &HashSet<String>,
        defined_classes: &HashSet<String>,
        caller: &str,
        line_offset: u32,
    ) -> Vec<CallSite> {
        let mut calls = Vec::new();
        let mut refs = HashSet::new();

        // Walk the node tree
        for child in walk_tree(*node) {
            match child.kind() {
                "call" => {
                    // Get the function being called
                    if let Some(func_node) = child.child_by_field_name("function") {
                        let line = child.start_position().row as u32 + 1 + line_offset;

                        match func_node.kind() {
                            "identifier" => {
                                // Direct call: func()
                                let target = get_node_text(&func_node, source).to_string();
                                let call_type = if defined_funcs.contains(&target)
                                    || defined_classes.contains(&target)
                                {
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
                            "attribute" => {
                                // Attribute call: obj.method()
                                let full_target = get_node_text(&func_node, source).to_string();
                                // Extract receiver (obj) from obj.method
                                let receiver = if let Some(obj_node) =
                                    func_node.child_by_field_name("object")
                                {
                                    Some(get_node_text(&obj_node, source).to_string())
                                } else {
                                    // Fallback: split on first dot
                                    full_target.split('.').next().map(|s| s.to_string())
                                };

                                calls.push(CallSite::new(
                                    caller.to_string(),
                                    full_target,
                                    CallType::Attr,
                                    Some(line),
                                    None,
                                    receiver,
                                    None,
                                ));
                            }
                            _ => {
                                // Other call patterns (subscript, etc.)
                                let target = get_node_text(&func_node, source).to_string();
                                calls.push(CallSite::new(
                                    caller.to_string(),
                                    target,
                                    CallType::Direct,
                                    Some(line),
                                    None,
                                    None,
                                    None,
                                ));
                            }
                        }
                    }
                }
                "identifier" => {
                    // Check for function references (not in calls, but used as values)
                    let name = get_node_text(&child, source);
                    if defined_funcs.contains(name) {
                        // Check if this identifier is NOT the function part of a call
                        if let Some(parent) = child.parent() {
                            if parent.kind() != "call"
                                && parent.child_by_field_name("function").as_ref() != Some(&child)
                            {
                                refs.insert(name.to_string());
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // Add function references
        for ref_name in refs {
            let line = node.start_position().row as u32 + 1;
            calls.push(CallSite::new(
                caller.to_string(),
                ref_name,
                CallType::Ref,
                Some(line),
                None,
                None,
                None,
            ));
        }

        calls
    }
}

impl CallGraphLanguageSupport for PythonHandler {
    fn name(&self) -> &str {
        "python"
    }

    fn extensions(&self) -> &[&str] {
        &[".py", ".pyi"]
    }

    fn parse_imports(&self, source: &str, _path: &Path) -> Result<Vec<ImportDef>, ParseError> {
        let tree = self.parse_source(source)?;
        let source_bytes = source.as_bytes();
        let mut imports = Vec::new();

        for node in walk_tree(tree.root_node()) {
            match node.kind() {
                "import_statement" | "import_from_statement" => {
                    imports.extend(self.parse_import_statement(&node, source_bytes));
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

        // Extract calls from each function (includes default params and decorators)
        for node in walk_tree(tree.root_node()) {
            if node.kind() == "function_definition" {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let func_name = get_node_text(&name_node, source_bytes).to_string();

                    // FIX: Determine if this function is a method inside a class
                    // by walking up the parent chain to find the enclosing class.
                    // This ensures calls from ClassA.method and ClassB.method are
                    // recorded separately with qualified caller names.
                    let mut caller_name = func_name.clone();
                    let mut current = node.parent();
                    while let Some(parent) = current {
                        if parent.kind() == "block" {
                            if let Some(gp) = parent.parent() {
                                if gp.kind() == "class_definition" {
                                    if let Some(class_name_node) = gp.child_by_field_name("name") {
                                        let class_name =
                                            get_node_text(&class_name_node, source_bytes);
                                        caller_name = format!("{}.{}", class_name, func_name);
                                    }
                                    break;
                                }
                            }
                        }
                        current = parent.parent();
                    }

                    let mut func_calls = Vec::new();

                    // Pattern 9: Extract calls from decorators
                    // In tree-sitter-python, decorated functions are wrapped in
                    // `decorated_definition` which has `decorator` + `function_definition`
                    // as siblings. The decorator is NOT a child of function_definition.
                    if let Some(parent) = node.parent() {
                        if parent.kind() == "decorated_definition" {
                            for i in 0..parent.child_count() {
                                if let Some(child) = parent.child(i) {
                                    if child.kind() == "decorator" {
                                        // Only extract actual calls from decorators
                                        // @app.route("/api") has a call node inside
                                        // @login_required does NOT (just identifier/attribute)
                                        let decorator_calls = self.extract_calls_from_node(
                                            &child,
                                            source_bytes,
                                            &defined_funcs,
                                            &defined_classes,
                                            &caller_name,
                                            0,
                                        );
                                        func_calls.extend(decorator_calls);
                                    }
                                }
                            }
                        }
                    }

                    // Pattern 6/7: Extract calls from default parameter values
                    if let Some(params_node) = node.child_by_field_name("parameters") {
                        let param_calls = self.extract_calls_from_node(
                            &params_node,
                            source_bytes,
                            &defined_funcs,
                            &defined_classes,
                            &caller_name,
                            0,
                        );
                        func_calls.extend(param_calls);
                    }

                    // Extract calls from the function body (existing behavior)
                    if let Some(body_node) = node.child_by_field_name("body") {
                        let calls = self.extract_calls_from_node(
                            &body_node,
                            source_bytes,
                            &defined_funcs,
                            &defined_classes,
                            &caller_name,
                            0,
                        );
                        func_calls.extend(calls);
                    }

                    if !func_calls.is_empty() {
                        calls_by_func
                            .entry(caller_name)
                            .or_default()
                            .extend(func_calls);
                    }
                }
            }
        }

        // Pattern 3/21: Extract calls from class body field initializers
        for node in walk_tree(tree.root_node()) {
            if node.kind() == "class_definition" {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let class_name = get_node_text(&name_node, source_bytes).to_string();

                    if let Some(body) = node.child_by_field_name("body") {
                        let mut class_calls = Vec::new();
                        // Walk direct children of the class body (block node)
                        // Skip function_definition and class_definition (methods/nested classes)
                        for i in 0..body.named_child_count() {
                            if let Some(child) = body.named_child(i) {
                                if matches!(
                                    child.kind(),
                                    "function_definition"
                                        | "class_definition"
                                        | "decorated_definition"
                                ) {
                                    continue;
                                }
                                // Extract calls from class-level statements
                                // e.g., timeout = compute_timeout(), name = Column(String(50))
                                let calls = self.extract_calls_from_node(
                                    &child,
                                    source_bytes,
                                    &defined_funcs,
                                    &defined_classes,
                                    &class_name,
                                    0,
                                );
                                class_calls.extend(calls);
                            }
                        }
                        if !class_calls.is_empty() {
                            calls_by_func
                                .entry(class_name)
                                .or_default()
                                .extend(class_calls);
                        }
                    }
                }
            }
        }

        // Extract module-level calls into synthetic <module> function
        let mut module_calls = Vec::new();
        for node in tree.root_node().children(&mut tree.root_node().walk()) {
            // Skip function and class definitions
            if matches!(node.kind(), "function_definition" | "class_definition") {
                continue;
            }

            // Extract calls from this module-level statement
            let calls = self.extract_calls_from_node(
                &node,
                source_bytes,
                &defined_funcs,
                &defined_classes,
                "<module>",
                0,
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

                        // Check if inside a class
                        let mut class_name = None;
                        let mut parent = node.parent();
                        while let Some(p) = parent {
                            if p.kind() == "block" {
                                if let Some(gp) = p.parent() {
                                    if gp.kind() == "class_definition" {
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
                "class_definition" => {
                    if let Some(name_node) = node.child_by_field_name("name") {
                        let class_name = get_node_text(&name_node, source_bytes).to_string();
                        let line = node.start_position().row as u32 + 1;
                        let end_line = node.end_position().row as u32 + 1;

                        // Collect base classes from argument_list
                        let mut bases = Vec::new();
                        if let Some(args) = node.child_by_field_name("superclasses") {
                            for i in 0..args.child_count() {
                                if let Some(arg) = args.child(i) {
                                    if arg.kind() == "identifier" {
                                        bases.push(get_node_text(&arg, source_bytes).to_string());
                                    }
                                }
                            }
                        }

                        // Collect method names from the body
                        let mut methods = Vec::new();
                        if let Some(body) = node.child_by_field_name("body") {
                            for i in 0..body.named_child_count() {
                                if let Some(child) = body.named_child(i) {
                                    if child.kind() == "function_definition" {
                                        if let Some(fn_name) = child.child_by_field_name("name") {
                                            methods.push(
                                                get_node_text(&fn_name, source_bytes).to_string(),
                                            );
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
        let handler = PythonHandler::new();
        handler.parse_imports(source, Path::new("test.py")).unwrap()
    }

    fn extract_calls(source: &str) -> HashMap<String, Vec<CallSite>> {
        let handler = PythonHandler::new();
        let tree = handler.parse_source(source).unwrap();
        handler
            .extract_calls(Path::new("test.py"), source, &tree)
            .unwrap()
    }

    // -------------------------------------------------------------------------
    // Import Parsing Tests
    // -------------------------------------------------------------------------

    mod import_tests {
        use super::*;

        #[test]
        fn test_parse_import_simple() {
            let imports = parse_imports("import os");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].module, "os");
            assert!(!imports[0].is_from);
            assert!(imports[0].names.is_empty());
        }

        #[test]
        fn test_parse_import_dotted() {
            let imports = parse_imports("import os.path");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].module, "os.path");
        }

        #[test]
        fn test_parse_import_as() {
            let imports = parse_imports("import numpy as np");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].module, "numpy");
            assert_eq!(imports[0].alias, Some("np".to_string()));
        }

        #[test]
        fn test_parse_from_import() {
            let imports = parse_imports("from os import path");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].module, "os");
            assert!(imports[0].is_from);
            assert_eq!(imports[0].names, vec!["path"]);
        }

        #[test]
        fn test_parse_from_import_multiple() {
            let imports = parse_imports("from os import path, getcwd");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].module, "os");
            assert_eq!(imports[0].names.len(), 2);
            assert!(imports[0].names.contains(&"path".to_string()));
            assert!(imports[0].names.contains(&"getcwd".to_string()));
        }

        #[test]
        fn test_parse_from_import_as() {
            let imports = parse_imports("from os import path as p");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].names, vec!["path"]);
            let aliases = imports[0].aliases.as_ref().unwrap();
            assert_eq!(aliases.get("p"), Some(&"path".to_string()));
        }

        #[test]
        fn test_parse_relative_import_level1() {
            let imports = parse_imports("from . import types");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].level, 1);
            assert!(imports[0].is_relative());
            assert_eq!(imports[0].names, vec!["types"]);
        }

        #[test]
        fn test_parse_relative_import_level2() {
            let imports = parse_imports("from ..utils import helper");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].level, 2);
            assert_eq!(imports[0].module, "utils");
            assert_eq!(imports[0].names, vec!["helper"]);
        }

        #[test]
        fn test_parse_relative_import_level1_with_module() {
            let imports = parse_imports("from .core import MyClass");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].level, 1);
            assert_eq!(imports[0].module, "core");
            assert_eq!(imports[0].names, vec!["MyClass"]);
        }

        #[test]
        fn test_parse_wildcard_import() {
            let imports = parse_imports("from pkg import *");
            assert_eq!(imports.len(), 1);
            assert!(imports[0].is_wildcard());
            assert_eq!(imports[0].names, vec!["*"]);
        }

        #[test]
        fn test_parse_type_checking_import() {
            let source = r#"
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from mymodule import MyClass
"#;
            let imports = parse_imports(source);
            // Find the MyClass import
            let myclass_import = imports
                .iter()
                .find(|i| i.names.contains(&"MyClass".to_string()));
            assert!(myclass_import.is_some());
            assert!(myclass_import.unwrap().is_type_checking);
        }

        #[test]
        fn test_parse_multiple_imports() {
            let source = r#"
import os
import sys
from pathlib import Path
from collections import defaultdict, Counter
"#;
            let imports = parse_imports(source);
            assert_eq!(imports.len(), 4);
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
def main():
    print("hello")
    some_external_func()
"#;
            let calls = extract_calls(source);
            let main_calls = calls.get("main").unwrap();
            assert!(main_calls.iter().any(|c| c.target == "print"));
            assert!(main_calls.iter().any(|c| c.target == "some_external_func"));
        }

        #[test]
        fn test_extract_calls_intra_file() {
            let source = r#"
def helper():
    pass

def main():
    helper()
"#;
            let calls = extract_calls(source);
            let main_calls = calls.get("main").unwrap();
            let helper_call = main_calls.iter().find(|c| c.target == "helper").unwrap();
            assert_eq!(helper_call.call_type, CallType::Intra);
        }

        #[test]
        fn test_extract_calls_method() {
            let source = r#"
def main():
    obj.method()
    self.internal()
"#;
            let calls = extract_calls(source);
            let main_calls = calls.get("main").unwrap();

            let obj_method = main_calls
                .iter()
                .find(|c| c.target == "obj.method")
                .unwrap();
            assert_eq!(obj_method.call_type, CallType::Attr);
            assert_eq!(obj_method.receiver, Some("obj".to_string()));

            let self_method = main_calls
                .iter()
                .find(|c| c.target == "self.internal")
                .unwrap();
            assert_eq!(self_method.call_type, CallType::Attr);
            assert_eq!(self_method.receiver, Some("self".to_string()));
        }

        #[test]
        fn test_extract_calls_chained() {
            let source = r#"
def main():
    a.b.c.d()
"#;
            let calls = extract_calls(source);
            let main_calls = calls.get("main").unwrap();
            assert!(main_calls.iter().any(|c| c.target == "a.b.c.d"));
        }

        #[test]
        fn test_extract_calls_class_instantiation() {
            let source = r#"
class MyClass:
    pass

def main():
    obj = MyClass()
"#;
            let calls = extract_calls(source);
            let main_calls = calls.get("main").unwrap();
            let instantiation = main_calls.iter().find(|c| c.target == "MyClass").unwrap();
            assert_eq!(instantiation.call_type, CallType::Intra);
        }

        #[test]
        fn test_extract_calls_module_level() {
            let source = r#"
def helper():
    pass

# Module-level call
result = helper()
"#;
            let calls = extract_calls(source);
            assert!(calls.contains_key("<module>"));
            let module_calls = calls.get("<module>").unwrap();
            assert!(module_calls.iter().any(|c| c.target == "helper"));
        }

        #[test]
        fn test_extract_calls_with_line_numbers() {
            let source = r#"def main():
    first_call()
    second_call()
"#;
            let calls = extract_calls(source);
            let main_calls = calls.get("main").unwrap();

            let first = main_calls
                .iter()
                .find(|c| c.target == "first_call")
                .unwrap();
            let second = main_calls
                .iter()
                .find(|c| c.target == "second_call")
                .unwrap();

            // Line numbers should be 1-indexed and different
            assert!(first.line.is_some());
            assert!(second.line.is_some());
            assert!(second.line.unwrap() > first.line.unwrap());
        }
    }

    // -------------------------------------------------------------------------
    // New Pattern Tests: Default Params, Decorators, Class Body Calls
    // -------------------------------------------------------------------------

    mod new_pattern_tests {
        use super::*;

        // --- Pattern 6/7: Default parameter calls ---

        #[test]
        fn test_default_param_direct_call() {
            let source = r#"
def greet(default_name=get_default()):
    pass
"#;
            let calls = extract_calls(source);
            let greet_calls = calls.get("greet").expect("greet should have calls");
            assert!(
                greet_calls.iter().any(|c| c.target == "get_default"),
                "get_default() in default param should be extracted. Got: {:?}",
                greet_calls
            );
        }

        #[test]
        fn test_default_param_method_call() {
            let _source = r#"
def greet(formatter=str.upper):
    pass
"#;
            // str.upper is NOT a call (no parens), so no calls expected from default params
            // But if it were str.upper(), it would be a call. Let's test the actual call case:
            let source_with_call = r#"
def greet(formatter=str.upper()):
    pass
"#;
            let calls = extract_calls(source_with_call);
            let greet_calls = calls.get("greet").expect("greet should have calls");
            assert!(
                greet_calls.iter().any(|c| c.target == "str.upper"),
                "str.upper() in default param should be extracted. Got: {:?}",
                greet_calls
            );
        }

        #[test]
        fn test_default_param_in_class_method() {
            let source = r#"
class Service:
    def __init__(self, db=connect_db(), timeout=compute_timeout()):
        pass
"#;
            let calls = extract_calls(source);
            // After fix, caller name is qualified with class name
            let init_calls = calls
                .get("Service.__init__")
                .expect("Service.__init__ should have calls");
            assert!(
                init_calls.iter().any(|c| c.target == "connect_db"),
                "connect_db() in default param should be extracted. Got: {:?}",
                init_calls
            );
            assert!(
                init_calls.iter().any(|c| c.target == "compute_timeout"),
                "compute_timeout() in default param should be extracted. Got: {:?}",
                init_calls
            );
        }

        #[test]
        fn test_default_param_multiple() {
            let source = r#"
def configure(a=make_a(), b=make_b(), c="static"):
    pass
"#;
            let calls = extract_calls(source);
            let conf_calls = calls.get("configure").expect("configure should have calls");
            assert!(
                conf_calls.iter().any(|c| c.target == "make_a"),
                "make_a() should be extracted. Got: {:?}",
                conf_calls
            );
            assert!(
                conf_calls.iter().any(|c| c.target == "make_b"),
                "make_b() should be extracted. Got: {:?}",
                conf_calls
            );
            // "static" is not a call
            assert!(
                !conf_calls.iter().any(|c| c.target == "static"),
                "static string should not be a call"
            );
        }

        // --- Pattern 9: Decorator calls ---

        #[test]
        fn test_decorator_call_with_args() {
            let source = r#"
@app.route("/api")
def handle_api():
    pass
"#;
            let calls = extract_calls(source);
            let api_calls = calls
                .get("handle_api")
                .expect("handle_api should have calls");
            assert!(
                api_calls.iter().any(|c| c.target == "app.route"),
                "app.route(\"/api\") decorator should be extracted. Got: {:?}",
                api_calls
            );
        }

        #[test]
        fn test_decorator_without_args_not_call() {
            let source = r#"
@login_required
def dashboard():
    pass
"#;
            let calls = extract_calls(source);
            // @login_required without () is NOT a call, just a reference
            // dashboard should have no calls (or only non-decorator calls)
            let dashboard_calls = calls.get("dashboard");
            if let Some(dc) = dashboard_calls {
                assert!(
                    !dc.iter().any(|c| c.target == "login_required"),
                    "login_required without () should NOT be extracted as a call. Got: {:?}",
                    dc
                );
            }
        }

        #[test]
        fn test_decorator_nested_call() {
            let source = r#"
@pytest.mark.parametrize("x", [1, 2])
def test_something():
    pass
"#;
            let calls = extract_calls(source);
            let test_calls = calls
                .get("test_something")
                .expect("test_something should have calls");
            assert!(
                test_calls
                    .iter()
                    .any(|c| c.target == "pytest.mark.parametrize"),
                "pytest.mark.parametrize() decorator should be extracted. Got: {:?}",
                test_calls
            );
        }

        #[test]
        fn test_decorator_simple_call() {
            let source = r#"
@my_decorator()
def my_func():
    pass
"#;
            let calls = extract_calls(source);
            let func_calls = calls.get("my_func").expect("my_func should have calls");
            assert!(
                func_calls.iter().any(|c| c.target == "my_decorator"),
                "my_decorator() decorator should be extracted. Got: {:?}",
                func_calls
            );
        }

        // --- Pattern 3/21: Class body field initializer calls ---

        #[test]
        fn test_class_body_field_call() {
            let source = r#"
class Config:
    timeout = compute_timeout()
    handler = create_handler()
    CONSTANT = "no call here"
"#;
            let calls = extract_calls(source);
            let config_calls = calls.get("Config").expect("Config should have calls");
            assert!(
                config_calls.iter().any(|c| c.target == "compute_timeout"),
                "compute_timeout() in class body should be extracted. Got: {:?}",
                config_calls
            );
            assert!(
                config_calls.iter().any(|c| c.target == "create_handler"),
                "create_handler() in class body should be extracted. Got: {:?}",
                config_calls
            );
        }

        #[test]
        fn test_class_body_dsl_nested_calls() {
            let source = r#"
class User(Base):
    name = Column(String(50))
    age = Column(Integer())
"#;
            let calls = extract_calls(source);
            let user_calls = calls.get("User").expect("User should have calls");
            assert!(
                user_calls.iter().any(|c| c.target == "Column"),
                "Column() in class body should be extracted. Got: {:?}",
                user_calls
            );
            assert!(
                user_calls.iter().any(|c| c.target == "String"),
                "String() in class body should be extracted. Got: {:?}",
                user_calls
            );
            assert!(
                user_calls.iter().any(|c| c.target == "Integer"),
                "Integer() in class body should be extracted. Got: {:?}",
                user_calls
            );
        }

        #[test]
        fn test_class_body_no_false_positives() {
            let source = r#"
class Config:
    NAME = "static"
    VALUE = 42
    ITEMS = [1, 2, 3]
"#;
            let calls = extract_calls(source);
            // No calls in class body (all static assignments)
            assert!(
                !calls.contains_key("Config"),
                "Config should have no calls for static assignments. Got: {:?}",
                calls.get("Config")
            );
        }

        #[test]
        fn test_class_body_method_call() {
            let source = r#"
class MyModel(Model):
    objects = Manager()
    db = get_connection()
"#;
            let calls = extract_calls(source);
            let model_calls = calls.get("MyModel").expect("MyModel should have calls");
            assert!(
                model_calls.iter().any(|c| c.target == "Manager"),
                "Manager() should be extracted. Got: {:?}",
                model_calls
            );
            assert!(
                model_calls.iter().any(|c| c.target == "get_connection"),
                "get_connection() should be extracted. Got: {:?}",
                model_calls
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
            let handler = PythonHandler::new();
            assert_eq!(handler.name(), "python");
        }

        #[test]
        fn test_handler_extensions() {
            let handler = PythonHandler::new();
            let exts = handler.extensions();
            assert!(exts.contains(&".py"));
            assert!(exts.contains(&".pyi"));
        }

        #[test]
        fn test_handler_supports() {
            let handler = PythonHandler::new();
            assert!(handler.supports("python"));
            assert!(handler.supports("Python"));
            assert!(handler.supports("PYTHON"));
            assert!(!handler.supports("javascript"));
        }

        #[test]
        fn test_handler_supports_extension() {
            let handler = PythonHandler::new();
            assert!(handler.supports_extension(".py"));
            assert!(handler.supports_extension(".pyi"));
            assert!(handler.supports_extension(".PY"));
            assert!(!handler.supports_extension(".js"));
        }
    }

    // Debug test for cross-scope method extraction
    #[test]
    fn test_extract_group_shell_complete() {
        // Simplified version of the click/core.py Group.shell_complete structure
        let source = r#"
class Command:
    def shell_complete(self, ctx, incomplete):
        return []

class Group(Command):
    def get_command(self, ctx, cmd_name):
        pass
    
    def shell_complete(self, ctx, incomplete):
        results = []
        results.extend(super().shell_complete(ctx, incomplete))
        return results
"#;
        let handler = PythonHandler::new();
        let tree = handler.parse_source(source).unwrap();
        let (funcs, _classes) = handler
            .extract_definitions(source, Path::new("test.py"), &tree)
            .unwrap();

        // Check that both shell_complete methods are found
        let command_shell_complete = funcs.iter().find(|f| {
            f.name == "shell_complete" && f.class_name.as_ref().is_some_and(|n| n == "Command")
        });
        let group_shell_complete = funcs.iter().find(|f| {
            f.name == "shell_complete" && f.class_name.as_ref().is_some_and(|n| n == "Group")
        });

        assert!(
            command_shell_complete.is_some(),
            "Command.shell_complete should be found"
        );
        assert!(
            group_shell_complete.is_some(),
            "Group.shell_complete should be found"
        );

        // Now check extract_calls
        let calls = handler
            .extract_calls(Path::new("test.py"), source, &tree)
            .unwrap();

        // Group.shell_complete should have calls
        // After fix, caller name is qualified with class name
        let group_method_calls = calls.get("Group.shell_complete");
        println!("Calls from Group.shell_complete: {:?}", group_method_calls);

        // It should call super().shell_complete()
        assert!(
            group_method_calls.is_some(),
            "Group.shell_complete should have calls recorded"
        );
    }

    // Test for the actual bug: generator expression calls
    #[test]
    fn test_extract_calls_in_generator_expression() {
        let source = r#"
def _complete_visible_commands(ctx, incomplete):
    return []

class Command:
    def shell_complete(self, ctx, incomplete):
        results = []
        results.extend(
            (name, cmd)
            for name, cmd in _complete_visible_commands(ctx, incomplete)
            if name
        )
        return results
"#;
        let handler = PythonHandler::new();
        let tree = handler.parse_source(source).unwrap();
        let calls = handler
            .extract_calls(Path::new("test.py"), source, &tree)
            .unwrap();

        println!("All calls: {:?}", calls);

        // shell_complete should have a call to _complete_visible_commands
        // After fix, caller name is qualified with class name
        let shell_calls = calls
            .get("Command.shell_complete")
            .expect("Command.shell_complete should have calls");
        let helper_call = shell_calls
            .iter()
            .find(|c| c.target == "_complete_visible_commands");

        assert!(
            helper_call.is_some(),
            "Should find call to _complete_visible_commands in generator expression. Got: {:?}",
            shell_calls
        );

        // And it should be Intra since it's defined in the same file
        let call = helper_call.unwrap();
        assert_eq!(
            call.call_type,
            CallType::Intra,
            "Call to same-file function should be Intra"
        );
    }

    // Test with the actual click/core.py file structure
    #[test]
    #[ignore] // Requires external fixture: /tmp/purity-realworld/python-click/src/click/core.py
    fn test_extract_from_real_click_core() {
        use std::fs;

        let source =
            fs::read_to_string("/tmp/purity-realworld/python-click/src/click/core.py").unwrap();
        let handler = PythonHandler::new();
        let tree = handler.parse_source(&source).unwrap();

        let (funcs, _classes) = handler
            .extract_definitions(&source, Path::new("core.py"), &tree)
            .unwrap();

        // Check that _complete_visible_commands is found
        let helper_func = funcs
            .iter()
            .find(|f| f.name == "_complete_visible_commands");
        println!(
            "_complete_visible_commands found: {:?}",
            helper_func.is_some()
        );

        // Check that Command.shell_complete and Group.shell_complete are found
        let command_methods: Vec<_> = funcs
            .iter()
            .filter(|f| {
                f.name == "shell_complete"
                    && f.class_name.as_ref().is_some_and(|n| n == "Command")
            })
            .collect();
        let group_methods: Vec<_> = funcs
            .iter()
            .filter(|f| {
                f.name == "shell_complete" && f.class_name.as_ref().is_some_and(|n| n == "Group")
            })
            .collect();

        println!(
            "Command.shell_complete found: {} times",
            command_methods.len()
        );
        println!("Group.shell_complete found: {} times", group_methods.len());

        for m in &command_methods {
            println!("  Command.shell_complete at line {}-{}", m.line, m.end_line);
        }
        for m in &group_methods {
            println!("  Group.shell_complete at line {}-{}", m.line, m.end_line);
        }

        // Now check calls
        let calls = handler
            .extract_calls(Path::new("core.py"), &source, &tree)
            .unwrap();

        // Find calls to _complete_visible_commands
        for (caller, call_sites) in &calls {
            for call in call_sites {
                if call.target == "_complete_visible_commands" {
                    println!(
                        "Found call from '{}' to _complete_visible_commands (type: {:?})",
                        caller, call.call_type
                    );
                }
            }
        }

        // The bug: Group.shell_complete is NOT in the nodes, but Command.shell_complete is
        // Let's verify this
        let group_shell_complete_node = funcs.iter().find(|f| {
            f.name == "shell_complete" && f.class_name.as_ref().is_some_and(|n| n == "Group")
        });
        let command_shell_complete_node = funcs.iter().find(|f| {
            f.name == "shell_complete" && f.class_name.as_ref().is_some_and(|n| n == "Command")
        });

        println!("\nIn extract_definitions:");
        println!(
            "  Group.shell_complete: {:?}",
            group_shell_complete_node.is_some()
        );
        println!(
            "  Command.shell_complete: {:?}",
            command_shell_complete_node.is_some()
        );

        // These should both be Some
        assert!(
            group_shell_complete_node.is_some(),
            "Group.shell_complete should be found by extract_definitions"
        );
        assert!(
            command_shell_complete_node.is_some(),
            "Command.shell_complete should be found by extract_definitions"
        );
    }

    // Test for the actual bug: cross-scope intra-file calls from methods to top-level functions
    #[test]
    fn test_extract_calls_method_to_toplevel() {
        let source = r#"
def helper_func():
    pass

class MyClass:
    def method(self):
        helper_func()
"#;
        let handler = PythonHandler::new();
        let tree = handler.parse_source(source).unwrap();
        let calls = handler
            .extract_calls(Path::new("test.py"), source, &tree)
            .unwrap();

        // The method should have a call to helper_func marked as Intra
        // After the fix, the caller name is qualified as "MyClass.method"
        let method_calls = calls
            .get("MyClass.method")
            .expect("MyClass.method should have calls");
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

    // Test that demonstrates the fix: multiple methods with same name in different classes
    // should now have their calls recorded separately with qualified caller names
    #[test]
    fn test_multiple_methods_same_name() {
        let source = r#"
def _helper():
    pass

class Command:
    def shell_complete(self):
        _helper()

class Group(Command):
    def shell_complete(self):
        _helper()
"#;
        let handler = PythonHandler::new();
        let tree = handler.parse_source(source).unwrap();
        let calls = handler
            .extract_calls(Path::new("test.py"), source, &tree)
            .unwrap();

        println!("Calls: {:?}", calls);

        // After the fix, calls should be recorded with qualified names
        // Command.shell_complete and Group.shell_complete are separate entries
        let command_calls = calls.get("Command.shell_complete");
        let group_calls = calls.get("Group.shell_complete");

        assert!(
            command_calls.is_some(),
            "Should have calls from Command.shell_complete"
        );
        assert!(
            group_calls.is_some(),
            "Should have calls from Group.shell_complete"
        );

        // Each should have 1 call to _helper
        let cmd_helper_calls: Vec<_> = command_calls
            .unwrap()
            .iter()
            .filter(|c| c.target == "_helper")
            .collect();
        let group_helper_calls: Vec<_> = group_calls
            .unwrap()
            .iter()
            .filter(|c| c.target == "_helper")
            .collect();

        assert_eq!(
            cmd_helper_calls.len(),
            1,
            "Command.shell_complete should have 1 call to _helper"
        );
        assert_eq!(
            group_helper_calls.len(),
            1,
            "Group.shell_complete should have 1 call to _helper"
        );

        // The old unqualified name should NOT exist (or be empty)
        let old_shell_calls = calls.get("shell_complete");
        assert!(
            old_shell_calls.is_none() || old_shell_calls.unwrap().is_empty(),
            "Unqualified 'shell_complete' should not have calls after fix"
        );
    }

    // Test the exact scenario from the bug report: two classes with same method name
    #[test]
    fn test_two_classes_same_method_name() {
        let source = r#"
def helper():
    pass

class A:
    def method(self):
        helper()  # Line 7

class B:
    def method(self):
        helper()  # Line 11
"#;
        let handler = PythonHandler::new();
        let tree = handler.parse_source(source).unwrap();
        let calls = handler
            .extract_calls(Path::new("test.py"), source, &tree)
            .unwrap();

        println!("Calls: {:?}", calls);

        // Both A.method and B.method should be separate entries
        let a_method_calls = calls.get("A.method");
        let b_method_calls = calls.get("B.method");

        assert!(a_method_calls.is_some(), "Should have calls from A.method");
        assert!(b_method_calls.is_some(), "Should have calls from B.method");

        // Each should have 1 call to helper
        let a_helper: Vec<_> = a_method_calls
            .unwrap()
            .iter()
            .filter(|c| c.target == "helper")
            .collect();
        let b_helper: Vec<_> = b_method_calls
            .unwrap()
            .iter()
            .filter(|c| c.target == "helper")
            .collect();

        assert_eq!(a_helper.len(), 1, "A.method should have 1 call to helper");
        assert_eq!(b_helper.len(), 1, "B.method should have 1 call to helper");

        // Both should be Intra calls
        assert_eq!(a_helper[0].call_type, CallType::Intra);
        assert_eq!(b_helper[0].call_type, CallType::Intra);
    }

    // Test with the actual click/core.py to verify extraction works
    #[test]
    #[ignore] // Requires external fixture: /tmp/purity-realworld/python-click/src/click/core.py
    fn test_click_core_extraction() {
        use std::fs;

        let source =
            fs::read_to_string("/tmp/purity-realworld/python-click/src/click/core.py").unwrap();
        let handler = PythonHandler::new();
        let tree = handler.parse_source(&source).unwrap();
        let calls = handler
            .extract_calls(Path::new("core.py"), &source, &tree)
            .unwrap();

        // Count calls to _complete_visible_commands
        let mut total_calls = 0;
        let mut callers = Vec::new();
        for (caller, call_sites) in &calls {
            for call in call_sites {
                if call.target == "_complete_visible_commands" {
                    total_calls += 1;
                    callers.push(caller.clone());
                }
            }
        }

        println!("Total calls to _complete_visible_commands: {}", total_calls);
        println!("Callers: {:?}", callers);

        // Should have at least 2 calls (from Command.shell_complete and Group.shell_complete)
        assert!(
            total_calls >= 2,
            "Should have at least 2 calls to _complete_visible_commands, found {}",
            total_calls
        );

        // Should have Command.shell_complete and Group.shell_complete as callers
        assert!(
            callers.iter().any(|c| c.contains("Command.shell_complete")),
            "Should have Command.shell_complete as caller"
        );
        assert!(
            callers.iter().any(|c| c.contains("Group.shell_complete")),
            "Should have Group.shell_complete as caller"
        );
    }
}
