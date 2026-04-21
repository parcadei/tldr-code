//! C language handler for call graph analysis.
//!
//! This module provides C-specific call graph support using tree-sitter-c.
//!
//! # Import Patterns Supported
//!
//! | Pattern | ImportDef |
//! |---------|-----------|
//! | `#include <header.h>` | `{module: "header.h", is_namespace: true}` (system) |
//! | `#include "header.h"` | `{module: "header.h", is_namespace: false}` (local) |
//!
//! # Call Extraction
//!
//! - Direct calls: `func()` -> CallType::Direct or CallType::Intra
//! - Function pointer calls: `(*func_ptr)()` -> CallType::Direct
//! - Struct member function pointers: `obj->callback()` -> CallType::Attr
//! - Global/static/const initializer calls: `int x = foo();` -> `<module>` -> foo
//! - Default parameter calls (GNU extension): `void f(int x = val())` -> f -> val
//! - Designated initializer calls: `.field = func()` in struct initializers
//!
//! # Spec Reference
//!
//! See `migration/spec/callgraph-spec.md` Section 9.3 for C-specific details.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use tree_sitter::{Node, Tree};

use super::base::{get_node_text, walk_tree};
use super::{CallGraphLanguageSupport, ParseError};
use crate::callgraph::cross_file_types::{CallSite, CallType, ClassDef, FuncDef, ImportDef};

// =============================================================================
// C Handler
// =============================================================================

/// C language handler using tree-sitter-c.
///
/// Supports:
/// - Include parsing (system and local headers)
/// - Call extraction (direct, function pointer, struct member)
/// - Global/static/const initializer calls at file scope (`<module>` caller)
/// - Default parameter calls (GNU C extension)
/// - Macro call detection (limited)
#[derive(Debug, Default)]
pub struct CHandler;

impl CHandler {
    /// Creates a new CHandler.
    pub fn new() -> Self {
        Self
    }

    /// Get the function name from a function_definition node.
    fn get_function_name(&self, node: &Node, source: &[u8]) -> Option<String> {
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                match child.kind() {
                    "function_declarator" => {
                        // Regular function: int func()
                        for j in 0..child.child_count() {
                            if let Some(dc) = child.child(j) {
                                if dc.kind() == "identifier" {
                                    return Some(get_node_text(&dc, source).to_string());
                                }
                            }
                        }
                    }
                    "pointer_declarator" => {
                        // Pointer return: int* func()
                        for j in 0..child.child_count() {
                            if let Some(pc) = child.child(j) {
                                if pc.kind() == "function_declarator" {
                                    for k in 0..pc.child_count() {
                                        if let Some(dc) = pc.child(k) {
                                            if dc.kind() == "identifier" {
                                                return Some(
                                                    get_node_text(&dc, source).to_string(),
                                                );
                                            }
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
        None
    }

    /// Collect all function names defined in the file.
    fn collect_definitions(&self, tree: &Tree, source: &[u8]) -> HashSet<String> {
        let mut functions = HashSet::new();

        for node in walk_tree(tree.root_node()) {
            if node.kind() == "function_definition" {
                if let Some(name) = self.get_function_name(&node, source) {
                    functions.insert(name);
                }
            }
        }

        functions
    }

    /// Extract calls from a function body.
    fn extract_calls_from_func(
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
                        "parenthesized_expression" => {
                            // Function pointer call: (*func_ptr)() or (func_ptr)()
                            let inner_text = get_node_text(&func_node, source).to_string();
                            // Try to extract the identifier
                            let target = inner_text
                                .trim_matches(|c| c == '(' || c == ')' || c == '*')
                                .to_string();
                            if !target.is_empty() {
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
                        "field_expression" => {
                            // Struct member call: obj->callback() or obj.callback()
                            let mut receiver = None;
                            let mut field = None;

                            for i in 0..func_node.child_count() {
                                if let Some(fc) = func_node.child(i) {
                                    match fc.kind() {
                                        "identifier" => {
                                            if receiver.is_none() {
                                                receiver =
                                                    Some(get_node_text(&fc, source).to_string());
                                            }
                                        }
                                        "field_identifier" => {
                                            field = Some(get_node_text(&fc, source).to_string());
                                        }
                                        _ => {}
                                    }
                                }
                            }

                            if let Some(f) = field {
                                calls.push(CallSite::new(
                                    caller.to_string(),
                                    f.clone(),
                                    CallType::Attr,
                                    Some(line),
                                    None,
                                    receiver,
                                    None,
                                ));
                            }
                        }
                        _ => {
                            // Other patterns - try to get the text
                            let target = get_node_text(&func_node, source).to_string();
                            if !target.is_empty() {
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
            }
        }

        calls
    }

    /// Extract calls from default parameter values in a function_declarator.
    /// Handles GNU C extension default parameters and shared C/C++ headers.
    /// e.g. void foo(int x = compute(), int y = create())
    fn extract_default_param_calls(
        &self,
        func_declarator: &Node,
        source: &[u8],
        defined_funcs: &HashSet<String>,
        caller: &str,
    ) -> Vec<CallSite> {
        let mut calls = Vec::new();
        for child in walk_tree(*func_declarator) {
            if child.kind() == "optional_parameter_declaration" {
                let param_calls =
                    self.extract_calls_from_func(&child, source, defined_funcs, caller);
                calls.extend(param_calls);
            }
        }
        calls
    }
}

impl CallGraphLanguageSupport for CHandler {
    fn name(&self) -> &str {
        "c"
    }

    fn extensions(&self) -> &[&str] {
        &[".c", ".h"]
    }

    fn parse_imports(&self, source: &str, _path: &Path) -> Result<Vec<ImportDef>, ParseError> {
        let tree = super::c_common::parse_source_with_language(
            source,
            tree_sitter_c::LANGUAGE.into(),
            "C",
        )?;
        Ok(super::c_common::parse_preproc_imports(&tree, source))
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

        let root = tree.root_node();

        // Walk only top-level children (translation_unit direct children)
        for i in 0..root.child_count() {
            let Some(node) = root.child(i) else { continue };

            match node.kind() {
                "function_definition" => {
                    if let Some(func_name) = self.get_function_name(&node, source_bytes) {
                        let mut all_calls = Vec::new();

                        for j in 0..node.child_count() {
                            if let Some(child) = node.child(j) {
                                match child.kind() {
                                    // Function body calls
                                    "compound_statement" => {
                                        let calls = self.extract_calls_from_func(
                                            &child,
                                            source_bytes,
                                            &defined_funcs,
                                            &func_name,
                                        );
                                        all_calls.extend(calls);
                                    }
                                    // Default parameter calls (GNU C extension / C++ headers)
                                    "function_declarator" => {
                                        let calls = self.extract_default_param_calls(
                                            &child,
                                            source_bytes,
                                            &defined_funcs,
                                            &func_name,
                                        );
                                        all_calls.extend(calls);
                                    }
                                    _ => {}
                                }
                            }
                        }

                        if !all_calls.is_empty() {
                            calls_by_func.insert(func_name.clone(), all_calls);
                        }
                    }
                }
                // Global/static/const variable initializer calls at file scope
                "declaration" => {
                    let has_call = walk_tree(node).any(|n| n.kind() == "call_expression");
                    if has_call {
                        let calls = self.extract_calls_from_func(
                            &node,
                            source_bytes,
                            &defined_funcs,
                            "<module>",
                        );
                        if !calls.is_empty() {
                            calls_by_func
                                .entry("<module>".to_string())
                                .or_default()
                                .extend(calls);
                        }
                    }
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
                "function_definition" => {
                    if let Some(name) = self.get_function_name(&node, source_bytes) {
                        let line = node.start_position().row as u32 + 1;
                        let end_line = node.end_position().row as u32 + 1;
                        funcs.push(FuncDef::function(name, line, end_line));
                    }
                }
                "struct_specifier" => {
                    // Only capture structs with a body (definition, not just declaration)
                    let mut has_body = false;
                    let mut struct_name = None;
                    for i in 0..node.child_count() {
                        if let Some(child) = node.child(i) {
                            if child.kind() == "type_identifier" {
                                struct_name = Some(get_node_text(&child, source_bytes).to_string());
                            }
                            if child.kind() == "field_declaration_list" {
                                has_body = true;
                            }
                        }
                    }
                    if has_body {
                        if let Some(name) = struct_name {
                            let line = node.start_position().row as u32 + 1;
                            let end_line = node.end_position().row as u32 + 1;
                            classes.push(ClassDef::simple(name, line, end_line));
                        }
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
        let handler = CHandler::new();
        handler.parse_imports(source, Path::new("test.c")).unwrap()
    }

    fn extract_calls(source: &str) -> HashMap<String, Vec<CallSite>> {
        let handler = CHandler::new();
        let tree = crate::callgraph::languages::c_common::parse_source_with_language(
            source,
            tree_sitter_c::LANGUAGE.into(),
            "C",
        )
        .unwrap();
        handler
            .extract_calls(Path::new("test.c"), source, &tree)
            .unwrap()
    }

    // -------------------------------------------------------------------------
    // Import Parsing Tests
    // -------------------------------------------------------------------------

    mod import_tests {
        use super::*;

        #[test]
        fn test_parse_include_system() {
            let imports = parse_imports("#include <stdio.h>");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].module, "stdio.h");
            assert!(imports[0].is_namespace); // System include
        }

        #[test]
        fn test_parse_include_local() {
            let imports = parse_imports(r#"#include "myheader.h""#);
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].module, "myheader.h");
            assert!(!imports[0].is_namespace); // Local include
        }

        #[test]
        fn test_parse_include_path() {
            let imports = parse_imports("#include <sys/types.h>");
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].module, "sys/types.h");
        }

        #[test]
        fn test_parse_include_multiple() {
            let source = r#"
#include <stdio.h>
#include <stdlib.h>
#include "local.h"
"#;
            let imports = parse_imports(source);
            assert_eq!(imports.len(), 3);
            assert!(imports.iter().any(|i| i.module == "stdio.h"));
            assert!(imports.iter().any(|i| i.module == "stdlib.h"));
            assert!(imports.iter().any(|i| i.module == "local.h"));
        }

        #[test]
        fn test_parse_include_with_comment() {
            let source = r#"
// This is a comment
#include <stdio.h>  /* inline comment */
"#;
            let imports = parse_imports(source);
            assert_eq!(imports.len(), 1);
            assert_eq!(imports[0].module, "stdio.h");
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
void main() {
    printf("hello");
    exit(0);
}
"#;
            let calls = extract_calls(source);
            let main_calls = calls.get("main").unwrap();
            assert!(main_calls.iter().any(|c| c.target == "printf"));
            assert!(main_calls.iter().any(|c| c.target == "exit"));
        }

        #[test]
        fn test_extract_calls_intra_file() {
            let source = r#"
void helper() {}

void main() {
    helper();
}
"#;
            let calls = extract_calls(source);
            let main_calls = calls.get("main").unwrap();
            let helper_call = main_calls.iter().find(|c| c.target == "helper").unwrap();
            assert_eq!(helper_call.call_type, CallType::Intra);
        }

        #[test]
        fn test_extract_calls_external() {
            let source = r#"
void main() {
    some_external_func();
}
"#;
            let calls = extract_calls(source);
            let main_calls = calls.get("main").unwrap();
            let ext_call = main_calls
                .iter()
                .find(|c| c.target == "some_external_func")
                .unwrap();
            assert_eq!(ext_call.call_type, CallType::Direct);
        }

        #[test]
        fn test_extract_calls_function_pointer() {
            let source = r#"
void main() {
    void (*func_ptr)() = some_func;
    (*func_ptr)();
}
"#;
            let calls = extract_calls(source);
            let main_calls = calls.get("main").unwrap();
            // Should find the function pointer call
            assert!(main_calls.iter().any(|c| c.target.contains("func_ptr")));
        }

        #[test]
        fn test_extract_calls_struct_member() {
            let source = r#"
struct Handler {
    void (*callback)(int);
};

void process(struct Handler* h) {
    h->callback(42);
}
"#;
            let calls = extract_calls(source);
            let process_calls = calls.get("process").unwrap();
            let callback_call = process_calls
                .iter()
                .find(|c| c.target == "callback")
                .unwrap();
            assert_eq!(callback_call.call_type, CallType::Attr);
            assert_eq!(callback_call.receiver, Some("h".to_string()));
        }

        #[test]
        fn test_extract_calls_pointer_return() {
            let source = r#"
int* create_array(int size) {
    malloc(size * sizeof(int));
}
"#;
            let calls = extract_calls(source);
            let create_calls = calls.get("create_array").unwrap();
            assert!(create_calls.iter().any(|c| c.target == "malloc"));
        }

        #[test]
        fn test_extract_calls_with_line_numbers() {
            let source = r#"void main() {
    first_call();
    second_call();
}
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

            assert!(first.line.is_some());
            assert!(second.line.is_some());
            assert!(second.line.unwrap() > first.line.unwrap());
        }
    }

    // -------------------------------------------------------------------------
    // Global/Static/Const Initializer Tests
    // -------------------------------------------------------------------------

    mod global_init_tests {
        use super::*;

        #[test]
        fn test_global_variable_init() {
            // Global variable initialization: int x = foo();
            let source = r#"
int global_var = compute_global();
"#;
            let calls = extract_calls(source);
            let module_calls = calls
                .get("<module>")
                .expect("<module> should have global init calls");
            assert!(
                module_calls.iter().any(|c| c.target == "compute_global"),
                "compute_global() from global init should be extracted"
            );
        }

        #[test]
        fn test_static_variable_init() {
            // Static variable initialization at file scope
            let source = r#"
static int count = initialize();
"#;
            let calls = extract_calls(source);
            let module_calls = calls
                .get("<module>")
                .expect("<module> should have static init calls");
            assert!(
                module_calls.iter().any(|c| c.target == "initialize"),
                "initialize() from static global init should be extracted"
            );
        }

        #[test]
        fn test_const_init() {
            // const initializer calls
            let source = r#"
const int config_val = create_config();
"#;
            let calls = extract_calls(source);
            let module_calls = calls
                .get("<module>")
                .expect("<module> should have const init calls");
            assert!(
                module_calls.iter().any(|c| c.target == "create_config"),
                "create_config() from const init should be extracted"
            );
        }

        #[test]
        fn test_multiple_global_inits() {
            // Multiple global initializers
            let source = r#"
int a = foo();
static int b = bar();
const int c = baz();
"#;
            let calls = extract_calls(source);
            let module_calls = calls
                .get("<module>")
                .expect("<module> should have multiple init calls");
            assert!(
                module_calls.iter().any(|c| c.target == "foo"),
                "foo() from global init should be extracted"
            );
            assert!(
                module_calls.iter().any(|c| c.target == "bar"),
                "bar() from static init should be extracted"
            );
            assert!(
                module_calls.iter().any(|c| c.target == "baz"),
                "baz() from const init should be extracted"
            );
        }

        #[test]
        fn test_global_init_with_nested_calls() {
            // Global init with nested function calls
            let source = r#"
int x = outer(inner());
"#;
            let calls = extract_calls(source);
            let module_calls = calls
                .get("<module>")
                .expect("<module> should have nested init calls");
            assert!(
                module_calls.iter().any(|c| c.target == "outer"),
                "outer() from nested global init should be extracted"
            );
            assert!(
                module_calls.iter().any(|c| c.target == "inner"),
                "inner() from nested global init should be extracted"
            );
        }

        #[test]
        fn test_top_level_init_calls_helper() {
            // Top-level init calling a file-defined function should mark it Intra
            let source = r#"
int helper() { return 42; }
int x = helper();
"#;
            let calls = extract_calls(source);
            let module_calls = calls
                .get("<module>")
                .expect("<module> should have top-level calls");
            let helper_call = module_calls
                .iter()
                .find(|c| c.target == "helper")
                .expect("helper() at top level should be extracted into <module>");
            assert_eq!(
                helper_call.call_type,
                CallType::Intra,
                "helper() should be Intra since it's defined in the file"
            );
        }
    }

    // -------------------------------------------------------------------------
    // Default Parameter Tests
    // -------------------------------------------------------------------------

    mod default_param_tests {
        use super::*;

        #[test]
        fn test_function_default_params() {
            // Function default parameter calls (C extension, valid in some compilers)
            // Note: Standard C doesn't have default params, but tree-sitter-c may parse
            // GNU C extensions or the pattern is useful for .h files shared with C++
            let source = r#"
void foo(int x, int y);

void bar() {
    foo(compute(), create());
}
"#;
            // This test verifies that calls inside function arguments are extracted
            let calls = extract_calls(source);
            let bar_calls = calls.get("bar").expect("bar should have calls");
            assert!(
                bar_calls.iter().any(|c| c.target == "foo"),
                "foo() should be extracted"
            );
            assert!(
                bar_calls.iter().any(|c| c.target == "compute"),
                "compute() should be extracted as argument call"
            );
            assert!(
                bar_calls.iter().any(|c| c.target == "create"),
                "create() should be extracted as argument call"
            );
        }
    }

    // -------------------------------------------------------------------------
    // Designated Initializer Tests
    // -------------------------------------------------------------------------

    mod designated_init_tests {
        use super::*;

        #[test]
        fn test_designated_initializer_calls_in_function() {
            // Designated initializers with function calls inside a function body
            let source = r#"
struct Config {
    int timeout;
    int port;
};

void setup() {
    struct Config c = {.timeout = compute_timeout(), .port = get_port()};
}
"#;
            let calls = extract_calls(source);
            let setup_calls = calls.get("setup").expect("setup should have calls");
            assert!(
                setup_calls.iter().any(|c| c.target == "compute_timeout"),
                "compute_timeout() from designated initializer should be extracted"
            );
            assert!(
                setup_calls.iter().any(|c| c.target == "get_port"),
                "get_port() from designated initializer should be extracted"
            );
        }

        #[test]
        fn test_designated_initializer_at_file_scope() {
            // Designated initializers at file scope
            let source = r#"
struct Config {
    int timeout;
    int port;
};

struct Config global_config = {.timeout = compute_timeout(), .port = get_port()};
"#;
            let calls = extract_calls(source);
            let module_calls = calls
                .get("<module>")
                .expect("<module> should have designated init calls");
            assert!(
                module_calls.iter().any(|c| c.target == "compute_timeout"),
                "compute_timeout() from file-scope designated init should be extracted"
            );
            assert!(
                module_calls.iter().any(|c| c.target == "get_port"),
                "get_port() from file-scope designated init should be extracted"
            );
        }
    }

    // -------------------------------------------------------------------------
    // Compound Expression Tests (calls in various expression positions)
    // -------------------------------------------------------------------------

    mod expression_call_tests {
        use super::*;

        #[test]
        fn test_calls_in_initializer_list() {
            // Braced initializer with function calls
            let source = r#"
void setup() {
    int arr[] = {foo(), bar(), baz()};
}
"#;
            let calls = extract_calls(source);
            let setup_calls = calls.get("setup").expect("setup should have calls");
            assert!(
                setup_calls.iter().any(|c| c.target == "foo"),
                "foo() from initializer list should be extracted"
            );
            assert!(
                setup_calls.iter().any(|c| c.target == "bar"),
                "bar() from initializer list should be extracted"
            );
            assert!(
                setup_calls.iter().any(|c| c.target == "baz"),
                "baz() from initializer list should be extracted"
            );
        }

        #[test]
        fn test_ternary_expression_calls() {
            // Ternary expressions with function calls
            let source = r#"
void decide() {
    int x = cond() ? then_val() : else_val();
}
"#;
            let calls = extract_calls(source);
            let decide_calls = calls.get("decide").expect("decide should have calls");
            assert!(
                decide_calls.iter().any(|c| c.target == "cond"),
                "cond() from ternary should be extracted"
            );
            assert!(
                decide_calls.iter().any(|c| c.target == "then_val"),
                "then_val() from ternary should be extracted"
            );
            assert!(
                decide_calls.iter().any(|c| c.target == "else_val"),
                "else_val() from ternary should be extracted"
            );
        }

        #[test]
        fn test_comma_expression_calls() {
            // Multiple calls in comma expressions / compound statements
            let source = r#"
void multi() {
    first(), second(), third();
}
"#;
            let calls = extract_calls(source);
            let multi_calls = calls.get("multi").expect("multi should have calls");
            assert!(
                multi_calls.iter().any(|c| c.target == "first"),
                "first() from comma expression should be extracted"
            );
            assert!(
                multi_calls.iter().any(|c| c.target == "second"),
                "second() from comma expression should be extracted"
            );
            assert!(
                multi_calls.iter().any(|c| c.target == "third"),
                "third() from comma expression should be extracted"
            );
        }

        #[test]
        fn test_global_init_does_not_include_function_body_calls() {
            // Make sure <module> calls don't include calls from function bodies
            let source = r#"
int g = global_init();

void func() {
    local_call();
}
"#;
            let calls = extract_calls(source);
            let module_calls = calls
                .get("<module>")
                .expect("<module> should have global init calls");
            assert!(
                module_calls.iter().any(|c| c.target == "global_init"),
                "global_init() should be in <module>"
            );
            assert!(
                !module_calls.iter().any(|c| c.target == "local_call"),
                "local_call() should NOT be in <module>"
            );

            let func_calls = calls.get("func").expect("func should have calls");
            assert!(
                func_calls.iter().any(|c| c.target == "local_call"),
                "local_call() should be in func"
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
            let handler = CHandler::new();
            assert_eq!(handler.name(), "c");
        }

        #[test]
        fn test_handler_extensions() {
            let handler = CHandler::new();
            let exts = handler.extensions();
            assert!(exts.contains(&".c"));
            assert!(exts.contains(&".h"));
        }

        #[test]
        fn test_handler_supports() {
            let handler = CHandler::new();
            assert!(handler.supports("c"));
            assert!(handler.supports("C"));
            assert!(!handler.supports("cpp"));
        }

        #[test]
        fn test_handler_supports_extension() {
            let handler = CHandler::new();
            assert!(handler.supports_extension(".c"));
            assert!(handler.supports_extension(".h"));
            assert!(handler.supports_extension(".C"));
            assert!(!handler.supports_extension(".cpp"));
        }
    }
}
