//! Java language handler for call graph analysis.
//!
//! This module provides Java-specific call graph support using tree-sitter-java.
//!
//! # Import Patterns Supported
//!
//! | Pattern | ImportDef |
//! |---------|-----------|
//! | `import java.util.List;` | `{module: "java.util.List"}` |
//! | `import java.util.*;` | `{module: "java.util.*", names: ["*"]}` |
//! | `import static java.lang.Math.PI;` | `{module: "java.lang.Math.PI", is_static: true}` |
//! | `import static java.util.Arrays.*;` | `{module: "java.util.Arrays.*", is_static: true}` |
//!
//! # Call Extraction
//!
//! - Direct calls: `method()` -> CallType::Direct or CallType::Intra
//! - Method calls: `obj.method()` -> CallType::Attr
//! - Static calls: `Class.method()` -> CallType::Attr
//! - Constructor calls: `new Class()` -> CallType::Direct
//! - Chained calls: `obj.method1().method2()` -> multiple CallType::Attr
//!
//! # Spec Reference
//!
//! See `migration/spec/callgraph-spec.md` Section 9.5 for Java-specific details.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use tree_sitter::{Node, Parser, Tree};

use super::base::{get_node_text, walk_tree};
use super::common::extend_calls_if_any;
use super::{CallGraphLanguageSupport, ParseError};
use crate::callgraph::cross_file_types::{CallSite, CallType, ClassDef, FuncDef, ImportDef};

// =============================================================================
// Java Handler
// =============================================================================

/// Java language handler using tree-sitter-java.
///
/// Supports:
/// - Import parsing (standard, wildcard, static imports)
/// - Call extraction (direct, method, constructor, static)
/// - Class and interface method tracking
/// - Nested class support
#[derive(Debug, Default)]
pub struct JavaHandler;

impl JavaHandler {
    /// Creates a new JavaHandler.
    pub fn new() -> Self {
        Self
    }

    /// Parse the source code into a tree-sitter Tree.
    fn parse_source(&self, source: &str) -> Result<Tree, ParseError> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_java::LANGUAGE.into())
            .map_err(|e| ParseError::ParseFailed {
                file: std::path::PathBuf::new(),
                message: format!("Failed to set Java language: {}", e),
            })?;

        parser
            .parse(source, None)
            .ok_or_else(|| ParseError::ParseFailed {
                file: std::path::PathBuf::new(),
                message: "Parser returned None".to_string(),
            })
    }

    /// Parse an import declaration node.
    fn parse_import_node(&self, node: &Node, source: &[u8]) -> Option<ImportDef> {
        if node.kind() != "import_declaration" {
            return None;
        }

        let text = get_node_text(node, source);
        let is_static = text.contains("static ");
        let is_wildcard = text.trim_end_matches(';').ends_with('*');

        // Find the scoped_identifier or identifier for the import path
        let mut module: Option<String> = None;

        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                match child.kind() {
                    "scoped_identifier" => {
                        module = Some(get_node_text(&child, source).to_string());
                    }
                    "identifier" => {
                        if module.is_none() {
                            module = Some(get_node_text(&child, source).to_string());
                        }
                    }
                    "asterisk" => {
                        // Wildcard import
                        if let Some(ref mut m) = module {
                            if !m.ends_with(".*") {
                                m.push_str(".*");
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        let module = module?;

        let mut import_def = if is_wildcard {
            let mut imp = ImportDef::from_import(module, vec!["*".to_string()]);
            imp.is_namespace = true;
            imp
        } else {
            ImportDef::simple_import(module)
        };

        if is_static {
            // Mark as static import using a custom field
            // We'll use the 'is_type_checking' field to indicate static (a bit of a hack)
            // Or we can extend ImportDef later
            import_def.is_type_checking = true; // Using as is_static marker
        }

        Some(import_def)
    }

    /// Collect all class, interface, and method definitions.
    fn collect_definitions(
        &self,
        tree: &Tree,
        source: &[u8],
    ) -> (HashSet<String>, HashSet<String>) {
        let mut methods = HashSet::new();
        let mut classes = HashSet::new();

        for node in walk_tree(tree.root_node()) {
            match node.kind() {
                "method_declaration" => {
                    // Get method name
                    if let Some(name) = self.get_identifier_from_node(&node, source) {
                        methods.insert(name);
                    }
                }
                "constructor_declaration" => {
                    // Constructor name matches class name
                    if let Some(name) = self.get_identifier_from_node(&node, source) {
                        methods.insert(name.clone());
                        classes.insert(name);
                    }
                }
                "class_declaration" | "interface_declaration" | "enum_declaration" => {
                    if let Some(name) = self.get_identifier_from_node(&node, source) {
                        classes.insert(name);
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
                if child.kind() == "identifier" {
                    return Some(get_node_text(&child, source).to_string());
                }
            }
        }
        None
    }

    /// Extract calls from a method/constructor body.
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
                "method_invocation" => {
                    let line = child.start_position().row as u32 + 1;

                    // Parse method invocation: obj.method() or method()
                    let mut object_name: Option<String> = None;
                    let mut method_name: Option<String> = None;
                    let mut saw_dot = false;
                    let mut first_identifier: Option<String> = None;

                    for i in 0..child.child_count() {
                        if let Some(c) = child.child(i) {
                            match c.kind() {
                                "identifier" => {
                                    let text = get_node_text(&c, source).to_string();
                                    if first_identifier.is_none() {
                                        first_identifier = Some(text);
                                    } else if saw_dot {
                                        // This is the method name after a dot
                                        object_name = first_identifier.take();
                                        method_name = Some(text);
                                    } else {
                                        method_name = Some(text);
                                    }
                                }
                                "." => {
                                    saw_dot = true;
                                }
                                "this" => {
                                    object_name = Some("this".to_string());
                                }
                                "super" => {
                                    object_name = Some("super".to_string());
                                }
                                "field_access" => {
                                    // obj.field.method() - get the full receiver
                                    object_name = Some(get_node_text(&c, source).to_string());
                                }
                                "argument_list" => {
                                    // Skip argument list
                                }
                                _ => {}
                            }
                        }
                    }

                    // If no method_name found, first_identifier is the method
                    if method_name.is_none() {
                        method_name = first_identifier;
                    }

                    if let Some(method) = method_name {
                        if let Some(obj) = object_name {
                            // Method call on object
                            let target = format!("{}.{}", obj, method);
                            calls.push(CallSite::new(
                                caller.to_string(),
                                target,
                                CallType::Attr,
                                Some(line),
                                None,
                                Some(obj),
                                None,
                            ));
                        } else {
                            // Direct method call
                            let call_type = if defined_methods.contains(&method) {
                                CallType::Intra
                            } else {
                                CallType::Direct
                            };
                            calls.push(CallSite::new(
                                caller.to_string(),
                                method,
                                call_type,
                                Some(line),
                                None,
                                None,
                                None,
                            ));
                        }
                    }
                }
                "object_creation_expression" => {
                    // new ClassName()
                    let line = child.start_position().row as u32 + 1;

                    for i in 0..child.child_count() {
                        if let Some(c) = child.child(i) {
                            if c.kind() == "type_identifier" {
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

    fn recurse_children_for_call_extraction(
        &self,
        node: Node,
        source: &[u8],
        defined_methods: &HashSet<String>,
        defined_classes: &HashSet<String>,
        calls_by_func: &mut HashMap<String, Vec<CallSite>>,
        current_class: &mut Option<String>,
    ) {
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                self.process_extract_calls_node(
                    child,
                    source,
                    defined_methods,
                    defined_classes,
                    calls_by_func,
                    current_class,
                );
            }
        }
    }

    fn process_extract_calls_node(
        &self,
        node: Node,
        source: &[u8],
        defined_methods: &HashSet<String>,
        defined_classes: &HashSet<String>,
        calls_by_func: &mut HashMap<String, Vec<CallSite>>,
        current_class: &mut Option<String>,
    ) {
        match node.kind() {
            "class_declaration" | "interface_declaration" | "enum_declaration" => {
                self.handle_type_declaration_calls(
                    node,
                    source,
                    defined_methods,
                    defined_classes,
                    calls_by_func,
                    current_class,
                );
            }
            "method_declaration" | "constructor_declaration" => {
                self.handle_method_or_constructor_calls(
                    node,
                    source,
                    defined_methods,
                    defined_classes,
                    calls_by_func,
                    current_class,
                );
            }
            "field_declaration" => {
                self.handle_field_declaration_calls(
                    node,
                    source,
                    defined_methods,
                    defined_classes,
                    calls_by_func,
                    current_class,
                );
            }
            "static_initializer" => {
                self.handle_static_initializer_calls(
                    node,
                    source,
                    defined_methods,
                    defined_classes,
                    calls_by_func,
                    current_class,
                );
            }
            "block" => {
                self.handle_block_calls(
                    node,
                    source,
                    defined_methods,
                    defined_classes,
                    calls_by_func,
                    current_class,
                );
            }
            "enum_constant" => {
                self.handle_enum_constant_calls(
                    node,
                    source,
                    defined_methods,
                    defined_classes,
                    calls_by_func,
                    current_class,
                );
            }
            _ => {
                self.recurse_children_for_call_extraction(
                    node,
                    source,
                    defined_methods,
                    defined_classes,
                    calls_by_func,
                    current_class,
                );
            }
        }
    }

    fn handle_type_declaration_calls(
        &self,
        node: Node,
        source: &[u8],
        defined_methods: &HashSet<String>,
        defined_classes: &HashSet<String>,
        calls_by_func: &mut HashMap<String, Vec<CallSite>>,
        current_class: &mut Option<String>,
    ) {
        let mut class_name: Option<String> = None;
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if child.kind() == "identifier" {
                    class_name = Some(get_node_text(&child, source).to_string());
                    break;
                }
            }
        }

        let old_class = current_class.take();
        *current_class = class_name;
        self.recurse_children_for_call_extraction(
            node,
            source,
            defined_methods,
            defined_classes,
            calls_by_func,
            current_class,
        );
        *current_class = old_class;
    }

    fn handle_method_or_constructor_calls(
        &self,
        node: Node,
        source: &[u8],
        defined_methods: &HashSet<String>,
        defined_classes: &HashSet<String>,
        calls_by_func: &mut HashMap<String, Vec<CallSite>>,
        current_class: &Option<String>,
    ) {
        let mut method_name: Option<String> = None;
        let mut body: Option<Node> = None;

        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                match child.kind() {
                    "identifier" => {
                        if method_name.is_none() {
                            method_name = Some(get_node_text(&child, source).to_string());
                        }
                    }
                    "block" | "constructor_body" => body = Some(child),
                    _ => {}
                }
            }
        }

        let (Some(name), Some(body_node)) = (method_name, body) else {
            return;
        };
        let caller = if let Some(class) = current_class {
            format!("{class}.{name}")
        } else {
            name
        };
        let calls = self.extract_calls_from_node(
            &body_node,
            source,
            defined_methods,
            defined_classes,
            &caller,
        );
        if !calls.is_empty() {
            calls_by_func.insert(caller, calls);
        }
    }

    fn handle_field_declaration_calls(
        &self,
        node: Node,
        source: &[u8],
        defined_methods: &HashSet<String>,
        defined_classes: &HashSet<String>,
        calls_by_func: &mut HashMap<String, Vec<CallSite>>,
        current_class: &Option<String>,
    ) {
        let Some(class) = current_class.as_deref() else {
            return;
        };
        let is_static = (0..node.child_count()).any(|i| {
            node.child(i).is_some_and(|child| {
                child.kind() == "modifiers" && get_node_text(&child, source).contains("static")
            })
        });
        let caller = if is_static {
            format!("{class}.<clinit>")
        } else {
            format!("{class}.<init>")
        };
        let calls =
            self.extract_calls_from_node(&node, source, defined_methods, defined_classes, &caller);
        extend_calls_if_any(calls_by_func, caller, calls);
    }

    fn handle_static_initializer_calls(
        &self,
        node: Node,
        source: &[u8],
        defined_methods: &HashSet<String>,
        defined_classes: &HashSet<String>,
        calls_by_func: &mut HashMap<String, Vec<CallSite>>,
        current_class: &Option<String>,
    ) {
        let Some(class) = current_class.as_deref() else {
            return;
        };
        let caller = format!("{class}.<clinit>");
        let calls =
            self.extract_calls_from_node(&node, source, defined_methods, defined_classes, &caller);
        extend_calls_if_any(calls_by_func, caller, calls);
    }

    fn handle_block_calls(
        &self,
        node: Node,
        source: &[u8],
        defined_methods: &HashSet<String>,
        defined_classes: &HashSet<String>,
        calls_by_func: &mut HashMap<String, Vec<CallSite>>,
        current_class: &mut Option<String>,
    ) {
        let is_instance_init = node
            .parent()
            .is_some_and(|parent| parent.kind() == "class_body");
        if !is_instance_init {
            self.recurse_children_for_call_extraction(
                node,
                source,
                defined_methods,
                defined_classes,
                calls_by_func,
                current_class,
            );
            return;
        }

        let Some(class) = current_class.as_deref() else {
            return;
        };
        let caller = format!("{class}.<init>");
        let calls =
            self.extract_calls_from_node(&node, source, defined_methods, defined_classes, &caller);
        extend_calls_if_any(calls_by_func, caller, calls);
    }

    fn handle_enum_constant_calls(
        &self,
        node: Node,
        source: &[u8],
        defined_methods: &HashSet<String>,
        defined_classes: &HashSet<String>,
        calls_by_func: &mut HashMap<String, Vec<CallSite>>,
        current_class: &Option<String>,
    ) {
        let Some(class) = current_class.as_deref() else {
            return;
        };
        let caller = format!("{class}.<clinit>");
        let calls =
            self.extract_calls_from_node(&node, source, defined_methods, defined_classes, &caller);
        extend_calls_if_any(calls_by_func, caller, calls);
    }
}

impl CallGraphLanguageSupport for JavaHandler {
    fn name(&self) -> &str {
        "java"
    }

    fn extensions(&self) -> &[&str] {
        &[".java"]
    }

    fn parse_imports(&self, source: &str, _path: &Path) -> Result<Vec<ImportDef>, ParseError> {
        let tree = self.parse_source(source)?;
        let source_bytes = source.as_bytes();
        let mut imports = Vec::new();

        for node in walk_tree(tree.root_node()) {
            if node.kind() == "import_declaration" {
                if let Some(imp) = self.parse_import_node(&node, source_bytes) {
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
        let (defined_methods, defined_classes) = self.collect_definitions(tree, source_bytes);
        let mut calls_by_func: HashMap<String, Vec<CallSite>> = HashMap::new();
        let mut current_class: Option<String> = None;
        self.process_extract_calls_node(
            tree.root_node(),
            source_bytes,
            &defined_methods,
            &defined_classes,
            &mut calls_by_func,
            &mut current_class,
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
                "method_declaration" | "constructor_declaration" => {
                    if let Some(name) = self.get_identifier_from_node(&node, source_bytes) {
                        let line = node.start_position().row as u32 + 1;
                        let end_line = node.end_position().row as u32 + 1;

                        // Check if inside a class
                        let mut class_name = None;
                        let mut parent = node.parent();
                        while let Some(p) = parent {
                            if p.kind() == "class_body" {
                                if let Some(gp) = p.parent() {
                                    if gp.kind() == "class_declaration"
                                        || gp.kind() == "interface_declaration"
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
                "class_declaration" | "interface_declaration" | "enum_declaration" => {
                    if let Some(name) = self.get_identifier_from_node(&node, source_bytes) {
                        let line = node.start_position().row as u32 + 1;
                        let end_line = node.end_position().row as u32 + 1;

                        // Collect method names and base classes
                        let mut methods = Vec::new();
                        let mut bases = Vec::new();

                        for i in 0..node.child_count() {
                            if let Some(child) = node.child(i) {
                                if child.kind() == "superclass"
                                    || child.kind() == "super_interfaces"
                                {
                                    for j in 0..child.child_count() {
                                        if let Some(tc) = child.child(j) {
                                            if tc.kind() == "type_identifier" {
                                                bases.push(
                                                    get_node_text(&tc, source_bytes).to_string(),
                                                );
                                            }
                                            if tc.kind() == "type_list" {
                                                for k in 0..tc.child_count() {
                                                    if let Some(t) = tc.child(k) {
                                                        if t.kind() == "type_identifier" {
                                                            bases.push(
                                                                get_node_text(&t, source_bytes)
                                                                    .to_string(),
                                                            );
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                if child.kind() == "class_body" {
                                    for j in 0..child.named_child_count() {
                                        if let Some(member) = child.named_child(j) {
                                            if member.kind() == "method_declaration"
                                                || member.kind() == "constructor_declaration"
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
        let handler = JavaHandler::new();
        handler
            .parse_imports(source, Path::new("Test.java"))
            .unwrap()
    }

    fn extract_calls(source: &str) -> HashMap<String, Vec<CallSite>> {
        let handler = JavaHandler::new();
        let tree = handler.parse_source(source).unwrap();
        handler
            .extract_calls(Path::new("Test.java"), source, &tree)
            .unwrap()
    }

    // -------------------------------------------------------------------------
    // Import Parsing Tests
    // -------------------------------------------------------------------------

    mod import_tests {
        use super::*;

        #[test]
        fn test_parse_simple_import() {
            let imports = parse_imports("import java.util.List;");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].module, "java.util.List");
            assert!(!imports[0].is_wildcard());
        }

        #[test]
        fn test_parse_wildcard_import() {
            let imports = parse_imports("import java.util.*;");
            assert_eq!(imports.len(), 1);
            assert!(imports[0].module.contains("java.util"));
            assert!(imports[0].is_wildcard() || imports[0].is_namespace);
        }

        #[test]
        fn test_parse_static_import() {
            let imports = parse_imports("import static java.lang.Math.PI;");
            assert_eq!(imports.len(), 1);
            assert!(imports[0].module.contains("Math"));
            assert!(imports[0].is_type_checking); // We use this as is_static marker
        }

        #[test]
        fn test_parse_static_wildcard_import() {
            let imports = parse_imports("import static java.util.Arrays.*;");
            assert_eq!(imports.len(), 1);
            assert!(imports[0].module.contains("Arrays"));
        }

        #[test]
        fn test_parse_multiple_imports() {
            let source = r#"
import java.util.List;
import java.util.Map;
import java.io.*;
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
public class Test {
    public void main() {
        System.out.println("hello");
        helper();
    }
}
"#;
            let calls = extract_calls(source);
            let main_calls = calls.get("Test.main").unwrap();
            assert!(main_calls.iter().any(|c| c.target.contains("println")));
            assert!(main_calls.iter().any(|c| c.target == "helper"));
        }

        #[test]
        fn test_extract_calls_intra_file() {
            let source = r#"
public class Calculator {
    public int add(int a, int b) {
        return a + b;
    }

    public int calculate() {
        return add(1, 2);
    }
}
"#;
            let calls = extract_calls(source);
            let calc_calls = calls.get("Calculator.calculate").unwrap();
            let add_call = calc_calls.iter().find(|c| c.target == "add").unwrap();
            assert_eq!(add_call.call_type, CallType::Intra);
        }

        #[test]
        fn test_extract_calls_constructor() {
            let source = r#"
public class Factory {
    public User createUser() {
        return new User();
    }
}
"#;
            let calls = extract_calls(source);
            let create_calls = calls.get("Factory.createUser").unwrap();
            assert!(create_calls.iter().any(|c| c.target == "User"));
        }

        #[test]
        fn test_extract_calls_method_on_object() {
            let source = r#"
public class Service {
    private Repository repo;

    public void process() {
        repo.save(data);
        this.validate();
    }
}
"#;
            let calls = extract_calls(source);
            let process_calls = calls.get("Service.process").unwrap();
            assert!(process_calls.iter().any(|c| c.target.contains("save")));
            assert!(process_calls.iter().any(|c| c.target.contains("validate")));
        }

        #[test]
        fn test_extract_calls_static_method() {
            let source = r#"
public class MathUtils {
    public double calculate() {
        return Math.sqrt(16);
    }
}
"#;
            let calls = extract_calls(source);
            let calc_calls = calls.get("MathUtils.calculate").unwrap();
            assert!(calc_calls.iter().any(|c| c.target.contains("sqrt")));
        }

        #[test]
        fn test_extract_calls_with_line_numbers() {
            let source = r#"public class Test {
    public void test() {
        first();
        second();
    }
}"#;
            let calls = extract_calls(source);
            let test_calls = calls.get("Test.test").unwrap();

            let first = test_calls.iter().find(|c| c.target == "first").unwrap();
            let second = test_calls.iter().find(|c| c.target == "second").unwrap();

            assert!(first.line.is_some());
            assert!(second.line.is_some());
            assert!(second.line.unwrap() > first.line.unwrap());
        }

        #[test]
        fn test_extract_calls_method_to_toplevel() {
            // Two classes with methods of the same name calling the same top-level function
            // Calls should be recorded separately with qualified names
            let source = r#"
public class FirstClass {
    public void process() {
        helper();
        sharedUtil();
    }
}

public class SecondClass {
    public void process() {
        otherHelper();
        sharedUtil();
    }
}

// Top-level helper functions (simulated as static in a utils class pattern)
class Utils {
    static void sharedUtil() {}
}
"#;
            let calls = extract_calls(source);

            // Verify FirstClass.process exists and calls sharedUtil
            let first_process = calls
                .get("FirstClass.process")
                .expect("FirstClass.process should exist");
            assert!(
                first_process.iter().any(|c| c.target == "sharedUtil"),
                "FirstClass.process should call sharedUtil"
            );
            assert!(
                first_process.iter().any(|c| c.target == "helper"),
                "FirstClass.process should call helper"
            );

            // Verify SecondClass.process exists and calls sharedUtil
            let second_process = calls
                .get("SecondClass.process")
                .expect("SecondClass.process should exist");
            assert!(
                second_process.iter().any(|c| c.target == "sharedUtil"),
                "SecondClass.process should call sharedUtil"
            );
            assert!(
                second_process.iter().any(|c| c.target == "otherHelper"),
                "SecondClass.process should call otherHelper"
            );

            // Verify the two process methods are distinct entries (not merged)
            assert!(
                calls.contains_key("FirstClass.process") && calls.contains_key("SecondClass.process"),
                "Both FirstClass.process and SecondClass.process should be present as separate keys"
            );
        }
    }

    // -------------------------------------------------------------------------
    // Class-Level Pattern Tests (field init, static init, instance init, enum, lambda)
    // -------------------------------------------------------------------------

    mod class_level_pattern_tests {
        use super::*;

        #[test]
        fn test_field_initializer_with_call() {
            let source = r#"
public class Config {
    private Handler handler = createHandler();

    static Handler createHandler() { return null; }
}
"#;
            let calls = extract_calls(source);
            // Field initializer call should be attributed to Config.<init>
            let init_calls = calls
                .get("Config.<init>")
                .expect("Config.<init> should have calls");
            assert!(
                init_calls.iter().any(|c| c.target == "createHandler"),
                "Field init should extract createHandler() call; got: {:?}",
                init_calls
            );
        }

        #[test]
        fn test_static_field_initializer_with_method_call() {
            let source = r#"
public class Config {
    private static Settings settings = Settings.load();
}
"#;
            let calls = extract_calls(source);
            // Static field initializer should be attributed to Config.<clinit>
            let clinit_calls = calls
                .get("Config.<clinit>")
                .expect("Config.<clinit> should have calls");
            assert!(
                clinit_calls.iter().any(|c| c.target.contains("load")),
                "Static field init should extract Settings.load() call; got: {:?}",
                clinit_calls
            );
        }

        #[test]
        fn test_static_initializer_block() {
            let source = r#"
public class Registry {
    static {
        initialize();
        configure();
    }

    static void initialize() {}
    static void configure() {}
}
"#;
            let calls = extract_calls(source);
            let clinit_calls = calls
                .get("Registry.<clinit>")
                .expect("Registry.<clinit> should have calls");
            assert!(
                clinit_calls.iter().any(|c| c.target == "initialize"),
                "Static init block should extract initialize(); got: {:?}",
                clinit_calls
            );
            assert!(
                clinit_calls.iter().any(|c| c.target == "configure"),
                "Static init block should extract configure(); got: {:?}",
                clinit_calls
            );
        }

        #[test]
        fn test_instance_initializer_block() {
            let source = r#"
public class Widget {
    {
        setup();
    }

    void setup() {}
}
"#;
            let calls = extract_calls(source);
            let init_calls = calls
                .get("Widget.<init>")
                .expect("Widget.<init> should have calls");
            assert!(
                init_calls.iter().any(|c| c.target == "setup"),
                "Instance init block should extract setup(); got: {:?}",
                init_calls
            );
        }

        #[test]
        fn test_enum_constructor_args() {
            let source = r#"
public class Palette {
    enum Color {
        RED(createRed()),
        BLUE(createBlue());
        Color(Object o) {}
    }

    static Object createRed() { return null; }
    static Object createBlue() { return null; }
}
"#;
            let calls = extract_calls(source);
            let clinit_calls = calls
                .get("Color.<clinit>")
                .expect("Color.<clinit> should have calls");
            assert!(
                clinit_calls.iter().any(|c| c.target == "createRed"),
                "Enum constant should extract createRed(); got: {:?}",
                clinit_calls
            );
            assert!(
                clinit_calls.iter().any(|c| c.target == "createBlue"),
                "Enum constant should extract createBlue(); got: {:?}",
                clinit_calls
            );
        }

        #[test]
        fn test_lambda_in_field_initializer() {
            let source = r#"
import java.util.function.Consumer;
public class App {
    Consumer<String> fn = s -> transform(s);

    static Object transform(Object o) { return o; }
}
"#;
            let calls = extract_calls(source);
            // Lambda in field init -> caller is App.<init>
            let init_calls = calls
                .get("App.<init>")
                .expect("App.<init> should have calls");
            assert!(
                init_calls.iter().any(|c| c.target == "transform"),
                "Lambda in field init should extract transform(); got: {:?}",
                init_calls
            );
        }

        #[test]
        fn test_anonymous_class_in_field_initializer() {
            let source = r#"
public class TaskManager {
    Runnable task = new Runnable() {
        public void run() { compute(); }
    };

    static Object compute() { return null; }
}
"#;
            let calls = extract_calls(source);
            // The anonymous class field init should have the Runnable constructor
            // attributed to TaskManager.<init>, and compute() inside run()
            // should be attributed to run or TaskManager.<init>
            let init_calls = calls
                .get("TaskManager.<init>")
                .expect("TaskManager.<init> should have calls");
            assert!(
                init_calls.iter().any(|c| c.target == "Runnable"),
                "Anonymous class field init should extract new Runnable(); got: {:?}",
                init_calls
            );
        }

        #[test]
        fn test_multiple_field_initializers() {
            let source = r#"
public class Multi {
    private A a = createA();
    private static B b = createB();
    private C c = new C();

    static A createA() { return null; }
    static B createB() { return null; }
}
"#;
            let calls = extract_calls(source);

            // Instance fields -> Multi.<init>
            let init_calls = calls
                .get("Multi.<init>")
                .expect("Multi.<init> should have calls");
            assert!(
                init_calls.iter().any(|c| c.target == "createA"),
                "Instance field init should extract createA(); got: {:?}",
                init_calls
            );
            assert!(
                init_calls.iter().any(|c| c.target == "C"),
                "Instance field init should extract new C(); got: {:?}",
                init_calls
            );

            // Static field -> Multi.<clinit>
            let clinit_calls = calls
                .get("Multi.<clinit>")
                .expect("Multi.<clinit> should have calls");
            assert!(
                clinit_calls.iter().any(|c| c.target == "createB"),
                "Static field init should extract createB(); got: {:?}",
                clinit_calls
            );
        }

        #[test]
        fn test_combined_static_and_instance_init() {
            let source = r#"
public class Combined {
    private Handler handler = createHandler();

    static {
        bootstrap();
    }

    {
        wireUp();
    }

    static void bootstrap() {}
    void wireUp() {}
    static Handler createHandler() { return null; }
}
"#;
            let calls = extract_calls(source);

            // Instance: field init + instance init block -> Combined.<init>
            let init_calls = calls
                .get("Combined.<init>")
                .expect("Combined.<init> should have calls");
            assert!(
                init_calls.iter().any(|c| c.target == "createHandler"),
                "Field init should extract createHandler(); got: {:?}",
                init_calls
            );
            assert!(
                init_calls.iter().any(|c| c.target == "wireUp"),
                "Instance init block should extract wireUp(); got: {:?}",
                init_calls
            );

            // Static init block -> Combined.<clinit>
            let clinit_calls = calls
                .get("Combined.<clinit>")
                .expect("Combined.<clinit> should have calls");
            assert!(
                clinit_calls.iter().any(|c| c.target == "bootstrap"),
                "Static init block should extract bootstrap(); got: {:?}",
                clinit_calls
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
            let handler = JavaHandler::new();
            assert_eq!(handler.name(), "java");
        }

        #[test]
        fn test_handler_extensions() {
            let handler = JavaHandler::new();
            let exts = handler.extensions();
            assert!(exts.contains(&".java"));
            assert_eq!(exts.len(), 1);
        }

        #[test]
        fn test_handler_supports() {
            let handler = JavaHandler::new();
            assert!(handler.supports("java"));
            assert!(handler.supports("Java"));
            assert!(handler.supports("JAVA"));
            assert!(!handler.supports("kotlin"));
        }

        #[test]
        fn test_handler_supports_extension() {
            let handler = JavaHandler::new();
            assert!(handler.supports_extension(".java"));
            assert!(handler.supports_extension(".JAVA"));
            assert!(!handler.supports_extension(".kt"));
        }
    }
}
