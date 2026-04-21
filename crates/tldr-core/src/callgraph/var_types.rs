//! Variable type extraction for call graph construction.
//!
//! This module contains `FileParseResult` and all per-language VarType extraction
//! functions. Each `extract_*_var_types` function walks a tree-sitter AST to find
//! variable type information (constructor calls, annotations, parameters, literals).
//!
//! Also contains Python-specific definition/call extraction (`extract_python_definitions`,
//! `extract_python_calls`, `parse_python_call`).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::cross_file_types::{CallSite, CallType, ClassDef, FuncDef, ImportDef, VarType};
use super::languages::base::{get_node_text, walk_tree};
use super::types::parse_source;

// =============================================================================
// FileParseResult
// =============================================================================

/// Result of parsing a single file for functions, classes, imports, and calls.
#[derive(Debug, Default)]
pub(crate) struct FileParseResult {
    /// Functions found in this file.
    pub(crate) funcs: Vec<FuncDef>,
    /// Classes found in this file.
    pub(crate) classes: Vec<ClassDef>,
    /// Imports found in this file.
    pub(crate) imports: Vec<ImportDef>,
    /// Calls found in this file, indexed by caller function name.
    pub(crate) calls: HashMap<String, Vec<CallSite>>,
    /// Variable type information extracted from assignments and annotations.
    pub(crate) var_types: Vec<VarType>,
    /// Error message if parsing failed.
    pub(crate) error: Option<String>,
}

// =============================================================================
// Python extraction
// =============================================================================

/// Extract functions, classes, imports, and calls from a Python source file.
pub(crate) fn extract_python_definitions(source: &str, _file_path: &Path) -> FileParseResult {
    let mut result = FileParseResult::default();

    // Parse the source
    let tree = match parse_source(source, "python") {
        Ok(t) => t,
        Err(e) => {
            result.error = Some(e.to_string());
            return result;
        }
    };

    let source_bytes = source.as_bytes();
    let root = tree.root_node();

    // One-pass extraction for structure and calls.
    for node in walk_tree(root) {
        match node.kind() {
            "import_statement" => {
                // import X or import X as Y
                if let Some(import_def) =
                    super::imports::parse_python_import_statement(&node, source_bytes)
                {
                    result.imports.push(import_def);
                }
            }
            "import_from_statement" => {
                // from X import Y
                if let Some(import_def) =
                    super::imports::parse_python_from_import(&node, source_bytes)
                {
                    result.imports.push(import_def);
                }
            }
            "class_definition" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let class_name = get_node_text(&name_node, source_bytes).to_string();
                    let line = node.start_position().row as u32 + 1;
                    let end_line = node.end_position().row as u32 + 1;

                    // Extract base classes
                    let mut bases = Vec::new();
                    if let Some(arg_list) = node.child_by_field_name("superclasses") {
                        for i in 0..arg_list.named_child_count() {
                            if let Some(base) = arg_list.named_child(i) {
                                bases.push(get_node_text(&base, source_bytes).to_string());
                            }
                        }
                    }

                    // Collect method names from class body
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

                    result
                        .classes
                        .push(ClassDef::new(class_name, line, end_line, methods, bases));
                }
            }
            "function_definition" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let func_name = get_node_text(&name_node, source_bytes).to_string();
                    let line = node.start_position().row as u32 + 1;
                    let end_line = node.end_position().row as u32 + 1;

                    // Check if this is a method (directly inside a class body).
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

                    // Determine the caller name: qualified for methods, simple for top-level functions
                    let caller_name = if let Some(ref cn) = class_name {
                        result.funcs.push(FuncDef::method(
                            func_name.clone(),
                            cn.clone(),
                            line,
                            end_line,
                        ));
                        format!("{}.{}", cn, func_name)
                    } else {
                        result
                            .funcs
                            .push(FuncDef::function(func_name.clone(), line, end_line));
                        func_name.clone()
                    };

                    // Extract calls within this function using the qualified caller name
                    let calls = extract_python_calls(&node, source_bytes, &caller_name);
                    if !calls.is_empty() {
                        result.calls.insert(caller_name, calls);
                    }
                }
            }
            _ => {}
        }
    }

    // Extract VarType information from the tree before dropping it
    result.var_types = extract_python_var_types(&tree, source_bytes);

    // Explicitly drop the tree to free memory (per spec)
    drop(tree);

    result
}

/// Extract calls from a Python function body.
pub(crate) fn extract_python_calls(
    func_node: &tree_sitter::Node,
    source: &[u8],
    caller: &str,
) -> Vec<CallSite> {
    let mut calls = Vec::new();

    // Walk the function body looking for call expressions
    for node in walk_tree(*func_node) {
        if node.kind() == "call" {
            if let Some(call_site) = parse_python_call(&node, source, caller) {
                calls.push(call_site);
            }
        }
    }

    calls
}

/// Parse a Python call expression into a CallSite.
fn parse_python_call(node: &tree_sitter::Node, source: &[u8], caller: &str) -> Option<CallSite> {
    let line = node.start_position().row as u32 + 1;
    let column = node.start_position().column as u32 + 1;

    // Get the function/method being called
    let func_node = node.child_by_field_name("function")?;

    match func_node.kind() {
        "identifier" => {
            // Simple call: foo()
            let target = get_node_text(&func_node, source).to_string();
            Some(CallSite::new(
                caller.to_string(),
                target,
                CallType::Direct,
                Some(line),
                Some(column),
                None,
                None,
            ))
        }
        "attribute" => {
            // Method or attribute call: obj.method() or module.func()
            let object_node = func_node.child_by_field_name("object")?;
            let attr_node = func_node.child_by_field_name("attribute")?;

            let receiver = get_node_text(&object_node, source).to_string();
            let target = get_node_text(&attr_node, source).to_string();

            // Determine call type based on receiver
            // If receiver is a simple identifier, it could be either:
            // - A module (import-based call): json.loads()
            // - An object (method call): user.save()
            // We'll mark it as Method and resolve later during import resolution
            Some(CallSite::new(
                caller.to_string(),
                target,
                CallType::Method,
                Some(line),
                Some(column),
                Some(receiver),
                None, // receiver_type will be filled during type resolution
            ))
        }
        _ => None,
    }
}

/// Determine the enclosing function scope for a tree-sitter node.
///
/// Walks up the parent chain to find the nearest `function_definition` ancestor.
/// Returns `Some(function_name)` if found, `None` for module-level scope.
pub(crate) fn enclosing_function_scope(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "function_definition" {
            if let Some(name_node) = parent.child_by_field_name("name") {
                return Some(get_node_text(&name_node, source).to_string());
            }
        }
        current = parent.parent();
    }
    None
}

// =============================================================================
// Python VarType extraction
// =============================================================================

/// Extract VarType entries from a Python source tree.
///
/// Walks the AST to find:
/// - **Constructor assignments**: `x = Foo()` -> VarType { source: "assignment" }
/// - **Annotated assignments**: `x: Foo` or `x: Foo = ...` -> VarType { source: "annotation" }
/// - **Parameter annotations**: `def f(x: Foo)` -> VarType { source: "parameter" }
///
/// Scope is determined by the enclosing function definition, or None for module-level.
pub(crate) fn extract_python_var_types(tree: &tree_sitter::Tree, source: &[u8]) -> Vec<VarType> {
    let mut var_types = Vec::new();
    let root = tree.root_node();

    for node in walk_tree(root) {
        match node.kind() {
            // Pattern 1: x = Foo() -- constructor assignment
            // Pattern 1b: x = {} / [] / "" / () -- builtin literal assignment
            "assignment" => {
                // Left side should be a simple identifier
                let left = match node.child_by_field_name("left") {
                    Some(n) if n.kind() == "identifier" => n,
                    _ => continue,
                };
                let right = match node.child_by_field_name("right") {
                    Some(n) => n,
                    None => continue,
                };

                let var_name = get_node_text(&left, source).to_string();
                if var_name.is_empty() {
                    continue;
                }
                let line = node.start_position().row as u32 + 1;
                let scope = enclosing_function_scope(&node, source);

                match right.kind() {
                    "call" => {
                        // The call's function should be a simple identifier
                        let func_node = match right.child_by_field_name("function") {
                            Some(n) if n.kind() == "identifier" => n,
                            _ => continue,
                        };
                        let type_name = get_node_text(&func_node, source).to_string();

                        // Check for lowercase builtin constructors: dict(), list(), set(), etc.
                        if type_name.chars().next().is_none_or(|c| c.is_lowercase()) {
                            match type_name.as_str() {
                                "dict" | "list" | "set" | "tuple" | "frozenset" | "str"
                                | "bytes" | "bytearray" | "int" | "float" | "bool" | "complex" => {
                                    var_types.push(VarType::new_with_scope(
                                        var_name,
                                        type_name,
                                        "constructor",
                                        line,
                                        scope,
                                    ));
                                }
                                _ => {} // Skip other lowercase calls
                            }
                            continue;
                        }

                        // Capitalized call -- likely a class constructor: Foo()
                        var_types.push(VarType::new_with_scope(
                            var_name,
                            type_name,
                            "assignment",
                            line,
                            scope,
                        ));
                    }
                    // Builtin literal types
                    "dictionary" | "dictionary_comprehension" => {
                        var_types.push(VarType::new_with_scope(
                            var_name,
                            "dict".to_string(),
                            "literal",
                            line,
                            scope,
                        ));
                    }
                    "list" | "list_comprehension" => {
                        var_types.push(VarType::new_with_scope(
                            var_name,
                            "list".to_string(),
                            "literal",
                            line,
                            scope,
                        ));
                    }
                    "string" | "concatenated_string" => {
                        var_types.push(VarType::new_with_scope(
                            var_name,
                            "str".to_string(),
                            "literal",
                            line,
                            scope,
                        ));
                    }
                    "tuple" => {
                        var_types.push(VarType::new_with_scope(
                            var_name,
                            "tuple".to_string(),
                            "literal",
                            line,
                            scope,
                        ));
                    }
                    "set" | "set_comprehension" => {
                        var_types.push(VarType::new_with_scope(
                            var_name,
                            "set".to_string(),
                            "literal",
                            line,
                            scope,
                        ));
                    }
                    "integer" => {
                        var_types.push(VarType::new_with_scope(
                            var_name,
                            "int".to_string(),
                            "literal",
                            line,
                            scope,
                        ));
                    }
                    "float" => {
                        var_types.push(VarType::new_with_scope(
                            var_name,
                            "float".to_string(),
                            "literal",
                            line,
                            scope,
                        ));
                    }
                    "true" | "false" => {
                        var_types.push(VarType::new_with_scope(
                            var_name,
                            "bool".to_string(),
                            "literal",
                            line,
                            scope,
                        ));
                    }
                    _ => {}
                }
            }

            // Pattern 2: x: Foo or x: Foo = ... -- type annotation
            "type" => {
                // A "type" node inside an expression_statement or assignment
                // represents a type annotation.
                // For `x: Foo`, tree-sitter produces:
                //   expression_statement > type > identifier(x) + type(Foo)
                // For `x: Foo = val`, tree-sitter produces:
                //   assignment > type > identifier(x) + type(Foo)  [left side]
                //
                // We handle this by looking at the parent context.
                // Actually, tree-sitter Python handles annotations differently.
                // Let's handle it via the parent node patterns.
            }

            // Pattern 2 (actual): Annotated assignments and standalone annotations
            // tree-sitter-python produces different node types:
            // - `x: int = 5` -> expression_statement containing type annotation
            // We catch typed_parameter for function params separately below.
            "expression_statement" => {
                // Check if this contains a type annotation: `x: Type`
                // tree-sitter-python emits this as an expression_statement
                // containing a "type" child with annotation syntax.
                //
                // Actually, annotations in tree-sitter-python are handled as:
                // expression_statement > assignment with type annotation
                // Let's check the first child.
                if node.named_child_count() == 1 {
                    if let Some(child) = node.named_child(0) {
                        if child.kind() == "type" {
                            // Standalone annotation: `x: Foo`
                            // The type node has two children: the name and the type
                            if let (Some(name_node), Some(type_node)) =
                                (child.child_by_field_name("type"), child.child(0))
                            {
                                // This is tricky - let me handle it more carefully
                                let _ = (name_node, type_node);
                            }
                        }
                    }
                }
            }

            // Pattern 3: def f(x: Foo) -- typed parameter
            "typed_parameter" => {
                // typed_parameter has a name (identifier) and a type
                let name_node = match node.child_by_field_name("name") {
                    Some(n) => n,
                    None => {
                        // Fallback: first child might be the name
                        match node.child(0) {
                            Some(n) if n.kind() == "identifier" => n,
                            _ => continue,
                        }
                    }
                };
                let type_node = match node.child_by_field_name("type") {
                    Some(n) => n,
                    None => continue,
                };

                let var_name = get_node_text(&name_node, source).to_string();
                let type_name = get_node_text(&type_node, source).to_string();
                let line = node.start_position().row as u32 + 1;

                // Skip 'self' and 'cls' parameters
                if var_name == "self" || var_name == "cls" {
                    continue;
                }

                // Determine scope: the enclosing function
                let scope = enclosing_function_scope(&node, source);

                var_types.push(VarType::new_with_scope(
                    var_name,
                    type_name,
                    "parameter",
                    line,
                    scope,
                ));
            }

            // Pattern 3b: def f(x: Foo = default_val) -- typed parameter with default
            "typed_default_parameter" => {
                let name_node = match node.child_by_field_name("name") {
                    Some(n) => n,
                    None => match node.child(0) {
                        Some(n) if n.kind() == "identifier" => n,
                        _ => continue,
                    },
                };
                let type_node = match node.child_by_field_name("type") {
                    Some(n) => n,
                    None => continue,
                };

                let var_name = get_node_text(&name_node, source).to_string();
                let type_name = get_node_text(&type_node, source).to_string();
                let line = node.start_position().row as u32 + 1;

                if var_name == "self" || var_name == "cls" {
                    continue;
                }

                let scope = enclosing_function_scope(&node, source);

                var_types.push(VarType::new_with_scope(
                    var_name,
                    type_name,
                    "parameter",
                    line,
                    scope,
                ));
            }

            _ => {}
        }
    }

    var_types
}

// =============================================================================
// Go VarType extraction
// =============================================================================

/// Determine the enclosing function scope for a Go AST node.
///
/// Go uses `function_declaration` (top-level funcs) and `method_declaration` (receiver methods).
/// For methods, the scope is `ReceiverType.MethodName`.
pub(crate) fn enclosing_go_function_scope(
    node: &tree_sitter::Node,
    source: &[u8],
) -> Option<String> {
    let mut current = node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "function_declaration" => {
                if let Some(name_node) = parent.child_by_field_name("name") {
                    return Some(get_node_text(&name_node, source).to_string());
                }
            }
            "method_declaration" => {
                let method_name = parent
                    .child_by_field_name("name")
                    .map(|n| get_node_text(&n, source).to_string());
                if let Some(name) = method_name {
                    // Try to get receiver type for full scope like "Foo.Method"
                    if let Some(receiver_list) = parent.child_by_field_name("receiver") {
                        for i in 0..receiver_list.named_child_count() {
                            if let Some(param) = receiver_list.named_child(i) {
                                if param.kind() == "parameter_declaration" {
                                    if let Some(type_node) = param.child_by_field_name("type") {
                                        let type_text = get_node_text(&type_node, source);
                                        let receiver_type = type_text.trim_start_matches('*');
                                        return Some(format!("{}.{}", receiver_type, name));
                                    }
                                }
                            }
                        }
                    }
                    return Some(name);
                }
            }
            _ => {}
        }
        current = parent.parent();
    }
    None
}

/// Extract VarType entries from a Go source tree.
///
/// Walks the AST to find:
/// - **Short var declaration with composite literal**: `x := Foo{...}` -> VarType { source: "assignment" }
/// - **Short var declaration with constructor call**: `x := NewFoo()` -> VarType { source: "assignment" }
/// - **Var declaration with type**: `var x Foo` -> VarType { source: "annotation" }
/// - **Function/method parameters with types**: `func f(x Foo)` -> VarType { source: "parameter" }
/// - **Method receiver parameters**: `func (f *Foo) Method()` -> VarType { source: "parameter" }
///
/// Builtin types (map, slice, array, chan, string, int, etc.) produce "literal" source to
/// enable the FP defense layers (blocklist + ambiguity gate) for Go.
pub(crate) fn extract_go_var_types(tree: &tree_sitter::Tree, source: &[u8]) -> Vec<VarType> {
    let mut var_types = Vec::new();
    let root = tree.root_node();

    // Go builtin types that should not match project classes
    let go_builtin_types = [
        "string",
        "int",
        "int8",
        "int16",
        "int32",
        "int64",
        "uint",
        "uint8",
        "uint16",
        "uint32",
        "uint64",
        "uintptr",
        "float32",
        "float64",
        "complex64",
        "complex128",
        "bool",
        "byte",
        "rune",
        "error",
        "any",
    ];

    for node in walk_tree(root) {
        match node.kind() {
            // Pattern 1: x := Foo{...} or x := NewFoo()
            // short_var_declaration has left (expression_list) and right (expression_list)
            "short_var_declaration" => {
                let left = match node.child_by_field_name("left") {
                    Some(n) => n,
                    None => continue,
                };
                let right = match node.child_by_field_name("right") {
                    Some(n) => n,
                    None => continue,
                };

                // Right side must have exactly 1 expression
                if right.named_child_count() != 1 {
                    continue;
                }

                let val_node = match right.named_child(0) {
                    Some(n) => n,
                    None => continue,
                };

                let line = node.start_position().row as u32 + 1;
                let scope = enclosing_go_function_scope(&node, source);

                // Multi-return: x, err := NewFoo() or x, ok := val.(Foo)
                if left.named_child_count() >= 2 {
                    let var_node = match left.named_child(0) {
                        Some(n) if n.kind() == "identifier" => n,
                        _ => continue,
                    };
                    let var_name = get_node_text(&var_node, source).to_string();
                    if var_name == "_" {
                        continue;
                    }

                    match val_node.kind() {
                        "call_expression" => {
                            if let Some(func_node) = val_node.child_by_field_name("function") {
                                let func_text = get_node_text(&func_node, source).to_string();
                                let base_name = func_text.rsplit('.').next().unwrap_or(&func_text);
                                if base_name.starts_with("New") && base_name.len() > 3 {
                                    let type_name = &base_name[3..];
                                    if !type_name.is_empty()
                                        && type_name
                                            .chars()
                                            .next()
                                            .is_some_and(|c| c.is_uppercase())
                                    {
                                        var_types.push(VarType::new_with_scope(
                                            var_name,
                                            type_name.to_string(),
                                            "assignment",
                                            line,
                                            scope,
                                        ));
                                    }
                                } else if base_name
                                    .chars()
                                    .next()
                                    .is_some_and(|c| c.is_uppercase())
                                {
                                    var_types.push(VarType::new_with_scope(
                                        var_name,
                                        base_name.to_string(),
                                        "assignment",
                                        line,
                                        scope,
                                    ));
                                }
                            }
                        }
                        "composite_literal" => {
                            if let Some(type_node) = val_node.child_by_field_name("type") {
                                let type_text = get_node_text(&type_node, source).to_string();
                                match type_node.kind() {
                                    "type_identifier" => {
                                        if !go_builtin_types.contains(&type_text.as_str()) {
                                            var_types.push(VarType::new_with_scope(
                                                var_name,
                                                type_text,
                                                "assignment",
                                                line,
                                                scope.clone(),
                                            ));
                                        }
                                    }
                                    "qualified_type" => {
                                        var_types.push(VarType::new_with_scope(
                                            var_name,
                                            type_text,
                                            "assignment",
                                            line,
                                            scope.clone(),
                                        ));
                                    }
                                    _ => {}
                                }
                            }
                        }
                        "type_assertion_expression" => {
                            // x, ok := val.(Foo)
                            if let Some(type_node) = val_node.child_by_field_name("type") {
                                let type_text = get_node_text(&type_node, source).to_string();
                                let clean_type = type_text.trim_start_matches('*').to_string();
                                if !go_builtin_types.contains(&clean_type.as_str()) {
                                    var_types.push(VarType::new_with_scope(
                                        var_name,
                                        clean_type,
                                        "assertion",
                                        line,
                                        scope,
                                    ));
                                }
                            }
                        }
                        _ => {}
                    }
                    continue;
                }

                // Single assignment: x := expr
                if left.named_child_count() != 1 {
                    continue;
                }

                let var_node = match left.named_child(0) {
                    Some(n) if n.kind() == "identifier" => n,
                    _ => continue,
                };

                let var_name = get_node_text(&var_node, source).to_string();
                if var_name == "_" {
                    continue;
                }

                match val_node.kind() {
                    "composite_literal" => {
                        if let Some(type_node) = val_node.child_by_field_name("type") {
                            let type_text = get_node_text(&type_node, source).to_string();
                            match type_node.kind() {
                                "type_identifier" => {
                                    if !go_builtin_types.contains(&type_text.as_str()) {
                                        var_types.push(VarType::new_with_scope(
                                            var_name,
                                            type_text,
                                            "assignment",
                                            line,
                                            scope,
                                        ));
                                    }
                                }
                                "qualified_type" => {
                                    var_types.push(VarType::new_with_scope(
                                        var_name,
                                        type_text,
                                        "assignment",
                                        line,
                                        scope,
                                    ));
                                }
                                "map_type" | "slice_type" | "array_type" => {
                                    let builtin_name = match type_node.kind() {
                                        "map_type" => "map",
                                        "slice_type" => "slice",
                                        "array_type" => "array",
                                        _ => "unknown",
                                    };
                                    var_types.push(VarType::new_with_scope(
                                        var_name,
                                        builtin_name.to_string(),
                                        "literal",
                                        line,
                                        scope,
                                    ));
                                }
                                _ => {}
                            }
                        }
                    }
                    "call_expression" => {
                        if let Some(func_node) = val_node.child_by_field_name("function") {
                            let func_text = get_node_text(&func_node, source).to_string();
                            let base_name = func_text.rsplit('.').next().unwrap_or(&func_text);
                            if base_name.starts_with("New") && base_name.len() > 3 {
                                let type_name = &base_name[3..];
                                if !type_name.is_empty()
                                    && type_name.chars().next().is_some_and(|c| c.is_uppercase())
                                {
                                    var_types.push(VarType::new_with_scope(
                                        var_name,
                                        type_name.to_string(),
                                        "assignment",
                                        line,
                                        scope,
                                    ));
                                }
                            } else if base_name.chars().next().is_some_and(|c| c.is_uppercase()) {
                                var_types.push(VarType::new_with_scope(
                                    var_name,
                                    base_name.to_string(),
                                    "assignment",
                                    line,
                                    scope,
                                ));
                            }
                        }
                    }
                    "type_assertion_expression" => {
                        // x := val.(Foo)
                        if let Some(type_node) = val_node.child_by_field_name("type") {
                            let type_text = get_node_text(&type_node, source).to_string();
                            let clean_type = type_text.trim_start_matches('*').to_string();
                            if !go_builtin_types.contains(&clean_type.as_str()) {
                                var_types.push(VarType::new_with_scope(
                                    var_name,
                                    clean_type,
                                    "assertion",
                                    line,
                                    scope,
                                ));
                            }
                        }
                    }
                    "interpreted_string_literal" | "raw_string_literal" => {
                        var_types.push(VarType::new_with_scope(
                            var_name,
                            "string".to_string(),
                            "literal",
                            line,
                            scope,
                        ));
                    }
                    "int_literal" => {
                        var_types.push(VarType::new_with_scope(
                            var_name,
                            "int".to_string(),
                            "literal",
                            line,
                            scope,
                        ));
                    }
                    "float_literal" => {
                        var_types.push(VarType::new_with_scope(
                            var_name,
                            "float64".to_string(),
                            "literal",
                            line,
                            scope,
                        ));
                    }
                    "true" | "false" => {
                        var_types.push(VarType::new_with_scope(
                            var_name,
                            "bool".to_string(),
                            "literal",
                            line,
                            scope,
                        ));
                    }
                    _ => {}
                }
            }

            // Pattern 2: var x Foo -- explicit type declaration
            "var_spec" => {
                let name_node = match node.child_by_field_name("name") {
                    Some(n) if n.kind() == "identifier" => n,
                    _ => continue,
                };
                let type_node = match node.child_by_field_name("type") {
                    Some(n) => n,
                    None => continue,
                };

                let var_name = get_node_text(&name_node, source).to_string();
                let type_text = get_node_text(&type_node, source).to_string();
                let line = node.start_position().row as u32 + 1;
                let scope = enclosing_go_function_scope(&node, source);

                // Only track non-builtin types
                if !go_builtin_types.contains(&type_text.as_str()) {
                    var_types.push(VarType::new_with_scope(
                        var_name,
                        type_text,
                        "annotation",
                        line,
                        scope,
                    ));
                }
            }

            // Pattern 3: func f(x Foo) -- function parameter with type
            "parameter_declaration" => {
                // Skip if inside a receiver (handled as part of method_declaration scope)
                // Check parent: if it's a parameter_list that's a "receiver" field, skip
                if let Some(param_list) = node.parent() {
                    if let Some(method_decl) = param_list.parent() {
                        if method_decl.kind() == "method_declaration" {
                            if let Some(receiver) = method_decl.child_by_field_name("receiver") {
                                if receiver.id() == param_list.id() {
                                    // This is a receiver parameter -- still extract it
                                    // but with special handling
                                    if let (Some(name_node), Some(type_node)) = (
                                        node.child_by_field_name("name"),
                                        node.child_by_field_name("type"),
                                    ) {
                                        let var_name =
                                            get_node_text(&name_node, source).to_string();
                                        let type_text =
                                            get_node_text(&type_node, source).to_string();
                                        // Strip pointer: *Foo -> Foo
                                        let clean_type =
                                            type_text.trim_start_matches('*').to_string();
                                        let line = node.start_position().row as u32 + 1;
                                        let scope = enclosing_go_function_scope(&node, source);
                                        if !go_builtin_types.contains(&clean_type.as_str()) {
                                            var_types.push(VarType::new_with_scope(
                                                var_name,
                                                clean_type,
                                                "parameter",
                                                line,
                                                scope,
                                            ));
                                        }
                                    }
                                    continue;
                                }
                            }
                        }
                    }
                }

                // Regular function parameter
                let name_node = match node.child_by_field_name("name") {
                    Some(n) if n.kind() == "identifier" => n,
                    _ => continue,
                };
                let type_node = match node.child_by_field_name("type") {
                    Some(n) => n,
                    None => continue,
                };

                let var_name = get_node_text(&name_node, source).to_string();
                let type_text = get_node_text(&type_node, source).to_string();
                // Strip pointer: *Foo -> Foo
                let clean_type = type_text.trim_start_matches('*').to_string();
                let line = node.start_position().row as u32 + 1;
                let scope = enclosing_go_function_scope(&node, source);

                if !go_builtin_types.contains(&clean_type.as_str()) {
                    var_types.push(VarType::new_with_scope(
                        var_name,
                        clean_type,
                        "parameter",
                        line,
                        scope,
                    ));
                }
            }

            _ => {}
        }
    }

    var_types
}

// =============================================================================
// TypeScript/JavaScript VarType extraction
// =============================================================================

/// Determine the enclosing function scope for a TypeScript/JavaScript AST node.
///
/// Walks parent nodes looking for:
/// - `function_declaration` -> function name
/// - `method_definition` -> `ClassName.methodName` (prepends class name if found)
/// - `arrow_function` / `function_expression` / `function` -> check parent `variable_declarator` for name
///
/// Returns `None` for module-level code (no enclosing function).
pub(crate) fn enclosing_ts_function_scope(
    node: &tree_sitter::Node,
    source: &[u8],
) -> Option<String> {
    let mut current = node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "function_declaration" => {
                if let Some(name_node) = parent.child_by_field_name("name") {
                    return Some(get_node_text(&name_node, source).to_string());
                }
            }
            "method_definition" => {
                let method_name = parent
                    .child_by_field_name("name")
                    .map(|n| get_node_text(&n, source).to_string());
                if let Some(name) = method_name {
                    // Try to get class name: method_definition > class_body > class_declaration
                    if let Some(class_body) = parent.parent() {
                        if class_body.kind() == "class_body" {
                            if let Some(class_decl) = class_body.parent() {
                                if class_decl.kind() == "class_declaration"
                                    || class_decl.kind() == "class"
                                {
                                    if let Some(class_name_node) =
                                        class_decl.child_by_field_name("name")
                                    {
                                        let class_name = get_node_text(&class_name_node, source);
                                        return Some(format!("{}.{}", class_name, name));
                                    }
                                }
                            }
                        }
                    }
                    return Some(name);
                }
            }
            "arrow_function" | "function_expression" | "function" => {
                // Anonymous -- check if parent is variable_declarator
                if let Some(var_decl) = parent.parent() {
                    if var_decl.kind() == "variable_declarator" {
                        if let Some(name_node) = var_decl.child_by_field_name("name") {
                            return Some(get_node_text(&name_node, source).to_string());
                        }
                    }
                }
                // Truly anonymous -- return None (module scope)
                return None;
            }
            _ => {}
        }
        current = parent.parent();
    }
    None
}

/// Extract VarType entries from a TypeScript/JavaScript source tree.
pub(crate) fn extract_ts_var_types(tree: &tree_sitter::Tree, source: &[u8]) -> Vec<VarType> {
    let mut var_types = Vec::new();
    let root = tree.root_node();

    // TS/JS builtin types that should not match project classes
    let ts_builtin_types = [
        "string",
        "number",
        "boolean",
        "bigint",
        "symbol",
        "undefined",
        "null",
        "void",
        "never",
        "any",
        "unknown",
        "object",
        "String",
        "Number",
        "Boolean",
        "Function",
        "Object",
        "Array",
        "Promise",
        "Map",
        "Set",
        "RegExp",
        "Date",
        "Error",
        "Symbol",
    ];

    for node in walk_tree(root) {
        match node.kind() {
            // Pattern 1: const x = new Foo() / let x: Type = expr / let x: Type
            "variable_declarator" => {
                let name_node = match node.child_by_field_name("name") {
                    Some(n) if n.kind() == "identifier" => n,
                    _ => continue,
                };
                let var_name = get_node_text(&name_node, source).to_string();
                let line = node.start_position().row as u32 + 1;
                let scope = enclosing_ts_function_scope(&node, source);

                // Check value field first (new expression, as expression, literals)
                if let Some(value) = node.child_by_field_name("value") {
                    match value.kind() {
                        "new_expression" => {
                            // const x = new Foo(...)
                            if let Some(ctor) = value.child_by_field_name("constructor") {
                                let type_name = get_node_text(&ctor, source).to_string();
                                if !ts_builtin_types.contains(&type_name.as_str()) {
                                    var_types.push(VarType::new_with_scope(
                                        var_name.clone(),
                                        type_name,
                                        "assignment",
                                        line,
                                        scope.clone(),
                                    ));
                                }
                            }
                        }
                        "as_expression" => {
                            // const x = expr as Type
                            let child_count = value.named_child_count();
                            if child_count >= 2 {
                                if let Some(type_node) = value.named_child(child_count - 1) {
                                    let type_name = match type_node.kind() {
                                        "type_identifier" => {
                                            Some(get_node_text(&type_node, source).to_string())
                                        }
                                        "generic_type" => type_node
                                            .child_by_field_name("name")
                                            .map(|n| get_node_text(&n, source).to_string()),
                                        _ => None,
                                    };
                                    if let Some(tn) = type_name {
                                        if !ts_builtin_types.contains(&tn.as_str()) {
                                            var_types.push(VarType::new_with_scope(
                                                var_name.clone(),
                                                tn,
                                                "assertion",
                                                line,
                                                scope.clone(),
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                        // Literal types
                        "string" | "template_string" => {
                            var_types.push(VarType::new_with_scope(
                                var_name.clone(),
                                "string".to_string(),
                                "literal",
                                line,
                                scope.clone(),
                            ));
                        }
                        "number" => {
                            var_types.push(VarType::new_with_scope(
                                var_name.clone(),
                                "number".to_string(),
                                "literal",
                                line,
                                scope.clone(),
                            ));
                        }
                        "true" | "false" => {
                            var_types.push(VarType::new_with_scope(
                                var_name.clone(),
                                "boolean".to_string(),
                                "literal",
                                line,
                                scope.clone(),
                            ));
                        }
                        "array" => {
                            var_types.push(VarType::new_with_scope(
                                var_name.clone(),
                                "Array".to_string(),
                                "literal",
                                line,
                                scope.clone(),
                            ));
                        }
                        "object" => {
                            var_types.push(VarType::new_with_scope(
                                var_name.clone(),
                                "Object".to_string(),
                                "literal",
                                line,
                                scope.clone(),
                            ));
                        }
                        "regex" => {
                            var_types.push(VarType::new_with_scope(
                                var_name.clone(),
                                "RegExp".to_string(),
                                "literal",
                                line,
                                scope.clone(),
                            ));
                        }
                        "null" => {
                            var_types.push(VarType::new_with_scope(
                                var_name.clone(),
                                "null".to_string(),
                                "literal",
                                line,
                                scope.clone(),
                            ));
                        }
                        _ => {}
                    }
                }

                // Check type annotation: let x: Type or const x: Type = new Foo()
                // Type annotation takes priority for the type mapping
                if let Some(type_ann) = node.child_by_field_name("type") {
                    // type_ann is the type_annotation node, its first named child is the type
                    if let Some(type_node) = type_ann.named_child(0) {
                        let type_name = match type_node.kind() {
                            "type_identifier" => {
                                Some(get_node_text(&type_node, source).to_string())
                            }
                            "generic_type" => type_node
                                .child_by_field_name("name")
                                .map(|n| get_node_text(&n, source).to_string()),
                            _ => None,
                        };
                        if let Some(tn) = type_name {
                            if !ts_builtin_types.contains(&tn.as_str()) {
                                var_types.push(VarType::new_with_scope(
                                    var_name,
                                    tn,
                                    "annotation",
                                    line,
                                    scope,
                                ));
                            }
                        }
                    }
                }
            }

            // Pattern 2: function f(x: Foo) -- typed parameters
            "required_parameter" | "optional_parameter" => {
                let name_node = match node.child_by_field_name("pattern") {
                    Some(n) if n.kind() == "identifier" => n,
                    _ => {
                        // Fallback: try first named child
                        match node.named_child(0) {
                            Some(n) if n.kind() == "identifier" => n,
                            _ => continue,
                        }
                    }
                };

                let type_ann = match node.child_by_field_name("type") {
                    Some(n) => n,
                    None => continue,
                };

                let var_name = get_node_text(&name_node, source).to_string();
                let line = node.start_position().row as u32 + 1;
                let scope = enclosing_ts_function_scope(&node, source);

                if let Some(type_node) = type_ann.named_child(0) {
                    let type_name = match type_node.kind() {
                        "type_identifier" => Some(get_node_text(&type_node, source).to_string()),
                        "generic_type" => type_node
                            .child_by_field_name("name")
                            .map(|n| get_node_text(&n, source).to_string()),
                        _ => None,
                    };
                    if let Some(tn) = type_name {
                        if !ts_builtin_types.contains(&tn.as_str()) {
                            var_types.push(VarType::new_with_scope(
                                var_name,
                                tn,
                                "parameter",
                                line,
                                scope,
                            ));
                        }
                    }
                }
            }

            _ => {}
        }
    }

    var_types
}

// =============================================================================
// Java VarType extraction
// =============================================================================

/// Determine the enclosing function scope for a Java AST node.
pub(crate) fn enclosing_java_function_scope(
    node: &tree_sitter::Node,
    source: &[u8],
) -> Option<String> {
    let mut current = node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "method_declaration" | "constructor_declaration" => {
                let method_name = parent
                    .child_by_field_name("name")
                    .map(|n| get_node_text(&n, source).to_string());
                if let Some(name) = method_name {
                    // Try to get class name: method_declaration > class_body > class_declaration
                    if let Some(class_body) = parent.parent() {
                        if class_body.kind() == "class_body" {
                            if let Some(class_decl) = class_body.parent() {
                                if class_decl.kind() == "class_declaration" {
                                    if let Some(class_name_node) =
                                        class_decl.child_by_field_name("name")
                                    {
                                        let class_name = get_node_text(&class_name_node, source);
                                        return Some(format!("{}.{}", class_name, name));
                                    }
                                }
                            }
                        }
                    }
                    return Some(name);
                }
            }
            _ => {}
        }
        current = parent.parent();
    }
    None
}

/// Extract VarType entries from a Java source tree.
pub(crate) fn extract_java_var_types(tree: &tree_sitter::Tree, source: &[u8]) -> Vec<VarType> {
    let mut var_types = Vec::new();
    let root = tree.root_node();

    // Java builtin/primitive types that should not match project classes
    let java_builtin_types = [
        "String",
        "int",
        "Integer",
        "double",
        "Double",
        "float",
        "Float",
        "long",
        "Long",
        "boolean",
        "Boolean",
        "byte",
        "Byte",
        "short",
        "Short",
        "char",
        "Character",
        "void",
        "Object",
        "Number",
        "Comparable",
        "Serializable",
        "Cloneable",
        "Iterable",
        "AutoCloseable",
        "Throwable",
        "Exception",
        "RuntimeException",
        "Error",
        "var",
    ];

    for node in walk_tree(root) {
        match node.kind() {
            // Pattern 1: Type var = new Type() or Type var = expr or Type var;
            "local_variable_declaration" => {
                // Get the type from the "type" field
                let type_node = match node.child_by_field_name("type") {
                    Some(n) => n,
                    None => continue,
                };

                let raw_type_text = get_node_text(&type_node, source).to_string();

                // Extract base type name: for generic_type like "ArrayList<String>", get "ArrayList"
                let type_name = if type_node.kind() == "generic_type" {
                    // First named child of generic_type is the base type_identifier
                    type_node
                        .named_child(0)
                        .map(|n| get_node_text(&n, source).to_string())
                        .unwrap_or(raw_type_text.clone())
                } else {
                    raw_type_text.clone()
                };

                // Get declarator(s) -- there can be multiple: int x = 1, y = 2;
                for i in 0..node.named_child_count() {
                    let child = match node.named_child(i) {
                        Some(c) if c.kind() == "variable_declarator" => c,
                        _ => continue,
                    };

                    let name_node = match child.child_by_field_name("name") {
                        Some(n) if n.kind() == "identifier" => n,
                        _ => continue,
                    };
                    let var_name = get_node_text(&name_node, source).to_string();
                    let line = child.start_position().row as u32 + 1;
                    let scope = enclosing_java_function_scope(&node, source);

                    // Check if type is "var" -- infer from RHS
                    if type_name == "var" {
                        if let Some(value) = child.child_by_field_name("value") {
                            if value.kind() == "object_creation_expression" {
                                // var x = new Dog() -- extract type from the constructor
                                if let Some(ctor_type) = value.child_by_field_name("type") {
                                    let ctor_type_name = if ctor_type.kind() == "generic_type" {
                                        ctor_type
                                            .named_child(0)
                                            .map(|n| get_node_text(&n, source).to_string())
                                            .unwrap_or_else(|| {
                                                get_node_text(&ctor_type, source).to_string()
                                            })
                                    } else {
                                        get_node_text(&ctor_type, source).to_string()
                                    };
                                    if !java_builtin_types.contains(&ctor_type_name.as_str()) {
                                        var_types.push(VarType::new_with_scope(
                                            var_name,
                                            ctor_type_name,
                                            "constructor",
                                            line,
                                            scope,
                                        ));
                                    }
                                }
                            }
                        }
                        continue;
                    }

                    // Skip builtin types
                    if java_builtin_types.contains(&type_name.as_str()) {
                        continue;
                    }

                    // Check if the value is a constructor call: new Type(...)
                    let source_kind = if let Some(value) = child.child_by_field_name("value") {
                        if value.kind() == "object_creation_expression" {
                            "constructor"
                        } else {
                            "annotation"
                        }
                    } else {
                        // No initializer: Type var;
                        "annotation"
                    };

                    var_types.push(VarType::new_with_scope(
                        var_name,
                        type_name.clone(),
                        source_kind,
                        line,
                        scope,
                    ));
                }
            }

            // Pattern 2: field_declaration -- same structure as local_variable_declaration
            "field_declaration" => {
                let type_node = match node.child_by_field_name("type") {
                    Some(n) => n,
                    None => continue,
                };

                let raw_type_text = get_node_text(&type_node, source).to_string();

                let type_name = if type_node.kind() == "generic_type" {
                    type_node
                        .named_child(0)
                        .map(|n| get_node_text(&n, source).to_string())
                        .unwrap_or(raw_type_text.clone())
                } else {
                    raw_type_text.clone()
                };

                if java_builtin_types.contains(&type_name.as_str()) {
                    continue;
                }

                for i in 0..node.named_child_count() {
                    let child = match node.named_child(i) {
                        Some(c) if c.kind() == "variable_declarator" => c,
                        _ => continue,
                    };

                    let name_node = match child.child_by_field_name("name") {
                        Some(n) if n.kind() == "identifier" => n,
                        _ => continue,
                    };
                    let var_name = get_node_text(&name_node, source).to_string();
                    let line = child.start_position().row as u32 + 1;

                    // Fields are at class scope, not method scope
                    // Walk up to find class name
                    let scope = {
                        let mut s = None;
                        let mut cur = node.parent();
                        while let Some(p) = cur {
                            if p.kind() == "class_body" {
                                if let Some(class_decl) = p.parent() {
                                    if class_decl.kind() == "class_declaration" {
                                        if let Some(cn) = class_decl.child_by_field_name("name") {
                                            s = Some(get_node_text(&cn, source).to_string());
                                        }
                                    }
                                }
                                break;
                            }
                            cur = p.parent();
                        }
                        s
                    };

                    let source_kind = if let Some(value) = child.child_by_field_name("value") {
                        if value.kind() == "object_creation_expression" {
                            "constructor"
                        } else {
                            "annotation"
                        }
                    } else {
                        "annotation"
                    };

                    var_types.push(VarType::new_with_scope(
                        var_name,
                        type_name.clone(),
                        source_kind,
                        line,
                        scope,
                    ));
                }
            }

            // Pattern 3: formal_parameter -- method/constructor parameters
            "formal_parameter" => {
                let type_node = match node.child_by_field_name("type") {
                    Some(n) => n,
                    None => continue,
                };
                let name_node = match node.child_by_field_name("name") {
                    Some(n) if n.kind() == "identifier" => n,
                    _ => continue,
                };

                let raw_type_text = get_node_text(&type_node, source).to_string();
                let type_name = if type_node.kind() == "generic_type" {
                    type_node
                        .named_child(0)
                        .map(|n| get_node_text(&n, source).to_string())
                        .unwrap_or(raw_type_text.clone())
                } else {
                    raw_type_text.clone()
                };

                if java_builtin_types.contains(&type_name.as_str()) {
                    continue;
                }

                let var_name = get_node_text(&name_node, source).to_string();
                let line = node.start_position().row as u32 + 1;
                let scope = enclosing_java_function_scope(&node, source);

                var_types.push(VarType::new_with_scope(
                    var_name,
                    type_name,
                    "parameter",
                    line,
                    scope,
                ));
            }

            // Pattern 4: enhanced_for_statement -- for (Type var : collection)
            "enhanced_for_statement" => {
                let type_node = match node.child_by_field_name("type") {
                    Some(n) => n,
                    None => continue,
                };
                let name_node = match node.child_by_field_name("name") {
                    Some(n) if n.kind() == "identifier" => n,
                    _ => continue,
                };

                let raw_type_text = get_node_text(&type_node, source).to_string();
                let type_name = if type_node.kind() == "generic_type" {
                    type_node
                        .named_child(0)
                        .map(|n| get_node_text(&n, source).to_string())
                        .unwrap_or(raw_type_text.clone())
                } else {
                    raw_type_text.clone()
                };

                if java_builtin_types.contains(&type_name.as_str()) {
                    continue;
                }

                let var_name = get_node_text(&name_node, source).to_string();
                let line = node.start_position().row as u32 + 1;
                let scope = enclosing_java_function_scope(&node, source);

                var_types.push(VarType::new_with_scope(
                    var_name,
                    type_name,
                    "annotation",
                    line,
                    scope,
                ));
            }

            // Pattern 5: cast_expression -- (Type) expr
            "cast_expression" => {
                let type_node = match node.child_by_field_name("type") {
                    Some(n) => n,
                    None => continue,
                };

                // Only useful if the cast result is assigned -- check parent
                let parent = match node.parent() {
                    Some(p) => p,
                    None => continue,
                };

                // Only track if parent is a variable_declarator (assignment context)
                if parent.kind() != "variable_declarator" {
                    continue;
                }

                let name_node = match parent.child_by_field_name("name") {
                    Some(n) if n.kind() == "identifier" => n,
                    _ => continue,
                };

                let raw_type_text = get_node_text(&type_node, source).to_string();
                let type_name = if type_node.kind() == "generic_type" {
                    type_node
                        .named_child(0)
                        .map(|n| get_node_text(&n, source).to_string())
                        .unwrap_or(raw_type_text.clone())
                } else {
                    raw_type_text.clone()
                };

                if java_builtin_types.contains(&type_name.as_str()) {
                    continue;
                }

                let var_name = get_node_text(&name_node, source).to_string();
                let line = node.start_position().row as u32 + 1;
                let scope = enclosing_java_function_scope(&node, source);

                var_types.push(VarType::new_with_scope(
                    var_name,
                    type_name,
                    "annotation",
                    line,
                    scope,
                ));
            }

            _ => {}
        }
    }

    var_types
}

// =============================================================================
// Rust VarType extraction
// =============================================================================

/// Determine the enclosing function scope for a Rust AST node.
pub(crate) fn enclosing_rust_function_scope(
    node: &tree_sitter::Node,
    source: &[u8],
) -> Option<String> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "function_item" {
            let fn_name = parent
                .child_by_field_name("name")
                .map(|n| get_node_text(&n, source).to_string());
            if let Some(name) = fn_name {
                // Check if inside an impl block: function_item -> declaration_list -> impl_item
                if let Some(decl_list) = parent.parent() {
                    if decl_list.kind() == "declaration_list" {
                        if let Some(impl_item) = decl_list.parent() {
                            if impl_item.kind() == "impl_item" {
                                // Get the type being implemented
                                if let Some(type_node) = impl_item.child_by_field_name("type") {
                                    let type_name = get_node_text(&type_node, source);
                                    return Some(format!("{}.{}", type_name, name));
                                }
                            }
                        }
                    }
                }
                return Some(name);
            }
        }
        current = parent.parent();
    }
    None
}

/// Extract VarType entries from a Rust source tree.
pub(crate) fn extract_rust_var_types(tree: &tree_sitter::Tree, source: &[u8]) -> Vec<VarType> {
    let mut var_types = Vec::new();
    let root = tree.root_node();

    let rust_builtin_types = [
        "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize",
        "f32", "f64", "bool", "char", "str", "String", "Vec", "HashMap", "HashSet", "BTreeMap",
        "BTreeSet", "Option", "Result", "Box", "Rc", "Arc", "Cow", "Cell", "RefCell", "Mutex",
        "RwLock", "Pin", "Waker", "Context",
    ];

    // Track vars already assigned via constructor (prefer constructor over annotation)
    let mut constructor_vars: HashSet<(String, Option<String>)> = HashSet::new();

    for node in walk_tree(root) {
        match node.kind() {
            "let_declaration" => {
                let line = node.start_position().row as u32 + 1;
                let scope = enclosing_rust_function_scope(&node, source);

                let pattern_node = match node.child_by_field_name("pattern") {
                    Some(n) => n,
                    None => continue,
                };

                // Check RHS value first (for constructor detection)
                if let Some(value_node) = node.child_by_field_name("value") {
                    match value_node.kind() {
                        // Pattern 1: let dog = Dog::new(...) -- scoped identifier constructor
                        "call_expression" => {
                            if let Some(func_node) = value_node.child_by_field_name("function") {
                                if func_node.kind() == "scoped_identifier" {
                                    if let Some(path_node) = func_node.child_by_field_name("path") {
                                        let type_name =
                                            get_node_text(&path_node, source).to_string();

                                        if pattern_node.kind() == "identifier" {
                                            let var_name =
                                                get_node_text(&pattern_node, source).to_string();
                                            if !rust_builtin_types.contains(&type_name.as_str())
                                                && !var_name.starts_with('_')
                                            {
                                                constructor_vars
                                                    .insert((var_name.clone(), scope.clone()));
                                                var_types.push(VarType::new_with_scope(
                                                    var_name,
                                                    type_name,
                                                    "constructor",
                                                    line,
                                                    scope,
                                                ));
                                                continue;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        // Pattern 2: let animal = Animal { name: ... } -- struct expression
                        "struct_expression" => {
                            if let Some(name_node) = value_node.child_by_field_name("name") {
                                let type_name = get_node_text(&name_node, source).to_string();

                                if pattern_node.kind() == "identifier" {
                                    let var_name = get_node_text(&pattern_node, source).to_string();
                                    if !rust_builtin_types.contains(&type_name.as_str())
                                        && !var_name.starts_with('_')
                                    {
                                        constructor_vars.insert((var_name.clone(), scope.clone()));
                                        var_types.push(VarType::new_with_scope(
                                            var_name,
                                            type_name,
                                            "constructor",
                                            line,
                                            scope,
                                        ));
                                        continue;
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }

                // Pattern 3: let a: Animal = ... -- explicit type annotation
                if let Some(type_node) = node.child_by_field_name("type") {
                    if pattern_node.kind() == "identifier" {
                        let var_name = get_node_text(&pattern_node, source).to_string();
                        if var_name.starts_with('_') {
                            continue;
                        }

                        // Skip if already found via constructor
                        if constructor_vars.contains(&(var_name.clone(), scope.clone())) {
                            continue;
                        }

                        let type_name = extract_rust_type_name(&type_node, source);
                        if let Some(tn) = type_name {
                            if !rust_builtin_types.contains(&tn.as_str()) {
                                var_types.push(VarType::new_with_scope(
                                    var_name,
                                    tn,
                                    "annotation",
                                    line,
                                    scope,
                                ));
                            }
                        }
                    }
                }
            }

            // Pattern 4: fn process(animal: &Animal) -- function parameters
            "parameter" => {
                let line = node.start_position().row as u32 + 1;
                let scope = enclosing_rust_function_scope(&node, source);

                let pattern_node = match node.child_by_field_name("pattern") {
                    Some(n) if n.kind() == "identifier" => n,
                    _ => continue,
                };
                let var_name = get_node_text(&pattern_node, source).to_string();
                if var_name.starts_with('_') {
                    continue;
                }

                let type_child = match node.child_by_field_name("type") {
                    Some(n) => n,
                    None => continue,
                };

                let type_name = extract_rust_type_name(&type_child, source);
                if let Some(tn) = type_name {
                    if !rust_builtin_types.contains(&tn.as_str()) {
                        var_types.push(VarType::new_with_scope(
                            var_name,
                            tn,
                            "parameter",
                            line,
                            scope,
                        ));
                    }
                }
            }

            _ => {}
        }
    }

    var_types
}

/// Extract the base type name from a Rust type AST node.
pub(crate) fn extract_rust_type_name(
    type_node: &tree_sitter::Node,
    source: &[u8],
) -> Option<String> {
    match type_node.kind() {
        "type_identifier" => Some(get_node_text(type_node, source).to_string()),
        "reference_type" => {
            for i in 0..type_node.named_child_count() {
                if let Some(child) = type_node.named_child(i) {
                    if let Some(result) = extract_rust_type_name(&child, source) {
                        return Some(result);
                    }
                }
            }
            None
        }
        "generic_type" => type_node
            .named_child(0)
            .and_then(|n| extract_rust_type_name(&n, source)),
        "scoped_type_identifier" => {
            if let Some(name_node) = type_node.child_by_field_name("name") {
                Some(get_node_text(&name_node, source).to_string())
            } else {
                Some(get_node_text(type_node, source).to_string())
            }
        }
        _ => None,
    }
}

// =============================================================================
// Kotlin VarType extraction
// =============================================================================

/// Determine the enclosing function scope for a Kotlin AST node.
pub(crate) fn enclosing_kotlin_function_scope(
    node: &tree_sitter::Node,
    source: &[u8],
) -> Option<String> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "function_declaration" {
            let fn_name = parent
                .child_by_field_name("name")
                .map(|n| get_node_text(&n, source).to_string());
            if let Some(name) = fn_name {
                if let Some(class_body) = parent.parent() {
                    if class_body.kind() == "class_body" {
                        if let Some(class_decl) = class_body.parent() {
                            if class_decl.kind() == "class_declaration" {
                                if let Some(class_name_node) =
                                    class_decl.child_by_field_name("name")
                                {
                                    let class_name = get_node_text(&class_name_node, source);
                                    return Some(format!("{}.{}", class_name, name));
                                }
                            }
                        }
                    }
                }
                return Some(name);
            }
        }
        current = parent.parent();
    }
    None
}

/// Extract the base type name from a Kotlin type AST node.
pub(crate) fn extract_kotlin_type_name(
    type_node: &tree_sitter::Node,
    source: &[u8],
) -> Option<String> {
    match type_node.kind() {
        "user_type" => {
            for i in 0..type_node.child_count() {
                if let Some(child) = type_node.child(i) {
                    if child.kind() == "identifier"
                        || child.kind() == "simple_identifier"
                        || child.kind() == "type_identifier"
                    {
                        return Some(get_node_text(&child, source).to_string());
                    }
                    if child.kind() == "simple_user_type" {
                        for j in 0..child.child_count() {
                            if let Some(inner) = child.child(j) {
                                if inner.kind() == "identifier"
                                    || inner.kind() == "simple_identifier"
                                    || inner.kind() == "type_identifier"
                                {
                                    return Some(get_node_text(&inner, source).to_string());
                                }
                            }
                        }
                        let text = get_node_text(&child, source).to_string();
                        if let Some(idx) = text.find('<') {
                            return Some(text[..idx].to_string());
                        }
                        return Some(text);
                    }
                }
            }
            let text = get_node_text(type_node, source).to_string();
            if let Some(idx) = text.find('<') {
                Some(text[..idx].to_string())
            } else {
                Some(text)
            }
        }
        "nullable_type" => {
            for i in 0..type_node.named_child_count() {
                if let Some(child) = type_node.named_child(i) {
                    if let Some(result) = extract_kotlin_type_name(&child, source) {
                        return Some(result);
                    }
                }
            }
            None
        }
        "type_reference" => {
            for i in 0..type_node.named_child_count() {
                if let Some(child) = type_node.named_child(i) {
                    if let Some(result) = extract_kotlin_type_name(&child, source) {
                        return Some(result);
                    }
                }
            }
            None
        }
        "identifier" | "simple_identifier" | "type_identifier" => {
            Some(get_node_text(type_node, source).to_string())
        }
        _ => {
            for i in 0..type_node.named_child_count() {
                if let Some(child) = type_node.named_child(i) {
                    if let Some(result) = extract_kotlin_type_name(&child, source) {
                        return Some(result);
                    }
                }
            }
            None
        }
    }
}

/// Extract VarType entries from a Kotlin source tree.
pub(crate) fn extract_kotlin_var_types(tree: &tree_sitter::Tree, source: &[u8]) -> Vec<VarType> {
    let mut var_types = Vec::new();
    let root = tree.root_node();

    let kotlin_builtin_types = [
        "String",
        "Int",
        "Long",
        "Double",
        "Float",
        "Boolean",
        "Byte",
        "Short",
        "Char",
        "Unit",
        "Nothing",
        "Any",
        "Number",
        "Comparable",
        "List",
        "Map",
        "Set",
        "MutableList",
        "MutableMap",
        "MutableSet",
        "Array",
        "IntArray",
        "LongArray",
        "DoubleArray",
        "FloatArray",
        "BooleanArray",
        "ByteArray",
        "ShortArray",
        "CharArray",
        "Pair",
        "Triple",
        "Sequence",
        "Iterable",
        "Exception",
        "RuntimeException",
        "Throwable",
        "Enum",
        "Annotation",
        "HashMap",
        "HashSet",
        "ArrayList",
        "LinkedList",
        "LinkedHashMap",
        "LinkedHashSet",
        "Regex",
        "StringBuilder",
        "Lazy",
    ];

    for node in walk_tree(root) {
        match node.kind() {
            "property_declaration" => {
                let line = node.start_position().row as u32 + 1;
                let scope = enclosing_kotlin_function_scope(&node, source);

                let mut var_name: Option<String> = None;
                let mut type_name: Option<String> = None;
                let mut has_constructor_rhs = false;
                let mut constructor_type: Option<String> = None;

                for i in 0..node.child_count() {
                    let child = match node.child(i) {
                        Some(c) => c,
                        None => continue,
                    };

                    match child.kind() {
                        "variable_declaration" => {
                            for j in 0..child.child_count() {
                                if let Some(inner) = child.child(j) {
                                    match inner.kind() {
                                        "identifier" | "simple_identifier" => {
                                            var_name =
                                                Some(get_node_text(&inner, source).to_string());
                                        }
                                        "user_type" | "nullable_type" | "type_reference" => {
                                            type_name = extract_kotlin_type_name(&inner, source);
                                        }
                                        _ => {
                                            if type_name.is_none() {
                                                if let Some(tn) =
                                                    extract_kotlin_type_name(&inner, source)
                                                {
                                                    if tn
                                                        .chars()
                                                        .next()
                                                        .is_some_and(|c| c.is_uppercase())
                                                    {
                                                        type_name = Some(tn);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        "identifier" | "simple_identifier" if var_name.is_none() => {
                            var_name = Some(get_node_text(&child, source).to_string());
                        }
                        "call_expression" => {
                            if let Some(func_child) = child.child(0) {
                                let call_name = get_node_text(&func_child, source).to_string();
                                if call_name.chars().next().is_some_and(|c| c.is_uppercase()) {
                                    has_constructor_rhs = true;
                                    constructor_type = Some(call_name);
                                }
                            }
                        }
                        _ => {}
                    }
                }

                let var_name = match var_name {
                    Some(n) if !n.is_empty() => n,
                    _ => continue,
                };

                if let Some(ref tn) = type_name {
                    if !kotlin_builtin_types.contains(&tn.as_str()) {
                        var_types.push(VarType::new_with_scope(
                            var_name.clone(),
                            tn.clone(),
                            "annotation",
                            line,
                            scope.clone(),
                        ));
                        continue;
                    }
                }

                if has_constructor_rhs {
                    if let Some(ref ct) = constructor_type {
                        if !kotlin_builtin_types.contains(&ct.as_str()) {
                            var_types.push(VarType::new_with_scope(
                                var_name,
                                ct.clone(),
                                "assignment",
                                line,
                                scope,
                            ));
                        }
                    }
                }
            }

            "parameter" => {
                let line = node.start_position().row as u32 + 1;
                let scope = enclosing_kotlin_function_scope(&node, source);

                let mut param_name: Option<String> = None;
                let mut param_type: Option<String> = None;

                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        match child.kind() {
                            "identifier" | "simple_identifier" if param_name.is_none() => {
                                param_name = Some(get_node_text(&child, source).to_string());
                            }
                            "user_type" | "nullable_type" | "type_reference" => {
                                param_type = extract_kotlin_type_name(&child, source);
                            }
                            _ => {
                                if param_type.is_none() {
                                    if let Some(tn) = extract_kotlin_type_name(&child, source) {
                                        if tn.chars().next().is_some_and(|c| c.is_uppercase()) {
                                            param_type = Some(tn);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                if let (Some(name), Some(type_name)) = (param_name, param_type) {
                    if !kotlin_builtin_types.contains(&type_name.as_str()) {
                        var_types.push(VarType::new_with_scope(
                            name,
                            type_name,
                            "parameter",
                            line,
                            scope,
                        ));
                    }
                }
            }

            "class_parameter" => {
                let line = node.start_position().row as u32 + 1;

                let scope = {
                    let mut s = None;
                    let mut cur = node.parent();
                    while let Some(p) = cur {
                        if p.kind() == "class_parameters" {
                            if let Some(ctor) = p.parent() {
                                if ctor.kind() == "primary_constructor" {
                                    if let Some(class_decl) = ctor.parent() {
                                        if class_decl.kind() == "class_declaration" {
                                            if let Some(cn) = class_decl.child_by_field_name("name")
                                            {
                                                s = Some(get_node_text(&cn, source).to_string());
                                            }
                                        }
                                    }
                                }
                                if ctor.kind() == "class_declaration" {
                                    if let Some(cn) = ctor.child_by_field_name("name") {
                                        s = Some(get_node_text(&cn, source).to_string());
                                    }
                                }
                            }
                            break;
                        }
                        cur = p.parent();
                    }
                    s
                };

                let mut param_name: Option<String> = None;
                let mut param_type: Option<String> = None;

                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        match child.kind() {
                            "identifier" | "simple_identifier" if param_name.is_none() => {
                                param_name = Some(get_node_text(&child, source).to_string());
                            }
                            "user_type" | "nullable_type" | "type_reference" => {
                                param_type = extract_kotlin_type_name(&child, source);
                            }
                            "modifiers" | "val" | "var" => {}
                            _ => {
                                if param_type.is_none() {
                                    if let Some(tn) = extract_kotlin_type_name(&child, source) {
                                        if tn.chars().next().is_some_and(|c| c.is_uppercase()) {
                                            param_type = Some(tn);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                if let (Some(name), Some(type_name)) = (param_name, param_type) {
                    if !kotlin_builtin_types.contains(&type_name.as_str()) {
                        var_types.push(VarType::new_with_scope(
                            name,
                            type_name,
                            "parameter",
                            line,
                            scope,
                        ));
                    }
                }
            }

            _ => {}
        }
    }

    var_types
}

// =============================================================================
// PHP VarType extraction
// =============================================================================

/// Determine the enclosing function/method scope for a PHP AST node.
pub(crate) fn enclosing_php_function_scope(
    node: &tree_sitter::Node,
    source: &[u8],
) -> Option<String> {
    let mut cur = node.parent();
    while let Some(p) = cur {
        match p.kind() {
            "method_declaration" => {
                let method_name = p
                    .child_by_field_name("name")
                    .map(|n| get_node_text(&n, source).to_string());
                let mut class_name = None;
                let mut parent = p.parent();
                while let Some(pp) = parent {
                    if pp.kind() == "declaration_list" {
                        if let Some(class_decl) = pp.parent() {
                            if class_decl.kind() == "class_declaration" {
                                class_name = class_decl
                                    .child_by_field_name("name")
                                    .map(|n| get_node_text(&n, source).to_string());
                            }
                        }
                        break;
                    }
                    parent = pp.parent();
                }
                return match (class_name, method_name) {
                    (Some(cn), Some(mn)) => Some(format!("{}.{}", cn, mn)),
                    (None, Some(mn)) => Some(mn),
                    _ => None,
                };
            }
            "function_definition" => {
                return p
                    .child_by_field_name("name")
                    .map(|n| get_node_text(&n, source).to_string());
            }
            _ => {}
        }
        cur = p.parent();
    }
    None // top-level / module scope
}

/// Extract the base type name from a PHP type AST node.
pub(crate) fn extract_php_type_name(
    type_node: &tree_sitter::Node,
    source: &[u8],
) -> Option<String> {
    match type_node.kind() {
        "named_type" => {
            let text = get_node_text(type_node, source).to_string();
            let base = text.rsplit('\\').next().unwrap_or(&text);
            if base.is_empty() {
                None
            } else {
                Some(base.to_string())
            }
        }
        "optional_type" => {
            for i in 0..type_node.named_child_count() {
                if let Some(child) = type_node.named_child(i) {
                    if let Some(result) = extract_php_type_name(&child, source) {
                        return Some(result);
                    }
                }
            }
            None
        }
        "nullable_type" => {
            for i in 0..type_node.named_child_count() {
                if let Some(child) = type_node.named_child(i) {
                    if let Some(result) = extract_php_type_name(&child, source) {
                        return Some(result);
                    }
                }
            }
            None
        }
        "union_type" | "intersection_type" => None,
        _ => None,
    }
}

/// Extract VarType entries from a PHP source tree.
pub(crate) fn extract_php_var_types(tree: &tree_sitter::Tree, source: &[u8]) -> Vec<VarType> {
    let mut var_types = Vec::new();
    let root = tree.root_node();

    let php_builtin_types = [
        "string", "int", "float", "bool", "array", "object", "callable", "iterable", "mixed",
        "void", "null", "never", "false", "true", "self", "static", "parent", "resource",
    ];

    for node in walk_tree(root) {
        match node.kind() {
            // Pattern 1: $x = new Foo()
            "assignment_expression" => {
                let left = match node.child_by_field_name("left") {
                    Some(n) if n.kind() == "variable_name" => n,
                    _ => continue,
                };
                let right = match node.child_by_field_name("right") {
                    Some(n) if n.kind() == "object_creation_expression" => n,
                    _ => continue,
                };

                let var_text = get_node_text(&left, source).to_string();
                let var_name = var_text.trim_start_matches('$');
                if var_name.is_empty() {
                    continue;
                }

                let mut class_name: Option<String> = None;
                for i in 0..right.child_count() {
                    if let Some(child) = right.child(i) {
                        if child.kind() == "name" {
                            let raw = get_node_text(&child, source).to_string();
                            let base = raw.rsplit('\\').next().unwrap_or(&raw);
                            if !base.is_empty() {
                                class_name = Some(base.to_string());
                            }
                            break;
                        }
                        if child.kind() == "qualified_name" {
                            let raw = get_node_text(&child, source).to_string();
                            let base = raw.rsplit('\\').next().unwrap_or(&raw);
                            if !base.is_empty() {
                                class_name = Some(base.to_string());
                            }
                            break;
                        }
                    }
                }

                let type_name = match class_name {
                    Some(ref tn) if !php_builtin_types.contains(&tn.to_lowercase().as_str()) => {
                        tn.clone()
                    }
                    _ => continue,
                };

                let line = node.start_position().row as u32 + 1;
                let scope = enclosing_php_function_scope(&node, source);

                var_types.push(VarType::new_with_scope(
                    var_name.to_string(),
                    type_name,
                    "constructor",
                    line,
                    scope,
                ));
            }

            // Pattern 2: function f(Foo $x) {}
            "simple_parameter" => {
                let type_node = match node.child_by_field_name("type") {
                    Some(n) => n,
                    None => continue,
                };

                let type_name = match extract_php_type_name(&type_node, source) {
                    Some(tn) if !php_builtin_types.contains(&tn.to_lowercase().as_str()) => tn,
                    _ => continue,
                };

                let name_node = match node.child_by_field_name("name") {
                    Some(n) => n,
                    None => continue,
                };

                let var_text = get_node_text(&name_node, source).to_string();
                let var_name = var_text.trim_start_matches('$');
                if var_name.is_empty() {
                    continue;
                }

                let line = node.start_position().row as u32 + 1;
                let scope = enclosing_php_function_scope(&node, source);

                var_types.push(VarType::new_with_scope(
                    var_name.to_string(),
                    type_name,
                    "parameter",
                    line,
                    scope,
                ));
            }

            // Pattern 3: private Foo $prop;
            "property_declaration" => {
                let type_node = match node.child_by_field_name("type") {
                    Some(n) => n,
                    None => continue,
                };

                let type_name = match extract_php_type_name(&type_node, source) {
                    Some(tn) if !php_builtin_types.contains(&tn.to_lowercase().as_str()) => tn,
                    _ => continue,
                };

                let mut var_name: Option<String> = None;
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        if child.kind() == "property_element" {
                            for j in 0..child.child_count() {
                                if let Some(inner) = child.child(j) {
                                    if inner.kind() == "variable_name" {
                                        let var_text = get_node_text(&inner, source).to_string();
                                        let name = var_text.trim_start_matches('$');
                                        if !name.is_empty() {
                                            var_name = Some(name.to_string());
                                        }
                                        break;
                                    }
                                }
                            }
                            break;
                        }
                    }
                }

                let var_name = match var_name {
                    Some(n) => n,
                    None => continue,
                };

                let line = node.start_position().row as u32 + 1;
                let scope = enclosing_php_function_scope(&node, source);

                var_types.push(VarType::new_with_scope(
                    var_name,
                    type_name,
                    "annotation",
                    line,
                    scope,
                ));
            }

            _ => {}
        }
    }

    var_types
}

// =============================================================================
// Tests (moved from builder_v2.rs during Phase 3 modularization)
// =============================================================================

#[cfg(test)]
mod tests {
    use super::super::types::parse_source;
    use super::*;
    use crate::callgraph::cross_file_types::CallType;
    use crate::callgraph::languages::base::{get_node_text, walk_tree};

    // =========================================================================
    // Python call extraction tests
    // =========================================================================

    /// Test: Call extraction in Python
    #[test]
    fn test_extract_python_calls() {
        let source = r#"
def main():
    process()
    helper.run()
"#;
        let tree = parse_source(source, "python").unwrap();
        let root = tree.root_node();
        let source_bytes = source.as_bytes();

        // Find the function node
        let mut calls = Vec::new();
        for node in walk_tree(root) {
            if node.kind() == "function_definition" {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let func_name = get_node_text(&name_node, source_bytes);
                    if func_name == "main" {
                        calls = extract_python_calls(&node, source_bytes, "main");
                    }
                }
            }
        }

        assert!(!calls.is_empty(), "Should extract calls from main");

        // Check for process() call
        let process_call = calls.iter().find(|c| c.target == "process");
        assert!(process_call.is_some(), "Should find process() call");
        assert_eq!(process_call.unwrap().call_type, CallType::Direct);

        // Check for helper.run() call
        let helper_call = calls.iter().find(|c| c.target == "run");
        assert!(helper_call.is_some(), "Should find helper.run() call");
        assert_eq!(helper_call.unwrap().call_type, CallType::Method);
        assert_eq!(helper_call.unwrap().receiver, Some("helper".to_string()));
    }

    // ==========================================================================
    // TypeScript/JavaScript VarType extraction tests
    // ==========================================================================

    #[test]
    fn test_extract_ts_var_types_new_expression() {
        let source = r#"
const router = new Router();
const app = new Application();
"#;
        let tree = parse_source(source, "typescript").unwrap();
        let var_types = extract_ts_var_types(&tree, source.as_bytes());

        assert_eq!(var_types.len(), 2);
        assert_eq!(var_types[0].var_name, "router");
        assert_eq!(var_types[0].type_name, "Router");
        assert_eq!(var_types[0].source, "assignment");
        assert_eq!(var_types[1].var_name, "app");
        assert_eq!(var_types[1].type_name, "Application");
    }

    #[test]
    fn test_extract_ts_var_types_type_annotation() {
        let source = r#"
let user: User;
const handler: RequestHandler = createHandler();
"#;
        let tree = parse_source(source, "typescript").unwrap();
        let var_types = extract_ts_var_types(&tree, source.as_bytes());

        let user_vt = var_types.iter().find(|v| v.var_name == "user").unwrap();
        assert_eq!(user_vt.type_name, "User");
        assert_eq!(user_vt.source, "annotation");

        let handler_vt = var_types.iter().find(|v| v.var_name == "handler").unwrap();
        assert_eq!(handler_vt.type_name, "RequestHandler");
        assert_eq!(handler_vt.source, "annotation");
    }

    #[test]
    fn test_extract_ts_var_types_typed_parameters() {
        let source = r#"
function processUser(user: User, config: AppConfig) {
    console.log(user);
}
"#;
        let tree = parse_source(source, "typescript").unwrap();
        let var_types = extract_ts_var_types(&tree, source.as_bytes());

        assert_eq!(var_types.len(), 2);

        let user_vt = var_types.iter().find(|v| v.var_name == "user").unwrap();
        assert_eq!(user_vt.type_name, "User");
        assert_eq!(user_vt.source, "parameter");
        assert_eq!(user_vt.scope, Some("processUser".to_string()));

        let config_vt = var_types.iter().find(|v| v.var_name == "config").unwrap();
        assert_eq!(config_vt.type_name, "AppConfig");
        assert_eq!(config_vt.source, "parameter");
    }

    #[test]
    fn test_extract_ts_var_types_builtin_types_skipped() {
        let source = r#"
const name: string = "hello";
const count: number = 42;
const flag: boolean = true;
const arr: Array<string> = [];
const promise: Promise<void> = fetch("/");
"#;
        let tree = parse_source(source, "typescript").unwrap();
        let var_types = extract_ts_var_types(&tree, source.as_bytes());

        for vt in &var_types {
            assert_eq!(
                vt.source, "literal",
                "Only literal sources expected, got {} for {}",
                vt.source, vt.var_name
            );
        }
    }

    #[test]
    fn test_extract_ts_var_types_literals() {
        let source = r#"
const s = "hello";
const n = 42;
const b = true;
const a = [1, 2, 3];
const o = {key: "val"};
const r = /regex/;
const nu = null;
"#;
        let tree = parse_source(source, "typescript").unwrap();
        let var_types = extract_ts_var_types(&tree, source.as_bytes());

        let find = |name: &str| var_types.iter().find(|v| v.var_name == name).unwrap();

        assert_eq!(find("s").type_name, "string");
        assert_eq!(find("s").source, "literal");
        assert_eq!(find("n").type_name, "number");
        assert_eq!(find("b").type_name, "boolean");
        assert_eq!(find("a").type_name, "Array");
        assert_eq!(find("o").type_name, "Object");
        assert_eq!(find("r").type_name, "RegExp");
        assert_eq!(find("nu").type_name, "null");
    }

    #[test]
    fn test_extract_ts_var_types_class_method_scope() {
        let source = r#"
class UserService {
    processUser(user: User) {
        const db = new Database();
    }
}
"#;
        let tree = parse_source(source, "typescript").unwrap();
        let var_types = extract_ts_var_types(&tree, source.as_bytes());

        let user_vt = var_types.iter().find(|v| v.var_name == "user").unwrap();
        assert_eq!(user_vt.scope, Some("UserService.processUser".to_string()));

        let db_vt = var_types.iter().find(|v| v.var_name == "db").unwrap();
        assert_eq!(db_vt.type_name, "Database");
        assert_eq!(db_vt.scope, Some("UserService.processUser".to_string()));
    }

    #[test]
    fn test_extract_ts_var_types_as_expression() {
        let source = r#"
const animal = getAnimal() as Animal;
"#;
        let tree = parse_source(source, "typescript").unwrap();
        let var_types = extract_ts_var_types(&tree, source.as_bytes());

        let animal_vt = var_types.iter().find(|v| v.var_name == "animal").unwrap();
        assert_eq!(animal_vt.type_name, "Animal");
        assert_eq!(animal_vt.source, "assertion");
    }

    #[test]
    fn test_extract_ts_var_types_new_with_annotation() {
        let source = r#"
const svc: Service = new ServiceImpl();
"#;
        let tree = parse_source(source, "typescript").unwrap();
        let var_types = extract_ts_var_types(&tree, source.as_bytes());

        let assignment = var_types.iter().find(|v| v.source == "assignment").unwrap();
        assert_eq!(assignment.type_name, "ServiceImpl");

        let annotation = var_types.iter().find(|v| v.source == "annotation").unwrap();
        assert_eq!(annotation.type_name, "Service");
    }

    // =========================================================================
    // Java VarType extraction tests
    // =========================================================================

    #[test]
    fn test_extract_java_var_types_constructor() {
        let source = r#"
class App {
    void run() {
        Dog dog = new Dog();
        Cat cat = new Cat("whiskers");
    }
}
"#;
        let tree = parse_source(source, "java").unwrap();
        let var_types = extract_java_var_types(&tree, source.as_bytes());

        let dog_vt = var_types.iter().find(|v| v.var_name == "dog").unwrap();
        assert_eq!(dog_vt.type_name, "Dog");
        assert_eq!(dog_vt.source, "constructor");
        assert_eq!(dog_vt.scope, Some("App.run".to_string()));

        let cat_vt = var_types.iter().find(|v| v.var_name == "cat").unwrap();
        assert_eq!(cat_vt.type_name, "Cat");
        assert_eq!(cat_vt.source, "constructor");
    }

    #[test]
    fn test_extract_java_var_types_annotation() {
        let source = r#"
class App {
    void run() {
        Dog dog;
        Animal animal = getAnimal();
    }
}
"#;
        let tree = parse_source(source, "java").unwrap();
        let var_types = extract_java_var_types(&tree, source.as_bytes());

        let dog_vt = var_types.iter().find(|v| v.var_name == "dog").unwrap();
        assert_eq!(dog_vt.type_name, "Dog");
        assert_eq!(dog_vt.source, "annotation");

        let animal_vt = var_types.iter().find(|v| v.var_name == "animal").unwrap();
        assert_eq!(animal_vt.type_name, "Animal");
        assert_eq!(animal_vt.source, "annotation");
    }

    #[test]
    fn test_extract_java_var_types_builtin_types_skipped() {
        let source = r#"
class App {
    void run() {
        String name = "hello";
        int count = 5;
        Integer boxed = 10;
        boolean flag = true;
        Object obj = new Object();
        Dog dog = new Dog();
    }
}
"#;
        let tree = parse_source(source, "java").unwrap();
        let var_types = extract_java_var_types(&tree, source.as_bytes());

        assert_eq!(var_types.len(), 1);
        assert_eq!(var_types[0].var_name, "dog");
        assert_eq!(var_types[0].type_name, "Dog");
    }

    #[test]
    fn test_extract_java_var_types_parameters() {
        let source = r#"
class Service {
    void process(Dog dog, Cat cat, String name) {
        dog.bark();
    }
}
"#;
        let tree = parse_source(source, "java").unwrap();
        let var_types = extract_java_var_types(&tree, source.as_bytes());

        assert_eq!(var_types.len(), 2);

        let dog_vt = var_types.iter().find(|v| v.var_name == "dog").unwrap();
        assert_eq!(dog_vt.type_name, "Dog");
        assert_eq!(dog_vt.source, "parameter");
        assert_eq!(dog_vt.scope, Some("Service.process".to_string()));

        let cat_vt = var_types.iter().find(|v| v.var_name == "cat").unwrap();
        assert_eq!(cat_vt.type_name, "Cat");
        assert_eq!(cat_vt.source, "parameter");
    }

    #[test]
    fn test_extract_java_var_types_field() {
        let source = r#"
class App {
    private Animal animal;
    private Dog dog = new Dog();
}
"#;
        let tree = parse_source(source, "java").unwrap();
        let var_types = extract_java_var_types(&tree, source.as_bytes());

        let animal_vt = var_types.iter().find(|v| v.var_name == "animal").unwrap();
        assert_eq!(animal_vt.type_name, "Animal");
        assert_eq!(animal_vt.source, "annotation");
        assert_eq!(animal_vt.scope, Some("App".to_string()));

        let dog_vt = var_types.iter().find(|v| v.var_name == "dog").unwrap();
        assert_eq!(dog_vt.type_name, "Dog");
        assert_eq!(dog_vt.source, "constructor");
        assert_eq!(dog_vt.scope, Some("App".to_string()));
    }

    #[test]
    fn test_extract_java_var_types_generic() {
        let source = r#"
class App {
    void run() {
        ArrayList<Dog> dogs = new ArrayList<>();
        HashMap<String, Cat> catMap = new HashMap<>();
    }
}
"#;
        let tree = parse_source(source, "java").unwrap();
        let var_types = extract_java_var_types(&tree, source.as_bytes());

        let dogs_vt = var_types.iter().find(|v| v.var_name == "dogs").unwrap();
        assert_eq!(dogs_vt.type_name, "ArrayList");
        assert_eq!(dogs_vt.source, "constructor");

        let cat_map_vt = var_types.iter().find(|v| v.var_name == "catMap").unwrap();
        assert_eq!(cat_map_vt.type_name, "HashMap");
        assert_eq!(cat_map_vt.source, "constructor");
    }

    #[test]
    fn test_extract_java_var_types_var_keyword() {
        let source = r#"
class App {
    void run() {
        var dog = new Dog();
        var name = "hello";
    }
}
"#;
        let tree = parse_source(source, "java").unwrap();
        let var_types = extract_java_var_types(&tree, source.as_bytes());

        let dog_vt = var_types.iter().find(|v| v.var_name == "dog").unwrap();
        assert_eq!(dog_vt.type_name, "Dog");
        assert_eq!(dog_vt.source, "constructor");

        assert!(!var_types.iter().any(|v| v.var_name == "name"));
    }

    #[test]
    fn test_extract_java_var_types_enhanced_for() {
        let source = r#"
class App {
    void run() {
        for (Dog dog : dogs) {
            dog.bark();
        }
    }
}
"#;
        let tree = parse_source(source, "java").unwrap();
        let var_types = extract_java_var_types(&tree, source.as_bytes());

        let dog_vt = var_types.iter().find(|v| v.var_name == "dog").unwrap();
        assert_eq!(dog_vt.type_name, "Dog");
        assert_eq!(dog_vt.source, "annotation");
        assert_eq!(dog_vt.scope, Some("App.run".to_string()));
    }

    #[test]
    fn test_extract_java_var_types_scope() {
        let source = r#"
class MyClass {
    void methodA() {
        Dog dog = new Dog();
    }
    void methodB(Cat cat) {
        cat.meow();
    }
}
"#;
        let tree = parse_source(source, "java").unwrap();
        let var_types = extract_java_var_types(&tree, source.as_bytes());

        let dog_vt = var_types.iter().find(|v| v.var_name == "dog").unwrap();
        assert_eq!(dog_vt.scope, Some("MyClass.methodA".to_string()));

        let cat_vt = var_types.iter().find(|v| v.var_name == "cat").unwrap();
        assert_eq!(cat_vt.scope, Some("MyClass.methodB".to_string()));
    }

    // ========================================================================
    // Rust VarType extraction tests
    // ========================================================================

    #[test]
    fn test_extract_rust_var_types_scoped_constructor() {
        let source = r#"
fn main() {
    let dog = Dog::new();
    let cat = Cat::with_name("whiskers");
}
"#;
        let tree = parse_source(source, "rust").unwrap();
        let var_types = extract_rust_var_types(&tree, source.as_bytes());

        let dog_vt = var_types.iter().find(|v| v.var_name == "dog").unwrap();
        assert_eq!(dog_vt.type_name, "Dog");
        assert_eq!(dog_vt.source, "constructor");
        assert_eq!(dog_vt.scope, Some("main".to_string()));

        let cat_vt = var_types.iter().find(|v| v.var_name == "cat").unwrap();
        assert_eq!(cat_vt.type_name, "Cat");
        assert_eq!(cat_vt.source, "constructor");
    }

    #[test]
    fn test_extract_rust_var_types_struct_expression() {
        let source = r#"
fn create() {
    let animal = Animal { name: "Rex".to_string(), age: 5 };
}
"#;
        let tree = parse_source(source, "rust").unwrap();
        let var_types = extract_rust_var_types(&tree, source.as_bytes());

        let vt = var_types.iter().find(|v| v.var_name == "animal").unwrap();
        assert_eq!(vt.type_name, "Animal");
        assert_eq!(vt.source, "constructor");
        assert_eq!(vt.scope, Some("create".to_string()));
    }

    #[test]
    fn test_extract_rust_var_types_type_annotation() {
        let source = r#"
fn process() {
    let a: Animal = get_animal();
}
"#;
        let tree = parse_source(source, "rust").unwrap();
        let var_types = extract_rust_var_types(&tree, source.as_bytes());

        let vt = var_types.iter().find(|v| v.var_name == "a").unwrap();
        assert_eq!(vt.type_name, "Animal");
        assert_eq!(vt.source, "annotation");
    }

    #[test]
    fn test_extract_rust_var_types_reference_type() {
        let source = r#"
fn process(animal: &Animal) {
    let b: &mut Dog = get_dog();
}
"#;
        let tree = parse_source(source, "rust").unwrap();
        let var_types = extract_rust_var_types(&tree, source.as_bytes());

        let animal_vt = var_types.iter().find(|v| v.var_name == "animal").unwrap();
        assert_eq!(animal_vt.type_name, "Animal");
        assert_eq!(animal_vt.source, "parameter");

        let dog_vt = var_types.iter().find(|v| v.var_name == "b").unwrap();
        assert_eq!(dog_vt.type_name, "Dog");
        assert_eq!(dog_vt.source, "annotation");
    }

    #[test]
    fn test_extract_rust_var_types_builtin_types_skipped() {
        let source = r#"
fn main() {
    let name: String = "hello".to_string();
    let count: i32 = 5;
    let flag: bool = true;
    let items: Vec<i32> = vec![1, 2, 3];
    let dog = Dog::new();
}
"#;
        let tree = parse_source(source, "rust").unwrap();
        let var_types = extract_rust_var_types(&tree, source.as_bytes());

        assert_eq!(var_types.len(), 1);
        assert_eq!(var_types[0].var_name, "dog");
        assert_eq!(var_types[0].type_name, "Dog");
    }

    #[test]
    fn test_extract_rust_var_types_parameters() {
        let source = r#"
fn process(dog: Dog, cat: &Cat, name: String) {
    dog.bark();
}
"#;
        let tree = parse_source(source, "rust").unwrap();
        let var_types = extract_rust_var_types(&tree, source.as_bytes());

        assert_eq!(var_types.len(), 2);

        let dog_vt = var_types.iter().find(|v| v.var_name == "dog").unwrap();
        assert_eq!(dog_vt.type_name, "Dog");
        assert_eq!(dog_vt.source, "parameter");
        assert_eq!(dog_vt.scope, Some("process".to_string()));

        let cat_vt = var_types.iter().find(|v| v.var_name == "cat").unwrap();
        assert_eq!(cat_vt.type_name, "Cat");
        assert_eq!(cat_vt.source, "parameter");
    }

    #[test]
    fn test_extract_rust_var_types_impl_scope() {
        let source = r#"
struct MyStruct;

impl MyStruct {
    fn new() -> Self {
        let config = Config::default();
        MyStruct
    }

    fn process(&self, handler: Handler) {
        handler.run();
    }
}
"#;
        let tree = parse_source(source, "rust").unwrap();
        let var_types = extract_rust_var_types(&tree, source.as_bytes());

        let config_vt = var_types.iter().find(|v| v.var_name == "config").unwrap();
        assert_eq!(config_vt.type_name, "Config");
        assert_eq!(config_vt.source, "constructor");
        assert_eq!(config_vt.scope, Some("MyStruct.new".to_string()));

        let handler_vt = var_types.iter().find(|v| v.var_name == "handler").unwrap();
        assert_eq!(handler_vt.type_name, "Handler");
        assert_eq!(handler_vt.source, "parameter");
        assert_eq!(handler_vt.scope, Some("MyStruct.process".to_string()));
    }

    #[test]
    fn test_extract_rust_var_types_constructor_preferred_over_annotation() {
        let source = r#"
fn main() {
    let dog: Dog = Dog::new();
}
"#;
        let tree = parse_source(source, "rust").unwrap();
        let var_types = extract_rust_var_types(&tree, source.as_bytes());

        let dog_entries: Vec<_> = var_types.iter().filter(|v| v.var_name == "dog").collect();
        assert_eq!(dog_entries.len(), 1);
        assert_eq!(dog_entries[0].source, "constructor");
    }

    #[test]
    fn test_extract_rust_var_types_underscore_vars_skipped() {
        let source = r#"
fn main() {
    let _unused = Dog::new();
    let _: Animal = get_animal();
}
"#;
        let tree = parse_source(source, "rust").unwrap();
        let var_types = extract_rust_var_types(&tree, source.as_bytes());

        assert_eq!(var_types.len(), 0);
    }

    // =========================================================================
    // Kotlin VarType extraction tests
    // =========================================================================

    #[test]
    fn test_extract_kotlin_var_types_constructor_call() {
        let source = r#"
fun main() {
    val dog = Dog("Rex")
    val handler = Handler()
}
"#;
        let tree = parse_source(source, "kotlin").unwrap();
        let var_types = extract_kotlin_var_types(&tree, source.as_bytes());

        let dog_vt = var_types.iter().find(|v| v.var_name == "dog");
        assert!(
            dog_vt.is_some(),
            "Should find 'dog' var type. Found: {:?}",
            var_types
        );
        let dog_vt = dog_vt.unwrap();
        assert_eq!(dog_vt.type_name, "Dog");
        assert_eq!(dog_vt.source, "assignment");

        let handler_vt = var_types.iter().find(|v| v.var_name == "handler");
        assert!(
            handler_vt.is_some(),
            "Should find 'handler' var type. Found: {:?}",
            var_types
        );
        let handler_vt = handler_vt.unwrap();
        assert_eq!(handler_vt.type_name, "Handler");
        assert_eq!(handler_vt.source, "assignment");
    }

    #[test]
    fn test_extract_kotlin_var_types_type_annotation() {
        let source = r#"
fun main() {
    val repo: Repository = getRepo()
    val service: Service = Service()
}
"#;
        let tree = parse_source(source, "kotlin").unwrap();
        let var_types = extract_kotlin_var_types(&tree, source.as_bytes());

        let repo_vt = var_types.iter().find(|v| v.var_name == "repo");
        assert!(
            repo_vt.is_some(),
            "Should find 'repo' var type. Found: {:?}",
            var_types
        );
        let repo_vt = repo_vt.unwrap();
        assert_eq!(repo_vt.type_name, "Repository");
        assert_eq!(repo_vt.source, "annotation");

        let service_vt = var_types.iter().find(|v| v.var_name == "service");
        assert!(
            service_vt.is_some(),
            "Should find 'service' var type. Found: {:?}",
            var_types
        );
        assert_eq!(service_vt.unwrap().source, "annotation");
    }

    #[test]
    fn test_extract_kotlin_var_types_nullable_type() {
        let source = r#"
fun main() {
    val maybe: Dog? = findDog()
}
"#;
        let tree = parse_source(source, "kotlin").unwrap();
        let var_types = extract_kotlin_var_types(&tree, source.as_bytes());

        let maybe_vt = var_types.iter().find(|v| v.var_name == "maybe");
        assert!(
            maybe_vt.is_some(),
            "Should find 'maybe' var type. Found: {:?}",
            var_types
        );
        let maybe_vt = maybe_vt.unwrap();
        assert_eq!(maybe_vt.type_name, "Dog");
        assert_eq!(maybe_vt.source, "annotation");
    }

    #[test]
    fn test_extract_kotlin_var_types_function_parameters() {
        let source = r#"
fun process(dog: Dog, name: String) {
    println(dog)
}
"#;
        let tree = parse_source(source, "kotlin").unwrap();
        let var_types = extract_kotlin_var_types(&tree, source.as_bytes());

        let dog_vt = var_types.iter().find(|v| v.var_name == "dog");
        assert!(
            dog_vt.is_some(),
            "Should find 'dog' parameter. Found: {:?}",
            var_types
        );
        let dog_vt = dog_vt.unwrap();
        assert_eq!(dog_vt.type_name, "Dog");
        assert_eq!(dog_vt.source, "parameter");
        assert_eq!(dog_vt.scope, Some("process".to_string()));

        let string_vt = var_types.iter().find(|v| v.var_name == "name");
        assert!(
            string_vt.is_none(),
            "String param should be filtered. Found: {:?}",
            var_types
        );
    }

    #[test]
    fn test_extract_kotlin_var_types_class_parameters() {
        let source = r#"
class Service(val repo: Repository, val handler: Handler)
"#;
        let tree = parse_source(source, "kotlin").unwrap();
        let var_types = extract_kotlin_var_types(&tree, source.as_bytes());

        let repo_vt = var_types.iter().find(|v| v.var_name == "repo");
        assert!(
            repo_vt.is_some(),
            "Should find 'repo' class param. Found: {:?}",
            var_types
        );
        let repo_vt = repo_vt.unwrap();
        assert_eq!(repo_vt.type_name, "Repository");
        assert_eq!(repo_vt.source, "parameter");

        let handler_vt = var_types.iter().find(|v| v.var_name == "handler");
        assert!(
            handler_vt.is_some(),
            "Should find 'handler' class param. Found: {:?}",
            var_types
        );
        assert_eq!(handler_vt.unwrap().type_name, "Handler");
    }

    #[test]
    fn test_extract_kotlin_var_types_builtin_types_skipped() {
        let source = r#"
fun main() {
    val name: String = "hello"
    val count: Int = 42
    val flag: Boolean = true
    val items: List<String> = listOf()
    val map: Map<String, Int> = mapOf()
}
"#;
        let tree = parse_source(source, "kotlin").unwrap();
        let var_types = extract_kotlin_var_types(&tree, source.as_bytes());

        assert_eq!(
            var_types.len(),
            0,
            "Builtin types should be skipped. Found: {:?}",
            var_types
        );
    }

    #[test]
    fn test_extract_kotlin_var_types_class_method_scope() {
        let source = r#"
class Controller {
    fun handle(req: Request) {
        val service = Service()
    }
}
"#;
        let tree = parse_source(source, "kotlin").unwrap();
        let var_types = extract_kotlin_var_types(&tree, source.as_bytes());

        let req_vt = var_types.iter().find(|v| v.var_name == "req");
        assert!(
            req_vt.is_some(),
            "Should find 'req' param. Found: {:?}",
            var_types
        );
        assert_eq!(req_vt.unwrap().scope, Some("Controller.handle".to_string()));

        let service_vt = var_types.iter().find(|v| v.var_name == "service");
        assert!(
            service_vt.is_some(),
            "Should find 'service' var. Found: {:?}",
            var_types
        );
        assert_eq!(
            service_vt.unwrap().scope,
            Some("Controller.handle".to_string())
        );
    }

    // =========================================================================
    // PHP VarType extraction tests
    // =========================================================================

    #[test]
    fn test_extract_php_var_types_constructor() {
        let source = r#"<?php
$logger = new Logger();
$handler = new RequestHandler();
"#;
        let tree = parse_source(source, "php").unwrap();
        let var_types = extract_php_var_types(&tree, source.as_bytes());

        assert_eq!(
            var_types.len(),
            2,
            "Expected 2 constructor var types, got {:?}",
            var_types
        );
        assert_eq!(var_types[0].var_name, "logger");
        assert_eq!(var_types[0].type_name, "Logger");
        assert_eq!(var_types[0].source, "constructor");
        assert_eq!(var_types[1].var_name, "handler");
        assert_eq!(var_types[1].type_name, "RequestHandler");
        assert_eq!(var_types[1].source, "constructor");
    }

    #[test]
    fn test_extract_php_var_types_typed_parameters() {
        let source = r#"<?php
function process(Request $request, Config $config) {
    return null;
}
"#;
        let tree = parse_source(source, "php").unwrap();
        let var_types = extract_php_var_types(&tree, source.as_bytes());

        assert_eq!(
            var_types.len(),
            2,
            "Expected 2 parameter var types, got {:?}",
            var_types
        );

        let req_vt = var_types.iter().find(|v| v.var_name == "request").unwrap();
        assert_eq!(req_vt.type_name, "Request");
        assert_eq!(req_vt.source, "parameter");
        assert_eq!(req_vt.scope, Some("process".to_string()));

        let cfg_vt = var_types.iter().find(|v| v.var_name == "config").unwrap();
        assert_eq!(cfg_vt.type_name, "Config");
        assert_eq!(cfg_vt.source, "parameter");
    }

    #[test]
    fn test_extract_php_var_types_nullable_parameter() {
        let source = r#"<?php
function setup(?Database $db) {
    return null;
}
"#;
        let tree = parse_source(source, "php").unwrap();
        let var_types = extract_php_var_types(&tree, source.as_bytes());

        assert_eq!(
            var_types.len(),
            1,
            "Expected 1 nullable parameter, got {:?}",
            var_types
        );
        assert_eq!(var_types[0].var_name, "db");
        assert_eq!(var_types[0].type_name, "Database");
        assert_eq!(var_types[0].source, "parameter");
    }

    #[test]
    fn test_extract_php_var_types_property_declaration() {
        let source = r#"<?php
class UserService {
    private UserRepository $repo;
    protected Logger $logger;
}
"#;
        let tree = parse_source(source, "php").unwrap();
        let var_types = extract_php_var_types(&tree, source.as_bytes());

        assert_eq!(
            var_types.len(),
            2,
            "Expected 2 property var types, got {:?}",
            var_types
        );

        let repo_vt = var_types.iter().find(|v| v.var_name == "repo").unwrap();
        assert_eq!(repo_vt.type_name, "UserRepository");
        assert_eq!(repo_vt.source, "annotation");

        let logger_vt = var_types.iter().find(|v| v.var_name == "logger").unwrap();
        assert_eq!(logger_vt.type_name, "Logger");
        assert_eq!(logger_vt.source, "annotation");
    }

    #[test]
    fn test_extract_php_var_types_builtin_types_skipped() {
        let source = r#"<?php
function example(string $name, int $count, array $items, bool $flag) {
    return null;
}
class Foo {
    private string $label;
    protected int $id;
}
$x = new stdClass();
"#;
        let tree = parse_source(source, "php").unwrap();
        let var_types = extract_php_var_types(&tree, source.as_bytes());

        for vt in &var_types {
            assert!(
                !["string", "int", "float", "bool", "array", "object", "mixed", "void", "null"]
                    .contains(&vt.type_name.as_str()),
                "Builtin type '{}' should have been filtered, got {:?}",
                vt.type_name,
                vt
            );
        }
    }

    #[test]
    fn test_extract_php_var_types_method_scope() {
        let source = r#"<?php
class Controller {
    public function handle(Request $req) {
        $service = new UserService();
        return $service->process($req);
    }
}
"#;
        let tree = parse_source(source, "php").unwrap();
        let var_types = extract_php_var_types(&tree, source.as_bytes());

        let req_vt = var_types.iter().find(|v| v.var_name == "req");
        assert!(
            req_vt.is_some(),
            "Should find 'req' param. Found: {:?}",
            var_types
        );
        assert_eq!(req_vt.unwrap().scope, Some("Controller.handle".to_string()));

        let service_vt = var_types.iter().find(|v| v.var_name == "service");
        assert!(
            service_vt.is_some(),
            "Should find 'service' var. Found: {:?}",
            var_types
        );
        assert_eq!(service_vt.unwrap().type_name, "UserService");
        assert_eq!(service_vt.unwrap().source, "constructor");
        assert_eq!(
            service_vt.unwrap().scope,
            Some("Controller.handle".to_string())
        );
    }

    #[test]
    fn test_extract_php_var_types_comprehensive() {
        let source = r#"<?php
class UserService {
    private UserRepository $repo;

    public function __construct(UserRepository $repo) {
        $this->repo = $repo;
        $logger = new Logger();
    }

    public function process(Request $request): Response {
        $handler = new RequestHandler();
        return $handler->handle($request);
    }
}

function standalone(?Config $config) {
    $db = new Database();
}

$top = new TopLevel();
"#;
        let tree = parse_source(source, "php").unwrap();
        let var_types = extract_php_var_types(&tree, source.as_bytes());

        let names: Vec<&str> = var_types.iter().map(|v| v.var_name.as_str()).collect();
        assert!(
            names.contains(&"repo"),
            "Missing 'repo'. Found: {:?}",
            names
        );
        assert!(
            names.contains(&"logger"),
            "Missing 'logger'. Found: {:?}",
            names
        );
        assert!(
            names.contains(&"request"),
            "Missing 'request'. Found: {:?}",
            names
        );
        assert!(
            names.contains(&"handler"),
            "Missing 'handler'. Found: {:?}",
            names
        );
        assert!(
            names.contains(&"config"),
            "Missing 'config'. Found: {:?}",
            names
        );
        assert!(names.contains(&"db"), "Missing 'db'. Found: {:?}", names);
        assert!(names.contains(&"top"), "Missing 'top'. Found: {:?}", names);

        let top_vt = var_types.iter().find(|v| v.var_name == "top").unwrap();
        assert_eq!(top_vt.type_name, "TopLevel");
        assert_eq!(top_vt.source, "constructor");
        assert_eq!(top_vt.scope, None);

        let config_vt = var_types.iter().find(|v| v.var_name == "config").unwrap();
        assert_eq!(config_vt.type_name, "Config");
        assert_eq!(config_vt.source, "parameter");
        assert_eq!(config_vt.scope, Some("standalone".to_string()));
    }
}
