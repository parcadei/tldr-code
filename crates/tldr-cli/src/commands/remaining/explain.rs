//! Explain Command - Comprehensive Function Analysis
//!
//! The explain command provides a complete analysis of a function including:
//! - Signature extraction (params, return type, decorators, docstring)
//! - Purity analysis (pure/impure/unknown with effects)
//! - Complexity metrics (cyclomatic, blocks, edges, loops)
//! - Call relationships (callers and callees)
//!
//! # Example
//!
//! ```bash
//! # Analyze a function
//! tldr explain src/utils.py calculate_total
//!
//! # With call graph depth
//! tldr explain src/utils.py calculate_total --depth 3
//!
//! # Text output
//! tldr explain src/utils.py calculate_total --format text
//! ```

use std::collections::HashSet;
use std::path::PathBuf;

use anyhow::Result;
use clap::Args;
use tree_sitter::{Node, Parser};

use super::error::RemainingError;
use super::types::{CallInfo, ComplexityInfo, ExplainReport, ParamInfo, PurityInfo, SignatureInfo};

use crate::output::{OutputFormat, OutputWriter};
use tldr_core::types::Language;

// =============================================================================
// CLI Arguments
// =============================================================================

/// Provide comprehensive function analysis.
#[derive(Debug, Clone, Args)]
pub struct ExplainArgs {
    /// Source file to analyze
    pub file: PathBuf,

    /// Function name to explain
    pub function: String,

    /// Call graph depth for callers/callees
    #[arg(long, default_value = "2")]
    pub depth: u32,

    /// Output file (stdout if not specified)
    #[arg(long, short = 'o')]
    pub output: Option<PathBuf>,
}

// =============================================================================
// Constants
// =============================================================================

/// Known I/O operations that make a function impure
const IO_OPERATIONS: &[&str] = &[
    "print",
    "open",
    "read",
    "write",
    "readline",
    "readlines",
    "writelines",
    "input",
    "system",
    "popen",
    "exec",
    "eval",
    "request",
    "fetch",
    "urlopen",
    "execute",
    "executemany",
    "fetchone",
    "fetchall",
];

/// Known impure calls (non-deterministic or side-effecting)
const IMPURE_CALLS: &[&str] = &[
    "random",
    "randint",
    "choice",
    "shuffle",
    "sample",
    "uniform",
    "random.random",
    "random.randint",
    "random.choice",
    "random.shuffle",
    "time",
    "time.time",
    "datetime.now",
    "datetime.datetime.now",
    "uuid4",
    "uuid1",
    "uuid.uuid4",
    "uuid.uuid1",
    "logging.info",
    "logging.debug",
    "logging.warning",
    "logging.error",
    "os.system",
    "os.popen",
    "os.getenv",
    "os.environ",
    "os.mkdir",
    "os.remove",
    "requests.get",
    "requests.post",
    "requests.put",
    "requests.delete",
    "subprocess.run",
    "subprocess.call",
    "subprocess.Popen",
];

/// Collection mutation methods
const COLLECTION_MUTATIONS: &[&str] = &[
    "append",
    "extend",
    "insert",
    "remove",
    "pop",
    "clear",
    "update",
    "add",
    "discard",
    "setdefault",
    "sort",
    "reverse",
];

/// Known pure builtins
const PURE_BUILTINS: &[&str] = &[
    "len",
    "range",
    "int",
    "float",
    "str",
    "bool",
    "list",
    "dict",
    "set",
    "tuple",
    "sorted",
    "reversed",
    "enumerate",
    "zip",
    "map",
    "filter",
    "min",
    "max",
    "sum",
    "abs",
    "round",
    "isinstance",
    "issubclass",
    "type",
    "id",
    "hash",
    "repr",
    "next",
    "iter",
    "all",
    "any",
    "chr",
    "ord",
    "hex",
    "oct",
    "bin",
    "pow",
    "divmod",
    "super",
    "property",
    "staticmethod",
    "classmethod",
];

// =============================================================================
// Tree-sitter Multi-Language Parsing
// =============================================================================

/// Get function node kinds for a given language
fn get_function_node_kinds(language: Language) -> &'static [&'static str] {
    match language {
        Language::Python => &["function_definition", "async_function_definition"],
        Language::TypeScript | Language::JavaScript => &[
            "function_declaration",
            "arrow_function",
            "method_definition",
            "function",
        ],
        Language::Go => &["function_declaration", "method_declaration"],
        Language::Rust => &["function_item"],
        Language::Java => &["method_declaration", "constructor_declaration"],
        Language::Kotlin => &["function_declaration"],
        Language::CSharp => &["method_declaration", "constructor_declaration"],
        Language::Ruby => &["method", "singleton_method"],
        Language::Php => &["function_definition", "method_declaration"],
        Language::Scala => &["function_definition"],
        Language::Swift => &["function_declaration"],
        Language::C | Language::Cpp => &["function_definition"],
        Language::Lua | Language::Luau => &["function_declaration", "function_definition"],
        Language::Elixir => &["call"], // Elixir def/defp are call nodes
        Language::Ocaml => &["value_definition"],
    }
}

/// Initialize tree-sitter parser for the detected language
fn get_parser(language: Language) -> Result<Parser, RemainingError> {
    let mut parser = Parser::new();

    let ts_language = match language {
        Language::Python => tree_sitter_python::LANGUAGE.into(),
        Language::TypeScript => tree_sitter_typescript::LANGUAGE_TSX.into(),
        Language::JavaScript => tree_sitter_typescript::LANGUAGE_TSX.into(),
        Language::Go => tree_sitter_go::LANGUAGE.into(),
        Language::Rust => tree_sitter_rust::LANGUAGE.into(),
        Language::Java => tree_sitter_java::LANGUAGE.into(),
        Language::C => tree_sitter_c::LANGUAGE.into(),
        Language::Cpp => tree_sitter_cpp::LANGUAGE.into(),
        Language::CSharp => tree_sitter_c_sharp::LANGUAGE.into(),
        Language::Kotlin => tree_sitter_kotlin_ng::LANGUAGE.into(),
        Language::Scala => tree_sitter_scala::LANGUAGE.into(),
        Language::Php => tree_sitter_php::LANGUAGE_PHP.into(),
        Language::Ruby => tree_sitter_ruby::LANGUAGE.into(),
        Language::Lua => tree_sitter_lua::LANGUAGE.into(),
        Language::Luau => tree_sitter_luau::LANGUAGE.into(),
        Language::Elixir => tree_sitter_elixir::LANGUAGE.into(),
        Language::Ocaml => tree_sitter_ocaml::LANGUAGE_OCAML.into(),
        Language::Swift => tree_sitter_swift::LANGUAGE.into(),
    };

    parser.set_language(&ts_language).map_err(|e| {
        RemainingError::parse_error(PathBuf::new(), format!("Failed to set language: {}", e))
    })?;
    Ok(parser)
}

/// Get text for a node from source
fn node_text<'a>(node: Node, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
}

/// Get the line number (1-indexed) for a node
fn get_line_number(node: Node) -> u32 {
    node.start_position().row as u32 + 1
}

/// Get the end line number (1-indexed) for a node
fn get_end_line_number(node: Node) -> u32 {
    node.end_position().row as u32 + 1
}

// =============================================================================
// Function Finding
// =============================================================================

/// Find a function definition by name in the AST
fn find_function_node<'a>(
    root: Node<'a>,
    source: &[u8],
    function_name: &str,
    func_kinds: &[&str],
) -> Option<Node<'a>> {
    find_function_recursive(root, source, function_name, func_kinds)
}

fn find_function_recursive<'a>(
    node: Node<'a>,
    source: &[u8],
    function_name: &str,
    func_kinds: &[&str],
) -> Option<Node<'a>> {
    if func_kinds.contains(&node.kind()) {
        // Check if this function has the name we're looking for
        // Try field name first (most reliable)
        if let Some(name_node) = node.child_by_field_name("name") {
            let name = node_text(name_node, source);
            if name == function_name {
                return Some(node);
            }
        }
        // C/C++: function_definition -> declarator -> function_declarator -> identifier
        if let Some(declarator) = node.child_by_field_name("declarator") {
            if let Some(name) = extract_c_declarator_name_explain(declarator, source) {
                if name == function_name {
                    return Some(node);
                }
            }
        }
        // Fallback: search for identifier child (Python, etc.)
        for child in node.children(&mut node.walk()) {
            if child.kind() == "identifier" {
                let name = node_text(child, source);
                if name == function_name {
                    return Some(node);
                }
                break;
            }
        }
    }

    // Check for arrow functions in variable declarations (TS/JS pattern):
    // lexical_declaration / variable_declaration -> variable_declarator -> name + value(arrow_function)
    if matches!(node.kind(), "lexical_declaration" | "variable_declaration") {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "variable_declarator" {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let var_name = node_text(name_node, source);
                    if var_name == function_name {
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

    // Elixir: def/defp are `call` nodes where the first child identifier is "def"/"defp"
    // and the function name is in the arguments
    if node.kind() == "call" && func_kinds.contains(&"call") {
        for child in node.children(&mut node.walk()) {
            if child.kind() == "identifier" {
                let text = node_text(child, source);
                if text == "def" || text == "defp" {
                    if let Some(args) = child.next_sibling() {
                        if args.kind() == "arguments" || args.kind() == "call" {
                            if let Some(name_node) = args.child(0) {
                                let fname = if name_node.kind() == "call" {
                                    name_node
                                        .child(0)
                                        .map(|n| node_text(n, source))
                                        .unwrap_or("")
                                } else {
                                    node_text(name_node, source)
                                };
                                if fname == function_name {
                                    return Some(node);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // OCaml: value_definition -> let_binding -> pattern field contains the function name
    if node.kind() == "value_definition" {
        for child in node.children(&mut node.walk()) {
            if child.kind() == "let_binding" {
                if let Some(pattern_node) = child.child_by_field_name("pattern") {
                    let name = node_text(pattern_node, source);
                    if name == function_name {
                        return Some(node);
                    }
                }
            }
        }
    }

    // Recurse into children
    for child in node.children(&mut node.walk()) {
        if let Some(found) = find_function_recursive(child, source, function_name, func_kinds) {
            return Some(found);
        }
    }

    None
}

/// Recursively extract function name from C/C++ nested declarator chain
fn extract_c_declarator_name_explain(declarator: Node, source: &[u8]) -> Option<String> {
    match declarator.kind() {
        "identifier" | "field_identifier" => {
            let name = node_text(declarator, source).to_string();
            if !name.is_empty() {
                Some(name)
            } else {
                None
            }
        }
        "function_declarator"
        | "pointer_declarator"
        | "reference_declarator"
        | "parenthesized_declarator" => declarator
            .child_by_field_name("declarator")
            .and_then(|inner| extract_c_declarator_name_explain(inner, source)),
        _ => None,
    }
}

// =============================================================================
// Signature Extraction
// =============================================================================

/// Extract signature information from a function node
fn extract_signature(func_node: Node, source: &[u8], language: Language) -> SignatureInfo {
    let mut sig = SignatureInfo::new();

    // Check if async (language-specific)
    sig.is_async = match language {
        Language::Python => func_node.kind() == "async_function_definition",
        Language::TypeScript | Language::JavaScript => {
            // Check for async modifier
            let mut is_async = false;
            for child in func_node.children(&mut func_node.walk()) {
                if child.kind() == "async" {
                    is_async = true;
                    break;
                }
            }
            is_async
        }
        Language::Rust => {
            // Check for async keyword
            node_text(func_node, source).contains("async")
        }
        _ => false,
    };

    // Extract parameters
    if let Some(params_node) = func_node.child_by_field_name("parameters") {
        sig.params = extract_params(params_node, source);
    }

    // Extract return type
    if let Some(return_node) = func_node.child_by_field_name("return_type") {
        sig.return_type = Some(node_text(return_node, source).to_string());
    }

    // Extract decorators (look for decorated_definition parent or decorator children)
    sig.decorators = extract_decorators(func_node, source);

    // Extract docstring
    sig.docstring = extract_docstring(func_node, source);

    sig
}

/// Extract parameters from a parameters node
fn extract_params(params_node: Node, source: &[u8]) -> Vec<ParamInfo> {
    let mut params = Vec::new();

    for child in params_node.children(&mut params_node.walk()) {
        match child.kind() {
            "identifier" => {
                // Simple parameter without annotation
                let name = node_text(child, source);
                if name != "self" && name != "cls" {
                    params.push(ParamInfo::new(name));
                }
            }
            "typed_parameter" | "typed_default_parameter" => {
                // Parameter with type annotation
                let mut param = ParamInfo::new("");
                for part in child.children(&mut child.walk()) {
                    match part.kind() {
                        "identifier" => {
                            let name = node_text(part, source);
                            if name != "self" && name != "cls" && param.name.is_empty() {
                                param.name = name.to_string();
                            }
                        }
                        "type" => {
                            param.type_hint = Some(node_text(part, source).to_string());
                        }
                        _ => {}
                    }
                }
                // Only add if we got a name
                if !param.name.is_empty() {
                    params.push(param);
                }
            }
            "default_parameter" => {
                // Parameter with default value
                let mut param = ParamInfo::new("");
                let mut got_name = false;
                for part in child.children(&mut child.walk()) {
                    if part.kind() == "identifier" && !got_name {
                        let name = node_text(part, source);
                        if name != "self" && name != "cls" {
                            param.name = name.to_string();
                            got_name = true;
                        }
                    } else if got_name && param.default.is_none() && part.kind() != "=" {
                        param.default = Some(node_text(part, source).to_string());
                    }
                }
                if !param.name.is_empty() {
                    params.push(param);
                }
            }
            _ => {}
        }
    }

    params
}

/// Extract decorators
fn extract_decorators(func_node: Node, source: &[u8]) -> Vec<String> {
    let mut decorators = Vec::new();

    // Check if parent is decorated_definition
    if let Some(parent) = func_node.parent() {
        if parent.kind() == "decorated_definition" {
            for child in parent.children(&mut parent.walk()) {
                if child.kind() == "decorator" {
                    let text = node_text(child, source);
                    decorators.push(text.trim_start_matches('@').to_string());
                }
            }
        }
    }

    decorators
}

/// Extract docstring from function body
fn extract_docstring(func_node: Node, source: &[u8]) -> Option<String> {
    // Look for the function body (block)
    if let Some(body) = func_node.child_by_field_name("body") {
        // First statement in body might be a docstring
        if let Some(first_stmt) = body.child(0) {
            if first_stmt.kind() == "expression_statement" {
                if let Some(expr) = first_stmt.child(0) {
                    if expr.kind() == "string" {
                        let text = node_text(expr, source);
                        // Remove quotes
                        let cleaned = text
                            .trim_start_matches("\"\"\"")
                            .trim_start_matches("'''")
                            .trim_start_matches('"')
                            .trim_start_matches('\'')
                            .trim_end_matches("\"\"\"")
                            .trim_end_matches("'''")
                            .trim_end_matches('"')
                            .trim_end_matches('\'')
                            .trim();
                        return Some(cleaned.to_string());
                    }
                }
            }
        }
    }
    None
}

// =============================================================================
// Purity Analysis
// =============================================================================

/// Analyze purity of a function
fn analyze_purity(func_node: Node, source: &[u8]) -> PurityInfo {
    let mut effects = Vec::new();
    let mut has_unknown_calls = false;
    let mut has_any_calls = false;

    analyze_purity_recursive(
        func_node,
        source,
        &mut effects,
        &mut has_unknown_calls,
        &mut has_any_calls,
    );

    if !effects.is_empty() {
        // Has side effects -> impure
        PurityInfo::impure(effects)
    } else if has_unknown_calls {
        // No known side effects, but calls unknown functions -> unknown
        PurityInfo::unknown().with_confidence("medium")
    } else if has_any_calls {
        // All calls resolved to known-pure builtins -> pure
        PurityInfo::pure()
    } else {
        // No calls detected at all (empty body or pure computation like a+b).
        // Absence of evidence is not evidence of purity — classify as unknown
        // with low confidence since we have nothing to base a purity claim on.
        PurityInfo::unknown().with_confidence("low")
    }
}

fn analyze_purity_recursive(
    node: Node,
    source: &[u8],
    effects: &mut Vec<String>,
    has_unknown_calls: &mut bool,
    has_any_calls: &mut bool,
) {
    match node.kind() {
        "global_statement" | "nonlocal_statement" => {
            if !effects.contains(&"global_write".to_string()) {
                effects.push("global_write".to_string());
            }
        }
        "assignment" | "augmented_assignment" => {
            // Check for attribute writes (self.x = ...)
            if let Some(left) = node.child_by_field_name("left") {
                if left.kind() == "attribute"
                    && !effects.contains(&"attribute_write".to_string())
                {
                    effects.push("attribute_write".to_string());
                }
            }
        }
        "call" => {
            *has_any_calls = true;
            let call_name = extract_call_name(node, source);
            if let Some(name) = &call_name {
                // Check for I/O operations
                for &io_op in IO_OPERATIONS {
                    if name == io_op || name.ends_with(&format!(".{}", io_op)) {
                        if !effects.contains(&"io".to_string()) {
                            effects.push("io".to_string());
                        }
                        return;
                    }
                }

                // Check for impure calls
                for &impure in IMPURE_CALLS {
                    if name == impure || name.ends_with(impure) {
                        if !effects.contains(&"io".to_string()) {
                            effects.push("io".to_string());
                        }
                        return;
                    }
                }

                // Check for collection mutations
                let method_name = name.split('.').next_back().unwrap_or(name);
                for &mutation in COLLECTION_MUTATIONS {
                    if method_name == mutation {
                        if !effects.contains(&"collection_modify".to_string()) {
                            effects.push("collection_modify".to_string());
                        }
                        return;
                    }
                }

                // Check if it's a known pure builtin
                let base = name.split('.').next_back().unwrap_or(name);
                if !PURE_BUILTINS.contains(&name.as_str()) && !PURE_BUILTINS.contains(&base) {
                    *has_unknown_calls = true;
                }
            }
        }
        _ => {}
    }

    // Recurse into children
    for child in node.children(&mut node.walk()) {
        analyze_purity_recursive(child, source, effects, has_unknown_calls, has_any_calls);
    }
}

/// Extract call name from a call node
fn extract_call_name(node: Node, source: &[u8]) -> Option<String> {
    if let Some(func) = node.child_by_field_name("function") {
        return Some(extract_name_from_expr(func, source));
    }

    for child in node.children(&mut node.walk()) {
        match child.kind() {
            "identifier" => return Some(node_text(child, source).to_string()),
            "attribute" => return Some(extract_name_from_expr(child, source)),
            _ => continue,
        }
    }
    None
}

/// Extract a dotted name from an expression
fn extract_name_from_expr(node: Node, source: &[u8]) -> String {
    match node.kind() {
        "identifier" => node_text(node, source).to_string(),
        "attribute" => {
            let mut parts = Vec::new();
            let mut current = node;

            loop {
                if let Some(attr) = current.child_by_field_name("attribute") {
                    parts.push(node_text(attr, source).to_string());
                }

                if let Some(obj) = current.child_by_field_name("object") {
                    if obj.kind() == "attribute" {
                        current = obj;
                    } else if obj.kind() == "identifier" {
                        parts.push(node_text(obj, source).to_string());
                        break;
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }

            parts.reverse();
            parts.join(".")
        }
        _ => node_text(node, source).to_string(),
    }
}

// =============================================================================
// Complexity Analysis
// =============================================================================

/// Compute complexity metrics for a function
fn compute_complexity(func_node: Node) -> ComplexityInfo {
    let mut cyclomatic = 1; // Base complexity
    let mut num_blocks = 1;
    let mut num_edges = 0;
    let mut has_loops = false;

    count_complexity_recursive(
        func_node,
        &mut cyclomatic,
        &mut num_blocks,
        &mut num_edges,
        &mut has_loops,
    );

    ComplexityInfo::new(cyclomatic, num_blocks, num_edges, has_loops)
}

fn count_complexity_recursive(
    node: Node,
    cyclomatic: &mut u32,
    num_blocks: &mut u32,
    num_edges: &mut u32,
    has_loops: &mut bool,
) {
    match node.kind() {
        "if_statement" | "elif_clause" => {
            *cyclomatic += 1;
            *num_blocks += 1;
            *num_edges += 2;
        }
        "for_statement" | "while_statement" => {
            *cyclomatic += 1;
            *num_blocks += 1;
            *num_edges += 2;
            *has_loops = true;
        }
        "try_statement" => {
            *cyclomatic += 1;
            *num_blocks += 1;
            *num_edges += 1;
        }
        "except_clause" => {
            *cyclomatic += 1;
            *num_blocks += 1;
            *num_edges += 1;
        }
        "and_operator" | "or_operator" => {
            *cyclomatic += 1;
        }
        "conditional_expression" => {
            // Ternary: x if cond else y
            *cyclomatic += 1;
            *num_edges += 1;
        }
        "list_comprehension"
        | "set_comprehension"
        | "dictionary_comprehension"
        | "generator_expression" => {
            *cyclomatic += 1;
            *has_loops = true;
        }
        _ => {}
    }

    for child in node.children(&mut node.walk()) {
        count_complexity_recursive(child, cyclomatic, num_blocks, num_edges, has_loops);
    }
}

// =============================================================================
// Call Graph Analysis
// =============================================================================

/// Find callees (functions called by this function)
fn find_callees(
    func_node: Node,
    source: &[u8],
    file_path: &str,
    local_functions: &HashSet<String>,
) -> Vec<CallInfo> {
    let mut callees = Vec::new();
    find_callees_recursive(func_node, source, file_path, local_functions, &mut callees);
    callees
}

fn find_callees_recursive(
    node: Node,
    source: &[u8],
    file_path: &str,
    local_functions: &HashSet<String>,
    callees: &mut Vec<CallInfo>,
) {
    if node.kind() == "call" {
        if let Some(name) = extract_call_name(node, source) {
            // Get base name for local function check
            let base_name = name.split('.').next().unwrap_or(&name);

            // Add if it's a local function or a known call
            let file = if local_functions.contains(base_name) {
                file_path.to_string()
            } else {
                "<external>".to_string()
            };

            // Avoid duplicates
            if !callees.iter().any(|c| c.name == name) {
                callees.push(CallInfo::new(name, file, get_line_number(node)));
            }
        }
    }

    for child in node.children(&mut node.walk()) {
        find_callees_recursive(child, source, file_path, local_functions, callees);
    }
}

/// Find callers (functions that call this function) - searches the entire file
fn find_callers(
    root: Node,
    source: &[u8],
    target_function: &str,
    file_path: &str,
    func_kinds: &[&str],
) -> Vec<CallInfo> {
    let mut callers = Vec::new();
    find_callers_in_file(
        root,
        source,
        target_function,
        file_path,
        &mut callers,
        None,
        func_kinds,
    );
    callers
}

fn find_callers_in_file(
    node: Node,
    source: &[u8],
    target_function: &str,
    file_path: &str,
    callers: &mut Vec<CallInfo>,
    current_function: Option<&str>,
    func_kinds: &[&str],
) {
    if func_kinds.contains(&node.kind()) {
        // Get this function's name
        let mut func_name = None;

        // Try field name first
        if let Some(name_node) = node.child_by_field_name("name") {
            func_name = Some(node_text(name_node, source));
        } else {
            // Fallback: search for identifier child
            for child in node.children(&mut node.walk()) {
                if child.kind() == "identifier" {
                    func_name = Some(node_text(child, source));
                    break;
                }
            }
        }

        // Recurse with this function as current
        for child in node.children(&mut node.walk()) {
            find_callers_in_file(
                child,
                source,
                target_function,
                file_path,
                callers,
                func_name,
                func_kinds,
            );
        }
        return;
    } else if node.kind() == "call" {
        if let Some(name) = extract_call_name(node, source) {
            // Check if this call is to our target function
            let base = name.split('.').next_back().unwrap_or(&name);
            if base == target_function || name == target_function {
                if let Some(caller_name) = current_function {
                    // Avoid duplicates and self-references
                    if caller_name != target_function
                        && !callers.iter().any(|c| c.name == caller_name)
                    {
                        callers.push(CallInfo::new(caller_name, file_path, get_line_number(node)));
                    }
                }
            }
        }
    }

    for child in node.children(&mut node.walk()) {
        find_callers_in_file(
            child,
            source,
            target_function,
            file_path,
            callers,
            current_function,
            func_kinds,
        );
    }
}

/// Collect all function names in a file
fn collect_function_names(root: Node, source: &[u8], func_kinds: &[&str]) -> HashSet<String> {
    let mut names = HashSet::new();
    collect_function_names_recursive(root, source, &mut names, func_kinds);
    names
}

fn collect_function_names_recursive(
    node: Node,
    source: &[u8],
    names: &mut HashSet<String>,
    func_kinds: &[&str],
) {
    if func_kinds.contains(&node.kind()) {
        // Try field name first
        if let Some(name_node) = node.child_by_field_name("name") {
            names.insert(node_text(name_node, source).to_string());
        } else {
            // Fallback: search for identifier child
            for child in node.children(&mut node.walk()) {
                if child.kind() == "identifier" {
                    names.insert(node_text(child, source).to_string());
                    break;
                }
            }
        }
    }

    for child in node.children(&mut node.walk()) {
        collect_function_names_recursive(child, source, names, func_kinds);
    }
}

// =============================================================================
// Text Formatting
// =============================================================================

/// Format an ExplainReport as human-readable text
fn format_explain_text(report: &ExplainReport) -> String {
    let mut lines = Vec::new();

    lines.push(format!("Function: {}", report.function_name));
    lines.push(format!("File: {}", report.file));
    lines.push(format!("Lines: {}-{}", report.line_start, report.line_end));
    lines.push(format!("Language: {}", report.language));
    lines.push(String::new());

    // Signature
    lines.push("Signature:".to_string());
    if report.signature.is_async {
        lines.push("  async: yes".to_string());
    }
    lines.push(format!("  Parameters: {}", report.signature.params.len()));
    for param in &report.signature.params {
        let type_str = param.type_hint.as_deref().unwrap_or("untyped");
        lines.push(format!("    - {}: {}", param.name, type_str));
    }
    if let Some(ref ret) = report.signature.return_type {
        lines.push(format!("  Returns: {}", ret));
    }
    if !report.signature.decorators.is_empty() {
        lines.push(format!(
            "  Decorators: {}",
            report.signature.decorators.join(", ")
        ));
    }
    if let Some(ref doc) = report.signature.docstring {
        let preview = if doc.len() > 100 {
            format!("{}...", &doc[..100])
        } else {
            doc.clone()
        };
        lines.push(format!("  Docstring: {}", preview));
    }
    lines.push(String::new());

    // Purity
    lines.push("Purity:".to_string());
    lines.push(format!(
        "  Classification: {}",
        report.purity.classification
    ));
    lines.push(format!("  Confidence: {}", report.purity.confidence));
    if !report.purity.effects.is_empty() {
        lines.push(format!("  Effects: {}", report.purity.effects.join(", ")));
    }
    lines.push(String::new());

    // Complexity
    if let Some(ref cx) = report.complexity {
        lines.push("Complexity:".to_string());
        lines.push(format!("  Cyclomatic: {}", cx.cyclomatic));
        lines.push(format!("  Blocks: {}", cx.num_blocks));
        lines.push(format!("  Edges: {}", cx.num_edges));
        lines.push(format!("  Has loops: {}", cx.has_loops));
        lines.push(String::new());
    }

    // Callers
    if !report.callers.is_empty() {
        lines.push(format!("Callers ({}):", report.callers.len()));
        for caller in &report.callers {
            lines.push(format!(
                "  - {} ({}:{})",
                caller.name, caller.file, caller.line
            ));
        }
        lines.push(String::new());
    }

    // Callees
    if !report.callees.is_empty() {
        lines.push(format!("Callees ({}):", report.callees.len()));
        for callee in &report.callees {
            lines.push(format!(
                "  - {} ({}:{})",
                callee.name, callee.file, callee.line
            ));
        }
    }

    lines.join("\n")
}

// =============================================================================
// Entry Point
// =============================================================================

impl ExplainArgs {
    /// Run the explain command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        writer.progress(&format!(
            "Analyzing function {} in {}...",
            self.function,
            self.file.display()
        ));

        // Check file exists
        if !self.file.exists() {
            return Err(RemainingError::file_not_found(&self.file).into());
        }

        // Detect language from file extension
        let language = Language::from_path(&self.file)
            .ok_or_else(|| RemainingError::parse_error(&self.file, "Unsupported language"))?;

        // Get function node kinds for this language
        let func_kinds = get_function_node_kinds(language);

        // Read source
        let source = std::fs::read_to_string(&self.file)
            .map_err(|e| RemainingError::parse_error(&self.file, e.to_string()))?;
        let source_bytes = source.as_bytes();

        // Parse with tree-sitter
        let mut parser = get_parser(language)?;
        let tree = parser
            .parse(&source, None)
            .ok_or_else(|| RemainingError::parse_error(&self.file, "Failed to parse file"))?;

        let root = tree.root_node();

        // Find the function
        let func_node = find_function_node(root, source_bytes, &self.function, func_kinds)
            .ok_or_else(|| RemainingError::symbol_not_found(&self.function, &self.file))?;

        // Get file path string
        let file_path = self.file.to_string_lossy().to_string();

        // Get language name for report
        let language_name = match language {
            Language::Python => "python",
            Language::TypeScript => "typescript",
            Language::JavaScript => "javascript",
            Language::Go => "go",
            Language::Rust => "rust",
            Language::Java => "java",
            Language::C => "c",
            Language::Cpp => "cpp",
            Language::CSharp => "csharp",
            Language::Kotlin => "kotlin",
            Language::Scala => "scala",
            Language::Php => "php",
            Language::Ruby => "ruby",
            Language::Lua => "lua",
            Language::Luau => "luau",
            Language::Elixir => "elixir",
            Language::Ocaml => "ocaml",
            Language::Swift => "swift",
        };

        // Build report
        let mut report = ExplainReport::new(
            &self.function,
            &file_path,
            get_line_number(func_node),
            get_end_line_number(func_node),
            language_name,
        );

        // Extract signature
        report.signature = extract_signature(func_node, source_bytes, language);

        // Analyze purity
        report.purity = analyze_purity(func_node, source_bytes);

        // Compute complexity
        report.complexity = Some(compute_complexity(func_node));

        // Collect local function names for call graph analysis
        let local_functions = collect_function_names(root, source_bytes, func_kinds);

        // Find callees
        report.callees = find_callees(func_node, source_bytes, &file_path, &local_functions);

        // Find callers
        report.callers = find_callers(root, source_bytes, &self.function, &file_path, func_kinds);

        // Output based on format
        if writer.is_text() {
            let text = format_explain_text(&report);
            writer.write_text(&text)?;
        } else {
            writer.write(&report)?;
        }

        // Write to output file if specified
        if let Some(ref output_path) = self.output {
            let output_str = if format == OutputFormat::Text {
                format_explain_text(&report)
            } else {
                serde_json::to_string_pretty(&report)?
            };
            std::fs::write(output_path, &output_str)?;
        }

        Ok(())
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    

    const SAMPLE_CODE: &str = r#"
def calculate_total(items: list[dict], tax_rate: float = 0.1) -> float:
    """Calculate total price with tax.

    Args:
        items: List of items with 'price' key
        tax_rate: Tax rate as decimal (default 10%)

    Returns:
        Total price including tax
    """
    subtotal = sum(item['price'] for item in items)
    return subtotal * (1 + tax_rate)

def helper_function(x):
    return x * 2

def main():
    items = [{'price': 10}, {'price': 20}]
    total = calculate_total(items)
    doubled = helper_function(total)
    print(doubled)
"#;

    #[test]
    fn test_find_function() {
        let language = Language::Python;
        let func_kinds = get_function_node_kinds(language);
        let mut parser = get_parser(language).unwrap();
        let tree = parser.parse(SAMPLE_CODE, None).unwrap();
        let root = tree.root_node();

        let func = find_function_node(root, SAMPLE_CODE.as_bytes(), "calculate_total", func_kinds);
        assert!(func.is_some());

        let func = find_function_node(root, SAMPLE_CODE.as_bytes(), "nonexistent", func_kinds);
        assert!(func.is_none());
    }

    #[test]
    fn test_extract_signature() {
        let language = Language::Python;
        let func_kinds = get_function_node_kinds(language);
        let mut parser = get_parser(language).unwrap();
        let tree = parser.parse(SAMPLE_CODE, None).unwrap();
        let root = tree.root_node();

        let func = find_function_node(root, SAMPLE_CODE.as_bytes(), "calculate_total", func_kinds)
            .unwrap();
        let sig = extract_signature(func, SAMPLE_CODE.as_bytes(), language);

        assert_eq!(sig.params.len(), 2);
        assert_eq!(sig.params[0].name, "items");
        assert_eq!(sig.params[1].name, "tax_rate");
        assert!(sig.return_type.is_some());
        assert!(sig.docstring.is_some());
    }

    #[test]
    fn test_purity_analysis() {
        let language = Language::Python;
        let func_kinds = get_function_node_kinds(language);
        let mut parser = get_parser(language).unwrap();
        let tree = parser.parse(SAMPLE_CODE, None).unwrap();
        let root = tree.root_node();

        // calculate_total should be pure
        let func = find_function_node(root, SAMPLE_CODE.as_bytes(), "calculate_total", func_kinds)
            .unwrap();
        let purity = analyze_purity(func, SAMPLE_CODE.as_bytes());
        assert_eq!(purity.classification, "pure");

        // main calls print, so impure
        let func = find_function_node(root, SAMPLE_CODE.as_bytes(), "main", func_kinds).unwrap();
        let purity = analyze_purity(func, SAMPLE_CODE.as_bytes());
        assert_eq!(purity.classification, "impure");
        assert!(purity.effects.contains(&"io".to_string()));
    }

    #[test]
    fn test_complexity_analysis() {
        let code = r#"
def complex_func(x, y):
    if x > 0:
        if y > 0:
            return x + y
        else:
            return x
    else:
        for i in range(10):
            x += i
        return x
"#;
        let language = Language::Python;
        let func_kinds = get_function_node_kinds(language);
        let mut parser = get_parser(language).unwrap();
        let tree = parser.parse(code, None).unwrap();
        let root = tree.root_node();

        let func = find_function_node(root, code.as_bytes(), "complex_func", func_kinds).unwrap();
        let cx = compute_complexity(func);

        assert!(cx.cyclomatic > 1);
        assert!(cx.has_loops);
    }

    #[test]
    fn test_find_callees() {
        let language = Language::Python;
        let func_kinds = get_function_node_kinds(language);
        let mut parser = get_parser(language).unwrap();
        let tree = parser.parse(SAMPLE_CODE, None).unwrap();
        let root = tree.root_node();

        let local_funcs = collect_function_names(root, SAMPLE_CODE.as_bytes(), func_kinds);
        let func = find_function_node(root, SAMPLE_CODE.as_bytes(), "main", func_kinds).unwrap();
        let callees = find_callees(func, SAMPLE_CODE.as_bytes(), "test.py", &local_funcs);

        assert!(callees.iter().any(|c| c.name == "calculate_total"));
        assert!(callees.iter().any(|c| c.name == "helper_function"));
    }

    #[test]
    fn test_find_callers() {
        let language = Language::Python;
        let func_kinds = get_function_node_kinds(language);
        let mut parser = get_parser(language).unwrap();
        let tree = parser.parse(SAMPLE_CODE, None).unwrap();
        let root = tree.root_node();

        let callers = find_callers(
            root,
            SAMPLE_CODE.as_bytes(),
            "calculate_total",
            "test.py",
            func_kinds,
        );
        assert!(callers.iter().any(|c| c.name == "main"));
    }

    #[test]
    fn test_find_ts_arrow_function() {
        let ts_source = r#"
const getDuration = (start: Date, end: Date): number => {
    return end.getTime() - start.getTime();
};

function regularFunc(x: number): number {
    return x * 2;
}

export const processItems = (items: string[]) => {
    return items.map(i => i.trim());
};
"#;
        let language = Language::TypeScript;
        let func_kinds = get_function_node_kinds(language);
        let mut parser = get_parser(language).unwrap();
        let tree = parser.parse(ts_source, None).unwrap();
        let root = tree.root_node();

        // Regular function should always work
        let regular = find_function_node(root, ts_source.as_bytes(), "regularFunc", func_kinds);
        assert!(regular.is_some(), "Should find regular TS function");

        // Arrow function assigned to const should also work
        let arrow = find_function_node(root, ts_source.as_bytes(), "getDuration", func_kinds);
        assert!(
            arrow.is_some(),
            "Should find TS arrow function 'getDuration'"
        );

        // Exported arrow function should also work
        let exported = find_function_node(root, ts_source.as_bytes(), "processItems", func_kinds);
        assert!(
            exported.is_some(),
            "Should find exported TS arrow function 'processItems'"
        );
    }

    // =========================================================================
    // Bug: analyze_purity returns "pure" when it should return "unknown"
    // =========================================================================

    /// A function with no function body content (empty/pass) should classify
    /// as "unknown", not "pure". We have no evidence of purity -- the analysis
    /// simply found nothing.
    #[test]
    fn test_empty_function_is_unknown_not_pure() {
        let source = r#"
def empty_func():
    pass
"#;
        let language = Language::Python;
        let func_kinds = get_function_node_kinds(language);
        let mut parser = get_parser(language).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let root = tree.root_node();

        let func_node = find_function_node(root, source.as_bytes(), "empty_func", func_kinds);
        assert!(func_node.is_some(), "Should find empty_func");

        let purity = analyze_purity(func_node.unwrap(), source.as_bytes());

        // The buggy code returns "pure" because no effects and no unknown calls.
        // But "pass" means we found nothing -- not that we proved purity.
        // A truly empty function (just `pass`) has no evidence to support "pure".
        assert_ne!(
            purity.classification, "pure",
            "A function with only `pass` (no calls, no computation) should NOT be classified as \
             'pure' with high confidence. We have no evidence to support a purity claim. \
             Got classification='{}', confidence='{}'. Expected 'unknown'.",
            purity.classification, purity.confidence
        );
    }

    /// A function that calls other user-defined functions (not builtins, not IO)
    /// where those calls are unresolved should classify as "unknown", not "pure".
    ///
    /// The bug: when a call doesn't match IO_OPERATIONS, IMPURE_CALLS,
    /// COLLECTION_MUTATIONS, or PURE_BUILTINS, it sets has_unknown_calls=true.
    /// This case is actually handled correctly for unknown calls, BUT if the
    /// call name happens to match a PURE_BUILTIN substring, it incorrectly
    /// passes as pure. This test verifies the general "unknown calls" path works.
    #[test]
    fn test_function_with_unknown_calls_is_unknown() {
        let source = r#"
def my_func(x):
    result = compute_something(x)
    return transform_result(result)
"#;
        let language = Language::Python;
        let func_kinds = get_function_node_kinds(language);
        let mut parser = get_parser(language).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let root = tree.root_node();

        let func_node = find_function_node(root, source.as_bytes(), "my_func", func_kinds);
        assert!(func_node.is_some(), "Should find my_func");

        let purity = analyze_purity(func_node.unwrap(), source.as_bytes());

        // compute_something and transform_result are NOT in PURE_BUILTINS,
        // so has_unknown_calls should be true -> classification = "unknown"
        assert_eq!(
            purity.classification, "unknown",
            "Function calling unknown user functions should be 'unknown', got '{}'",
            purity.classification
        );
        assert_ne!(
            purity.confidence, "high",
            "Unknown classification should not have high confidence, got '{}'",
            purity.confidence
        );
    }

    /// A function that ONLY calls known-pure builtins should classify as "pure".
    /// This is the legitimate pure case.
    #[test]
    fn test_only_pure_builtins_is_pure() {
        let source = r#"
def pure_func(items):
    return len(items) + sum(items)
"#;
        let language = Language::Python;
        let func_kinds = get_function_node_kinds(language);
        let mut parser = get_parser(language).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let root = tree.root_node();

        let func_node = find_function_node(root, source.as_bytes(), "pure_func", func_kinds);
        assert!(func_node.is_some(), "Should find pure_func");

        let purity = analyze_purity(func_node.unwrap(), source.as_bytes());

        assert_eq!(
            purity.classification, "pure",
            "Function calling only pure builtins (len, sum) should be 'pure', got '{}'",
            purity.classification
        );
        assert_eq!(
            purity.confidence, "high",
            "Pure classification should have high confidence"
        );
    }

    /// A function with IO operations should classify as "impure".
    #[test]
    fn test_io_operations_is_impure() {
        let source = r#"
def impure_func(msg):
    print(msg)
    return True
"#;
        let language = Language::Python;
        let func_kinds = get_function_node_kinds(language);
        let mut parser = get_parser(language).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let root = tree.root_node();

        let func_node = find_function_node(root, source.as_bytes(), "impure_func", func_kinds);
        assert!(func_node.is_some(), "Should find impure_func");

        let purity = analyze_purity(func_node.unwrap(), source.as_bytes());

        assert_eq!(
            purity.classification, "impure",
            "Function with print() should be 'impure', got '{}'",
            purity.classification
        );
        assert_eq!(
            purity.confidence, "high",
            "Impure classification should have high confidence"
        );
        assert!(
            purity.effects.contains(&"io".to_string()),
            "Effects should contain 'io', got {:?}",
            purity.effects
        );
    }

    /// A function with only arithmetic (no calls at all) should be "unknown"
    /// because we have no positive evidence of purity -- the analysis simply
    /// didn't find any calls to classify.
    #[test]
    fn test_no_calls_arithmetic_only_is_unknown() {
        let source = r#"
def add(a, b):
    return a + b
"#;
        let language = Language::Python;
        let func_kinds = get_function_node_kinds(language);
        let mut parser = get_parser(language).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let root = tree.root_node();

        let func_node = find_function_node(root, source.as_bytes(), "add", func_kinds);
        assert!(func_node.is_some(), "Should find add");

        let purity = analyze_purity(func_node.unwrap(), source.as_bytes());

        // The bug: analyze_purity returns "pure" because no effects and
        // no unknown calls. But we have no positive evidence -- we just
        // didn't find any calls. The correct answer is "unknown" with
        // low confidence, or at minimum not "pure/high".
        assert_ne!(
            purity.classification, "pure",
            "A simple arithmetic function with no calls should NOT confidently be 'pure'. \
             The analysis found no calls to evaluate -- absence of evidence is not evidence \
             of purity. Got classification='{}', confidence='{}'. \
             Expected 'unknown' since no calls were analyzed.",
            purity.classification, purity.confidence
        );
    }
}
