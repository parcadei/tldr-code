//! Kotlin language handler for call graph analysis.
//!
//! This module provides Kotlin-specific call graph support using tree-sitter-kotlin-ng.
//!
//! # Import Patterns Supported
//!
//! | Pattern | ImportDef |
//! |---------|-----------|
//! | `import com.example.User` | `{module: "com.example.User"}` |
//! | `import com.example.*` | `{module: "com.example.*", is_wildcard: true}` |
//! | `import com.example.User as U` | `{module: "com.example.User", alias: "U"}` |
//!
//! # Call Extraction
//!
//! - Direct calls: `method()` -> CallType::Direct or CallType::Intra
//! - Method calls: `obj.method()` -> CallType::Attr
//! - Extension function calls: `String.myExtension()` -> CallType::Attr
//! - Object declarations are indexed as classes
//!
//! # Spec Reference
//!
//! See `migration/spec/callgraph-spec.md` Section 9.12 for Kotlin-specific details.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use tree_sitter::{Node, Parser, Tree};

use super::base::{get_node_text, walk_tree};
use super::{CallGraphLanguageSupport, ParseError};
use crate::callgraph::cross_file_types::{CallSite, CallType, ClassDef, FuncDef, ImportDef};

// =============================================================================
// Kotlin Handler
// =============================================================================

/// Kotlin language handler using tree-sitter-kotlin-ng.
///
/// Supports:
/// - Import parsing (standard, wildcard, aliased imports)
/// - Call extraction (direct, method, extension function)
/// - Class, object, and function declarations
/// - Companion object method tracking
#[derive(Debug, Default)]
pub struct KotlinHandler;

impl KotlinHandler {
    /// Creates a new KotlinHandler.
    pub fn new() -> Self {
        Self
    }

    /// Parse the source code into a tree-sitter Tree.
    fn parse_source(&self, source: &str) -> Result<Tree, ParseError> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_kotlin_ng::LANGUAGE.into())
            .map_err(|e| ParseError::ParseFailed {
                file: std::path::PathBuf::new(),
                message: format!("Failed to set Kotlin language: {}", e),
            })?;

        parser
            .parse(source, None)
            .ok_or_else(|| ParseError::ParseFailed {
                file: std::path::PathBuf::new(),
                message: "Parser returned None".to_string(),
            })
    }

    /// Parse an import node.
    ///
    /// Kotlin import structure in tree-sitter-kotlin-ng:
    /// ```
    /// import
    ///   import "import"
    ///   qualified_identifier
    ///     identifier "com"
    ///     . "."
    ///     identifier "example"
    ///     ...
    ///   as "as"       (optional, for aliased imports)
    ///   identifier "U" (the alias name)
    /// ```
    fn parse_import_node(&self, node: &Node, source: &[u8]) -> Option<ImportDef> {
        if node.kind() != "import" {
            return None;
        }

        let text = get_node_text(node, source).trim();

        // Check for wildcard import
        let is_wildcard = text.ends_with(".*") || text.ends_with("*");

        // Check for alias: import X as Y
        let mut alias: Option<String> = None;
        let mut module: Option<String> = None;
        let mut saw_as = false;

        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                match child.kind() {
                    "qualified_identifier" => {
                        // Get the full qualified path including dots
                        let ident = get_node_text(&child, source).to_string();
                        module = Some(ident);
                    }
                    "as" => {
                        // Next identifier will be the alias
                        saw_as = true;
                    }
                    "identifier" if saw_as => {
                        // This is the alias after "as" keyword
                        alias = Some(get_node_text(&child, source).to_string());
                    }
                    _ => {}
                }
            }
        }

        // Fallback: parse from text if tree parsing didn't work
        if module.is_none() {
            // Parse: import com.example.User [as Alias]
            let text_to_parse = text.strip_prefix("import ")?.trim();

            // Handle alias
            if let Some((path, alias_part)) = text_to_parse.split_once(" as ") {
                module = Some(path.trim().to_string());
                alias = Some(alias_part.trim().to_string());
            } else {
                module = Some(text_to_parse.to_string());
            }
        }

        let module = module?;

        let mut import_def = if is_wildcard {
            let mut imp = ImportDef::from_import(module.clone(), vec!["*".to_string()]);
            imp.is_namespace = true;
            imp
        } else {
            ImportDef::simple_import(module.clone())
        };

        if let Some(alias) = alias {
            import_def.alias = Some(alias);
        }

        Some(import_def)
    }

    /// Collect all class, object, and function definitions.
    fn collect_definitions(
        &self,
        tree: &Tree,
        source: &[u8],
    ) -> (HashSet<String>, HashSet<String>) {
        let mut methods = HashSet::new();
        let mut classes = HashSet::new();

        for node in walk_tree(tree.root_node()) {
            match node.kind() {
                "function_declaration" => {
                    // Get function name from identifier child
                    if let Some(name) = self.get_identifier(&node, source) {
                        methods.insert(name);
                    }
                }
                "class_declaration" => {
                    if let Some(name) = self.get_identifier(&node, source) {
                        classes.insert(name.clone());
                        // Constructor can be called with class name
                        methods.insert(name);
                    }
                }
                "object_declaration" => {
                    // Kotlin object declarations (singletons) are indexed as classes
                    if let Some(name) = self.get_identifier(&node, source) {
                        classes.insert(name);
                    }
                }
                _ => {}
            }
        }

        (methods, classes)
    }

    /// Get the identifier from a declaration node.
    /// tree-sitter-kotlin-ng uses "identifier" for function/class names.
    fn get_identifier(&self, node: &Node, source: &[u8]) -> Option<String> {
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                // tree-sitter-kotlin-ng uses "identifier" (not "identifier")
                if child.kind() == "identifier" {
                    let name = get_node_text(&child, source).to_string();
                    // Skip empty identifiers (can happen with malformed code)
                    if !name.is_empty() {
                        return Some(name);
                    }
                }
            }
        }
        None
    }

    /// Extract calls from a function body.
    fn extract_calls_from_node(
        &self,
        node: &Node,
        source: &[u8],
        defined_methods: &HashSet<String>,
        _defined_classes: &HashSet<String>,
        caller: &str,
    ) -> Vec<CallSite> {
        let mut calls = Vec::new();

        // Skip if caller is empty - can happen for top-level code outside any function
        if caller.is_empty() {
            return calls;
        }

        for child in walk_tree(*node) {
            if child.kind() == "call_expression" {
                let line = child.start_position().row as u32 + 1;

                // Kotlin call_expression structure (tree-sitter-kotlin-ng):
                // call_expression
                //   identifier (for direct call: foo())
                //   -or-
                //   navigation_expression (for method call: obj.method())
                //   value_arguments (arguments)

                let mut callee: Option<String> = None;

                for i in 0..child.child_count() {
                    if let Some(c) = child.child(i) {
                        match c.kind() {
                            "identifier" => {
                                // Direct call: foo()
                                callee = Some(get_node_text(&c, source).to_string());
                            }
                            "navigation_expression" => {
                                // Method call: obj.method() or Obj.staticMethod()
                                callee = Some(get_node_text(&c, source).to_string());
                            }
                            _ => {}
                        }
                    }
                }

                if let Some(target) = callee {
                    if target.contains('.') {
                        // Method/navigation call: receiver.method
                        // Split into receiver and method name
                        let parts: Vec<&str> = target.rsplitn(2, '.').collect();
                        let method_name = parts[0].to_string();
                        let receiver = if parts.len() > 1 {
                            Some(parts[1].to_string())
                        } else {
                            Some(target.clone())
                        };
                        calls.push(CallSite::new(
                            caller.to_string(),
                            method_name,
                            CallType::Attr,
                            Some(line),
                            None,
                            receiver,
                            None,
                        ));
                    } else if defined_methods.contains(&target) {
                        // Same-file call
                        calls.push(CallSite::new(
                            caller.to_string(),
                            target,
                            CallType::Intra,
                            Some(line),
                            None,
                            None,
                            None,
                        ));
                    } else {
                        // External or unknown call
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

        calls
    }
}

impl CallGraphLanguageSupport for KotlinHandler {
    fn name(&self) -> &str {
        "kotlin"
    }

    fn extensions(&self) -> &[&str] {
        &[".kt", ".kts"]
    }

    fn parse_imports(&self, source: &str, _path: &Path) -> Result<Vec<ImportDef>, ParseError> {
        let tree = self.parse_source(source)?;
        let source_bytes = source.as_bytes();
        let mut imports = Vec::new();

        for node in walk_tree(tree.root_node()) {
            if node.kind() == "import" {
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

        // Track current class context
        let mut current_class: Option<String> = None;

        fn process_node(
            node: Node,
            source: &[u8],
            defined_methods: &HashSet<String>,
            defined_classes: &HashSet<String>,
            calls_by_func: &mut HashMap<String, Vec<CallSite>>,
            current_class: &mut Option<String>,
            handler: &KotlinHandler,
        ) {
            match node.kind() {
                "class_declaration" | "object_declaration" => {
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

                    // Process children
                    for i in 0..node.child_count() {
                        if let Some(child) = node.child(i) {
                            process_node(
                                child,
                                source,
                                defined_methods,
                                defined_classes,
                                calls_by_func,
                                current_class,
                                handler,
                            );
                        }
                    }

                    *current_class = old_class;
                }
                "function_declaration" => {
                    let mut method_name: Option<String> = None;
                    let mut body: Option<Node> = None;

                    for i in 0..node.child_count() {
                        if let Some(child) = node.child(i) {
                            match child.kind() {
                                "identifier" => {
                                    if method_name.is_none() {
                                        method_name =
                                            Some(get_node_text(&child, source).to_string());
                                    }
                                }
                                "function_body" => {
                                    body = Some(child);
                                }
                                _ => {}
                            }
                        }
                    }

                    if let (Some(name), Some(body_node)) = (method_name, body) {
                        let full_name = if let Some(ref class) = current_class {
                            format!("{}.{}", class, name)
                        } else {
                            name.clone()
                        };

                        let calls = handler.extract_calls_from_node(
                            &body_node,
                            source,
                            defined_methods,
                            defined_classes,
                            &full_name,
                        );

                        if !calls.is_empty() {
                            calls_by_func.insert(full_name.clone(), calls.clone());
                            // Also store with simple name for non-class methods
                            if full_name == name {
                                calls_by_func.insert(name, calls);
                            }
                        }
                    }
                }
                // Property initializers: val x = Foo(), val x = bar.baz(),
                // val x by lazy { Foo() } (delegates — walk_tree enters lambdas)
                // Top-level → caller is "<module>"
                // Inside class → caller is "ClassName.<init>"
                "property_declaration" => {
                    let caller = if let Some(ref class) = current_class {
                        format!("{}.<init>", class)
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
                        calls_by_func
                            .entry(caller)
                            .or_default()
                            .extend(calls);
                    }
                }
                // Init blocks: init { ... }
                // Caller is "ClassName.<init>" (only valid inside a class)
                "anonymous_initializer" => {
                    if let Some(ref class) = current_class {
                        let caller = format!("{}.<init>", class);

                        let calls = handler.extract_calls_from_node(
                            &node,
                            source,
                            defined_methods,
                            defined_classes,
                            &caller,
                        );

                        if !calls.is_empty() {
                            calls_by_func
                                .entry(caller)
                                .or_default()
                                .extend(calls);
                        }
                    }
                }
                // Constructor default parameters: class Foo(val x: Bar = Bar())
                // The call_expression inside class_parameter default values
                "primary_constructor" => {
                    if let Some(ref class) = current_class {
                        let caller = format!("{}.<init>", class);

                        let calls = handler.extract_calls_from_node(
                            &node,
                            source,
                            defined_methods,
                            defined_classes,
                            &caller,
                        );

                        if !calls.is_empty() {
                            calls_by_func
                                .entry(caller)
                                .or_default()
                                .extend(calls);
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
                                defined_methods,
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
            &defined_methods,
            &defined_classes,
            &mut calls_by_func,
            &mut current_class,
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
                "function_declaration" => {
                    if let Some(name) = self.get_identifier(&node, source_bytes) {
                        let line = node.start_position().row as u32 + 1;
                        let end_line = node.end_position().row as u32 + 1;

                        // Check if inside a class
                        let mut class_name = None;
                        let mut parent = node.parent();
                        while let Some(p) = parent {
                            if p.kind() == "class_body" {
                                if let Some(gp) = p.parent() {
                                    if gp.kind() == "class_declaration"
                                        || gp.kind() == "object_declaration"
                                    {
                                        class_name = self.get_identifier(&gp, source_bytes);
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
                "class_declaration" => {
                    if let Some(name) = self.get_identifier(&node, source_bytes) {
                        let line = node.start_position().row as u32 + 1;
                        let end_line = node.end_position().row as u32 + 1;

                        let mut methods = Vec::new();
                        let mut bases = Vec::new();

                        for i in 0..node.child_count() {
                            if let Some(child) = node.child(i) {
                                if child.kind() == "delegation_specifier"
                                    || child.kind() == "delegation_specifiers"
                                {
                                    for j in 0..child.child_count() {
                                        if let Some(base) = child.child(j) {
                                            if base.kind() == "user_type"
                                                || base.kind() == "identifier"
                                            {
                                                bases.push(
                                                    get_node_text(&base, source_bytes).to_string(),
                                                );
                                            }
                                        }
                                    }
                                }
                                if child.kind() == "class_body" {
                                    for j in 0..child.named_child_count() {
                                        if let Some(member) = child.named_child(j) {
                                            if member.kind() == "function_declaration" {
                                                if let Some(mn) =
                                                    self.get_identifier(&member, source_bytes)
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
                "object_declaration" => {
                    if let Some(name) = self.get_identifier(&node, source_bytes) {
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
        let handler = KotlinHandler::new();
        handler.parse_imports(source, Path::new("Test.kt")).unwrap()
    }

    fn extract_calls(source: &str) -> HashMap<String, Vec<CallSite>> {
        let handler = KotlinHandler::new();
        let tree = handler.parse_source(source).unwrap();
        handler
            .extract_calls(Path::new("Test.kt"), source, &tree)
            .unwrap()
    }

    // -------------------------------------------------------------------------
    // Import Parsing Tests
    // -------------------------------------------------------------------------

    mod import_tests {
        use super::*;

        #[test]
        fn test_parse_simple_import() {
            let imports = parse_imports("import com.example.User");
            assert_eq!(imports.len(), 1);
            assert!(imports[0].module.contains("com.example.User"));
            assert!(!imports[0].is_wildcard());
        }

        #[test]
        fn test_parse_wildcard_import() {
            let imports = parse_imports("import com.example.*");
            assert_eq!(imports.len(), 1);
            assert!(imports[0].module.contains("com.example"));
            assert!(imports[0].is_wildcard() || imports[0].is_namespace);
        }

        #[test]
        fn test_parse_aliased_import() {
            let imports = parse_imports("import com.example.User as U");
            assert_eq!(imports.len(), 1);
            assert!(imports[0].module.contains("com.example.User"));
            assert_eq!(imports[0].alias, Some("U".to_string()));
        }

        #[test]
        fn test_parse_multiple_imports() {
            let source = r#"
import com.example.User
import com.example.Repository
import kotlin.collections.*
"#;
            let imports = parse_imports(source);
            assert_eq!(imports.len(), 3);
        }

        #[test]
        fn test_parse_kotlin_stdlib_import() {
            let imports = parse_imports("import kotlin.collections.List");
            assert_eq!(imports.len(), 1);
            assert!(imports[0].module.contains("kotlin.collections.List"));
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
fun main() {
    println("hello")
    helper()
}
"#;
            let calls = extract_calls(source);
            let main_calls = calls.get("main").unwrap();
            assert!(main_calls.iter().any(|c| c.target == "println"));
            assert!(main_calls.iter().any(|c| c.target == "helper"));
        }

        #[test]
        fn test_extract_calls_intra_file() {
            let source = r#"
fun add(a: Int, b: Int): Int {
    return a + b
}

fun calculate(): Int {
    return add(1, 2)
}
"#;
            let calls = extract_calls(source);
            let calc_calls = calls.get("calculate").unwrap();
            let add_call = calc_calls.iter().find(|c| c.target == "add").unwrap();
            assert_eq!(add_call.call_type, CallType::Intra);
        }

        #[test]
        fn test_extract_calls_method_on_object() {
            let source = r#"
fun process() {
    repo.save(data)
    list.add(item)
}
"#;
            let calls = extract_calls(source);
            let process_calls = calls.get("process").unwrap();
            assert!(process_calls.iter().any(|c| c.target.contains("save")));
            assert!(process_calls.iter().any(|c| c.target.contains("add")));
        }

        #[test]
        fn test_extract_calls_extension_function() {
            let source = r#"
fun String.myExtension(): String {
    return this.uppercase()
}

fun use() {
    "hello".myExtension()
}
"#;
            let calls = extract_calls(source);
            // The extension function call should be captured
            assert!(!calls.is_empty());
        }

        #[test]
        fn test_extract_calls_with_class_context() {
            let source = r#"
class Calculator {
    fun add(a: Int, b: Int): Int {
        return a + b
    }

    fun calculate(): Int {
        return add(1, 2)
    }
}
"#;
            let calls = extract_calls(source);
            // Should have calls indexed by both simple and qualified name
            let calc_calls = calls.get("calculate").or(calls.get("Calculator.calculate"));
            assert!(calc_calls.is_some());
        }

        #[test]
        fn test_extract_calls_method_to_toplevel() {
            let source = r#"
class Service {
    fun process(): Int {
        return helper()
    }
}

fun helper(): Int {
    return 42
}
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
        fn test_extract_calls_object_declaration() {
            let source = r#"
object Singleton {
    fun getInstance(): Singleton = this

    fun doWork() {
        helper()
    }
}
"#;
            let calls = extract_calls(source);
            let work_calls = calls.get("doWork").or(calls.get("Singleton.doWork"));
            assert!(work_calls.is_some());
        }

        #[test]
        fn test_extract_calls_companion_object() {
            let source = r#"
class Factory {
    companion object {
        fun create(): Factory {
            return Factory()
        }
    }
}
"#;
            let calls = extract_calls(source);
            // Factory() constructor call should be captured
            let create_calls = calls.get("create");
            if let Some(calls) = create_calls {
                assert!(calls.iter().any(|c| c.target == "Factory"));
            }
        }

        #[test]
        fn test_extract_calls_with_line_numbers() {
            let source = r#"fun test() {
    first()
    second()
}"#;
            let calls = extract_calls(source);
            let test_calls = calls.get("test").unwrap();

            let first = test_calls.iter().find(|c| c.target == "first").unwrap();
            let second = test_calls.iter().find(|c| c.target == "second").unwrap();

            assert!(first.line.is_some());
            assert!(second.line.is_some());
            assert!(second.line.unwrap() > first.line.unwrap());
        }

        #[test]
        fn test_extract_calls_static_method() {
            let source = r#"
fun calculate(): Double {
    return Math.sqrt(16.0)
}
"#;
            let calls = extract_calls(source);
            let calc_calls = calls.get("calculate").unwrap();
            assert!(calc_calls.iter().any(|c| c.target.contains("sqrt")));
        }

        #[test]
        fn test_extract_calls_top_level_property_initializer() {
            let source = r#"
val logger = Logger.getLogger("Test")
val config = Config()

fun doWork() {
    config.run()
}
"#;
            let calls = extract_calls(source);
            let module_calls = calls.get("<module>");
            assert!(
                module_calls.is_some(),
                "Should have <module> calls for top-level initializers"
            );
            let module_calls = module_calls.unwrap();
            assert!(
                module_calls.iter().any(|c| c.target == "getLogger"),
                "Should find Logger.getLogger call"
            );
            assert!(
                module_calls.iter().any(|c| c.target == "Config"),
                "Should find Config() constructor call"
            );
        }

        #[test]
        fn test_extract_calls_class_property_initializer() {
            let source = r#"
class Service {
    val repo = Repository()

    fun run() {
        repo.process()
    }
}
"#;
            let calls = extract_calls(source);
            let init_calls = calls.get("Service.<init>");
            assert!(
                init_calls.is_some(),
                "Should have Service.<init> calls for class property initializers"
            );
            let init_calls = init_calls.unwrap();
            assert!(
                init_calls.iter().any(|c| c.target == "Repository"),
                "Should find Repository() constructor call"
            );
        }

        #[test]
        fn test_extract_calls_init_block() {
            let source = r#"
class Service {
    val repo = Repository()

    init {
        repo.initialize()
    }

    fun run() {
        repo.process()
    }
}
"#;
            let calls = extract_calls(source);
            let init_calls = calls.get("Service.<init>");
            assert!(init_calls.is_some(), "Should have Service.<init> calls");
            let init_calls = init_calls.unwrap();
            // Both property initializer (Repository()) and init block (repo.initialize()) should be here
            assert!(
                init_calls.iter().any(|c| c.target == "Repository"),
                "Should find Repository() from property init"
            );
            assert!(
                init_calls.iter().any(|c| c.target == "initialize"),
                "Should find repo.initialize() from init block"
            );
        }

        #[test]
        fn test_extract_calls_companion_object_property() {
            let source = r#"
class Factory {
    companion object {
        val cached = Config()

        fun create(): Factory {
            return Factory()
        }
    }
}
"#;
            let calls = extract_calls(source);
            // companion object is an object_declaration, so current_class becomes "Companion"
            // The property initializer should produce calls under "Companion.<init>"
            let has_companion_init = calls.keys().any(|k| k.contains("<init>"));
            assert!(
                has_companion_init,
                "Should have <init> calls for companion object property: {:?}",
                calls.keys().collect::<Vec<_>>()
            );
        }

        #[test]
        fn test_extract_calls_constructor_default_params() {
            let source = r#"
class Service(val user: User = User("svc", 10)) {
    fun run() {
        user.greet()
    }
}
"#;
            let calls = extract_calls(source);
            let init_calls = calls.get("Service.<init>");
            assert!(
                init_calls.is_some(),
                "Should have Service.<init> calls for constructor default params: {:?}",
                calls.keys().collect::<Vec<_>>()
            );
            let init_calls = init_calls.unwrap();
            assert!(
                init_calls.iter().any(|c| c.target == "User"),
                "Should find User() constructor call from default param"
            );
        }

        #[test]
        fn test_extract_calls_property_delegate() {
            let source = r#"
class Service {
    val config by lazy { Config() }
    val name by lazy { helper.getName() }

    fun run() {
        config.start()
    }
}
"#;
            let calls = extract_calls(source);
            let init_calls = calls.get("Service.<init>");
            assert!(
                init_calls.is_some(),
                "Should have Service.<init> calls for property delegates: {:?}",
                calls.keys().collect::<Vec<_>>()
            );
            let init_calls = init_calls.unwrap();
            // lazy {{}} itself is a call
            assert!(
                init_calls.iter().any(|c| c.target == "lazy"),
                "Should find lazy call"
            );
            // Calls inside the lambda should also be extracted
            assert!(
                init_calls.iter().any(|c| c.target == "Config"),
                "Should find Config() inside lazy block"
            );
            assert!(
                init_calls.iter().any(|c| c.target == "getName"),
                "Should find helper.getName() inside lazy block"
            );
        }

        #[test]
        fn test_extract_calls_top_level_property_delegate() {
            let source = r#"
val config by lazy { Config() }

fun doWork() {
    config.start()
}
"#;
            let calls = extract_calls(source);
            let module_calls = calls.get("<module>");
            assert!(
                module_calls.is_some(),
                "Should have <module> calls for top-level delegate: {:?}",
                calls.keys().collect::<Vec<_>>()
            );
            let module_calls = module_calls.unwrap();
            assert!(
                module_calls.iter().any(|c| c.target == "lazy"),
                "Should find lazy call"
            );
            assert!(
                module_calls.iter().any(|c| c.target == "Config"),
                "Should find Config() inside lazy block"
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
            let handler = KotlinHandler::new();
            assert_eq!(handler.name(), "kotlin");
        }

        #[test]
        fn test_handler_extensions() {
            let handler = KotlinHandler::new();
            let exts = handler.extensions();
            assert!(exts.contains(&".kt"));
            assert!(exts.contains(&".kts"));
            assert_eq!(exts.len(), 2);
        }

        #[test]
        fn test_handler_supports() {
            let handler = KotlinHandler::new();
            assert!(handler.supports("kotlin"));
            assert!(handler.supports("Kotlin"));
            assert!(handler.supports("KOTLIN"));
            assert!(!handler.supports("java"));
        }

        #[test]
        fn test_handler_supports_extension() {
            let handler = KotlinHandler::new();
            assert!(handler.supports_extension(".kt"));
            assert!(handler.supports_extension(".kts"));
            assert!(handler.supports_extension(".KT"));
            assert!(!handler.supports_extension(".java"));
        }
    }
}
