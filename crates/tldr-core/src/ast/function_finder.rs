//! Shared function finder utilities for locating functions in tree-sitter ASTs.
//!
//! This module provides the canonical implementations of function-finding logic
//! used across CFG, DFG, metrics, and quality modules. All languages supported
//! by tree-sitter grammars are handled here.

use crate::types::Language;
use tree_sitter::Node;

/// Helper to recursively search for function_definition inside a node (e.g., wrapped in function_call).
/// Searches up to `max_depth` levels deep to handle patterns like `socket.protect(function() end)`.
fn find_function_in_node<'a>(node: Node<'a>, max_depth: usize) -> Option<Node<'a>> {
    if max_depth == 0 {
        return None;
    }

    // Direct function_definition
    if node.kind() == "function_definition" {
        return Some(node);
    }

    // Recurse into children (especially for function_call -> arguments -> function_definition)
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(func) = find_function_in_node(child, max_depth - 1) {
            return Some(func);
        }
    }

    None
}

/// Find a function node by name in the AST
pub fn find_function_node<'a>(
    root: Node<'a>,
    function_name: &str,
    language: Language,
    source: &str,
) -> Option<Node<'a>> {
    let func_kinds = get_function_node_kinds(language);

    let mut stack = vec![root];

    while let Some(node) = stack.pop() {
        // Check direct function nodes
        if func_kinds.contains(&node.kind()) {
            if let Some(name) = get_function_name(node, language, source) {
                if name == function_name
                    || name
                        .strip_prefix('#')
                        .is_some_and(|stripped| stripped == function_name)
                    // Lua/Luau: match short name for dot-indexed functions
                    // e.g. "Kong.init" matches search for "init"
                    || (matches!(language, Language::Lua | Language::Luau)
                        && name.contains('.')
                        && name
                            .rsplit('.')
                            .next()
                            .is_some_and(|short| short == function_name))
                {
                    return Some(node);
                }
            }
        }

        // Check for variable declarations with arrow functions (TypeScript/JavaScript pattern)
        // Pattern: const foo = () => {}  or  const foo = function() {}
        if matches!(language, Language::TypeScript | Language::JavaScript)
            && matches!(node.kind(), "lexical_declaration" | "variable_declaration")
        {
            let mut child_cursor = node.walk();
            for child in node.children(&mut child_cursor) {
                if child.kind() == "variable_declarator" {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let var_name = name_node.utf8_text(source.as_bytes()).unwrap_or("");
                        if var_name == function_name {
                            // Check if the value is a function
                            if let Some(value_node) = child.child_by_field_name("value") {
                                if matches!(
                                    value_node.kind(),
                                    "arrow_function"
                                        | "function"
                                        | "function_expression"
                                        | "generator_function"
                                ) {
                                    return Some(value_node);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Check for Lua/Luau assignment-based functions: M.request = function() end
        if matches!(language, Language::Lua | Language::Luau)
            && matches!(node.kind(), "assignment_statement" | "variable_assignment")
        {
            let mut child_cursor = node.walk();
            let children: Vec<_> = node.children(&mut child_cursor).collect();
            // Look for field_expression or dot_index_expression on LHS, function on RHS
            for child in &children {
                if matches!(child.kind(), "variable_list" | "assignment_variable_list") {
                    let mut inner_cursor = child.walk();
                    for inner in child.children(&mut inner_cursor) {
                        if matches!(inner.kind(), "field_expression" | "dot_index_expression") {
                            let lhs_text = inner.utf8_text(source.as_bytes()).unwrap_or("");
                            // Check if the field name matches (e.g. "M.request" -> "request")
                            if let Some(field_name) = lhs_text.rsplit('.').next() {
                                if field_name == function_name || lhs_text == function_name {
                                    // Find function_definition in RHS (handles both direct and wrapped)
                                    for rhs in &children {
                                        if matches!(
                                            rhs.kind(),
                                            "expression_list" | "assignment_expression_list"
                                        ) {
                                            let mut rhs_cursor = rhs.walk();
                                            for rhs_child in rhs.children(&mut rhs_cursor) {
                                                if let Some(func) = find_function_in_node(rhs_child, 3)
                                                {
                                                    return Some(func);
                                                }
                                            }
                                        }
                                        if let Some(func) = find_function_in_node(*rhs, 3) {
                                            return Some(func);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Add children to stack (reverse order for depth-first)
        let mut cursor = node.walk();
        let children: Vec<_> = node.children(&mut cursor).collect();
        for child in children.into_iter().rev() {
            stack.push(child);
        }
    }

    None
}

/// Get the node kinds that represent functions in each language
pub fn get_function_node_kinds(language: Language) -> &'static [&'static str] {
    match language {
        Language::Python => &["function_definition"],
        Language::TypeScript | Language::JavaScript => &[
            "function_declaration",
            "arrow_function",
            "method_definition",
            "function",
            "generator_function_declaration",
            "generator_function",
        ],
        Language::Go => &["function_declaration", "method_declaration"],
        Language::Rust => &["function_item"],
        Language::Java => &["method_declaration", "constructor_declaration"],
        Language::C | Language::Cpp => &["function_definition"],
        Language::Ruby => &["method", "singleton_method"],
        Language::Php => &["function_definition", "method_declaration"],
        Language::CSharp => &["method_declaration", "constructor_declaration"],
        Language::Kotlin => &["function_declaration"],
        Language::Scala => &["function_definition", "function_declaration"],
        Language::Elixir => &["call"], // Elixir uses def/defp which are calls
        Language::Lua | Language::Luau => &[
            "function_declaration",
            "function_definition",
            "local_function",
        ],
        Language::Swift => &["function_declaration", "init_declaration"],
        Language::Ocaml => &["let_binding", "value_definition"],
    }
}

/// Recursively extract the function name from a C/C++ declarator chain.
/// Handles: function_declarator, pointer_declarator, parenthesized_declarator,
/// identifier, field_identifier, qualified_identifier, destructor_name,
/// template_function.
fn extract_c_declarator_name(declarator: Node, source: &str) -> Option<String> {
    match declarator.kind() {
        "identifier" | "field_identifier" => {
            // field_identifier is used for methods defined inline in class bodies
            Some(
                declarator
                    .utf8_text(source.as_bytes())
                    .unwrap_or("")
                    .to_string(),
            )
        }
        "destructor_name" => {
            // ~ClassName - return as-is
            Some(
                declarator
                    .utf8_text(source.as_bytes())
                    .unwrap_or("")
                    .to_string(),
            )
        }
        "qualified_identifier" => {
            // C++ qualified name: Namespace::Class::method
            // Tree-sitter nests these recursively:
            //   qualified_identifier(Luau::Analysis::Normalizer::normalize)
            //     -> namespace_identifier(Luau), qualified_identifier(Analysis::...)
            //       -> ... -> identifier(normalize)
            // We need to find the deepest rightmost identifier.
            let mut cursor = declarator.walk();
            for child in declarator.children(&mut cursor) {
                // If there's a nested qualified_identifier, recurse into it
                if child.kind() == "qualified_identifier" {
                    return extract_c_declarator_name(child, source);
                }
            }
            // No nested qualified_identifier: look for terminal name nodes
            let mut cursor2 = declarator.walk();
            for child in declarator.children(&mut cursor2) {
                if matches!(child.kind(), "identifier" | "destructor_name") {
                    return Some(child.utf8_text(source.as_bytes()).unwrap_or("").to_string());
                }
                if child.kind() == "template_function" {
                    return child
                        .child_by_field_name("name")
                        .or_else(|| child.named_child(0))
                        .map(|n| n.utf8_text(source.as_bytes()).unwrap_or("").to_string());
                }
            }
            None
        }
        "template_function" => {
            // template<T> void foo() - extract identifier from template_function
            declarator
                .child_by_field_name("name")
                .or_else(|| declarator.named_child(0))
                .map(|n| n.utf8_text(source.as_bytes()).unwrap_or("").to_string())
        }
        "function_declarator" => {
            // function_declarator has a "declarator" field which is the name (identifier)
            if let Some(inner) = declarator.child_by_field_name("declarator") {
                return extract_c_declarator_name(inner, source);
            }
            None
        }
        "pointer_declarator" | "reference_declarator" => {
            // pointer_declarator wraps: * <inner_declarator>
            // reference_declarator wraps: & <inner_declarator>
            if let Some(inner) = declarator.child_by_field_name("declarator") {
                return extract_c_declarator_name(inner, source);
            }
            // Fallback: search children for function_declarator or identifier
            let mut cursor = declarator.walk();
            for child in declarator.children(&mut cursor) {
                if matches!(
                    child.kind(),
                    "function_declarator" | "identifier" | "field_identifier"
                ) {
                    return extract_c_declarator_name(child, source);
                }
            }
            None
        }
        "parenthesized_declarator" => {
            // parenthesized_declarator wraps: ( <inner_declarator> )
            let mut cursor = declarator.walk();
            for child in declarator.children(&mut cursor) {
                if child.is_named() {
                    if let Some(name) = extract_c_declarator_name(child, source) {
                        return Some(name);
                    }
                }
            }
            None
        }
        _ => None,
    }
}

/// Extract function name from a function node
pub fn get_function_name(node: Node, language: Language, source: &str) -> Option<String> {
    match language {
        Language::C | Language::Cpp => {
            // C/C++: function_definition -> declarator -> ... -> identifier
            // The declarator chain can be:
            //   function_declarator -> identifier (simple: int foo())
            //   pointer_declarator -> function_declarator -> identifier (pointer return: int *foo())
            //   identifier (rare, no parens)
            if let Some(declarator) = node.child_by_field_name("declarator") {
                return extract_c_declarator_name(declarator, source);
            }
            None
        }
        Language::Ruby => {
            // Ruby: method node has "name" field
            node.child_by_field_name("name")
                .map(|n| n.utf8_text(source.as_bytes()).unwrap_or("").to_string())
        }
        Language::Php => {
            // PHP function_definition has "name" field
            node.child_by_field_name("name")
                .map(|n| n.utf8_text(source.as_bytes()).unwrap_or("").to_string())
        }
        Language::Elixir => {
            // Elixir: def/defp are calls. The first argument after "def" is the function clause
            // Structure: (call (identifier "def") (arguments (call (identifier "func_name") ...)))
            if node.kind() == "call" {
                // First child should be "def" or "defp"
                let first_child = node.child(0)?;
                let first_text = first_child.utf8_text(source.as_bytes()).unwrap_or("");
                if first_text == "def" || first_text == "defp" {
                    // Second child: arguments containing the function name
                    if let Some(args) = node.child(1) {
                        // Could be directly an identifier or a call node
                        if args.kind() == "identifier" {
                            return Some(
                                args.utf8_text(source.as_bytes()).unwrap_or("").to_string(),
                            );
                        }
                        if args.kind() == "arguments" || args.kind() == "call" {
                            // Find the first identifier
                            let mut cursor = args.walk();
                            for child in args.children(&mut cursor) {
                                if child.kind() == "identifier" {
                                    return Some(
                                        child
                                            .utf8_text(source.as_bytes())
                                            .unwrap_or("")
                                            .to_string(),
                                    );
                                }
                                if child.kind() == "call" {
                                    if let Some(name) = child.child(0) {
                                        if name.kind() == "identifier" {
                                            return Some(
                                                name.utf8_text(source.as_bytes())
                                                    .unwrap_or("")
                                                    .to_string(),
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                None
            } else {
                None
            }
        }
        Language::Ocaml => {
            // OCaml: let_binding has "pattern" field (not "name")
            // Structure: (let_binding pattern: (value_name) body: ...)
            // For value_definition, it wraps let_binding(s)
            if node.kind() == "value_definition" {
                // Find the first let_binding child and recurse
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "let_binding" {
                        return get_function_name(child, language, source);
                    }
                }
                None
            } else {
                // let_binding: pattern field contains the name
                node.child_by_field_name("pattern")
                    .map(|n| n.utf8_text(source.as_bytes()).unwrap_or("").to_string())
            }
        }
        Language::Swift => {
            // Swift: function_declaration has "name" field, but init_declaration does not --
            // the keyword "init" IS the name.
            if node.kind() == "init_declaration" {
                return Some("init".to_string());
            }
            node.child_by_field_name("name")
                .map(|n| n.utf8_text(source.as_bytes()).unwrap_or("").to_string())
        }
        Language::Lua | Language::Luau => {
            // Lua/Luau: function_declaration may have a dot_index_expression as name
            // e.g. function Kong.init() -> name is dot_index_expression "Kong.init"
            if let Some(name_node) = node.child_by_field_name("name") {
                let name_text = name_node
                    .utf8_text(source.as_bytes())
                    .unwrap_or("")
                    .to_string();
                return Some(name_text);
            }
            // Fallback for local_function or other variants: search named children
            // for identifier or dot_index_expression
            if node.kind() == "local_function" || node.kind() == "function_declaration" {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if matches!(child.kind(), "identifier" | "dot_index_expression") {
                        return Some(child.utf8_text(source.as_bytes()).unwrap_or("").to_string());
                    }
                }
            }
            // For function_definition (anonymous), no name
            None
        }
        _ => {
            // Most languages use "name" field
            node.child_by_field_name("name")
                .map(|n| n.utf8_text(source.as_bytes()).unwrap_or("").to_string())
        }
    }
}

/// Get the body node of a function
pub fn get_function_body(func_node: Node, language: Language) -> Option<Node> {
    match language {
        Language::Python => func_node.child_by_field_name("body"),
        Language::TypeScript | Language::JavaScript => func_node.child_by_field_name("body"),
        Language::Go => func_node.child_by_field_name("body"),
        Language::Rust => func_node.child_by_field_name("body"),
        Language::Java => func_node.child_by_field_name("body"),
        Language::C | Language::Cpp => func_node.child_by_field_name("body"),
        Language::Ruby => func_node.child_by_field_name("body"),
        Language::Php => func_node.child_by_field_name("body"),
        Language::CSharp => func_node.child_by_field_name("body"),
        Language::Kotlin => {
            // Kotlin: function_declaration has function_body as a named child (not a field).
            // function_body contains a block with the actual statements.
            func_node.child_by_field_name("body").or_else(|| {
                let mut cursor = func_node.walk();
                for child in func_node.children(&mut cursor) {
                    if child.kind() == "function_body" {
                        // function_body may contain a block or a direct expression
                        let mut inner = child.walk();
                        for inner_child in child.children(&mut inner) {
                            if inner_child.kind() == "block" {
                                return Some(inner_child);
                            }
                        }
                        return Some(child);
                    }
                }
                None
            })
        }
        Language::Scala => func_node.child_by_field_name("body"),
        Language::Elixir => {
            // Elixir def body is inside a "do" block
            // Structure: (call "def" (arguments ...) (do_block (body)))
            let mut cursor = func_node.walk();
            for child in func_node.children(&mut cursor) {
                if child.kind() == "do_block" {
                    return Some(child);
                }
            }
            func_node.child_by_field_name("body")
        }
        Language::Lua | Language::Luau => func_node.child_by_field_name("body"),
        Language::Ocaml => {
            // OCaml: func_node may be value_definition or let_binding.
            // For value_definition, drill down to let_binding first.
            // For let_binding, the body field contains the expression.
            if func_node.kind() == "value_definition" {
                // Find let_binding child, then get its body
                let child_count = func_node.child_count();
                let mut binding_body = None;
                for i in 0..child_count {
                    if let Some(child) = func_node.child(i) {
                        if child.kind() == "let_binding" {
                            binding_body = child.child_by_field_name("body");
                            break;
                        }
                    }
                }
                binding_body.or(Some(func_node))
            } else {
                // Already a let_binding
                func_node.child_by_field_name("body").or(Some(func_node))
            }
        }
        _ => func_node.child_by_field_name("body"),
    }
}

/// Convenience: get function node kinds as a Vec (for callers that need Vec<&'static str>)
pub fn get_function_node_kinds_vec(language: Language) -> Vec<&'static str> {
    get_function_node_kinds(language).to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::parser::parse;

    // -- TypeScript generator function tests --

    #[test]
    fn test_ts_generator_function_declaration() {
        let source = r#"
function* genNumbers(): Generator<number> {
    yield 1;
    yield 2;
    yield 3;
}
"#;
        let tree = parse(source, Language::TypeScript).unwrap();
        let root = tree.root_node();
        let node = find_function_node(root, "genNumbers", Language::TypeScript, source);
        assert!(node.is_some(), "Should find generator function declaration");
        let node = node.unwrap();
        assert_eq!(node.kind(), "generator_function_declaration");
        let name = get_function_name(node, Language::TypeScript, source);
        assert_eq!(name.as_deref(), Some("genNumbers"));
        let body = get_function_body(node, Language::TypeScript);
        assert!(body.is_some(), "Should find body of generator function");
    }

    #[test]
    fn test_ts_async_generator_function() {
        let source = r#"
async function* asyncGen(): AsyncGenerator<string> {
    yield "hello";
    yield "world";
}
"#;
        let tree = parse(source, Language::TypeScript).unwrap();
        let root = tree.root_node();
        let node = find_function_node(root, "asyncGen", Language::TypeScript, source);
        assert!(
            node.is_some(),
            "Should find async generator function declaration"
        );
        let node = node.unwrap();
        assert_eq!(node.kind(), "generator_function_declaration");
        let name = get_function_name(node, Language::TypeScript, source);
        assert_eq!(name.as_deref(), Some("asyncGen"));
    }

    #[test]
    fn test_ts_generator_function_expression() {
        let source = r#"
const genArrow = function*(x: number): Generator<number> {
    yield x;
};
"#;
        let tree = parse(source, Language::TypeScript).unwrap();
        let root = tree.root_node();
        let node = find_function_node(root, "genArrow", Language::TypeScript, source);
        assert!(
            node.is_some(),
            "Should find generator function expression via const assignment"
        );
        let node = node.unwrap();
        assert_eq!(node.kind(), "generator_function");
        let body = get_function_body(node, Language::TypeScript);
        assert!(
            body.is_some(),
            "Should find body of generator function expression"
        );
    }

    // -- JavaScript generator function tests --

    #[test]
    fn test_js_generator_function_declaration() {
        let source = r#"
function* genNumbers() {
    yield 1;
    yield 2;
}
"#;
        let tree = parse(source, Language::JavaScript).unwrap();
        let root = tree.root_node();
        let node = find_function_node(root, "genNumbers", Language::JavaScript, source);
        assert!(
            node.is_some(),
            "Should find JS generator function declaration"
        );
        let node = node.unwrap();
        assert_eq!(node.kind(), "generator_function_declaration");
    }

    #[test]
    fn test_js_async_generator_function() {
        let source = r#"
async function* asyncIter() {
    yield "a";
    yield "b";
}
"#;
        let tree = parse(source, Language::JavaScript).unwrap();
        let root = tree.root_node();
        let node = find_function_node(root, "asyncIter", Language::JavaScript, source);
        assert!(node.is_some(), "Should find JS async generator function");
    }

    // -- TypeScript regular function tests (regression) --

    #[test]
    fn test_ts_regular_function() {
        let source = r#"
function greet(name: string): string {
    const greeting = "Hello, " + name;
    return greeting;
}
"#;
        let tree = parse(source, Language::TypeScript).unwrap();
        let root = tree.root_node();
        let node = find_function_node(root, "greet", Language::TypeScript, source);
        assert!(node.is_some(), "Should find regular function declaration");
        assert_eq!(node.unwrap().kind(), "function_declaration");
    }

    #[test]
    fn test_ts_arrow_function() {
        let source = r#"
const add = (a: number, b: number): number => {
    return a + b;
};
"#;
        let tree = parse(source, Language::TypeScript).unwrap();
        let root = tree.root_node();
        let node = find_function_node(root, "add", Language::TypeScript, source);
        assert!(node.is_some(), "Should find arrow function via const");
        assert_eq!(node.unwrap().kind(), "arrow_function");
    }

    #[test]
    fn test_ts_class_method() {
        let source = r#"
class MyClass {
    myMethod(x: number): number {
        return x * 2;
    }
}
"#;
        let tree = parse(source, Language::TypeScript).unwrap();
        let root = tree.root_node();
        let node = find_function_node(root, "myMethod", Language::TypeScript, source);
        assert!(node.is_some(), "Should find class method");
        assert_eq!(node.unwrap().kind(), "method_definition");
    }

    #[test]
    fn test_ts_exported_function() {
        let source = r#"
export function fetchData(url: string): Promise<string> {
    return fetch(url).then(r => r.text());
}
"#;
        let tree = parse(source, Language::TypeScript).unwrap();
        let root = tree.root_node();
        let node = find_function_node(root, "fetchData", Language::TypeScript, source);
        assert!(node.is_some(), "Should find exported function");
    }

    #[test]
    fn test_ts_exported_generator() {
        let source = r#"
export function* items(): Generator<number> {
    yield 1;
    yield 2;
}
"#;
        let tree = parse(source, Language::TypeScript).unwrap();
        let root = tree.root_node();
        let node = find_function_node(root, "items", Language::TypeScript, source);
        assert!(node.is_some(), "Should find exported generator function");
    }

    // -- DFG integration tests for generator functions --

    #[test]
    fn test_ts_generator_dfg() {
        use crate::dfg::get_dfg_context;
        let source = r#"
function* genNumbers(): Generator<number> {
    const x = 1;
    yield x;
    const y = 2;
    yield y;
}
"#;
        let result = get_dfg_context(source, "genNumbers", Language::TypeScript);
        assert!(
            result.is_ok(),
            "DFG should succeed for generator functions, got: {:?}",
            result.err()
        );
        let dfg = result.unwrap();
        assert_eq!(dfg.function, "genNumbers");
        assert!(
            !dfg.variables.is_empty(),
            "Should find variables in generator function"
        );
    }

    #[test]
    fn test_ts_async_generator_dfg() {
        use crate::dfg::get_dfg_context;
        let source = r#"
async function* asyncGen(): AsyncGenerator<string> {
    const msg = "hello";
    yield msg;
}
"#;
        let result = get_dfg_context(source, "asyncGen", Language::TypeScript);
        assert!(
            result.is_ok(),
            "DFG should succeed for async generator functions, got: {:?}",
            result.err()
        );
    }

    // -- CFG integration tests for generator functions --

    #[test]
    fn test_ts_generator_cfg() {
        use crate::cfg::get_cfg_context;
        let source = r#"
function* genNumbers(): Generator<number> {
    const x = 1;
    yield x;
}
"#;
        let result = get_cfg_context(source, "genNumbers", Language::TypeScript);
        assert!(result.is_ok());
        let cfg = result.unwrap();
        assert_eq!(cfg.function, "genNumbers");
        assert!(
            !cfg.blocks.is_empty(),
            "CFG should have blocks for generator function"
        );
    }

    // -- get_function_node_kinds tests --

    #[test]
    fn test_ts_node_kinds_include_generators() {
        let kinds = get_function_node_kinds(Language::TypeScript);
        assert!(
            kinds.contains(&"generator_function_declaration"),
            "TypeScript node kinds should include generator_function_declaration"
        );
        assert!(
            kinds.contains(&"generator_function"),
            "TypeScript node kinds should include generator_function"
        );
    }

    #[test]
    fn test_js_node_kinds_include_generators() {
        let kinds = get_function_node_kinds(Language::JavaScript);
        assert!(
            kinds.contains(&"generator_function_declaration"),
            "JavaScript node kinds should include generator_function_declaration"
        );
        assert!(
            kinds.contains(&"generator_function"),
            "JavaScript node kinds should include generator_function"
        );
    }

    // -- C pointer-returning function tests --

    #[test]
    fn test_c_pointer_returning_function() {
        let source = r#"
typedef struct { int x; } MyStruct;

MyStruct *createStruct(void) {
    int y = 1;
    return (MyStruct*)0;
}
"#;
        let tree = parse(source, Language::C).unwrap();
        let root = tree.root_node();
        let node = find_function_node(root, "createStruct", Language::C, source);
        assert!(
            node.is_some(),
            "Should find C function with pointer return type"
        );
        let node = node.unwrap();
        assert_eq!(node.kind(), "function_definition");
        let name = get_function_name(node, Language::C, source);
        assert_eq!(name.as_deref(), Some("createStruct"));
    }

    #[test]
    fn test_c_pointer_returning_function_dfg() {
        use crate::dfg::get_dfg_context;
        let source = r#"
typedef struct { int x; } MyStruct;

MyStruct *createStruct(int val) {
    int y = val + 1;
    return (MyStruct*)0;
}
"#;
        let result = get_dfg_context(source, "createStruct", Language::C);
        assert!(
            result.is_ok(),
            "DFG should succeed for C pointer-returning function, got: {:?}",
            result.err()
        );
        let dfg = result.unwrap();
        assert_eq!(dfg.function, "createStruct");
    }

    #[test]
    fn test_cpp_pointer_returning_function() {
        let source = r#"
struct Node { int val; };

Node *createNode(int x) {
    int temp = x * 2;
    return nullptr;
}
"#;
        let tree = parse(source, Language::Cpp).unwrap();
        let root = tree.root_node();
        let node = find_function_node(root, "createNode", Language::Cpp, source);
        assert!(
            node.is_some(),
            "Should find C++ function with pointer return type"
        );
        let name = get_function_name(node.unwrap(), Language::Cpp, source);
        assert_eq!(name.as_deref(), Some("createNode"));
    }

    // -- Swift init_declaration tests --

    #[test]
    fn test_swift_init_declaration() {
        let source = r#"
class App {
    init(port: Int) {
        let x = port + 1
    }
}
"#;
        let tree = parse(source, Language::Swift).unwrap();
        let root = tree.root_node();
        let node = find_function_node(root, "init", Language::Swift, source);
        assert!(node.is_some(), "Should find Swift init declaration");
    }

    #[test]
    fn test_swift_init_dfg() {
        use crate::dfg::get_dfg_context;
        let source = r#"
class Server {
    init(port: Int) {
        let addr = port + 1000
    }
}
"#;
        let result = get_dfg_context(source, "init", Language::Swift);
        assert!(
            result.is_ok(),
            "DFG should succeed for Swift init, got: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_swift_node_kinds_include_init() {
        let kinds = get_function_node_kinds(Language::Swift);
        assert!(
            kinds.contains(&"init_declaration"),
            "Swift node kinds should include init_declaration"
        );
    }

    // -- Lua dot-indexed function name tests --

    #[test]
    fn test_lua_dot_indexed_function_short_name() {
        let source = r#"
function Kong.init()
    local x = 1
    return x
end
"#;
        let tree = parse(source, Language::Lua).unwrap();
        let root = tree.root_node();
        // Should find "init" when searching by short name
        let node = find_function_node(root, "init", Language::Lua, source);
        assert!(
            node.is_some(),
            "Should find Lua dot-indexed function by short name 'init'"
        );
    }

    #[test]
    fn test_lua_dot_indexed_function_full_name() {
        let source = r#"
function Kong.init()
    local x = 1
    return x
end
"#;
        let tree = parse(source, Language::Lua).unwrap();
        let root = tree.root_node();
        // Should also find by full qualified name
        let node = find_function_node(root, "Kong.init", Language::Lua, source);
        assert!(
            node.is_some(),
            "Should find Lua dot-indexed function by full name 'Kong.init'"
        );
    }

    #[test]
    fn test_lua_dot_indexed_function_dfg() {
        use crate::dfg::get_dfg_context;
        let source = r#"
function M.request(url)
    local result = url .. "/api"
    return result
end
"#;
        let result = get_dfg_context(source, "request", Language::Lua);
        assert!(
            result.is_ok(),
            "DFG should succeed for Lua dot-indexed function by short name, got: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_luau_dot_indexed_function_short_name() {
        let source = r#"
function Module.process(data)
    local x = data + 1
    return x
end
"#;
        let tree = parse(source, Language::Luau).unwrap();
        let root = tree.root_node();
        let node = find_function_node(root, "process", Language::Luau, source);
        assert!(
            node.is_some(),
            "Should find Luau dot-indexed function by short name 'process'"
        );
    }

    // =========================================================================
    // C++ qualified method definition tests
    // =========================================================================

    #[test]
    fn test_cpp_qualified_method_definition() {
        // C++ method defined outside class body with ClassName::method syntax
        let source = r#"
class MyClass {
public:
    void externalMethod();
};

void MyClass::externalMethod() {
    int x = 1;
}
"#;
        let tree = parse(source, Language::Cpp).unwrap();
        let root = tree.root_node();
        let node = find_function_node(root, "externalMethod", Language::Cpp, source);
        assert!(
            node.is_some(),
            "Should find C++ qualified method definition (ClassName::method)"
        );
        let node = node.unwrap();
        assert_eq!(node.kind(), "function_definition");
        let name = get_function_name(node, Language::Cpp, source);
        assert_eq!(
            name.as_deref(),
            Some("externalMethod"),
            "get_function_name should extract bare name from qualified C++ method"
        );
    }

    #[test]
    fn test_cpp_qualified_method_dfg() {
        use crate::dfg::get_dfg_context;
        let source = r#"
void MyClass::compute(int a) {
    int b = a + 1;
    int c = b * 2;
}
"#;
        let result = get_dfg_context(source, "compute", Language::Cpp);
        assert!(
            result.is_ok(),
            "DFG should succeed for C++ qualified method, got: {:?}",
            result.err()
        );
        let dfg = result.unwrap();
        assert_eq!(dfg.function, "compute");
    }

    #[test]
    fn test_cpp_inline_class_method() {
        // C++ method defined inline inside class body
        let source = r#"
class MyClass {
public:
    void myMethod() {
        int x = 1;
    }
};
"#;
        let tree = parse(source, Language::Cpp).unwrap();
        let root = tree.root_node();
        let node = find_function_node(root, "myMethod", Language::Cpp, source);
        assert!(
            node.is_some(),
            "Should find C++ inline class method (field_identifier)"
        );
        let node = node.unwrap();
        assert_eq!(node.kind(), "function_definition");
        let name = get_function_name(node, Language::Cpp, source);
        assert_eq!(
            name.as_deref(),
            Some("myMethod"),
            "get_function_name should extract name from inline C++ class method"
        );
    }

    #[test]
    fn test_cpp_inline_class_method_dfg() {
        use crate::dfg::get_dfg_context;
        let source = r#"
class Widget {
public:
    int calculate(int a, int b) {
        int sum = a + b;
        int product = a * b;
        return sum;
    }
};
"#;
        let result = get_dfg_context(source, "calculate", Language::Cpp);
        assert!(
            result.is_ok(),
            "DFG should succeed for C++ inline class method, got: {:?}",
            result.err()
        );
        let dfg = result.unwrap();
        assert_eq!(dfg.function, "calculate");
        assert!(
            !dfg.variables.is_empty(),
            "Should find variables in inline class method"
        );
    }

    #[test]
    fn test_cpp_namespace_function() {
        // C++ function inside a namespace
        let source = r#"
namespace Foo {
    void bar() {
        int x = 1;
    }
}
"#;
        let tree = parse(source, Language::Cpp).unwrap();
        let root = tree.root_node();
        let node = find_function_node(root, "bar", Language::Cpp, source);
        assert!(node.is_some(), "Should find C++ function inside namespace");
    }

    #[test]
    fn test_cpp_const_qualified_method() {
        // C++ const method defined outside class
        let source = r#"
bool NormalizedStringType::isNever() const {
    return !isCofinite && singletons.empty();
}
"#;
        let tree = parse(source, Language::Cpp).unwrap();
        let root = tree.root_node();
        let node = find_function_node(root, "isNever", Language::Cpp, source);
        assert!(node.is_some(), "Should find C++ const qualified method");
        let name = get_function_name(node.unwrap(), Language::Cpp, source);
        assert_eq!(name.as_deref(), Some("isNever"));
    }

    #[test]
    fn test_cpp_nested_namespace_qualified_method() {
        // C++ method with deeply nested namespace::class::method
        let source = r#"
void Luau::Analysis::Normalizer::normalize() {
    int x = 1;
}
"#;
        let tree = parse(source, Language::Cpp).unwrap();
        let root = tree.root_node();
        let node = find_function_node(root, "normalize", Language::Cpp, source);
        assert!(node.is_some(), "Should find deeply qualified C++ method");
        let name = get_function_name(node.unwrap(), Language::Cpp, source);
        assert_eq!(name.as_deref(), Some("normalize"));
    }

    // =========================================================================
    // C qualified function tests (same issues apply to C with qualified names)
    // =========================================================================

    #[test]
    fn test_c_inline_struct_method_like() {
        // C doesn't have classes but let's ensure function_definition inside
        // struct declarations or other nesting still works
        let source = r#"
void process(int x) {
    int y = x + 1;
}
"#;
        let tree = parse(source, Language::C).unwrap();
        let root = tree.root_node();
        let node = find_function_node(root, "process", Language::C, source);
        assert!(node.is_some(), "Should find simple C function");
    }

    // =========================================================================
    // Lua/Luau local_function tests
    // =========================================================================

    #[test]
    fn test_lua_local_function_node_kinds() {
        // Verify that local_function is included in node kinds for Lua
        // This ensures consistency with ast_utils::function_node_kinds
        let kinds = get_function_node_kinds(Language::Lua);
        // Lua tree-sitter may or may not use "local_function" - but if ast_utils
        // includes it, function_finder should too for consistency
        let ast_kinds = crate::security::ast_utils::function_node_kinds(Language::Lua);
        for kind in ast_kinds {
            assert!(
                kinds.contains(kind),
                "function_finder should include '{}' which ast_utils includes for Lua",
                kind
            );
        }
    }

    #[test]
    fn test_luau_local_function_node_kinds() {
        let kinds = get_function_node_kinds(Language::Luau);
        let ast_kinds = crate::security::ast_utils::function_node_kinds(Language::Luau);
        for kind in ast_kinds {
            assert!(
                kinds.contains(kind),
                "function_finder should include '{}' which ast_utils includes for Luau",
                kind
            );
        }
    }
}
