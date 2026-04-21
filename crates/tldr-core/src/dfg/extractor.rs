//! DFG extraction from source code
//!
//! Extracts data flow graphs from functions using tree-sitter parsing.
//!
//! # Algorithm
//! 1. Parse source with tree-sitter
//! 2. Find function by name
//! 3. Extract all variable references (defs, uses, updates)
//! 4. Build CFG for the function
//! 5. Apply reaching definitions analysis
//! 6. Connect definitions to uses via edges
//!
//! # Variable Reference Identification (M7 documentation)
//!
//! ## Definitions
//! - Assignment targets: `x = ...`
//! - For loop variables: `for x in ...`
//! - Function parameters: `def f(x):`
//! - With statement variables: `with ... as x:`
//! - Exception handlers: `except E as x:`
//!
//! ## Updates
//! - Augmented assignment: `x += ...`, `x -= ...`
//! - Method calls that mutate: `x.append(...)`, `x.clear()`
//!
//! ## Uses
//! - Expression reads: `y = x + 1`
//! - Function arguments: `f(x)`
//! - Condition checks: `if x:`
//! - Return values: `return x`

use std::collections::{HashMap, HashSet};
use std::path::Path;

use tree_sitter::{Node, Tree};

use crate::ast::parser::parse;
use crate::ast::function_finder::{find_function_node, get_function_body};
use crate::cfg::get_cfg_context;
use crate::dfg::reaching::compute_reaching_definitions;
use crate::types::{CfgInfo, DataflowEdge, DfgInfo, Language, RefType, VarRef};
use crate::TldrResult;
use crate::TldrError;

/// Maximum recursion depth for nested structures
const MAX_DEPTH: usize = 50;

/// Extract DFG for a function from source code or file path
///
/// # Arguments
/// * `source_or_path` - Either source code string or path to a file
/// * `function_name` - Name of the function to extract DFG for
/// * `language` - Programming language
///
/// # Returns
/// * `Ok(DfgInfo)` - DFG with variable refs, edges, and variable list
/// * `Err(FunctionNotFound)` - If function doesn't exist
///
/// # Example
/// ```ignore
/// use tldr_core::dfg::get_dfg_context;
/// use tldr_core::Language;
///
/// let dfg = get_dfg_context("def foo(x): return x + 1", "foo", Language::Python)?;
/// assert_eq!(dfg.function, "foo");
/// assert!(dfg.variables.contains(&"x".to_string()));
/// ```
pub fn get_dfg_context(
    source_or_path: &str,
    function_name: &str,
    language: Language,
) -> TldrResult<DfgInfo> {
    // Determine if input is a file path or source code
    let (tree, source) = if Path::new(source_or_path).exists() {
        // Read file content
        let source = std::fs::read_to_string(Path::new(source_or_path))
            .map_err(crate::TldrError::IoError)?;
        // Parse with the provided language (not detected from extension)
        let tree = parse(&source, language)?;
        (tree, source)
    } else {
        let tree = parse(source_or_path, language)?;
        (tree, source_or_path.to_string())
    };

    // Extract DFG from the parsed tree
    extract_dfg_from_tree(&tree, &source, function_name, language)
}

/// Extract DFG from a parsed tree
fn extract_dfg_from_tree(
    tree: &Tree,
    source: &str,
    function_name: &str,
    language: Language,
) -> TldrResult<DfgInfo> {
    let root = tree.root_node();

    // Find the function node
    let func_node = find_function_node(root, function_name, language, source);

    match func_node {
        Some(node) => build_dfg_for_function(node, function_name, source, language),
        None => Err(TldrError::function_not_found(function_name)),
    }
}

/// Build DFG for a function node
fn build_dfg_for_function(
    func_node: Node,
    function_name: &str,
    source: &str,
    language: Language,
) -> TldrResult<DfgInfo> {
    let mut builder = DfgBuilder::new(function_name.to_string(), source, language);

    // First, extract function parameters as definitions
    builder.extract_parameters(func_node)?;

    // Get the function body and extract all variable references
    let body_node = get_function_body(func_node, language);
    if let Some(body) = body_node {
        builder.extract_refs_from_node(body, 0)?;
    }

    // Get CFG for reaching definitions analysis
    let cfg = get_cfg_context(source, function_name, language)?;

    // Build def-use chains using reaching definitions
    builder.build_def_use_chains(&cfg)?;

    builder.finalize()
}

/// Builder for constructing DFG
struct DfgBuilder<'a> {
    function_name: String,
    source: &'a str,
    language: Language,
    refs: Vec<VarRef>,
    variables: HashSet<String>,
}

impl<'a> DfgBuilder<'a> {
    fn new(function_name: String, source: &'a str, language: Language) -> Self {
        Self {
            function_name,
            source,
            language,
            refs: Vec::new(),
            variables: HashSet::new(),
        }
    }

    /// Extract function parameters as definitions
    fn extract_parameters(&mut self, func_node: Node) -> TldrResult<()> {
        let params_node = match self.language {
            Language::Python => func_node.child_by_field_name("parameters"),
            Language::TypeScript | Language::JavaScript => func_node.child_by_field_name("parameters"),
            Language::Go => func_node.child_by_field_name("parameters"),
            Language::Rust => func_node.child_by_field_name("parameters"),
            Language::Java => func_node.child_by_field_name("parameters"),
            Language::C | Language::Cpp => func_node.child_by_field_name("declarator")
                .and_then(|d| d.child_by_field_name("parameters")),
            Language::Ruby => func_node.child_by_field_name("parameters"),
            Language::Php => func_node.child_by_field_name("parameters"),
            Language::CSharp => func_node.child_by_field_name("parameters"),
            Language::Kotlin => {
                // Kotlin: function_declaration uses function_value_parameters (not "parameters" field)
                func_node.child_by_field_name("parameters").or_else(|| {
                    (0..func_node.child_count())
                        .filter_map(|i| func_node.child(i))
                        .find(|child| child.kind() == "function_value_parameters")
                })
            }
            Language::Scala => func_node.child_by_field_name("parameters"),
            Language::Lua | Language::Luau => func_node.child_by_field_name("parameters"),
            Language::Swift => None, // Swift parameters are direct children (handled below)
            _ => None,
        };

        if let Some(params) = params_node {
            self.extract_params_from_node(params)?;
        }

        // Elixir: parameters are identifiers inside the def call's arguments
        // Structure: (call "def" (arguments (call (identifier "foo") (arguments (identifier "x")))))
        if matches!(self.language, Language::Elixir) {
            self.extract_elixir_parameters(func_node)?;
        }

        // OCaml: parameters are "parameter" nodes containing "value_pattern" inside let_binding
        if matches!(self.language, Language::Ocaml) {
            self.extract_ocaml_parameters(func_node)?;
        }

        // Swift: parameters are direct "parameter" children of function_declaration
        // Each parameter has a simple_identifier child (the param name)
        if matches!(self.language, Language::Swift) {
            self.extract_swift_parameters(func_node)?;
        }

        Ok(())
    }

    /// Extract parameter names from a parameters node
    fn extract_params_from_node(&mut self, params_node: Node) -> TldrResult<()> {
        let mut cursor = params_node.walk();

        for child in params_node.children(&mut cursor) {
            self.extract_param_from_child(child);
        }

        Ok(())
    }

    fn extract_param_from_child(&mut self, child: Node) {
        match self.language {
            Language::Python => self.extract_python_param(child),
            Language::TypeScript | Language::JavaScript => self.extract_ts_js_param(child),
            Language::Go => self.extract_go_param(child),
            Language::Rust => self.extract_rust_param(child),
            Language::Java | Language::CSharp => self.extract_java_csharp_param(child),
            Language::Kotlin => self.extract_kotlin_param(child),
            Language::C | Language::Cpp => self.extract_c_cpp_param(child),
            Language::Ruby => self.extract_ruby_param(child),
            Language::Php => self.extract_php_param(child),
            Language::Lua | Language::Luau => self.extract_lua_param(child),
            Language::Swift => self.extract_swift_param(child),
            _ => {}
        }
    }

    fn extract_python_param(&mut self, child: Node) {
        if child.kind() == "identifier" {
            self.add_ref_from_node(child, RefType::Definition);
            return;
        }
        if child.kind() != "typed_parameter" && child.kind() != "default_parameter" {
            return;
        }
        if let Some(name_node) = child.child_by_field_name("name") {
            self.add_ref_from_node(name_node, RefType::Definition);
            return;
        }
        if let Some(identifier) = first_child_of_kind(child, "identifier") {
            self.add_ref_from_node(identifier, RefType::Definition);
        }
    }

    fn extract_ts_js_param(&mut self, child: Node) {
        if child.kind() != "identifier" && child.kind() != "required_parameter" {
            return;
        }
        if let Some(pattern) = child.child_by_field_name("pattern") {
            self.add_ref_from_node(pattern, RefType::Definition);
        } else if child.kind() == "identifier" {
            self.add_ref_from_node(child, RefType::Definition);
        }
    }

    fn extract_go_param(&mut self, child: Node) {
        if child.kind() != "parameter_declaration" {
            return;
        }
        let mut inner = child.walk();
        for inner_child in child.children(&mut inner) {
            if inner_child.kind() == "identifier" {
                self.add_ref_from_node(inner_child, RefType::Definition);
            }
        }
    }

    fn extract_rust_param(&mut self, child: Node) {
        if child.kind() != "parameter" {
            return;
        }
        if let Some(pattern) = child.child_by_field_name("pattern") {
            if pattern.kind() == "identifier" {
                self.add_ref_from_node(pattern, RefType::Definition);
            }
        }
    }

    fn extract_java_csharp_param(&mut self, child: Node) {
        if child.kind() != "formal_parameter"
            && child.kind() != "spread_parameter"
            && child.kind() != "parameter"
        {
            return;
        }
        if let Some(name) = child.child_by_field_name("name") {
            if name.kind() == "identifier" {
                self.add_ref_from_node(name, RefType::Definition);
            }
        }
    }

    fn extract_kotlin_param(&mut self, child: Node) {
        if child.kind() != "parameter" {
            return;
        }
        if let Some(name) = child.child_by_field_name("name") {
            if name.kind() == "identifier" {
                self.add_ref_from_node(name, RefType::Definition);
                return;
            }
        }
        if let Some(identifier) = first_child_of_kind(child, "identifier") {
            self.add_ref_from_node(identifier, RefType::Definition);
        }
    }

    fn extract_c_cpp_param(&mut self, child: Node) {
        if child.kind() != "parameter_declaration" {
            return;
        }
        let Some(declarator) = child.child_by_field_name("declarator") else {
            return;
        };
        if declarator.kind() == "identifier" {
            self.add_ref_from_node(declarator, RefType::Definition);
            return;
        }
        if declarator.kind() == "pointer_declarator" {
            if let Some(identifier) = first_child_of_kind(declarator, "identifier") {
                self.add_ref_from_node(identifier, RefType::Definition);
            }
        }
    }

    fn extract_ruby_param(&mut self, child: Node) {
        if child.kind() == "identifier" {
            self.add_ref_from_node(child, RefType::Definition);
            return;
        }
        if child.kind() != "optional_parameter"
            && child.kind() != "keyword_parameter"
            && child.kind() != "splat_parameter"
            && child.kind() != "hash_splat_parameter"
        {
            return;
        }
        if let Some(name) = child.child_by_field_name("name") {
            if name.kind() == "identifier" {
                self.add_ref_from_node(name, RefType::Definition);
            }
        }
    }

    fn extract_php_param(&mut self, child: Node) {
        if child.kind() != "simple_parameter" && child.kind() != "variadic_parameter" {
            return;
        }
        if let Some(name) = child.child_by_field_name("name") {
            self.add_ref_from_node(name, RefType::Definition);
        }
    }

    fn extract_lua_param(&mut self, child: Node) {
        if child.kind() == "identifier" {
            self.add_ref_from_node(child, RefType::Definition);
        }
    }

    fn extract_swift_param(&mut self, child: Node) {
        if child.kind() == "simple_identifier" {
            self.add_ref_from_node(child, RefType::Definition);
            return;
        }
        if child.kind() != "parameter" {
            return;
        }
        if let Some(identifier) = first_child_of_kind(child, "simple_identifier") {
            self.add_ref_from_node(identifier, RefType::Definition);
        }
    }

    /// Add a variable reference from an AST node
    fn add_ref_from_node(&mut self, node: Node, ref_type: RefType) {
        let name = node.utf8_text(self.source.as_bytes()).unwrap_or("").to_string();
        if name.is_empty() || is_keyword(&name, self.language) {
            return;
        }

        let line = node.start_position().row as u32 + 1; // 1-indexed
        let column = node.start_position().column as u32;

        self.variables.insert(name.clone());
        self.refs.push(VarRef {
            name,
            ref_type,
            line,
            column,
            context: None,
            group_id: None,
        });
    }

    /// Extract all variable references from an AST node
    ///
    /// This is the core multi-language dispatch. Each language has different
    /// AST node kinds for assignments, declarations, and loops. We match
    /// on all known kinds and delegate to language-specific processing.
    fn extract_refs_from_node(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        if depth > MAX_DEPTH {
            return Ok(());
        }

        match node.kind() {
            // =================================================================
            // Python/Ruby assignment statements (Swift handled separately below)
            // =================================================================
            "assignment" if !matches!(self.language, Language::Swift) => {
                self.process_assignment(node, depth)?;
            }

            // Python expression_statement (may wrap assignment or other exprs)
            "expression_statement" => {
                self.process_expression_statement(node, depth)?;
            }

            // Python augmented assignment: x += 1
            "augmented_assignment" => {
                self.process_augmented_assignment(node)?;
            }

            // =================================================================
            // TypeScript/JavaScript declarations + Lua/Luau local declarations
            // =================================================================
            // JS/TS: let x = ...; const x = ...; var x = ...;
            // Lua/Luau: local x = ...
            "lexical_declaration" | "variable_declaration" => {
                match self.language {
                    Language::Lua | Language::Luau => {
                        self.process_lua_local_declaration(node, depth)?;
                    }
                    _ => {
                        self.process_js_ts_declaration(node, depth)?;
                    }
                }
            }

            // =================================================================
            // TypeScript/JavaScript/Java/C/C++/Rust assignment expressions
            // =================================================================
            "assignment_expression" => {
                self.process_c_style_assignment(node, depth)?;
            }

            // =================================================================
            // TypeScript/JavaScript/Java/C/C++ augmented assignment
            // =================================================================
            "augmented_assignment_expression" => {
                self.process_c_style_augmented_assignment(node, depth)?;
            }

            // =================================================================
            // Rust let declarations: let x = ...; let mut x = ...;
            // =================================================================
            "let_declaration" => {
                self.process_rust_let(node, depth)?;
            }

            // =================================================================
            // Go short var declaration: x := ...
            // =================================================================
            "short_var_declaration" => {
                self.process_go_short_var(node, depth)?;
            }

            // Go var declaration: var x = ...
            "var_declaration" => {
                self.process_go_var_declaration(node, depth)?;
            }

            // Go/Lua/Luau assignment statement: x = ...; x += ...
            "assignment_statement" => {
                match self.language {
                    Language::Lua | Language::Luau => {
                        self.process_lua_assignment_statement(node, depth)?;
                    }
                    _ => {
                        self.process_go_assignment(node, depth)?;
                    }
                }
            }

            // =================================================================
            // Java/C# local variable declaration: int x = ...;
            // =================================================================
            "local_variable_declaration" => {
                self.process_java_local_var(node, depth)?;
            }

            // =================================================================
            // C/C++ declaration: int x = ...;
            // =================================================================
            "declaration" if matches!(self.language, Language::C | Language::Cpp) => {
                self.process_c_declaration(node, depth)?;
            }

            // =================================================================
            // Ruby assignment: x = ...
            // Ruby uses "assignment" (same as Python, handled above)
            // =================================================================
            // Ruby operator_assignment: x += ...
            "operator_assignment" => {
                self.process_augmented_assignment(node)?;
            }

            // =================================================================
            // Kotlin/Swift property declaration: val x = ...; var x = ...; let x = ...
            // =================================================================
            "property_declaration" if matches!(self.language, Language::Kotlin | Language::Swift) => {
                match self.language {
                    Language::Swift => self.process_swift_property(node, depth)?,
                    _ => self.process_kotlin_property(node, depth)?,
                }
            }

            // =================================================================
            // Swift assignment: z = z + 1
            // AST: assignment -> directly_assignable_expression -> simple_identifier, =, expression
            // =================================================================
            "assignment" if matches!(self.language, Language::Swift) => {
                self.process_swift_assignment(node, depth)?;
            }

            // =================================================================
            // Scala val/var definitions
            // =================================================================
            "val_definition" | "var_definition" => {
                self.process_scala_val_var(node, depth)?;
            }

            // =================================================================
            // Elixir match operator: x = ... (pattern matching assignment)
            // Elixir grammar uses binary_operator with "=" for pattern matching
            // =================================================================
            "match_operator" if matches!(self.language, Language::Elixir) => {
                self.process_elixir_match(node, depth)?;
            }

            "binary_operator" if matches!(self.language, Language::Elixir) => {
                // Check if this is an assignment (= operator)
                let is_match = node.children(&mut node.walk())
                    .any(|c| {
                        !c.is_named() && c.utf8_text(self.source.as_bytes()).unwrap_or("") == "="
                    });
                if is_match {
                    self.process_elixir_match(node, depth)?;
                } else {
                    // Other binary operators: recurse into children for uses
                    let mut cursor = node.walk();
                    for child in node.children(&mut cursor) {
                        self.extract_refs_from_node(child, depth + 1)?;
                    }
                }
            }

            // =================================================================
            // OCaml let bindings: let x = ... in ...
            // =================================================================
            "let_expression" if matches!(self.language, Language::Ocaml) => {
                self.process_ocaml_let_expression(node, depth)?;
            }

            "value_definition" if matches!(self.language, Language::Ocaml) => {
                self.process_ocaml_value_definition(node, depth)?;
            }

            "let_binding" if matches!(self.language, Language::Ocaml) => {
                self.process_ocaml_let_binding(node, depth)?;
            }

            // =================================================================
            // For loops (loop variable is a definition)
            // =================================================================
            // Python: for x in items:
            // Go: for ... { }
            "for_statement" => {
                self.process_for_loop(node, depth)?;
            }

            // Python/JS: for x in items / for (x in obj)
            "for_in_statement" => {
                self.process_for_loop(node, depth)?;
            }

            // Rust: for x in items { }
            "for_expression" => {
                self.process_rust_for(node, depth)?;
            }

            // JS/TS: for (const x of items) { }
            "for_of_statement" => {
                self.process_js_for_of(node, depth)?;
            }

            // Java: for (int x : items) { }
            "enhanced_for_statement" => {
                self.process_java_enhanced_for(node, depth)?;
            }

            // PHP: foreach ($arr as $key => $val) { }
            "foreach_statement" => {
                self.process_php_foreach(node, depth)?;
            }

            // Go: for i, v := range items { }
            "range_clause" => {
                self.process_go_range(node, depth)?;
            }

            // Ruby: for x in items do ... end
            "for" if matches!(self.language, Language::Ruby) => {
                self.process_for_loop(node, depth)?;
            }

            // =================================================================
            // With statements (Python-specific)
            // =================================================================
            "with_statement" => {
                self.process_with_statement(node, depth)?;
            }

            // =================================================================
            // Exception handlers (multi-language)
            // =================================================================
            "except_clause" | "catch_clause" => {
                self.process_exception_handler(node, depth)?;
            }

            // =================================================================
            // Identifiers (potential uses) - all languages
            // =================================================================
            "identifier" => {
                let name = node.utf8_text(self.source.as_bytes()).unwrap_or("");
                if !name.is_empty() && !is_keyword(name, self.language) && self.is_use_context(node) {
                    self.add_ref_from_node(node, RefType::Use);
                }
            }

            // PHP variable names: $x
            "variable_name" if matches!(self.language, Language::Php) => {
                let name = node.utf8_text(self.source.as_bytes()).unwrap_or("");
                if !name.is_empty() && self.is_use_context(node) {
                    self.add_ref_from_node(node, RefType::Use);
                }
            }

            // OCaml: value_name is used instead of identifier for variable names
            "value_name" if matches!(self.language, Language::Ocaml) => {
                let name = node.utf8_text(self.source.as_bytes()).unwrap_or("");
                if !name.is_empty() && !is_keyword(name, self.language) && self.is_ocaml_use_context(node) {
                    self.add_ref_from_node(node, RefType::Use);
                }
            }

            // Swift: simple_identifier is used instead of identifier
            "simple_identifier" if matches!(self.language, Language::Swift) => {
                let name = node.utf8_text(self.source.as_bytes()).unwrap_or("");
                if !name.is_empty() && !is_keyword(name, self.language) && self.is_swift_use_context(node) {
                    self.add_ref_from_node(node, RefType::Use);
                }
            }

            // =================================================================
            // Recurse into other nodes
            // =================================================================
            _ => {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    self.extract_refs_from_node(child, depth + 1)?;
                }
            }
        }

        Ok(())
    }

    /// Process a Python expression_statement (may wrap assignment, augmented_assignment, etc.)
    fn process_expression_statement(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.extract_refs_from_node(child, depth + 1)?;
        }
        Ok(())
    }

    /// Process an assignment statement (Python "assignment", Ruby "assignment")
    fn process_assignment(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        // assignment has "left" and "right" fields
        if let Some(left) = node.child_by_field_name("left") {
            self.extract_assignment_targets(left)?;
        }

        // Process the right side for uses
        if let Some(right) = node.child_by_field_name("right") {
            self.extract_refs_from_node(right, depth + 1)?;
        }

        Ok(())
    }

    /// Extract assignment targets (definitions)
    fn extract_assignment_targets(&mut self, target: Node) -> TldrResult<()> {
        match target.kind() {
            "identifier" => {
                self.add_ref_from_node(target, RefType::Definition);
            }
            // Python unpacking
            "tuple" | "list" | "pattern_list" => {
                let mut cursor = target.walk();
                for child in target.children(&mut cursor) {
                    self.extract_assignment_targets(child)?;
                }
            }
            // TS/JS destructuring patterns
            "object_pattern" | "array_pattern" => {
                let mut cursor = target.walk();
                for child in target.children(&mut cursor) {
                    if child.kind() == "identifier"
                        || child.kind() == "shorthand_property_identifier_pattern"
                        || child.kind() == "shorthand_property_identifier"
                    {
                        self.add_ref_from_node(child, RefType::Definition);
                    } else {
                        self.extract_assignment_targets(child)?;
                    }
                }
            }
            // Python: x.attr = ...
            "attribute" => {
                if let Some(obj) = target.child_by_field_name("object") {
                    if obj.kind() == "identifier" {
                        self.add_ref_from_node(obj, RefType::Update);
                    }
                }
            }
            // TS/JS/Java: x.field = ...
            "member_expression" => {
                if let Some(obj) = target.child_by_field_name("object") {
                    if obj.kind() == "identifier" {
                        self.add_ref_from_node(obj, RefType::Update);
                    }
                }
            }
            // Go: x.field = ...
            "selector_expression" => {
                if let Some(operand) = target.child_by_field_name("operand") {
                    if operand.kind() == "identifier" {
                        self.add_ref_from_node(operand, RefType::Update);
                    }
                }
            }
            // Rust: x.field = ...
            "field_expression" => {
                if let Some(value) = target.child_by_field_name("value") {
                    if value.kind() == "identifier" {
                        self.add_ref_from_node(value, RefType::Update);
                    }
                }
            }
            // Python: x[i] = ...
            "subscript" => {
                if let Some(obj) = target.child_by_field_name("value") {
                    if obj.kind() == "identifier" {
                        self.add_ref_from_node(obj, RefType::Update);
                    }
                }
            }
            // TS/JS/Java: x[i] = ...
            "subscript_expression" => {
                if let Some(obj) = target.child_by_field_name("object") {
                    if obj.kind() == "identifier" {
                        self.add_ref_from_node(obj, RefType::Update);
                    }
                }
            }
            // Go: x[i] = ...
            "index_expression" => {
                if let Some(operand) = target.child_by_field_name("operand") {
                    if operand.kind() == "identifier" {
                        self.add_ref_from_node(operand, RefType::Update);
                    }
                }
            }
            // PHP variable name
            "variable_name" => {
                self.add_ref_from_node(target, RefType::Definition);
            }
            _ => {}
        }

        Ok(())
    }

    /// Process augmented assignment (x += ...)
    fn process_augmented_assignment(&mut self, node: Node) -> TldrResult<()> {
        if let Some(left) = node.child_by_field_name("left") {
            if left.kind() == "identifier" {
                self.add_ref_from_node(left, RefType::Update);
            }
        }

        // The right side contains uses
        if let Some(right) = node.child_by_field_name("right") {
            self.extract_refs_from_node(right, 1)?;
        }

        Ok(())
    }

    /// Process for loop (loop variable is a definition)
    fn process_for_loop(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        // Python: for_statement has "left" (loop var) and "right" (iterable)
        if let Some(left) = node.child_by_field_name("left") {
            self.extract_assignment_targets(left)?;
        }

        // The iterable is a use
        if let Some(right) = node.child_by_field_name("right") {
            self.extract_refs_from_node(right, depth + 1)?;
        }

        // Process the body
        if let Some(body) = node.child_by_field_name("body") {
            self.extract_refs_from_node(body, depth + 1)?;
        }

        Ok(())
    }

    /// Process with statement
    fn process_with_statement(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "with_item" {
                // Process the context expression (use)
                if let Some(value) = child.child_by_field_name("value") {
                    self.extract_refs_from_node(value, depth + 1)?;
                }
                // Process the alias (definition)
                if let Some(alias) = child.child_by_field_name("alias") {
                    if alias.kind() == "identifier" {
                        self.add_ref_from_node(alias, RefType::Definition);
                    }
                }
            }
        }

        // Process the body
        if let Some(body) = node.child_by_field_name("body") {
            self.extract_refs_from_node(body, depth + 1)?;
        }

        Ok(())
    }

    /// Process exception handler
    fn process_exception_handler(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        // except Exception as e: -> e is a definition
        if let Some(name) = node.child_by_field_name("name") {
            if name.kind() == "identifier" {
                self.add_ref_from_node(name, RefType::Definition);
            }
        }

        // Process the body
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "block" {
                self.extract_refs_from_node(child, depth + 1)?;
            }
        }

        Ok(())
    }

    // =====================================================================
    // TypeScript/JavaScript processing
    // =====================================================================

    /// Process JS/TS declaration: let x = ...; const x = ...; var x = ...;
    /// AST: lexical_declaration -> variable_declarator (name, value)
    ///      variable_declaration -> variable_declarator (name, value)
    fn process_js_ts_declaration(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "variable_declarator" {
                // "name" field is the variable name
                if let Some(name_node) = child.child_by_field_name("name") {
                    if name_node.kind() == "identifier" {
                        self.add_ref_from_node(name_node, RefType::Definition);
                    } else {
                        // Could be destructuring pattern
                        self.extract_assignment_targets(name_node)?;
                    }
                }
                // "value" field is the initializer
                if let Some(value) = child.child_by_field_name("value") {
                    self.extract_refs_from_node(value, depth + 1)?;
                }
            }
        }
        Ok(())
    }

    /// Process JS/TS for-of: for (const item of items) { ... }
    fn process_js_for_of(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        // left field: the loop variable declaration or identifier
        if let Some(left) = node.child_by_field_name("left") {
            // Could be a lexical_declaration like "const item"
            if left.kind() == "lexical_declaration" || left.kind() == "variable_declaration" {
                let mut cursor = left.walk();
                for child in left.children(&mut cursor) {
                    if child.kind() == "variable_declarator" {
                        if let Some(name_node) = child.child_by_field_name("name") {
                            if name_node.kind() == "identifier" {
                                self.add_ref_from_node(name_node, RefType::Definition);
                            }
                        }
                    }
                }
            } else if left.kind() == "identifier" {
                self.add_ref_from_node(left, RefType::Definition);
            }
        }

        // right field: the iterable (use)
        if let Some(right) = node.child_by_field_name("right") {
            self.extract_refs_from_node(right, depth + 1)?;
        }

        // body
        if let Some(body) = node.child_by_field_name("body") {
            self.extract_refs_from_node(body, depth + 1)?;
        }

        Ok(())
    }

    // =====================================================================
    // C-style assignment processing (TS/JS/Java/C/C++/Rust)
    // =====================================================================

    /// Process C-style assignment expression: x = ...
    /// Used by TS/JS, Java, C, C++, Rust
    fn process_c_style_assignment(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        if let Some(left) = node.child_by_field_name("left") {
            if left.kind() == "identifier" {
                self.add_ref_from_node(left, RefType::Definition);
            } else {
                // Could be member expression, subscript, etc.
                self.extract_assignment_targets(left)?;
            }
        }

        if let Some(right) = node.child_by_field_name("right") {
            self.extract_refs_from_node(right, depth + 1)?;
        }

        Ok(())
    }

    /// Process C-style augmented assignment: x += ..., x -= ...
    fn process_c_style_augmented_assignment(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        if let Some(left) = node.child_by_field_name("left") {
            if left.kind() == "identifier" {
                self.add_ref_from_node(left, RefType::Update);
            }
        }

        if let Some(right) = node.child_by_field_name("right") {
            self.extract_refs_from_node(right, depth + 1)?;
        }

        Ok(())
    }

    // =====================================================================
    // Rust processing
    // =====================================================================

    /// Process Rust let declaration: let x = ...; let mut x = ...;
    /// AST: let_declaration -> pattern (identifier), value
    fn process_rust_let(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        // "pattern" field contains the binding
        if let Some(pattern) = node.child_by_field_name("pattern") {
            if pattern.kind() == "identifier" {
                self.add_ref_from_node(pattern, RefType::Definition);
            } else if pattern.kind() == "mut_pattern" {
                // let mut x = ... -> mut_pattern has an inner identifier
                let mut cursor = pattern.walk();
                for child in pattern.children(&mut cursor) {
                    if child.kind() == "identifier" {
                        self.add_ref_from_node(child, RefType::Definition);
                        break;
                    }
                }
            } else if pattern.kind() == "tuple_pattern" {
                // let (a, b) = ...
                let mut cursor = pattern.walk();
                for child in pattern.children(&mut cursor) {
                    if child.kind() == "identifier" {
                        self.add_ref_from_node(child, RefType::Definition);
                    }
                }
            }
        }

        // "value" field contains the initializer
        if let Some(value) = node.child_by_field_name("value") {
            self.extract_refs_from_node(value, depth + 1)?;
        }

        Ok(())
    }

    /// Process Rust for expression: for x in items { ... }
    /// AST: for_expression -> pattern, value, body
    fn process_rust_for(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        // "pattern" field: loop variable
        if let Some(pattern) = node.child_by_field_name("pattern") {
            if pattern.kind() == "identifier" {
                self.add_ref_from_node(pattern, RefType::Definition);
            } else if pattern.kind() == "tuple_pattern" {
                let mut cursor = pattern.walk();
                for child in pattern.children(&mut cursor) {
                    if child.kind() == "identifier" {
                        self.add_ref_from_node(child, RefType::Definition);
                    }
                }
            }
        }

        // "value" field: the iterable (use)
        if let Some(value) = node.child_by_field_name("value") {
            self.extract_refs_from_node(value, depth + 1)?;
        }

        // "body" field: loop body
        if let Some(body) = node.child_by_field_name("body") {
            self.extract_refs_from_node(body, depth + 1)?;
        }

        Ok(())
    }

    // =====================================================================
    // Go processing
    // =====================================================================

    /// Process Go short var declaration: x := ...
    /// AST: short_var_declaration -> left (expression_list), right (expression_list)
    fn process_go_short_var(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        if let Some(left) = node.child_by_field_name("left") {
            self.extract_go_lhs_identifiers(left)?;
        }

        if let Some(right) = node.child_by_field_name("right") {
            self.extract_refs_from_node(right, depth + 1)?;
        }

        Ok(())
    }

    /// Process Go var declaration: var x int = 10
    /// AST: var_declaration -> var_spec (name, type, value)
    fn process_go_var_declaration(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "var_spec" {
                // "name" field contains identifier(s)
                if let Some(name) = child.child_by_field_name("name") {
                    if name.kind() == "identifier" {
                        self.add_ref_from_node(name, RefType::Definition);
                    }
                }
                // Also try iterating children for multiple names
                let mut inner_cursor = child.walk();
                for inner_child in child.children(&mut inner_cursor) {
                    if inner_child.kind() == "identifier" {
                        // Check if this is before the type/value (it's a name)
                        let name_text = inner_child.utf8_text(self.source.as_bytes()).unwrap_or("");
                        if !name_text.is_empty() && !is_keyword(name_text, self.language) {
                            // Only add if not already added via field name
                            let already_added = self.refs.iter().any(|r| {
                                r.name == name_text
                                    && r.line == inner_child.start_position().row as u32 + 1
                                    && r.ref_type == RefType::Definition
                            });
                            if !already_added {
                                self.add_ref_from_node(inner_child, RefType::Definition);
                            }
                        }
                    }
                }
                // Process value for uses
                if let Some(value) = child.child_by_field_name("value") {
                    self.extract_refs_from_node(value, depth + 1)?;
                }
            }
        }

        Ok(())
    }

    /// Process Go assignment statement: x = ...; x += ...
    fn process_go_assignment(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        if let Some(left) = node.child_by_field_name("left") {
            // Check for operator: if it's += etc, it's an update
            let is_update = node.children(&mut node.walk())
                .any(|c| {
                    let text = c.utf8_text(self.source.as_bytes()).unwrap_or("");
                    text.ends_with('=') && text != "="
                });

            if is_update {
                self.extract_go_lhs_identifiers_as(left, RefType::Update)?;
            } else {
                self.extract_go_lhs_identifiers_as(left, RefType::Definition)?;
            }
        }

        if let Some(right) = node.child_by_field_name("right") {
            self.extract_refs_from_node(right, depth + 1)?;
        }

        Ok(())
    }

    /// Extract Go left-hand-side identifiers as definitions
    fn extract_go_lhs_identifiers(&mut self, node: Node) -> TldrResult<()> {
        self.extract_go_lhs_identifiers_as(node, RefType::Definition)
    }

    /// Extract Go left-hand-side identifiers with specified ref type
    fn extract_go_lhs_identifiers_as(&mut self, node: Node, ref_type: RefType) -> TldrResult<()> {
        if node.kind() == "identifier" {
            let name = node.utf8_text(self.source.as_bytes()).unwrap_or("");
            if name != "_" {
                self.add_ref_from_node(node, ref_type);
            }
        } else if node.kind() == "expression_list" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    let name = child.utf8_text(self.source.as_bytes()).unwrap_or("");
                    if name != "_" {
                        self.add_ref_from_node(child, ref_type);
                    }
                }
            }
        }
        Ok(())
    }

    /// Process Go range clause: for i, v := range items { }
    fn process_go_range(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        // Range clause has "left" (expression_list with i, v) and "right" (iterable)
        if let Some(left) = node.child_by_field_name("left") {
            self.extract_go_lhs_identifiers(left)?;
        }

        if let Some(right) = node.child_by_field_name("right") {
            self.extract_refs_from_node(right, depth + 1)?;
        }

        Ok(())
    }

    // =====================================================================
    // Java processing
    // =====================================================================

    /// Process Java local variable declaration: int x = ...;
    /// AST: local_variable_declaration -> type, declarator (variable_declarator)
    fn process_java_local_var(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "variable_declarator" {
                // "name" field is the variable name
                if let Some(name_node) = child.child_by_field_name("name") {
                    if name_node.kind() == "identifier" {
                        self.add_ref_from_node(name_node, RefType::Definition);
                    }
                }
                // "value" field is the initializer
                if let Some(value) = child.child_by_field_name("value") {
                    self.extract_refs_from_node(value, depth + 1)?;
                }
            }
        }

        Ok(())
    }

    /// Process Java enhanced for: for (int item : items) { ... }
    /// AST: enhanced_for_statement -> type, name, value, body
    fn process_java_enhanced_for(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        // "name" field: loop variable
        if let Some(name) = node.child_by_field_name("name") {
            if name.kind() == "identifier" {
                self.add_ref_from_node(name, RefType::Definition);
            }
        }

        // "value" field: the iterable
        if let Some(value) = node.child_by_field_name("value") {
            self.extract_refs_from_node(value, depth + 1)?;
        }

        // "body" field: loop body
        if let Some(body) = node.child_by_field_name("body") {
            self.extract_refs_from_node(body, depth + 1)?;
        }

        Ok(())
    }

    // =====================================================================
    // C/C++ processing
    // =====================================================================

    /// Process C/C++ declaration: int x = ...; int x, y;
    /// AST: declaration -> type, declarator (init_declarator or identifier)
    fn process_c_declaration(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "init_declarator" {
                // init_declarator has "declarator" and "value" fields
                if let Some(declarator) = child.child_by_field_name("declarator") {
                    if declarator.kind() == "identifier" {
                        self.add_ref_from_node(declarator, RefType::Definition);
                    } else if declarator.kind() == "pointer_declarator" {
                        // *x = ... -> find the identifier
                        let mut inner = declarator.walk();
                        for inner_child in declarator.children(&mut inner) {
                            if inner_child.kind() == "identifier" {
                                self.add_ref_from_node(inner_child, RefType::Definition);
                                break;
                            }
                        }
                    }
                }
                if let Some(value) = child.child_by_field_name("value") {
                    self.extract_refs_from_node(value, depth + 1)?;
                }
            } else if child.kind() == "identifier" {
                // Plain declaration without initializer: int x;
                let text = child.utf8_text(self.source.as_bytes()).unwrap_or("");
                if !text.is_empty() && !is_keyword(text, self.language) {
                    self.add_ref_from_node(child, RefType::Definition);
                }
            }
        }

        Ok(())
    }

    // =====================================================================
    // PHP processing
    // =====================================================================

    /// Process PHP foreach: foreach ($arr as $key => $val) { }
    fn process_php_foreach(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        // Iterate children to find the loop variable(s)
        let mut cursor = node.walk();
        let mut found_as = false;
        for child in node.children(&mut cursor) {
            if child.kind() == "as" {
                found_as = true;
                continue;
            }
            if found_as && (child.kind() == "variable_name" || child.kind() == "pair") {
                if child.kind() == "variable_name" {
                    self.add_ref_from_node(child, RefType::Definition);
                } else {
                    // pair: $key => $val
                    let mut inner = child.walk();
                    for inner_child in child.children(&mut inner) {
                        if inner_child.kind() == "variable_name" {
                            self.add_ref_from_node(inner_child, RefType::Definition);
                        }
                    }
                }
                break;
            }
        }

        // Process body
        if let Some(body) = node.child_by_field_name("body") {
            self.extract_refs_from_node(body, depth + 1)?;
        }

        Ok(())
    }

    // =====================================================================
    // Scala processing
    // =====================================================================

    /// Process Scala val/var: val x = ...; var x = ...
    fn process_scala_val_var(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        // "pattern" field contains the binding
        if let Some(pattern) = node.child_by_field_name("pattern") {
            if pattern.kind() == "identifier" {
                self.add_ref_from_node(pattern, RefType::Definition);
            }
        }
        // Also try "name" field
        if let Some(name) = node.child_by_field_name("name") {
            if name.kind() == "identifier" {
                self.add_ref_from_node(name, RefType::Definition);
            }
        }

        // "value" or "body" field for the initializer
        if let Some(value) = node.child_by_field_name("value") {
            self.extract_refs_from_node(value, depth + 1)?;
        }
        if let Some(body) = node.child_by_field_name("body") {
            self.extract_refs_from_node(body, depth + 1)?;
        }

        Ok(())
    }

    // =====================================================================
    // Elixir processing
    // =====================================================================

    /// Process Elixir match operator: x = ... (pattern matching)
    /// Handles both "match_operator" and "binary_operator" with "=" operator
    fn process_elixir_match(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        // Try field names first (match_operator uses "left"/"right")
        let left = node.child_by_field_name("left");
        let right = node.child_by_field_name("right");

        if let Some(l) = left {
            if l.kind() == "identifier" {
                self.add_ref_from_node(l, RefType::Definition);
            }
        } else {
            // binary_operator: first named child is the LHS
            let mut cursor = node.walk();
            let named_children: Vec<_> = node.children(&mut cursor)
                .filter(|c| c.is_named())
                .collect();
            if named_children.len() >= 2 && named_children[0].kind() == "identifier" {
                self.add_ref_from_node(named_children[0], RefType::Definition);
            }
        }

        if let Some(r) = right {
            self.extract_refs_from_node(r, depth + 1)?;
        } else {
            // binary_operator: last named child is the RHS
            let mut cursor = node.walk();
            let named_children: Vec<_> = node.children(&mut cursor)
                .filter(|c| c.is_named())
                .collect();
            if named_children.len() >= 2 {
                self.extract_refs_from_node(*named_children.last().unwrap(), depth + 1)?;
            }
        }

        Ok(())
    }

    // =====================================================================
    // Lua/Luau processing
    // =====================================================================

    /// Process Lua local declaration: local x = ...
    /// AST: variable_declaration -> local, assignment_statement -> variable_list -> identifier(s), = , expression_list
    /// Also handles simpler form: variable_declaration -> local, identifier(s)
    fn process_lua_local_declaration(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "identifier" {
                // Simple form: local x (no assignment)
                self.add_ref_from_node(child, RefType::Definition);
            } else if child.kind() == "assignment_statement" {
                // Full form: local y = x + 1
                // assignment_statement has variable_list and expression_list children
                self.process_lua_assignment_statement(child, depth)?;
            }
        }
        Ok(())
    }

    /// Process Lua assignment statement: x = expr or x, y = expr1, expr2
    /// AST: assignment_statement -> variable_list -> identifier(s), = , expression_list -> expression(s)
    fn process_lua_assignment_statement(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "variable_list" {
                // Extract all identifiers as definitions
                let mut inner = child.walk();
                for inner_child in child.children(&mut inner) {
                    if inner_child.kind() == "identifier" {
                        self.add_ref_from_node(inner_child, RefType::Definition);
                    } else if inner_child.kind() == "dot_index_expression"
                        || inner_child.kind() == "bracket_index_expression"
                    {
                        // x.field = ... or x[i] = ... -> update
                        let mut deep = inner_child.walk();
                        for deep_child in inner_child.children(&mut deep) {
                            if deep_child.kind() == "identifier" {
                                self.add_ref_from_node(deep_child, RefType::Update);
                                break;
                            }
                        }
                    }
                }
            } else if child.kind() == "expression_list" {
                // Process the value expressions for uses
                self.extract_refs_from_node(child, depth + 1)?;
            }
        }
        Ok(())
    }

    // =====================================================================
    // Kotlin processing
    // =====================================================================

    /// Process Kotlin property declaration: val x = ...; var x = ...
    /// AST: property_declaration -> (val/var) variable_declaration (identifier) = expression
    fn process_kotlin_property(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        // Find the variable_declaration child which contains the identifier
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "variable_declaration" {
                // variable_declaration has an identifier child
                let mut inner = child.walk();
                for inner_child in child.children(&mut inner) {
                    if inner_child.kind() == "identifier" {
                        self.add_ref_from_node(inner_child, RefType::Definition);
                        break;
                    }
                }
            }
        }

        // Process the value expression (after "=")
        // The value is typically the last named child after "="
        let mut cursor2 = node.walk();
        let mut found_eq = false;
        for child in node.children(&mut cursor2) {
            if !child.is_named() && child.utf8_text(self.source.as_bytes()).unwrap_or("") == "=" {
                found_eq = true;
                continue;
            }
            if found_eq && child.is_named() {
                self.extract_refs_from_node(child, depth + 1)?;
            }
        }

        Ok(())
    }

    // =====================================================================
    // Swift processing
    // =====================================================================

    /// Process Swift property declaration: let y = ...; var z = ...
    /// AST: property_declaration -> value_binding_pattern (let/var), pattern -> simple_identifier, =, expression
    /// Also handles: property_declaration -> value_binding_pattern, typed_pattern -> pattern -> simple_identifier, type_annotation
    fn process_swift_property(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        // Find the variable name: look for pattern -> simple_identifier or simple_identifier
        let mut cursor = node.walk();
        let mut found_name = false;
        let mut found_eq = false;

        for child in node.children(&mut cursor) {
            match child.kind() {
                "pattern" => {
                    // pattern -> simple_identifier
                    let mut inner = child.walk();
                    for inner_child in child.children(&mut inner) {
                        if inner_child.kind() == "simple_identifier" {
                            self.add_ref_from_node(inner_child, RefType::Definition);
                            found_name = true;
                            break;
                        }
                    }
                }
                "typed_pattern" => {
                    // typed_pattern -> pattern -> simple_identifier, type_annotation
                    let mut inner = child.walk();
                    for inner_child in child.children(&mut inner) {
                        if inner_child.kind() == "pattern" {
                            let mut deep = inner_child.walk();
                            for deep_child in inner_child.children(&mut deep) {
                                if deep_child.kind() == "simple_identifier" {
                                    self.add_ref_from_node(deep_child, RefType::Definition);
                                    found_name = true;
                                    break;
                                }
                            }
                        }
                    }
                }
                "simple_identifier" if !found_name => {
                    // Direct simple_identifier child
                    self.add_ref_from_node(child, RefType::Definition);
                    found_name = true;
                }
                _ => {
                    if !child.is_named() && child.utf8_text(self.source.as_bytes()).unwrap_or("") == "=" {
                        found_eq = true;
                        continue;
                    }
                    if found_eq && child.is_named() {
                        // Process the value expression for uses
                        self.extract_refs_from_node(child, depth + 1)?;
                    }
                }
            }
        }

        Ok(())
    }

    /// Process Swift assignment: z = z + 1
    /// AST: assignment -> directly_assignable_expression -> simple_identifier, =, expression
    fn process_swift_assignment(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        let mut cursor = node.walk();
        let mut found_eq = false;
        let mut processed_target = false;

        for child in node.children(&mut cursor) {
            if child.kind() == "directly_assignable_expression" {
                // Find simple_identifier inside directly_assignable_expression
                let mut inner = child.walk();
                for inner_child in child.children(&mut inner) {
                    if inner_child.kind() == "simple_identifier" {
                        self.add_ref_from_node(inner_child, RefType::Definition);
                        processed_target = true;
                        break;
                    }
                }
                // If no simple_identifier found, it could be a navigation_expression (member access)
                if !processed_target {
                    let mut inner2 = child.walk();
                    for inner_child in child.children(&mut inner2) {
                        if inner_child.kind() == "navigation_expression" {
                            // obj.field = ... -> obj is an Update
                            let mut deep = inner_child.walk();
                            for deep_child in inner_child.children(&mut deep) {
                                if deep_child.kind() == "simple_identifier" {
                                    self.add_ref_from_node(deep_child, RefType::Update);
                                    break;
                                }
                            }
                            processed_target = true;
                            break;
                        }
                    }
                }
            } else if !child.is_named() && child.utf8_text(self.source.as_bytes()).unwrap_or("") == "=" {
                found_eq = true;
            } else if found_eq && child.is_named() {
                // Process the value expression for uses
                self.extract_refs_from_node(child, depth + 1)?;
            }
        }

        Ok(())
    }

    /// Extract Swift function parameters as definitions
    /// Swift: function_declaration -> ... parameter (simple_identifier "x", :, user_type) ...
    /// Parameters are direct children of the function_declaration node.
    fn extract_swift_parameters(&mut self, func_node: Node) -> TldrResult<()> {
        let mut cursor = func_node.walk();
        for child in func_node.children(&mut cursor) {
            if child.kind() == "parameter" {
                // Find the first simple_identifier inside the parameter (the name)
                let mut inner = child.walk();
                for inner_child in child.children(&mut inner) {
                    if inner_child.kind() == "simple_identifier" {
                        self.add_ref_from_node(inner_child, RefType::Definition);
                        break;
                    }
                }
            }
        }

        Ok(())
    }

    // =====================================================================
    // Elixir parameter extraction
    // =====================================================================

    /// Extract Elixir function parameters as definitions
    /// Elixir: (call (identifier "def") (arguments (call (identifier "foo") (arguments (identifier "x")))))
    fn extract_elixir_parameters(&mut self, func_node: Node) -> TldrResult<()> {
        if func_node.kind() != "call" {
            return Ok(());
        }

        // Find the inner call with the function name and its arguments
        let mut cursor = func_node.walk();
        for child in func_node.children(&mut cursor) {
            if child.kind() == "arguments" {
                // Inside arguments, look for a call node (function name + params)
                let mut inner = child.walk();
                for inner_child in child.children(&mut inner) {
                    if inner_child.kind() == "call" {
                        // The call has arguments containing the parameters
                        if let Some(args) = inner_child.child(1) {
                            if args.kind() == "arguments" {
                                let mut args_cursor = args.walk();
                                for arg in args.children(&mut args_cursor) {
                                    if arg.kind() == "identifier" {
                                        self.add_ref_from_node(arg, RefType::Definition);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    // =====================================================================
    // OCaml processing
    // =====================================================================

    /// Extract OCaml function parameters as definitions
    /// OCaml: (let_binding (value_name "foo") (parameter (value_pattern "x")) body: ...)
    fn extract_ocaml_parameters(&mut self, func_node: Node) -> TldrResult<()> {
        // For value_definition, drill into let_binding
        let binding = if func_node.kind() == "value_definition" {
            let mut cursor = func_node.walk();
            let mut found = None;
            for child in func_node.children(&mut cursor) {
                if child.kind() == "let_binding" {
                    found = Some(child);
                    break;
                }
            }
            found
        } else if func_node.kind() == "let_binding" {
            Some(func_node)
        } else {
            None
        };

        if let Some(binding) = binding {
            let mut cursor = binding.walk();
            for child in binding.children(&mut cursor) {
                if child.kind() == "parameter" {
                    // parameter contains value_pattern with the param name
                    let mut inner = child.walk();
                    for inner_child in child.children(&mut inner) {
                        if inner_child.kind() == "value_pattern"
                            || inner_child.kind() == "value_name"
                            || inner_child.kind() == "identifier"
                        {
                            self.add_ref_from_node(inner_child, RefType::Definition);
                            break;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Process OCaml let_expression: let x = expr in body
    /// AST: (let_expression (value_definition (let_binding ...)) "in" body)
    fn process_ocaml_let_expression(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.extract_refs_from_node(child, depth + 1)?;
        }
        Ok(())
    }

    /// Process OCaml value_definition: let binding(s)
    fn process_ocaml_value_definition(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "let_binding" {
                self.process_ocaml_let_binding(child, depth)?;
            }
        }
        Ok(())
    }

    /// Process OCaml let_binding: pattern = expression
    /// AST: (let_binding (value_name "y") = (expression))
    fn process_ocaml_let_binding(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        // The pattern/name is the first value_name child
        if let Some(pattern) = node.child_by_field_name("pattern") {
            if pattern.kind() == "value_name" || pattern.kind() == "identifier" {
                self.add_ref_from_node(pattern, RefType::Definition);
            }
        } else {
            // Fallback: find first value_name child (before "=")
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "value_name" {
                    self.add_ref_from_node(child, RefType::Definition);
                    break;
                }
                if !child.is_named() && child.utf8_text(self.source.as_bytes()).unwrap_or("") == "=" {
                    break; // Stop before the RHS
                }
            }
        }

        // Process the body/value for uses
        if let Some(body) = node.child_by_field_name("body") {
            self.extract_refs_from_node(body, depth + 1)?;
        } else {
            // Fallback: process children after "=" for uses
            let mut cursor = node.walk();
            let mut found_eq = false;
            for child in node.children(&mut cursor) {
                if !child.is_named() && child.utf8_text(self.source.as_bytes()).unwrap_or("") == "=" {
                    found_eq = true;
                    continue;
                }
                if found_eq && child.is_named() {
                    self.extract_refs_from_node(child, depth + 1)?;
                }
            }
        }

        Ok(())
    }

    /// Check if an OCaml value_name is in a use context
    fn is_ocaml_use_context(&self, node: Node) -> bool {
        if let Some(parent) = node.parent() {
            match parent.kind() {
                // Not a use if we're the pattern in a let_binding
                "let_binding" => {
                    if let Some(pattern) = parent.child_by_field_name("pattern") {
                        if self.node_contains(pattern, node) {
                            return false;
                        }
                    }
                    // Also check: first value_name before "=" is a definition
                    let mut cursor = parent.walk();
                    for child in parent.children(&mut cursor) {
                        if child.kind() == "value_name" && child.id() == node.id() {
                            return false; // This is the binding name
                        }
                        if !child.is_named() && child.utf8_text(self.source.as_bytes()).unwrap_or("") == "=" {
                            break;
                        }
                    }
                }
                // Not a use if we're a parameter
                "parameter" => {
                    return false;
                }
                "value_definition" => {
                    return false; // Will be handled by value_definition processor
                }
                _ => {}
            }
        }
        true
    }

    /// Check if a Swift simple_identifier is in a use context (not assignment target)
    fn is_swift_use_context(&self, node: Node) -> bool {
        if let Some(parent) = node.parent() {
            match parent.kind() {
                // Not a use if we're a definition target inside property_declaration
                "pattern" => {
                    // pattern is inside property_declaration -> this is a definition
                    if let Some(grandparent) = parent.parent() {
                        if grandparent.kind() == "property_declaration"
                            || grandparent.kind() == "typed_pattern"
                        {
                            return false;
                        }
                    }
                }
                "property_declaration" => {
                    return false; // Handled by process_swift_property
                }
                // Not a use if we're the target of an assignment
                "directly_assignable_expression" => {
                    if let Some(grandparent) = parent.parent() {
                        if grandparent.kind() == "assignment" {
                            return false; // Handled by process_swift_assignment
                        }
                    }
                }
                // Not a use if we're a parameter name
                "parameter" => {
                    return false;
                }
                // Not a use if we're in a function declaration name position
                "function_declaration" => {
                    return false;
                }
                // Not a use if it's a type annotation (like Int)
                "type_identifier" | "user_type" | "type_annotation" => {
                    return false;
                }
                // Navigation expression (member access): foo.bar -> foo is a use, bar is not
                "navigation_expression" => {
                    // The suffix (after .) is not a use - only the target object is
                    if let Some(suffix) = parent.child_by_field_name("suffix") {
                        if suffix.id() == node.id() {
                            return false;
                        }
                    }
                    // Check if this is the last simple_identifier (method/field name)
                    let mut cursor = parent.walk();
                    let children: Vec<_> = parent.children(&mut cursor)
                        .filter(|c| c.kind() == "simple_identifier")
                        .collect();
                    if children.len() >= 2 {
                        if let Some(last) = children.last() {
                            if last.id() == node.id() {
                                return false; // This is the field/method name, not a use
                            }
                        }
                    }
                }
                // Value binding pattern (let/var keyword) - not relevant
                "value_binding_pattern" => {
                    return false;
                }
                _ => {}
            }
        }
        // Also check grandparent for nested cases
        if let Some(parent) = node.parent() {
            if let Some(grandparent) = parent.parent() {
                // typed_pattern -> pattern -> simple_identifier (this is a definition)
                if parent.kind() == "pattern" && grandparent.kind() == "typed_pattern" {
                    return false;
                }
            }
        }
        true
    }

    /// Check if an identifier is in a "use" context (not assignment target)
    ///
    /// Multi-language: checks parent node kinds for all supported languages
    /// to determine if this identifier is a target of assignment/declaration
    /// (and therefore NOT a use).
    fn is_use_context(&self, node: Node) -> bool {
        if let Some(parent) = node.parent() {
            if let Some(is_use) = self.parent_use_context(parent, node) {
                return is_use;
            }
        }
        if let Some(is_use) = self.expression_list_grandparent_use_context(node) {
            return is_use;
        }
        true
    }

    fn parent_use_context(&self, parent: Node, node: Node) -> Option<bool> {
        let kind = parent.kind();

        if matches!(kind, "assignment" | "for_statement" | "for_in_statement" | "augmented_assignment") {
            return self.left_field_contains(parent, node).map(|contains| !contains);
        }
        if matches!(
            kind,
            "parameters"
                | "parameter"
                | "typed_parameter"
                | "default_parameter"
                | "formal_parameters"
                | "required_parameter"
                | "optional_parameter"
                | "parameter_declaration"
                | "formal_parameter"
                | "function_value_parameters"
        ) {
            return Some(false);
        }
        if matches!(self.language, Language::Kotlin)
            && matches!(kind, "property_declaration" | "variable_declaration")
        {
            return Some(false);
        }

        if kind == "variable_declarator" {
            if let Some(name) = parent.child_by_field_name("name") {
                if name.id() == node.id() {
                    return Some(false);
                }
            }
        }
        if matches!(kind, "lexical_declaration" | "variable_declaration")
            && !matches!(self.language, Language::Lua | Language::Luau)
        {
            return Some(false);
        }
        if matches!(kind, "assignment_expression" | "augmented_assignment_expression") {
            if let Some(left) = parent.child_by_field_name("left") {
                if left.id() == node.id() {
                    return Some(false);
                }
            }
        }

        if kind == "let_declaration" || kind == "for_expression" {
            if let Some(pattern) = parent.child_by_field_name("pattern") {
                if self.node_contains(pattern, node) {
                    return Some(false);
                }
            }
        }
        if kind == "mut_pattern" {
            return Some(false);
        }

        if matches!(
            kind,
            "short_var_declaration" | "assignment_statement" | "range_clause"
        ) {
            if let Some(left) = parent.child_by_field_name("left") {
                if self.node_contains(left, node) {
                    return Some(false);
                }
            }
        }
        if kind == "var_spec" {
            if let Some(name) = parent.child_by_field_name("name") {
                if self.node_contains(name, node) {
                    return Some(false);
                }
            }
        }

        if kind == "local_variable_declaration" {
            return Some(false);
        }
        if kind == "enhanced_for_statement" {
            if let Some(name) = parent.child_by_field_name("name") {
                if name.id() == node.id() {
                    return Some(false);
                }
            }
        }

        if kind == "declaration" && matches!(self.language, Language::C | Language::Cpp) {
            return Some(false);
        }
        if kind == "init_declarator" {
            if let Some(declarator) = parent.child_by_field_name("declarator") {
                if self.node_contains(declarator, node) {
                    return Some(false);
                }
            }
        }

        if kind == "operator_assignment" {
            if let Some(left) = parent.child_by_field_name("left") {
                if left.id() == node.id() {
                    return Some(false);
                }
            }
        }

        if matches!(kind, "val_definition" | "var_definition") {
            if let Some(pattern) = parent.child_by_field_name("pattern") {
                if self.node_contains(pattern, node) {
                    return Some(false);
                }
            }
            if let Some(name) = parent.child_by_field_name("name") {
                if self.node_contains(name, node) {
                    return Some(false);
                }
            }
        }

        if matches!(self.language, Language::Elixir) && kind == "match_operator" {
            if let Some(left) = parent.child_by_field_name("left") {
                if self.node_contains(left, node) {
                    return Some(false);
                }
            }
        }
        if matches!(self.language, Language::Elixir)
            && kind == "binary_operator"
            && self.is_elixir_match_lhs(parent, node)
        {
            return Some(false);
        }

        None
    }

    fn expression_list_grandparent_use_context(&self, node: Node) -> Option<bool> {
        let parent = node.parent()?;
        if parent.kind() != "expression_list" {
            return None;
        }
        let grandparent = parent.parent()?;
        if !matches!(
            grandparent.kind(),
            "short_var_declaration" | "assignment_statement" | "range_clause"
        ) {
            return None;
        }
        let left = grandparent.child_by_field_name("left")?;
        Some(!self.node_contains(left, node))
    }

    fn left_field_contains(&self, node: Node, target: Node) -> Option<bool> {
        node.child_by_field_name("left")
            .map(|left| self.node_contains(left, target))
    }

    fn is_elixir_match_lhs(&self, parent: Node, node: Node) -> bool {
        let is_match = parent.children(&mut parent.walk()).any(|child| {
            !child.is_named() && child.utf8_text(self.source.as_bytes()).unwrap_or("") == "="
        });
        if !is_match {
            return false;
        }

        let mut cursor = parent.walk();
        for child in parent.children(&mut cursor) {
            if child.is_named() && child.id() == node.id() {
                return true;
            }
            if !child.is_named() && child.utf8_text(self.source.as_bytes()).unwrap_or("") == "=" {
                break;
            }
        }
        false
    }

    /// Check if ancestor node contains the target node
    fn node_contains(&self, ancestor: Node, target: Node) -> bool {
        if ancestor.id() == target.id() {
            return true;
        }
        let mut cursor = ancestor.walk();
        for child in ancestor.children(&mut cursor) {
            if self.node_contains(child, target) {
                return true;
            }
        }
        false
    }

    /// Build def-use chains using reaching definitions analysis
    fn build_def_use_chains(&mut self, cfg: &CfgInfo) -> TldrResult<()> {
        if cfg.blocks.is_empty() {
            return Ok(());
        }

        // Compute reaching definitions
        let _reaching = compute_reaching_definitions(cfg, &self.refs);

        // Build edges by connecting uses to their reaching definitions
        // (Edges will be added during finalize)

        Ok(())
    }

    /// Finalize and produce the DfgInfo
    fn finalize(self) -> TldrResult<DfgInfo> {
        // Build edges by connecting definitions to uses for the same variable
        let mut edges = Vec::new();

        // Group refs by variable
        let mut defs_by_var: HashMap<String, Vec<&VarRef>> = HashMap::new();
        let mut uses_by_var: HashMap<String, Vec<&VarRef>> = HashMap::new();

        for r in &self.refs {
            match r.ref_type {
                RefType::Definition | RefType::Update => {
                    defs_by_var.entry(r.name.clone()).or_default().push(r);
                }
                RefType::Use => {
                    uses_by_var.entry(r.name.clone()).or_default().push(r);
                }
            }
        }

        // For each variable, connect defs to uses
        // Simple heuristic: connect each def to uses that come after it
        for (var, defs) in &defs_by_var {
            if let Some(uses) = uses_by_var.get(var) {
                for def in defs {
                    for use_ref in uses {
                        // Connect if use comes after def (simple heuristic)
                        // A more sophisticated analysis would use reaching definitions
                        if use_ref.line >= def.line {
                            edges.push(DataflowEdge {
                                var: var.clone(),
                                def_line: def.line,
                                use_line: use_ref.line,
                                def_ref: (*def).clone(),
                                use_ref: (*use_ref).clone(),
                            });
                        }
                    }
                }
            }
        }

        let variables: Vec<String> = self.variables.into_iter().collect();

        Ok(DfgInfo {
            function: self.function_name,
            refs: self.refs,
            edges,
            variables,
        })
    }
}

fn first_child_of_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    for i in 0..node.child_count() {
        let child = node.child(i)?;
        if child.kind() == kind {
            return Some(child);
        }
    }
    None
}

/// Check if a name is a language keyword
fn is_keyword(name: &str, language: Language) -> bool {
    match language {
        Language::Python => matches!(
            name,
            "False" | "None" | "True" | "and" | "as" | "assert" | "async" | "await"
            | "break" | "class" | "continue" | "def" | "del" | "elif" | "else"
            | "except" | "finally" | "for" | "from" | "global" | "if" | "import"
            | "in" | "is" | "lambda" | "nonlocal" | "not" | "or" | "pass" | "raise"
            | "return" | "try" | "while" | "with" | "yield"
        ),
        Language::TypeScript | Language::JavaScript => matches!(
            name,
            "break" | "case" | "catch" | "class" | "const" | "continue" | "debugger"
            | "default" | "delete" | "do" | "else" | "enum" | "export" | "extends"
            | "false" | "finally" | "for" | "function" | "if" | "import" | "in"
            | "instanceof" | "let" | "new" | "null" | "return" | "static" | "super"
            | "switch" | "this" | "throw" | "true" | "try" | "typeof" | "undefined"
            | "var" | "void" | "while" | "with" | "yield"
        ),
        Language::Go => matches!(
            name,
            "break" | "case" | "chan" | "const" | "continue" | "default" | "defer"
            | "else" | "fallthrough" | "for" | "func" | "go" | "goto" | "if"
            | "import" | "interface" | "map" | "package" | "range" | "return"
            | "select" | "struct" | "switch" | "type" | "var" | "nil" | "true" | "false"
        ),
        Language::Rust => matches!(
            name,
            "as" | "break" | "const" | "continue" | "crate" | "else" | "enum"
            | "extern" | "false" | "fn" | "for" | "if" | "impl" | "in" | "let"
            | "loop" | "match" | "mod" | "move" | "mut" | "pub" | "ref" | "return"
            | "self" | "Self" | "static" | "struct" | "super" | "trait" | "true"
            | "type" | "unsafe" | "use" | "where" | "while" | "async" | "await"
            | "dyn"
        ),
        Language::Java => matches!(
            name,
            "abstract" | "assert" | "boolean" | "break" | "byte" | "case" | "catch"
            | "char" | "class" | "const" | "continue" | "default" | "do" | "double"
            | "else" | "enum" | "extends" | "false" | "final" | "finally" | "float"
            | "for" | "goto" | "if" | "implements" | "import" | "instanceof" | "int"
            | "interface" | "long" | "native" | "new" | "null" | "package" | "private"
            | "protected" | "public" | "return" | "short" | "static" | "strictfp"
            | "super" | "switch" | "synchronized" | "this" | "throw" | "throws"
            | "transient" | "true" | "try" | "void" | "volatile" | "while"
        ),
        Language::C | Language::Cpp => matches!(
            name,
            "auto" | "break" | "case" | "char" | "const" | "continue" | "default"
            | "do" | "double" | "else" | "enum" | "extern" | "float" | "for"
            | "goto" | "if" | "int" | "long" | "register" | "return" | "short"
            | "signed" | "sizeof" | "static" | "struct" | "switch" | "typedef"
            | "union" | "unsigned" | "void" | "volatile" | "while" | "NULL"
        ),
        Language::Ruby => matches!(
            name,
            "alias" | "and" | "begin" | "break" | "case" | "class" | "def" | "do"
            | "else" | "elsif" | "end" | "ensure" | "false" | "for" | "if" | "in"
            | "module" | "next" | "nil" | "not" | "or" | "redo" | "rescue" | "retry"
            | "return" | "self" | "super" | "then" | "true" | "undef" | "unless"
            | "until" | "when" | "while" | "yield" | "puts" | "print"
        ),
        Language::Php => matches!(
            name,
            "abstract" | "and" | "array" | "as" | "break" | "callable" | "case"
            | "catch" | "class" | "clone" | "const" | "continue" | "declare"
            | "default" | "die" | "do" | "echo" | "else" | "elseif" | "empty"
            | "enddeclare" | "endfor" | "endforeach" | "endif" | "endswitch"
            | "endwhile" | "eval" | "exit" | "extends" | "false" | "final"
            | "finally" | "fn" | "for" | "foreach" | "function" | "global" | "goto"
            | "if" | "implements" | "include" | "instanceof" | "interface" | "isset"
            | "list" | "match" | "namespace" | "new" | "null" | "or" | "print"
            | "private" | "protected" | "public" | "require" | "return" | "static"
            | "switch" | "throw" | "trait" | "true" | "try" | "unset" | "use"
            | "var" | "while" | "xor" | "yield"
        ),
        Language::Kotlin => matches!(
            name,
            "abstract" | "annotation" | "as" | "break" | "by" | "catch" | "class"
            | "companion" | "const" | "constructor" | "continue" | "crossinline"
            | "data" | "do" | "else" | "enum" | "external" | "false" | "final"
            | "finally" | "for" | "fun" | "if" | "import" | "in" | "infix"
            | "init" | "inline" | "inner" | "interface" | "internal" | "is"
            | "lateinit" | "noinline" | "null" | "object" | "open" | "operator"
            | "out" | "override" | "package" | "private" | "protected" | "public"
            | "reified" | "return" | "sealed" | "super" | "suspend" | "this"
            | "throw" | "true" | "try" | "typealias" | "val" | "var" | "vararg"
            | "when" | "where" | "while"
        ),
        Language::Elixir => matches!(
            name,
            "after" | "and" | "case" | "catch" | "cond" | "def" | "defp"
            | "defmodule" | "defstruct" | "defprotocol" | "defimpl" | "defmacro"
            | "do" | "else" | "end" | "false" | "fn" | "for" | "if" | "import"
            | "in" | "nil" | "not" | "or" | "raise" | "receive" | "require"
            | "rescue" | "true" | "try" | "unless" | "use" | "when" | "with"
        ),
        Language::Ocaml => matches!(
            name,
            "and" | "as" | "assert" | "begin" | "class" | "constraint" | "do"
            | "done" | "downto" | "else" | "end" | "exception" | "external"
            | "false" | "for" | "fun" | "function" | "functor" | "if" | "in"
            | "include" | "inherit" | "initializer" | "lazy" | "let" | "match"
            | "method" | "mod" | "module" | "mutable" | "new" | "nonrec"
            | "object" | "of" | "open" | "or" | "private" | "rec" | "sig"
            | "struct" | "then" | "to" | "true" | "try" | "type" | "val"
            | "virtual" | "when" | "while" | "with"
        ),
        Language::Swift => matches!(
            name,
            "associatedtype" | "break" | "case" | "catch" | "class" | "continue"
            | "default" | "defer" | "deinit" | "do" | "else" | "enum" | "extension"
            | "fallthrough" | "false" | "fileprivate" | "for" | "func" | "guard"
            | "if" | "import" | "in" | "init" | "inout" | "internal" | "is" | "let"
            | "nil" | "open" | "operator" | "private" | "protocol" | "public"
            | "repeat" | "rethrows" | "return" | "self" | "Self" | "static"
            | "struct" | "subscript" | "super" | "switch" | "throw" | "throws"
            | "true" | "try" | "typealias" | "var" | "where" | "while"
            | "Int" | "String" | "Bool" | "Double" | "Float" | "Array"
        ),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_function() {
        let source = r#"
def foo(x):
    y = x + 1
    return y
"#;
        let dfg = get_dfg_context(source, "foo", Language::Python).unwrap();
        assert_eq!(dfg.function, "foo");
        assert!(dfg.variables.contains(&"x".to_string()));
        assert!(dfg.variables.contains(&"y".to_string()));
    }

    #[test]
    fn test_extracts_definitions() {
        let source = r#"
def foo():
    x = 1
    y = 2
    return x + y
"#;
        let dfg = get_dfg_context(source, "foo", Language::Python).unwrap();

        let defs: Vec<_> = dfg.refs.iter()
            .filter(|r| r.ref_type == RefType::Definition)
            .collect();

        assert!(defs.iter().any(|r| r.name == "x"));
        assert!(defs.iter().any(|r| r.name == "y"));
    }

    #[test]
    fn test_extracts_uses() {
        let source = r#"
def foo(x):
    return x + 1
"#;
        let dfg = get_dfg_context(source, "foo", Language::Python).unwrap();

        let uses: Vec<_> = dfg.refs.iter()
            .filter(|r| r.ref_type == RefType::Use)
            .collect();

        assert!(uses.iter().any(|r| r.name == "x"));
    }

    #[test]
    fn test_function_not_found() {
        let source = "def foo(): pass";
        let result = get_dfg_context(source, "bar", Language::Python);
        assert!(result.is_err());
    }

    #[test]
    fn test_for_loop_variable() {
        let source = r#"
def foo(items):
    for item in items:
        print(item)
"#;
        let dfg = get_dfg_context(source, "foo", Language::Python).unwrap();

        // 'item' should be both a definition (loop var) and a use (in print)
        let item_defs: Vec<_> = dfg.refs.iter()
            .filter(|r| r.name == "item" && r.ref_type == RefType::Definition)
            .collect();

        assert!(!item_defs.is_empty(), "for loop variable should be a definition");
    }

    #[test]
    fn test_augmented_assignment() {
        let source = r#"
def foo():
    x = 0
    x += 1
    return x
"#;
        let dfg = get_dfg_context(source, "foo", Language::Python).unwrap();

        let updates: Vec<_> = dfg.refs.iter()
            .filter(|r| r.name == "x" && r.ref_type == RefType::Update)
            .collect();

        assert!(!updates.is_empty(), "augmented assignment should be an update");
    }

    #[test]
    fn test_def_use_edges() {
        let source = r#"
def foo():
    x = 1
    y = x + 2
    return y
"#;
        let dfg = get_dfg_context(source, "foo", Language::Python).unwrap();

        // Should have an edge from x's definition to x's use
        let x_edges: Vec<_> = dfg.edges.iter()
            .filter(|e| e.var == "x")
            .collect();

        assert!(!x_edges.is_empty(), "should have def-use edge for x");
    }

    // =========================================================================
    // Multi-language DFG tests
    // =========================================================================

    // --- TypeScript / JavaScript ---

    #[test]
    fn test_typescript_let_const_declaration() {
        let source = r#"
function foo(x: number) {
    let y = x + 1;
    const z = y * 2;
    return z;
}
"#;
        let dfg = get_dfg_context(source, "foo", Language::TypeScript).unwrap();
        assert_eq!(dfg.function, "foo");
        assert!(dfg.variables.contains(&"x".to_string()), "should find param x");
        assert!(dfg.variables.contains(&"y".to_string()), "should find let y");
        assert!(dfg.variables.contains(&"z".to_string()), "should find const z");

        // y and z should be definitions
        let defs: Vec<_> = dfg.refs.iter()
            .filter(|r| r.ref_type == RefType::Definition)
            .map(|r| r.name.as_str())
            .collect();
        assert!(defs.contains(&"y"), "y should be a definition, got defs: {:?}", defs);
        assert!(defs.contains(&"z"), "z should be a definition, got defs: {:?}", defs);
    }

    #[test]
    fn test_typescript_assignment_expression() {
        let source = r#"
function foo() {
    let x = 0;
    x = 5;
    x += 3;
    return x;
}
"#;
        let dfg = get_dfg_context(source, "foo", Language::TypeScript).unwrap();

        let x_defs: Vec<_> = dfg.refs.iter()
            .filter(|r| r.name == "x" && r.ref_type == RefType::Definition)
            .collect();
        assert!(!x_defs.is_empty(), "x should have at least one definition");

        let x_updates: Vec<_> = dfg.refs.iter()
            .filter(|r| r.name == "x" && r.ref_type == RefType::Update)
            .collect();
        assert!(!x_updates.is_empty(), "x += 3 should produce an update ref");
    }

    #[test]
    fn test_typescript_for_of_loop() {
        let source = r#"
function foo(items: number[]) {
    for (const item of items) {
        console.log(item);
    }
}
"#;
        let dfg = get_dfg_context(source, "foo", Language::TypeScript).unwrap();
        assert!(dfg.variables.contains(&"item".to_string()), "should find loop var item");

        let item_defs: Vec<_> = dfg.refs.iter()
            .filter(|r| r.name == "item" && r.ref_type == RefType::Definition)
            .collect();
        assert!(!item_defs.is_empty(), "for-of loop variable should be a definition");
    }

    #[test]
    fn test_javascript_var_declaration() {
        let source = r#"
function foo(x) {
    var y = x + 1;
    return y;
}
"#;
        let dfg = get_dfg_context(source, "foo", Language::JavaScript).unwrap();
        assert!(dfg.variables.contains(&"y".to_string()), "should find var y");

        let y_defs: Vec<_> = dfg.refs.iter()
            .filter(|r| r.name == "y" && r.ref_type == RefType::Definition)
            .collect();
        assert!(!y_defs.is_empty(), "var y should be a definition");
    }

    // --- Rust ---

    #[test]
    fn test_rust_let_declaration() {
        let source = r#"
fn foo(x: i32) -> i32 {
    let y = x + 1;
    let z = y * 2;
    z
}
"#;
        let dfg = get_dfg_context(source, "foo", Language::Rust).unwrap();
        assert_eq!(dfg.function, "foo");
        assert!(dfg.variables.contains(&"y".to_string()), "should find let y");
        assert!(dfg.variables.contains(&"z".to_string()), "should find let z");

        let defs: Vec<_> = dfg.refs.iter()
            .filter(|r| r.ref_type == RefType::Definition)
            .map(|r| r.name.as_str())
            .collect();
        assert!(defs.contains(&"y"), "y should be a definition, got defs: {:?}", defs);
        assert!(defs.contains(&"z"), "z should be a definition, got defs: {:?}", defs);
    }

    #[test]
    fn test_rust_assignment_expression() {
        let source = r#"
fn foo() -> i32 {
    let mut x = 0;
    x = 5;
    x += 3;
    x
}
"#;
        let dfg = get_dfg_context(source, "foo", Language::Rust).unwrap();

        // x = 5 should be a definition or update
        let x_all: Vec<_> = dfg.refs.iter()
            .filter(|r| r.name == "x")
            .collect();
        assert!(x_all.len() >= 3, "x should have at least 3 refs (def, reassign, use), got {}", x_all.len());
    }

    #[test]
    fn test_rust_for_expression() {
        let source = r#"
fn foo(items: Vec<i32>) {
    for item in items.iter() {
        println!("{}", item);
    }
}
"#;
        let dfg = get_dfg_context(source, "foo", Language::Rust).unwrap();
        assert!(dfg.variables.contains(&"item".to_string()), "should find loop var item");

        let item_defs: Vec<_> = dfg.refs.iter()
            .filter(|r| r.name == "item" && r.ref_type == RefType::Definition)
            .collect();
        assert!(!item_defs.is_empty(), "for loop variable should be a definition");
    }

    // --- Go ---

    #[test]
    fn test_go_short_var_declaration() {
        let source = r#"
func foo(x int) int {
    y := x + 1
    z := y * 2
    return z
}
"#;
        let dfg = get_dfg_context(source, "foo", Language::Go).unwrap();
        assert_eq!(dfg.function, "foo");
        assert!(dfg.variables.contains(&"y".to_string()), "should find y from :=");
        assert!(dfg.variables.contains(&"z".to_string()), "should find z from :=");

        let defs: Vec<_> = dfg.refs.iter()
            .filter(|r| r.ref_type == RefType::Definition)
            .map(|r| r.name.as_str())
            .collect();
        assert!(defs.contains(&"y"), "y should be a definition, got defs: {:?}", defs);
        assert!(defs.contains(&"z"), "z should be a definition, got defs: {:?}", defs);
    }

    #[test]
    fn test_go_var_declaration() {
        let source = r#"
func foo() int {
    var x int = 10
    var y = x + 1
    return y
}
"#;
        let dfg = get_dfg_context(source, "foo", Language::Go).unwrap();
        assert!(dfg.variables.contains(&"x".to_string()), "should find var x");
        assert!(dfg.variables.contains(&"y".to_string()), "should find var y");
    }

    #[test]
    fn test_go_assignment_statement() {
        let source = r#"
func foo() int {
    x := 0
    x = 5
    return x
}
"#;
        let dfg = get_dfg_context(source, "foo", Language::Go).unwrap();

        let x_refs: Vec<_> = dfg.refs.iter()
            .filter(|r| r.name == "x")
            .collect();
        // Should have: definition (:=), definition or update (=), use (return)
        assert!(x_refs.len() >= 3, "x should have at least 3 refs, got {}", x_refs.len());
    }

    #[test]
    fn test_go_range_loop() {
        let source = r#"
func foo(items []int) {
    for i, v := range items {
        fmt.Println(i, v)
    }
}
"#;
        let dfg = get_dfg_context(source, "foo", Language::Go).unwrap();
        assert!(dfg.variables.contains(&"i".to_string()), "should find range var i");
        assert!(dfg.variables.contains(&"v".to_string()), "should find range var v");
    }

    // --- Java ---

    #[test]
    fn test_java_local_variable_declaration() {
        let source = r#"
class Foo {
    int foo(int x) {
        int y = x + 1;
        int z = y * 2;
        return z;
    }
}
"#;
        let dfg = get_dfg_context(source, "foo", Language::Java).unwrap();
        assert_eq!(dfg.function, "foo");
        assert!(dfg.variables.contains(&"y".to_string()), "should find int y");
        assert!(dfg.variables.contains(&"z".to_string()), "should find int z");

        let defs: Vec<_> = dfg.refs.iter()
            .filter(|r| r.ref_type == RefType::Definition)
            .map(|r| r.name.as_str())
            .collect();
        assert!(defs.contains(&"y"), "y should be a definition, got defs: {:?}", defs);
        assert!(defs.contains(&"z"), "z should be a definition, got defs: {:?}", defs);
    }

    #[test]
    fn test_java_assignment_expression() {
        let source = r#"
class Foo {
    void foo() {
        int x = 0;
        x = 5;
        x += 3;
    }
}
"#;
        let dfg = get_dfg_context(source, "foo", Language::Java).unwrap();

        let x_defs: Vec<_> = dfg.refs.iter()
            .filter(|r| r.name == "x" && r.ref_type == RefType::Definition)
            .collect();
        assert!(!x_defs.is_empty(), "x should have at least one definition");
    }

    #[test]
    fn test_java_enhanced_for() {
        let source = r#"
class Foo {
    void foo(int[] items) {
        for (int item : items) {
            System.out.println(item);
        }
    }
}
"#;
        let dfg = get_dfg_context(source, "foo", Language::Java).unwrap();
        assert!(dfg.variables.contains(&"item".to_string()), "should find enhanced for var item");

        let item_defs: Vec<_> = dfg.refs.iter()
            .filter(|r| r.name == "item" && r.ref_type == RefType::Definition)
            .collect();
        assert!(!item_defs.is_empty(), "enhanced for variable should be a definition");
    }

    // --- C ---

    #[test]
    fn test_c_declaration() {
        let source = r#"
int foo(int x) {
    int y = x + 1;
    int z = y * 2;
    return z;
}
"#;
        let dfg = get_dfg_context(source, "foo", Language::C).unwrap();
        assert_eq!(dfg.function, "foo");
        assert!(dfg.variables.contains(&"y".to_string()), "should find int y");
        assert!(dfg.variables.contains(&"z".to_string()), "should find int z");
    }

    #[test]
    fn test_c_assignment_expression() {
        let source = r#"
void foo() {
    int x = 0;
    x = 5;
    x += 3;
}
"#;
        let dfg = get_dfg_context(source, "foo", Language::C).unwrap();

        let x_refs: Vec<_> = dfg.refs.iter()
            .filter(|r| r.name == "x")
            .collect();
        assert!(x_refs.len() >= 2, "x should have at least 2 refs, got {}: {:?}",
            x_refs.len(), x_refs.iter().map(|r| (&r.name, &r.ref_type, r.line)).collect::<Vec<_>>());
    }

    // --- Ruby ---

    #[test]
    fn test_ruby_assignment() {
        let source = r#"
def foo(x)
    y = x + 1
    z = y * 2
    z
end
"#;
        let dfg = get_dfg_context(source, "foo", Language::Ruby).unwrap();
        assert_eq!(dfg.function, "foo");
        assert!(dfg.variables.contains(&"y".to_string()), "should find y");
        assert!(dfg.variables.contains(&"z".to_string()), "should find z");
    }

    // --- PHP ---

    #[test]
    fn test_php_assignment() {
        let source = r#"<?php
function foo($x) {
    $y = $x + 1;
    $z = $y * 2;
    return $z;
}
"#;
        let dfg = get_dfg_context(source, "foo", Language::Php).unwrap();
        assert_eq!(dfg.function, "foo");
        // PHP variables include $ prefix in tree-sitter
        assert!(
            dfg.variables.contains(&"$y".to_string()) || dfg.variables.contains(&"y".to_string()),
            "should find y variable, got: {:?}", dfg.variables
        );
    }

    // --- Cross-language def-use edges ---

    #[test]
    fn test_typescript_def_use_edges() {
        let source = r#"
function foo() {
    let x = 1;
    let y = x + 2;
    return y;
}
"#;
        let dfg = get_dfg_context(source, "foo", Language::TypeScript).unwrap();

        let x_edges: Vec<_> = dfg.edges.iter()
            .filter(|e| e.var == "x")
            .collect();
        assert!(!x_edges.is_empty(), "should have def-use edge for x in TypeScript");
    }

    #[test]
    fn test_go_def_use_edges() {
        let source = r#"
func foo() int {
    x := 1
    y := x + 2
    return y
}
"#;
        let dfg = get_dfg_context(source, "foo", Language::Go).unwrap();

        let x_edges: Vec<_> = dfg.edges.iter()
            .filter(|e| e.var == "x")
            .collect();
        assert!(!x_edges.is_empty(), "should have def-use edge for x in Go");
    }

    #[test]
    fn test_rust_def_use_edges() {
        let source = r#"
fn foo() -> i32 {
    let x = 1;
    let y = x + 2;
    y
}
"#;
        let dfg = get_dfg_context(source, "foo", Language::Rust).unwrap();

        let x_edges: Vec<_> = dfg.edges.iter()
            .filter(|e| e.var == "x")
            .collect();
        assert!(!x_edges.is_empty(), "should have def-use edge for x in Rust");
    }

    // --- Python regression tests ---

    #[test]
    fn test_python_still_works_after_multilang() {
        let source = r#"
def foo(x):
    y = x + 1
    for item in [1, 2, 3]:
        y += item
    return y
"#;
        let dfg = get_dfg_context(source, "foo", Language::Python).unwrap();
        assert!(dfg.variables.contains(&"x".to_string()));
        assert!(dfg.variables.contains(&"y".to_string()));
        assert!(dfg.variables.contains(&"item".to_string()));

        let y_updates: Vec<_> = dfg.refs.iter()
            .filter(|r| r.name == "y" && r.ref_type == RefType::Update)
            .collect();
        assert!(!y_updates.is_empty(), "y += item should be an update");

        let item_defs: Vec<_> = dfg.refs.iter()
            .filter(|r| r.name == "item" && r.ref_type == RefType::Definition)
            .collect();
        assert!(!item_defs.is_empty(), "for item should be a definition");
    }

    // =========================================================================
    // NOGK language fix tests - These must produce definitions
    // =========================================================================

    #[test]
    fn test_kotlin_val_extraction() {
        let source = r#"
fun foo(x: Int): Int {
    val y = x + 1
    return y
}
"#;
        let dfg = get_dfg_context(source, "foo", Language::Kotlin).unwrap();

        let defs: Vec<_> = dfg.refs.iter()
            .filter(|r| r.ref_type == RefType::Definition)
            .map(|r| r.name.as_str())
            .collect();
        assert!(defs.contains(&"y"), "Kotlin val y should be a definition, got defs: {:?}", defs);
        assert!(defs.contains(&"x"), "Kotlin param x should be a definition, got defs: {:?}", defs);
    }

    #[test]
    fn test_kotlin_var_extraction() {
        let source = r#"
fun foo(): Int {
    var count = 0
    count = count + 1
    return count
}
"#;
        let dfg = get_dfg_context(source, "foo", Language::Kotlin).unwrap();

        let defs: Vec<_> = dfg.refs.iter()
            .filter(|r| matches!(r.ref_type, RefType::Definition | RefType::Update))
            .map(|r| r.name.as_str())
            .collect();
        assert!(!defs.is_empty(), "Kotlin var count should produce definitions, got defs: {:?}", defs);
    }

    #[test]
    fn test_elixir_match_extraction() {
        let source = r#"
def foo(x) do
  y = x + 1
  y
end
"#;
        let dfg = get_dfg_context(source, "foo", Language::Elixir).unwrap();

        let defs: Vec<_> = dfg.refs.iter()
            .filter(|r| r.ref_type == RefType::Definition)
            .map(|r| r.name.as_str())
            .collect();
        assert!(defs.contains(&"y"), "Elixir y = x + 1 should produce a definition, got defs: {:?}", defs);
    }

    #[test]
    fn test_scala_val_extraction() {
        let source = r#"
def foo(x: Int): Int = {
  val y = x + 1
  y
}
"#;
        let dfg = get_dfg_context(source, "foo", Language::Scala).unwrap();

        let defs: Vec<_> = dfg.refs.iter()
            .filter(|r| r.ref_type == RefType::Definition)
            .map(|r| r.name.as_str())
            .collect();
        assert!(defs.contains(&"y"), "Scala val y should be a definition, got defs: {:?}", defs);
    }

    #[test]
    fn test_cpp_declaration_extraction() {
        let source = r#"
int foo(int x) {
    int y = x + 1;
    return y;
}
"#;
        let dfg = get_dfg_context(source, "foo", Language::Cpp).unwrap();

        let defs: Vec<_> = dfg.refs.iter()
            .filter(|r| r.ref_type == RefType::Definition)
            .map(|r| r.name.as_str())
            .collect();
        assert!(defs.contains(&"y"), "C++ int y should be a definition, got defs: {:?}", defs);
        assert!(defs.contains(&"x"), "C++ param x should be a definition, got defs: {:?}", defs);
    }

    #[test]
    fn test_php_assignment_extraction() {
        let source = r#"<?php
function foo($x) {
    $y = $x + 1;
    return $y;
}
"#;
        let dfg = get_dfg_context(source, "foo", Language::Php).unwrap();

        let defs: Vec<_> = dfg.refs.iter()
            .filter(|r| r.ref_type == RefType::Definition)
            .map(|r| r.name.as_str())
            .collect();
        assert!(!defs.is_empty(), "PHP $y = $x + 1 should produce a definition, got defs: {:?}; all refs: {:?}",
            defs, dfg.refs);
    }

    #[test]
    fn test_ocaml_let_extraction() {
        let source = r#"
let foo x =
  let y = x + 1 in
  y
"#;
        let dfg = get_dfg_context(source, "foo", Language::Ocaml).unwrap();

        let defs: Vec<_> = dfg.refs.iter()
            .filter(|r| r.ref_type == RefType::Definition)
            .map(|r| r.name.as_str())
            .collect();
        assert!(defs.contains(&"y") || defs.contains(&"x"),
            "OCaml let y = x + 1 should produce definitions, got defs: {:?}; all refs: {:?}",
            defs, dfg.refs);
    }

    #[test]
    fn test_python_assignment_extracts_defs() {
        // Regression: Python should produce definitions in reaching-defs context
        let source = r#"
def foo(x):
    y = x + 1
    z = y * 2
    return z
"#;
        let dfg = get_dfg_context(source, "foo", Language::Python).unwrap();

        let defs: Vec<_> = dfg.refs.iter()
            .filter(|r| r.ref_type == RefType::Definition)
            .map(|r| r.name.as_str())
            .collect();
        assert!(defs.contains(&"y"), "Python y should be a definition, got defs: {:?}", defs);
        assert!(defs.contains(&"z"), "Python z should be a definition, got defs: {:?}", defs);
        assert!(defs.contains(&"x"), "Python param x should be a definition, got defs: {:?}", defs);
    }

    // =========================================================================
    // Reaching-defs NOGK fix tests
    // These tests verify that Lua and Swift DFG extraction produces
    // definitions for local variable declarations and assignments.
    // =========================================================================

    // --- Lua ---

    #[test]
    fn test_lua_local_declaration_produces_defs() {
        // Lua: `local y = x + 1` should produce a definition for y
        // AST: variable_declaration -> assignment_statement -> variable_list -> identifier
        let source = r#"function foo(x)
    local y = x + 1
    return y
end"#;
        let dfg = get_dfg_context(source, "foo", Language::Lua).unwrap();

        let defs: Vec<_> = dfg.refs.iter()
            .filter(|r| r.ref_type == RefType::Definition)
            .map(|r| r.name.as_str())
            .collect();
        assert!(defs.contains(&"y"),
            "Lua local y should be a definition, got defs: {:?}; all refs: {:?}", defs, dfg.refs);
    }

    #[test]
    fn test_lua_assignment_produces_defs() {
        // Lua: `z = y * 2` should produce a definition for z
        // AST: assignment_statement -> variable_list -> identifier
        let source = r#"function foo(x)
    local y = x + 1
    z = y * 2
    return z
end"#;
        let dfg = get_dfg_context(source, "foo", Language::Lua).unwrap();

        let defs: Vec<_> = dfg.refs.iter()
            .filter(|r| r.ref_type == RefType::Definition)
            .map(|r| r.name.as_str())
            .collect();
        assert!(defs.contains(&"z"),
            "Lua z = ... should be a definition, got defs: {:?}; all refs: {:?}", defs, dfg.refs);
    }

    #[test]
    fn test_lua_local_declaration_uses() {
        // Lua: `local y = x + 1` should produce a use for x
        let source = r#"function foo(x)
    local y = x + 1
    return y
end"#;
        let dfg = get_dfg_context(source, "foo", Language::Lua).unwrap();

        let uses: Vec<_> = dfg.refs.iter()
            .filter(|r| r.ref_type == RefType::Use)
            .map(|r| r.name.as_str())
            .collect();
        assert!(uses.contains(&"x"),
            "Lua x in `local y = x + 1` should be a use, got uses: {:?}", uses);
        assert!(uses.contains(&"y"),
            "Lua y in `return y` should be a use, got uses: {:?}", uses);
    }

    #[test]
    fn test_lua_param_extraction() {
        // Lua: function parameters should be definitions
        let source = r#"function foo(x, y)
    return x + y
end"#;
        let dfg = get_dfg_context(source, "foo", Language::Lua).unwrap();

        let defs: Vec<_> = dfg.refs.iter()
            .filter(|r| r.ref_type == RefType::Definition)
            .map(|r| r.name.as_str())
            .collect();
        assert!(defs.contains(&"x"),
            "Lua param x should be a definition, got defs: {:?}", defs);
        assert!(defs.contains(&"y"),
            "Lua param y should be a definition, got defs: {:?}", defs);
    }

    // --- Swift ---

    #[test]
    fn test_swift_let_declaration_produces_defs() {
        // Swift: `let y = x + 1` should produce a definition for y
        // AST: property_declaration -> pattern -> simple_identifier
        let source = r#"func foo(x: Int) -> Int {
    let y = x + 1
    return y
}"#;
        let dfg = get_dfg_context(source, "foo", Language::Swift).unwrap();

        let defs: Vec<_> = dfg.refs.iter()
            .filter(|r| r.ref_type == RefType::Definition)
            .map(|r| r.name.as_str())
            .collect();
        assert!(defs.contains(&"y"),
            "Swift let y should be a definition, got defs: {:?}; all refs: {:?}", defs, dfg.refs);
    }

    #[test]
    fn test_swift_var_declaration_produces_defs() {
        // Swift: `var z = y * 2` should produce a definition for z
        let source = r#"func foo(x: Int) -> Int {
    var z = x * 2
    z = z + 1
    return z
}"#;
        let dfg = get_dfg_context(source, "foo", Language::Swift).unwrap();

        let defs: Vec<_> = dfg.refs.iter()
            .filter(|r| matches!(r.ref_type, RefType::Definition | RefType::Update))
            .map(|r| r.name.as_str())
            .collect();
        assert!(defs.contains(&"z"),
            "Swift var z should be a definition, got defs: {:?}; all refs: {:?}", defs, dfg.refs);
    }

    #[test]
    fn test_swift_assignment_produces_defs() {
        // Swift: `z = z + 1` should produce a definition for z
        // AST: assignment -> directly_assignable_expression -> simple_identifier
        let source = r#"func foo(x: Int) -> Int {
    var z = x
    z = z + 1
    return z
}"#;
        let dfg = get_dfg_context(source, "foo", Language::Swift).unwrap();

        let z_defs: Vec<_> = dfg.refs.iter()
            .filter(|r| r.name == "z" && matches!(r.ref_type, RefType::Definition | RefType::Update))
            .collect();
        // At minimum: var z = x (def) and z = z + 1 (def)
        assert!(z_defs.len() >= 2,
            "Swift z should have at least 2 def/update refs (var z = x; z = z + 1), got {}: {:?}",
            z_defs.len(), z_defs);
    }

    #[test]
    fn test_swift_uses_extraction() {
        // Swift: uses should be extracted via simple_identifier
        let source = r#"func foo(x: Int) -> Int {
    let y = x + 1
    return y
}"#;
        let dfg = get_dfg_context(source, "foo", Language::Swift).unwrap();

        let uses: Vec<_> = dfg.refs.iter()
            .filter(|r| r.ref_type == RefType::Use)
            .map(|r| r.name.as_str())
            .collect();
        assert!(uses.contains(&"x"),
            "Swift x in `let y = x + 1` should be a use, got uses: {:?}", uses);
        assert!(uses.contains(&"y"),
            "Swift y in `return y` should be a use, got uses: {:?}", uses);
    }

    #[test]
    fn test_swift_param_extraction() {
        // Swift: function parameters should be definitions
        let source = r#"func foo(x: Int, y: Int) -> Int {
    return x + y
}"#;
        let dfg = get_dfg_context(source, "foo", Language::Swift).unwrap();

        let defs: Vec<_> = dfg.refs.iter()
            .filter(|r| r.ref_type == RefType::Definition)
            .map(|r| r.name.as_str())
            .collect();
        assert!(defs.contains(&"x"),
            "Swift param x should be a definition, got defs: {:?}", defs);
        assert!(defs.contains(&"y"),
            "Swift param y should be a definition, got defs: {:?}", defs);
    }
}
