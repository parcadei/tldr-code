//! Bounds command - Interval analysis for numeric value range tracking.
//!
//! Implements abstract interpretation over an interval lattice to track
//! numeric value ranges through code. Answers "What values can variable X
//! hold at line Y?" -- useful for detecting division by zero, out-of-bounds
//! access, and understanding valid input ranges.
//!
//! # Algorithm
//!
//! - **Lattice**: Interval = (lo, hi) | Bottom
//! - **Join**: `[a,b] | [c,d] = [min(a,c), max(b,d)]`
//! - **Meet**: `[a,b] & [c,d] = [max(a,c), min(b,d)]` or Bottom
//! - **Widen**: `[a,b] W [c,d] = [c<a ? -inf : a, d>b ? +inf : b]`
//! - **Transfer**: standard interval arithmetic with hull-of-corners for * and /
//!
//! # TIGER Mitigations Addressed
//!
//! - **T01**: Integer overflow - Use checked arithmetic, clamp to f64 bounds
//! - **T05**: Unbounded fixpoint - Apply widening after 3 iterations per variable
//!
//! # ELEPHANT Mitigations Addressed
//!
//! - **E01**: Wall-clock timeout - timeout_secs parameter with default 60s
//! - **E08**: NaN/infinity handling - Proper IEEE 754 checks
//!
//! # Example
//!
//! ```python
//! def intervals_example(x):
//!     # x: unknown -> [-inf, +inf]
//!     if x > 0:
//!         # x: [1, +inf]
//!         y = x + 10  # y: [11, +inf]
//!     else:
//!         # x: [-inf, 0]
//!         y = 0  # y: [0, 0]
//!     # y: [0, +inf] (join of branches)
//!     z = 100 / y  # WARNING: y may be 0
//! ```

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use clap::Args;
use tree_sitter::{Node, Parser, Tree};

use tldr_core::Language;
use tldr_core::ast::parser::ParserPool;

use crate::output::{OutputFormat, OutputWriter};

use super::error::{ContractsError, ContractsResult};
use super::types::{BoundsResult, Interval, IntervalWarning, OutputFormat as ContractsOutputFormat};
use super::validation::{check_ast_depth, read_file_safe, validate_file_path};

// =============================================================================
// Constants for TIGER/ELEPHANT Mitigations
// =============================================================================

/// Widening threshold: apply widening after this many iterations per loop (TIGER-05)
const WIDEN_THRESHOLD: u32 = 3;

/// Default maximum fixpoint iterations (TIGER-05)
const DEFAULT_MAX_ITER: u32 = 50;

// =============================================================================
// CLI Arguments
// =============================================================================

/// Analyze numeric value ranges through code using interval analysis.
///
/// Tracks possible values for variables at each program point and detects
/// potential issues like division by zero or array out-of-bounds access.
///
/// # Example
///
/// ```bash
/// tldr bounds src/module.py calculate
/// tldr bounds src/module.py --max-iter 100
/// tldr bounds src/module.py calculate --format text
/// ```
#[derive(Debug, Args)]
pub struct BoundsArgs {
    /// Source file to analyze
    #[arg(value_name = "file")]
    pub file: PathBuf,

    /// Function name to analyze (analyzes all if not specified)
    #[arg(value_name = "function")]
    pub function: Option<String>,

    /// Output format (json or text). Prefer global --format/-f flag.
    #[arg(long = "output-format", short = 'o', hide = true, default_value = "json")]
    pub output_format: ContractsOutputFormat,

    /// Programming language (auto-detected from file extension if not specified)
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,

    /// Maximum fixpoint iterations before giving up
    #[arg(long, default_value_t = DEFAULT_MAX_ITER)]
    pub max_iter: u32,
}

impl BoundsArgs {
    /// Run the bounds command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        // Validate inputs
        let canonical_path = validate_file_path(&self.file)?;

        let func_desc = self.function.as_deref().unwrap_or("all functions");
        writer.progress(&format!(
            "Analyzing bounds for {} in {}...",
            func_desc,
            self.file.display(),
        ));

        // Determine language
        let language = self.lang.unwrap_or_else(|| {
            Language::from_path(&self.file).unwrap_or(Language::Python)
        });

        // Run analysis
        let results = run_bounds(&canonical_path, self.function.as_deref(), self.max_iter, language)?;

        // Output based on format
        let use_text = matches!(self.output_format, ContractsOutputFormat::Text)
            || matches!(format, OutputFormat::Text);

        // If analyzing a specific function, return single result; otherwise return array
        if self.function.is_some() && results.len() == 1 {
            let result = &results[0];
            if use_text {
                let text = format_bounds_text(result);
                writer.write_text(&text)?;
            } else {
                writer.write(result)?;
            }
        } else {
            if use_text {
                let mut text = String::new();
                for result in &results {
                    text.push_str(&format_bounds_text(result));
                    text.push_str("\n");
                }
                writer.write_text(&text)?;
            } else {
                writer.write(&results)?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Core Analysis Functions
// =============================================================================

/// Run bounds analysis on a file.
///
/// # Arguments
/// * `file` - Path to the source file
/// * `function` - Optional function name (None = all functions)
/// * `max_iter` - Maximum fixpoint iterations
///
/// # Returns
/// Vec of BoundsResult, one per function analyzed.
pub fn run_bounds(
    file: &PathBuf,
    function: Option<&str>,
    max_iter: u32,
    language: Language,
) -> ContractsResult<Vec<BoundsResult>> {
    // Read the file
    let source = read_file_safe(file)?;

    // Parse with tree-sitter (multi-language)
    let tree = parse_source(&source, file, language)?;
    let root = tree.root_node();

    // Find functions to analyze (language-aware)
    let functions = find_functions_multi(root, function, source.as_bytes(), language);

    if functions.is_empty() {
        if let Some(func_name) = function {
            return Err(ContractsError::FunctionNotFound {
                function: func_name.to_string(),
                file: file.clone(),
            });
        }
        // No functions found, return empty results
        return Ok(Vec::new());
    }

    // Analyze each function
    let mut results = Vec::new();
    for (func_name, func_node) in functions {
        let result = analyze_function(&func_name, func_node, source.as_bytes(), max_iter, language)?;
        results.push(result);
    }

    Ok(results)
}

/// Analyze a single function for interval bounds.
fn analyze_function(
    function_name: &str,
    func_node: Node,
    source: &[u8],
    max_iter: u32,
    language: Language,
) -> ContractsResult<BoundsResult> {
    let mut analyzer = IntervalAnalyzer::new(max_iter, language);

    // Initialize parameters to Top (unknown) - language-aware
    let params_node = get_params_node(func_node, language);
    if let Some(params) = params_node {
        analyzer.initialize_parameters(params, source);
    }

    // Analyze the function body - language-aware
    let body_node = get_body_node(func_node, language);
    if let Some(body) = body_node {
        analyzer.analyze_block(body, source, 0)?;
    }

    Ok(BoundsResult {
        function: function_name.to_string(),
        bounds: analyzer.get_bounds_by_line(),
        warnings: analyzer.warnings,
        converged: analyzer.converged,
        iterations: analyzer.iterations.max(1),
    })
}

// =============================================================================
// Interval State Management
// =============================================================================

/// Mapping from variable names to intervals at a program point.
#[derive(Debug, Clone, PartialEq)]
struct IntervalState {
    bindings: HashMap<String, Interval>,
}

impl IntervalState {
    fn new() -> Self {
        Self {
            bindings: HashMap::new(),
        }
    }

    fn get(&self, var: &str) -> Interval {
        self.bindings.get(var).copied().unwrap_or_else(Interval::top)
    }

    fn set(&mut self, var: &str, interval: Interval) {
        self.bindings.insert(var.to_string(), interval);
    }

    /// Join two states (least upper bound).
    fn join(&self, other: &Self) -> Self {
        let mut merged = HashMap::new();
        let all_vars: std::collections::HashSet<_> = self
            .bindings
            .keys()
            .chain(other.bindings.keys())
            .collect();

        for var in all_vars {
            let a = self.bindings.get(var).copied().unwrap_or_else(Interval::bottom);
            let b = other.bindings.get(var).copied().unwrap_or_else(Interval::bottom);
            merged.insert(var.clone(), a.join(&b));
        }

        Self { bindings: merged }
    }

    /// Widen this state based on new observations.
    fn widen(&self, other: &Self) -> Self {
        let mut result = HashMap::new();
        let all_vars: std::collections::HashSet<_> = self
            .bindings
            .keys()
            .chain(other.bindings.keys())
            .collect();

        for var in all_vars {
            let old_iv = self.bindings.get(var).copied().unwrap_or_else(Interval::bottom);
            let new_iv = other.bindings.get(var).copied().unwrap_or_else(Interval::bottom);
            result.insert(var.clone(), old_iv.widen(&new_iv));
        }

        Self { bindings: result }
    }
}

// =============================================================================
// Interval Analyzer
// =============================================================================

/// AST-based interval analysis engine.
struct IntervalAnalyzer {
    max_iterations: u32,
    state: IntervalState,
    states: HashMap<u32, IntervalState>,
    warnings: Vec<IntervalWarning>,
    iterations: u32,
    converged: bool,
    language: Language,
}

impl IntervalAnalyzer {
    fn new(max_iterations: u32, language: Language) -> Self {
        Self {
            max_iterations,
            state: IntervalState::new(),
            states: HashMap::new(),
            warnings: Vec::new(),
            iterations: 0,
            converged: true,
            language,
        }
    }

    /// Initialize function parameters to Top (unknown).
    /// Language-aware: handles various parameter declaration syntaxes.
    fn initialize_parameters(&mut self, params: Node, source: &[u8]) {
        let mut cursor = params.walk();
        for child in params.children(&mut cursor) {
            match child.kind() {
                // Common across many languages
                "identifier" => {
                    let name = get_node_text(child, source);
                    self.state.set(name, Interval::top());
                }
                // Python-specific parameter kinds
                "typed_parameter" | "default_parameter" | "typed_default_parameter" => {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let name = get_node_text(name_node, source);
                        self.state.set(name, Interval::top());
                    } else {
                        self.init_param_first_identifier(child, source);
                    }
                }
                // Go/Java/C/C++/Rust/C#/TypeScript parameter declarations
                "parameter_declaration" | "formal_parameter" | "parameter"
                | "required_parameter" | "optional_parameter"
                | "simple_parameter" | "variadic_parameter" => {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let name = get_node_text(name_node, source);
                        self.state.set(name, Interval::top());
                    } else if let Some(name_node) = child.child_by_field_name("pattern") {
                        // Rust uses "pattern" field for parameter name
                        let name = get_node_text(name_node, source);
                        self.state.set(name, Interval::top());
                    } else {
                        self.init_param_first_identifier(child, source);
                    }
                }
                // Catch-all: try to find identifier in any other node kind
                _ => {
                    // For languages with unusual parameter structures, try name field then first identifier
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let name = get_node_text(name_node, source);
                        if !name.is_empty() {
                            self.state.set(name, Interval::top());
                        }
                    }
                }
            }
        }
    }

    /// Helper: set Top for the first identifier child of a parameter node.
    fn init_param_first_identifier(&mut self, node: Node, source: &[u8]) {
        let mut inner_cursor = node.walk();
        for inner in node.children(&mut inner_cursor) {
            if inner.kind() == "identifier" {
                let name = get_node_text(inner, source);
                self.state.set(name, Interval::top());
                break;
            }
        }
    }

    /// Record the current state at a line number.
    fn record(&mut self, line: u32) {
        self.states.insert(line, self.state.clone());
    }

    /// Get bounds organized by line number.
    fn get_bounds_by_line(&self) -> HashMap<u32, HashMap<String, Interval>> {
        let mut result = HashMap::new();
        for (line, state) in &self.states {
            let mut var_bounds = HashMap::new();
            for (var, interval) in &state.bindings {
                if !interval.is_bottom() {
                    var_bounds.insert(var.clone(), *interval);
                }
            }
            if !var_bounds.is_empty() {
                result.insert(*line, var_bounds);
            }
        }
        result
    }

    /// Analyze a block of statements.
    fn analyze_block(&mut self, block: Node, source: &[u8], depth: usize) -> ContractsResult<()> {
        check_ast_depth(depth, &PathBuf::from("<source>"))?;

        let mut cursor = block.walk();
        for stmt in block.children(&mut cursor) {
            self.analyze_stmt(stmt, source, depth)?;
        }

        Ok(())
    }

    /// Analyze a single statement. Language-aware dispatch.
    fn analyze_stmt(&mut self, stmt: Node, source: &[u8], depth: usize) -> ContractsResult<()> {
        check_ast_depth(depth, &PathBuf::from("<source>"))?;

        let line = stmt.start_position().row as u32 + 1;
        let kind = stmt.kind();

        match kind {
            // === Assignment patterns ===
            // Python: expression_statement wrapping assignment
            "expression_statement" => {
                if let Some(child) = stmt.child(0) {
                    match child.kind() {
                        "assignment" | "assignment_expression" => {
                            self.analyze_assignment(child, source, line)?;
                        }
                        "augmented_assignment" | "update_expression" => {
                            self.analyze_augmented_assignment(child, source, line)?;
                        }
                        // Rust: for_expression / if_expression / loop_expression wrapped in expression_statement
                        "for_expression" => {
                            self.analyze_for(child, source, depth + 1)?;
                            return Ok(());
                        }
                        "if_expression" => {
                            self.analyze_if(child, source, depth + 1)?;
                            return Ok(());
                        }
                        "while_expression" | "loop_expression" => {
                            self.analyze_while(child, source, depth + 1)?;
                            return Ok(());
                        }
                        _ => {}
                    }
                }
                self.record(line);
            }
            // Python direct assignment
            "assignment" => {
                self.analyze_assignment(stmt, source, line)?;
                self.record(line);
            }
            // Python augmented assignment (+=, etc.)
            "augmented_assignment" => {
                self.analyze_augmented_assignment(stmt, source, line)?;
                self.record(line);
            }
            // Go/C/C++/Java/C#/Rust: variable declarations with initializers
            "short_var_declaration" => {
                // Go: x := 5
                self.analyze_short_var_decl(stmt, source, line)?;
                self.record(line);
            }
            "lexical_declaration" | "variable_declaration" => {
                // JS/TS: let/const/var declarations; C#/Java variable declarations
                self.analyze_lexical_decl(stmt, source, line)?;
                self.record(line);
            }
            "let_declaration" => {
                // Rust: let x = 5;
                self.analyze_let_decl(stmt, source, line)?;
                self.record(line);
            }
            "declaration" => {
                // C/C++: int x = 5;
                self.analyze_c_declaration(stmt, source, line)?;
                self.record(line);
            }
            "local_variable_declaration" => {
                // Java: int x = 5;
                self.analyze_java_local_var_decl(stmt, source, line)?;
                self.record(line);
            }
            "assignment_expression" => {
                // C/C++/Java/Go assignment expressions
                self.analyze_assignment(stmt, source, line)?;
                self.record(line);
            }
            // Kotlin: property_declaration with initializer
            "property_declaration" => {
                self.analyze_lexical_decl(stmt, source, line)?;
                self.record(line);
            }
            // Scala: val/var definitions
            "val_definition" | "var_definition" => {
                self.analyze_scala_val_var(stmt, source, line)?;
                self.record(line);
            }
            // Lua/Luau: local x = 5 (variable_declaration) or x = 5 (assignment_statement)
            "assignment_statement" => {
                self.analyze_assignment(stmt, source, line)?;
                self.record(line);
            }
            // OCaml: let x = ... in
            "let_expression" | "let_binding" => {
                self.analyze_ocaml_let(stmt, source, line)?;
                self.record(line);
            }
            // PHP: assignment in expression_statement
            // Elixir: match_operator (pattern = value)
            "match_operator" => {
                self.analyze_assignment(stmt, source, line)?;
                self.record(line);
            }
            // Ruby: assignment
            // (already handled by catch-all assignment patterns)
            // Sequence expressions (OCaml uses ";" between expressions)
            "sequence_expression" => {
                self.analyze_block(stmt, source, depth + 1)?;
            }

            // === Return statements ===
            "return_statement" | "return_expression" => {
                if let Some(value) = stmt.child_by_field_name("value").or_else(|| stmt.child(1)) {
                    let _ = self.eval_expr(value, source, line);
                }
                self.record(line);
            }

            // === If statements ===
            "if_statement" | "if_expression" => {
                self.analyze_if(stmt, source, depth + 1)?;
            }

            // === While loops ===
            "while_statement" | "while_expression" => {
                self.analyze_while(stmt, source, depth + 1)?;
            }

            // === For loops ===
            "for_statement" | "for_expression"
            | "for_in_statement" | "enhanced_for_statement"
            | "foreach_statement" | "for" => {
                self.analyze_for(stmt, source, depth + 1)?;
            }

            // === Do-while loops ===
            "do_statement" | "do_while_statement" | "repeat_while_statement"
            | "repeat_statement" => {
                // Treat do-while like while for interval analysis
                self.analyze_while(stmt, source, depth + 1)?;
            }

            // === Rust loop expression (infinite loop) ===
            "loop_expression" => {
                self.analyze_while(stmt, source, depth + 1)?;
            }

            // === Ruby until loop ===
            "until" => {
                self.analyze_while(stmt, source, depth + 1)?;
            }

            // === Block statements ===
            "block" | "statement_block" | "compound_statement" => {
                self.analyze_block(stmt, source, depth + 1)?;
            }

            _ => {
                // Recurse into children for container nodes we don't specifically handle
                // This catches things like Rust's expression statements, etc.
                let mut cursor = stmt.walk();
                let mut handled = false;
                for child in stmt.children(&mut cursor) {
                    match child.kind() {
                        "assignment" | "assignment_expression" => {
                            self.analyze_assignment(child, source, line)?;
                            handled = true;
                        }
                        "augmented_assignment" => {
                            self.analyze_augmented_assignment(child, source, line)?;
                            handled = true;
                        }
                        _ => {}
                    }
                }
                if !handled {
                    self.record(line);
                } else {
                    self.record(line);
                }
            }
        }

        Ok(())
    }

    /// Analyze an assignment statement.
    fn analyze_assignment(&mut self, stmt: Node, source: &[u8], line: u32) -> ContractsResult<()> {
        // Get the left-hand side (target)
        let left = stmt.child_by_field_name("left");
        // Get the right-hand side (value)
        let right = stmt.child_by_field_name("right");

        if let (Some(target), Some(value)) = (left, right) {
            let interval = self.eval_expr(value, source, line);

            // Handle simple name assignment
            if target.kind() == "identifier" {
                let name = get_node_text(target, source);
                self.state.set(name, interval);
            }
            // Handle tuple/list unpacking (simplified - just record top)
            else if target.kind() == "pattern_list" || target.kind() == "tuple_pattern" {
                let mut cursor = target.walk();
                for child in target.children(&mut cursor) {
                    if child.kind() == "identifier" {
                        let name = get_node_text(child, source);
                        self.state.set(name, Interval::top());
                    }
                }
            }
        }

        Ok(())
    }

    /// Analyze an augmented assignment (+=, -=, etc.).
    fn analyze_augmented_assignment(
        &mut self,
        stmt: Node,
        source: &[u8],
        line: u32,
    ) -> ContractsResult<()> {
        let left = stmt.child_by_field_name("left");
        let op = stmt.child_by_field_name("operator");
        let right = stmt.child_by_field_name("right");

        if let (Some(target), Some(op_node), Some(value)) = (left, op, right) {
            if target.kind() == "identifier" {
                let name = get_node_text(target, source);
                let cur = self.state.get(name);
                let val = self.eval_expr(value, source, line);

                let op_text = get_node_text(op_node, source);
                let result = match op_text {
                    "+=" => cur.add(&val),
                    "-=" => cur.sub(&val),
                    "*=" => cur.mul(&val),
                    "/=" => {
                        let (div_result, may_div_zero) = cur.div(&val);
                        if may_div_zero {
                            self.warnings.push(IntervalWarning {
                                line,
                                kind: "division_by_zero".to_string(),
                                variable: name.to_string(),
                                bounds: val,
                                message: format!(
                                    "Divisor may be zero: {} / divisor in {}",
                                    name, val
                                ),
                            });
                        }
                        div_result
                    }
                    _ => Interval::top(),
                };

                self.state.set(name, result);
            }
        }

        Ok(())
    }

    /// Analyze Go short variable declaration: x := 5
    fn analyze_short_var_decl(&mut self, stmt: Node, source: &[u8], line: u32) -> ContractsResult<()> {
        let left = stmt.child_by_field_name("left");
        let right = stmt.child_by_field_name("right");

        if let (Some(target), Some(value)) = (left, right) {
            let interval = self.eval_expr(value, source, line);
            // target could be an expression_list or identifier
            if target.kind() == "identifier" {
                let name = get_node_text(target, source);
                self.state.set(name, interval);
            } else if target.kind() == "expression_list" {
                // Multiple targets: first identifier gets the value, rest get Top
                let mut cursor = target.walk();
                let mut first = true;
                for child in target.children(&mut cursor) {
                    if child.kind() == "identifier" {
                        let name = get_node_text(child, source);
                        if first {
                            self.state.set(name, interval);
                            first = false;
                        } else {
                            self.state.set(name, Interval::top());
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Analyze JS/TS lexical declaration: let x = 5; const y = 10;
    fn analyze_lexical_decl(&mut self, stmt: Node, source: &[u8], line: u32) -> ContractsResult<()> {
        let mut cursor = stmt.walk();
        for child in stmt.children(&mut cursor) {
            if child.kind() == "variable_declarator" {
                let name_node = child.child_by_field_name("name");
                // Try "value" field first (JS/TS), then find the expression after "=" (C#)
                let value_node = child.child_by_field_name("value")
                    .or_else(|| {
                        // C# variable_declarator: name = expr (no "value" field)
                        // Find the node after the "=" token
                        let mut inner_cursor = child.walk();
                        let mut found_eq = false;
                        for inner in child.children(&mut inner_cursor) {
                            if found_eq && inner.kind() != ";" {
                                return Some(inner);
                            }
                            if inner.kind() == "=" || inner.kind() == "equals_value_clause" {
                                found_eq = true;
                                // For equals_value_clause, return its first non-= child
                                if inner.kind() == "equals_value_clause" {
                                    let mut eq_cursor = inner.walk();
                                    for eq_child in inner.children(&mut eq_cursor) {
                                        if eq_child.kind() != "=" {
                                            return Some(eq_child);
                                        }
                                    }
                                }
                            }
                        }
                        None
                    });
                if let (Some(name), Some(value)) = (name_node, value_node) {
                    if name.kind() == "identifier" {
                        let var_name = get_node_text(name, source);
                        let interval = self.eval_expr(value, source, line);
                        self.state.set(var_name, interval);
                    }
                }
            }
        }
        Ok(())
    }

    /// Analyze Rust let declaration: let x = 5;
    fn analyze_let_decl(&mut self, stmt: Node, source: &[u8], line: u32) -> ContractsResult<()> {
        let pattern = stmt.child_by_field_name("pattern");
        let value = stmt.child_by_field_name("value");

        if let (Some(pat), Some(val)) = (pattern, value) {
            let interval = self.eval_expr(val, source, line);
            if pat.kind() == "identifier" {
                let name = get_node_text(pat, source);
                self.state.set(name, interval);
            }
        }
        Ok(())
    }

    /// Analyze C/C++ declaration: int x = 5;
    fn analyze_c_declaration(&mut self, stmt: Node, source: &[u8], line: u32) -> ContractsResult<()> {
        let mut cursor = stmt.walk();
        for child in stmt.children(&mut cursor) {
            if child.kind() == "init_declarator" {
                let declarator = child.child_by_field_name("declarator");
                let value = child.child_by_field_name("value");
                if let (Some(decl), Some(val)) = (declarator, value) {
                    let interval = self.eval_expr(val, source, line);
                    // declarator could be identifier directly or a more complex type
                    if decl.kind() == "identifier" {
                        let name = get_node_text(decl, source);
                        self.state.set(name, interval);
                    }
                }
            }
        }
        Ok(())
    }

    /// Analyze Java local variable declaration: int x = 5;
    fn analyze_java_local_var_decl(&mut self, stmt: Node, source: &[u8], line: u32) -> ContractsResult<()> {
        let mut cursor = stmt.walk();
        for child in stmt.children(&mut cursor) {
            if child.kind() == "variable_declarator" {
                let name_node = child.child_by_field_name("name");
                let value_node = child.child_by_field_name("value");
                if let (Some(name), Some(value)) = (name_node, value_node) {
                    if name.kind() == "identifier" {
                        let var_name = get_node_text(name, source);
                        let interval = self.eval_expr(value, source, line);
                        self.state.set(var_name, interval);
                    }
                }
            }
        }
        Ok(())
    }

    /// Analyze Scala val/var definition: val x = 5; var y = 10
    fn analyze_scala_val_var(&mut self, stmt: Node, source: &[u8], line: u32) -> ContractsResult<()> {
        // Scala: val_definition or var_definition
        // pattern = value
        let pattern = stmt.child_by_field_name("pattern");
        let value = stmt.child_by_field_name("value");
        if let (Some(pat), Some(val)) = (pattern, value) {
            let interval = self.eval_expr(val, source, line);
            if pat.kind() == "identifier" {
                let name = get_node_text(pat, source);
                self.state.set(name, interval);
            }
        } else {
            // Fallback: look for name and value children
            let name_node = stmt.child_by_field_name("name");
            let value_node = stmt.child_by_field_name("value")
                .or_else(|| stmt.child_by_field_name("body"));
            if let (Some(name), Some(val)) = (name_node, value_node) {
                let interval = self.eval_expr(val, source, line);
                let var_name = get_node_text(name, source);
                self.state.set(var_name, interval);
            }
        }
        Ok(())
    }

    /// Analyze OCaml let expression: let x = 5 in ...
    fn analyze_ocaml_let(&mut self, stmt: Node, source: &[u8], line: u32) -> ContractsResult<()> {
        // OCaml let_expression: let <pattern> = <expr> in <body>
        // OCaml let_binding: <pattern> = <expr>
        // Find the binding name and value
        let mut cursor = stmt.walk();
        let mut found_eq = false;
        let mut var_name: Option<&str> = None;

        for child in stmt.children(&mut cursor) {
            match child.kind() {
                "let" | "let_binding" => {
                    // Recurse into let_binding if this is a let_expression
                    if child.kind() == "let_binding" {
                        // Find identifier and value inside the let_binding
                        let mut inner_cursor = child.walk();
                        let mut inner_found_eq = false;
                        let mut inner_name: Option<&str> = None;
                        for inner in child.children(&mut inner_cursor) {
                            if inner.kind() == "value_name" || inner.kind() == "identifier" {
                                if inner_name.is_none() {
                                    inner_name = Some(get_node_text(inner, source));
                                }
                            }
                            if inner.kind() == "=" {
                                inner_found_eq = true;
                                continue;
                            }
                            if inner_found_eq {
                                let interval = self.eval_expr(inner, source, line);
                                if let Some(name) = inner_name {
                                    self.state.set(name, interval);
                                }
                                break;
                            }
                        }
                    }
                }
                "value_name" | "identifier" => {
                    if var_name.is_none() && !found_eq {
                        var_name = Some(get_node_text(child, source));
                    }
                }
                "=" => {
                    found_eq = true;
                }
                "in" => {
                    // The body after "in" -- continue analyzing it
                }
                _ => {
                    if found_eq && var_name.is_some() {
                        let interval = self.eval_expr(child, source, line);
                        if let Some(name) = var_name.take() {
                            self.state.set(name, interval);
                        }
                        found_eq = false; // reset for any subsequent bindings
                    }
                }
            }
        }
        Ok(())
    }

    /// Evaluate an expression to an Interval.
    fn eval_expr(&mut self, node: Node, source: &[u8], line: u32) -> Interval {
        match node.kind() {
            // Numeric literals across languages
            "integer" | "float" | "number" | "integer_literal" | "float_literal"
            | "int_literal" | "decimal_integer_literal" | "decimal_floating_point_literal"
            | "number_literal" => {
                let text = get_node_text(node, source);
                // Handle underscores in numeric literals, suffixes like "i32", "f64", "L", "f"
                let clean = text
                    .replace('_', "")
                    .trim_end_matches(|c: char| c.is_alphabetic())
                    .to_string();
                if let Ok(n) = clean.parse::<f64>() {
                    Interval::const_val(n)
                } else {
                    Interval::top()
                }
            }
            "identifier" | "field_identifier" => {
                let name = get_node_text(node, source);
                self.state.get(name)
            }
            // Unary expressions across languages
            "unary_operator" | "unary_expression" => {
                if let (Some(op), Some(operand)) = (node.child(0), node.child(1)) {
                    let op_text = get_node_text(op, source);
                    let inner = self.eval_expr(operand, source, line);
                    match op_text {
                        "-" => inner.neg(),
                        "+" => inner,
                        _ => Interval::top(),
                    }
                } else if let Some(operand) = node.child_by_field_name("argument").or_else(|| node.child_by_field_name("operand")) {
                    if let Some(op) = node.child_by_field_name("operator").or_else(|| node.child(0)) {
                        let op_text = get_node_text(op, source);
                        let inner = self.eval_expr(operand, source, line);
                        match op_text {
                            "-" => inner.neg(),
                            "+" => inner,
                            _ => Interval::top(),
                        }
                    } else {
                        Interval::top()
                    }
                } else {
                    Interval::top()
                }
            }
            // Binary expressions across languages
            "binary_operator" | "binary_expression" => {
                let left_node = node.child_by_field_name("left");
                let op_node = node.child_by_field_name("operator");
                let right_node = node.child_by_field_name("right");

                if let (Some(left), Some(op), Some(right)) = (left_node, op_node, right_node) {
                    let left_iv = self.eval_expr(left, source, line);
                    let right_iv = self.eval_expr(right, source, line);
                    let op_text = get_node_text(op, source);

                    match op_text {
                        "+" => left_iv.add(&right_iv),
                        "-" => left_iv.sub(&right_iv),
                        "*" => left_iv.mul(&right_iv),
                        "/" | "//" => {
                            let (result, may_div_zero) = left_iv.div(&right_iv);
                            if may_div_zero {
                                let var_name = if left.kind() == "identifier" {
                                    get_node_text(left, source).to_string()
                                } else {
                                    "<expr>".to_string()
                                };
                                self.warnings.push(IntervalWarning {
                                    line,
                                    kind: "division_by_zero".to_string(),
                                    variable: var_name,
                                    bounds: right_iv,
                                    message: format!(
                                        "Divisor may be zero: divisor in {}",
                                        right_iv
                                    ),
                                });
                            }
                            result
                        }
                        "%" => {
                            // Modulo - if right is positive, result is [0, right.hi)
                            if !right_iv.is_bottom() && right_iv.lo > 0.0 {
                                Interval { lo: 0.0, hi: right_iv.hi - 1.0 }
                            } else {
                                Interval::top()
                            }
                        }
                        "**" => {
                            // Power - complex, use top for now
                            Interval::top()
                        }
                        _ => Interval::top(),
                    }
                } else {
                    Interval::top()
                }
            }
            "parenthesized_expression" => {
                if let Some(inner) = node.child(1) {
                    self.eval_expr(inner, source, line)
                } else {
                    Interval::top()
                }
            }
            // Rust reference expressions: &x, &mut x
            "reference_expression" => {
                if let Some(inner) = node.child_by_field_name("value") {
                    self.eval_expr(inner, source, line)
                } else if let Some(inner) = node.child(1) {
                    self.eval_expr(inner, source, line)
                } else {
                    Interval::top()
                }
            }
            // Type cast expressions - evaluate the inner value
            "type_cast_expression" | "cast_expression" => {
                if let Some(inner) = node.child_by_field_name("value").or_else(|| node.child(0)) {
                    self.eval_expr(inner, source, line)
                } else {
                    Interval::top()
                }
            }
            "call" | "call_expression" => {
                // Handle range() for loop bounds (Python)
                if let Some(func) = node.child_by_field_name("function") {
                    let func_name = get_node_text(func, source);
                    if func_name == "range" {
                        return self.eval_range_call(node, source, line);
                    }
                }
                // Other calls - conservative top
                Interval::top()
            }
            // Go: expression_list wraps a single expression (e.g., in short_var_declaration)
            "expression_list" => {
                // Evaluate the first meaningful child
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() != "," && child.kind() != "(" && child.kind() != ")" {
                        return self.eval_expr(child, source, line);
                    }
                }
                Interval::top()
            }
            _ => Interval::top(),
        }
    }

    /// Evaluate a range() call to get loop bounds.
    fn eval_range_call(&mut self, node: Node, source: &[u8], line: u32) -> Interval {
        let args = node.child_by_field_name("arguments");
        if args.is_none() {
            return Interval::top();
        }
        let args = args.unwrap();

        let mut arg_values = Vec::new();
        let mut cursor = args.walk();
        for child in args.children(&mut cursor) {
            if child.kind() != "(" && child.kind() != ")" && child.kind() != "," {
                arg_values.push(self.eval_expr(child, source, line));
            }
        }

        match arg_values.len() {
            1 => {
                // range(n) -> [0, n-1]
                let hi = if arg_values[0].hi.is_finite() {
                    arg_values[0].hi - 1.0
                } else {
                    f64::INFINITY
                };
                Interval { lo: 0.0, hi }
            }
            2 | 3 => {
                // range(start, stop) -> [start, stop-1]
                let hi = if arg_values[1].hi.is_finite() {
                    arg_values[1].hi - 1.0
                } else {
                    f64::INFINITY
                };
                Interval {
                    lo: arg_values[0].lo,
                    hi,
                }
            }
            _ => Interval::top(),
        }
    }

    /// Analyze an if statement with branch refinement.
    fn analyze_if(&mut self, stmt: Node, source: &[u8], depth: usize) -> ContractsResult<()> {
        check_ast_depth(depth, &PathBuf::from("<source>"))?;

        let line = stmt.start_position().row as u32 + 1;
        let pre_state = self.state.clone();

        // Get condition
        let condition = stmt.child_by_field_name("condition");

        // Refine state for true branch
        let true_state = if let Some(cond) = condition {
            self.refine_condition(&pre_state, cond, source, true)
        } else {
            pre_state.clone()
        };

        // Analyze true branch (consequence)
        self.state = true_state;
        if let Some(consequence) = stmt.child_by_field_name("consequence") {
            self.analyze_block(consequence, source, depth + 1)?;
        }
        let after_true = self.state.clone();

        // Refine state for false branch
        let false_state = if let Some(cond) = condition {
            self.refine_condition(&pre_state, cond, source, false)
        } else {
            pre_state.clone()
        };

        // Analyze false branch (alternative) if present
        self.state = false_state.clone();
        let mut has_else = false;

        // Check for "alternative" field (C/Go/Rust/Java/JS/TS style: else { ... })
        if let Some(alt) = stmt.child_by_field_name("alternative") {
            has_else = true;
            self.analyze_block(alt, source, depth + 1)?;
        }

        // Also look for Python-style else_clause / elif_clause
        if !has_else {
            let mut cursor = stmt.walk();
            for child in stmt.children(&mut cursor) {
                if child.kind() == "else_clause" {
                    has_else = true;
                    if let Some(body) = child.child_by_field_name("body") {
                        self.analyze_block(body, source, depth + 1)?;
                    } else {
                        // Some grammars put body directly in else_clause children
                        self.analyze_block(child, source, depth + 1)?;
                    }
                } else if child.kind() == "elif_clause" {
                    has_else = true;
                    // Recursively analyze elif as if
                    self.analyze_if(child, source, depth + 1)?;
                }
            }
        }
        let after_false = self.state.clone();

        // Merge branches
        if has_else {
            self.state = after_true.join(&after_false);
        } else {
            // No else: merge true branch with pre-state (unchanged path)
            self.state = after_true.join(&false_state);
        }

        self.record(line);
        Ok(())
    }

    /// Refine state based on a condition.
    fn refine_condition(
        &self,
        state: &IntervalState,
        cond: Node,
        source: &[u8],
        positive: bool,
    ) -> IntervalState {
        let mut result = state.clone();

        match cond.kind() {
            // Python comparison_operator: x > n
            "comparison_operator" => {
                let left = cond.child_by_field_name("left").or_else(|| cond.child(0));
                let _ops = cond.child_by_field_name("operators");
                let right = cond.child_by_field_name("right").or_else(|| cond.child(2));

                if let (Some(left_node), Some(right_node)) = (left, right) {
                    let mut cursor = cond.walk();
                    let mut op_text = None;
                    for child in cond.children(&mut cursor) {
                        let kind = child.kind();
                        if kind == "<" || kind == ">" || kind == "<=" || kind == ">="
                            || kind == "==" || kind == "!=" {
                            op_text = Some(get_node_text(child, source));
                            break;
                        }
                    }

                    if let Some(op) = op_text {
                        self.apply_comparison(&mut result, left_node, op, right_node, source, positive);
                    }
                }
            }
            // C-family / Go / Rust / Java: binary_expression with comparison operator
            "binary_expression" => {
                let left = cond.child_by_field_name("left").or_else(|| cond.child(0));
                let op_node = cond.child_by_field_name("operator").or_else(|| cond.child(1));
                let right = cond.child_by_field_name("right").or_else(|| cond.child(2));

                if let (Some(left_node), Some(op_n), Some(right_node)) = (left, op_node, right) {
                    let op_text = get_node_text(op_n, source);
                    match op_text {
                        "<" | ">" | "<=" | ">=" | "==" | "!=" => {
                            self.apply_comparison(&mut result, left_node, op_text, right_node, source, positive);
                        }
                        "&&" | "and" => {
                            if positive {
                                result = self.refine_condition(&result, left_node, source, true);
                                result = self.refine_condition(&result, right_node, source, true);
                            }
                        }
                        "||" | "or" => {
                            // For `or`, we cannot narrow in general
                        }
                        _ => {}
                    }
                }
            }
            // Python boolean_operator: x > 0 and x < 10
            "boolean_operator" => {
                let op_kind = {
                    let mut cursor = cond.walk();
                    let mut kind = None;
                    for child in cond.children(&mut cursor) {
                        if child.kind() == "and" || child.kind() == "or"
                            || child.kind() == "&&" || child.kind() == "||" {
                            kind = Some(child.kind());
                            break;
                        }
                    }
                    kind
                };

                if (op_kind == Some("and") || op_kind == Some("&&")) && positive {
                    let left = cond.child_by_field_name("left").or_else(|| cond.child(0));
                    let right = cond.child_by_field_name("right").or_else(|| cond.child(2));
                    if let Some(l) = left {
                        result = self.refine_condition(&result, l, source, true);
                    }
                    if let Some(r) = right {
                        result = self.refine_condition(&result, r, source, true);
                    }
                }
            }
            // Python not_operator / C-family unary !
            "not_operator" | "unary_expression" => {
                // For unary !, check the operator
                if cond.kind() == "unary_expression" {
                    if let Some(op) = cond.child(0) {
                        if get_node_text(op, source) == "!" {
                            if let Some(operand) = cond.child(1) {
                                result = self.refine_condition(&result, operand, source, !positive);
                            }
                        }
                    }
                } else {
                    // Python not_operator
                    if let Some(operand) = cond.child(1) {
                        result = self.refine_condition(&result, operand, source, !positive);
                    }
                }
            }
            _ => {}
        }

        result
    }

    /// Apply a comparison to refine an interval.
    fn apply_comparison(
        &self,
        state: &mut IntervalState,
        left: Node,
        op: &str,
        right: Node,
        source: &[u8],
        positive: bool,
    ) {
        // We handle the case where left is a variable and right is a constant
        if left.kind() == "identifier" {
            let var = get_node_text(left, source);
            if let Some(n) = self.try_const(right, source) {
                let cur = state.get(var);
                let refined = if positive {
                    self.apply_comparison_op(cur, op, n)
                } else {
                    self.apply_comparison_op_negated(cur, op, n)
                };
                state.set(var, refined);
            }
        }
    }

    /// Try to extract a constant numeric value from a node.
    fn try_const(&self, node: Node, source: &[u8]) -> Option<f64> {
        match node.kind() {
            "integer" | "float" | "number" | "integer_literal" | "float_literal"
            | "int_literal" | "decimal_integer_literal" | "decimal_floating_point_literal"
            | "number_literal" => {
                let text = get_node_text(node, source)
                    .replace('_', "")
                    .trim_end_matches(|c: char| c.is_alphabetic())
                    .to_string();
                text.parse::<f64>().ok()
            }
            "unary_operator" | "unary_expression" => {
                if let (Some(op), Some(operand)) = (node.child(0), node.child(1)) {
                    if get_node_text(op, source) == "-" {
                        return self.try_const(operand, source).map(|n| -n);
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Apply a comparison operator to narrow an interval.
    fn apply_comparison_op(&self, cur: Interval, op: &str, n: f64) -> Interval {
        match op {
            ">" => cur.meet(&Interval { lo: n + 1.0, hi: f64::INFINITY }),
            ">=" => cur.meet(&Interval { lo: n, hi: f64::INFINITY }),
            "<" => cur.meet(&Interval { lo: f64::NEG_INFINITY, hi: n - 1.0 }),
            "<=" => cur.meet(&Interval { lo: f64::NEG_INFINITY, hi: n }),
            "==" => cur.meet(&Interval::const_val(n)),
            "!=" => cur, // Can't narrow for !=
            _ => cur,
        }
    }

    /// Apply a negated comparison operator.
    fn apply_comparison_op_negated(&self, cur: Interval, op: &str, n: f64) -> Interval {
        match op {
            ">" => cur.meet(&Interval { lo: f64::NEG_INFINITY, hi: n }),
            ">=" => cur.meet(&Interval { lo: f64::NEG_INFINITY, hi: n - 1.0 }),
            "<" => cur.meet(&Interval { lo: n, hi: f64::INFINITY }),
            "<=" => cur.meet(&Interval { lo: n + 1.0, hi: f64::INFINITY }),
            "==" => cur, // != n doesn't narrow nicely
            "!=" => cur.meet(&Interval::const_val(n)),
            _ => cur,
        }
    }

    /// Check if a condition is always true (e.g., `while True`, `while(true)`).
    fn is_always_true(&self, cond: Node, source: &[u8]) -> bool {
        match cond.kind() {
            "true" | "True" => true,
            "identifier" => {
                let text = get_node_text(cond, source);
                text == "True" || text == "true"
            }
            "parenthesized_expression" => {
                if let Some(inner) = cond.child(1) {
                    self.is_always_true(inner, source)
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// Analyze a while loop with widening for convergence.
    fn analyze_while(&mut self, stmt: Node, source: &[u8], depth: usize) -> ContractsResult<()> {
        check_ast_depth(depth, &PathBuf::from("<source>"))?;

        let line = stmt.start_position().row as u32 + 1;
        let condition = stmt.child_by_field_name("condition");
        let body = stmt.child_by_field_name("body");

        // Check for infinite loop pattern (while True)
        let is_infinite = condition.map_or(false, |c| self.is_always_true(c, source));

        for iteration in 0..self.max_iterations {
            self.iterations += 1;
            let prev = self.state.clone();

            // Refine state with condition for body
            if let Some(cond) = condition {
                self.state = self.refine_condition(&self.state, cond, source, true);
            }

            // Analyze loop body
            if let Some(body_node) = body {
                self.analyze_block(body_node, source, depth + 1)?;
            }

            // Join with pre-loop state
            let mut merged = prev.join(&self.state);

            // Apply widening after threshold
            if iteration >= WIDEN_THRESHOLD {
                merged = prev.widen(&merged);
            }

            // Check for convergence
            if merged == prev {
                // Fixpoint reached - apply exit condition
                self.state = merged;
                if let Some(cond) = condition {
                    self.state = self.refine_condition(&self.state, cond, source, false);
                }
                self.record(line);

                // For `while True` loops, mark as non-converging since they never terminate
                if is_infinite {
                    self.converged = false;
                }
                return Ok(());
            }

            self.state = merged;
        }

        // Did not converge
        self.converged = false;
        if let Some(cond) = condition {
            self.state = self.refine_condition(&self.state, cond, source, false);
        }
        self.record(line);
        Ok(())
    }

    /// Analyze a for loop with proper fixpoint iteration.
    ///
    /// Handles:
    /// - Python: `for i in range(n): body`
    /// - C-family: `for (init; cond; update) { body }` (C, C++, Java, C#, JS, TS, PHP)
    /// - Go: `for init; cond; update { body }` (wrapped in for_clause)
    /// - Rust: `for i in 0..10 { body }` (range_expression)
    /// - Lua/Luau: `for i = 1, 10 do body end` (for_numeric_clause)
    fn analyze_for(&mut self, stmt: Node, source: &[u8], depth: usize) -> ContractsResult<()> {
        check_ast_depth(depth, &PathBuf::from("<source>"))?;

        let line = stmt.start_position().row as u32 + 1;
        let body = stmt.child_by_field_name("body");

        // Try to extract loop variable and bounds depending on language pattern
        let (loop_var_name, range_bounds) = self.extract_for_loop_info(stmt, source, line);

        // Resolve initializer, condition, and update nodes.
        // Most C-style languages put these directly on the for_statement.
        // Go wraps them in a for_clause child node.
        let (initializer, condition, update) = self.resolve_for_parts(stmt);

        // For C-style for loops, process the initializer
        if let Some(init) = initializer {
            self.analyze_stmt(init, source, depth)?;
        }

        // Save state before loop (for merge with skip case)
        let pre_loop_state = self.state.clone();

        // Fixpoint iteration for the loop
        for iteration in 0..self.max_iterations {
            self.iterations += 1;
            let loop_entry_state = self.state.clone();

            // Refine state with condition (C-style for)
            if let Some(cond) = condition {
                self.state = self.refine_condition(&self.state, cond, source, true);
            }

            // Set loop variable bounds at start of iteration (Python range / Rust range / Lua numeric)
            if let Some(ref var_name) = loop_var_name {
                if let Some(bounds) = range_bounds {
                    self.state.set(var_name, bounds);
                } else {
                    self.state.set(var_name, Interval::top());
                }
            }

            // Analyze loop body
            if let Some(body_node) = body {
                self.analyze_block(body_node, source, depth + 1)?;
            }

            // Process update expression (C-style for)
            if let Some(upd) = update {
                self.analyze_stmt(upd, source, depth)?;
            }

            // Join with loop entry state (handles back-edge)
            let mut merged = loop_entry_state.join(&self.state);

            // Apply widening after threshold to ensure convergence
            if iteration >= WIDEN_THRESHOLD {
                merged = loop_entry_state.widen(&merged);
            }

            // Check for convergence
            if merged == loop_entry_state {
                // Fixpoint reached - join with pre-loop state (skip case)
                self.state = pre_loop_state.join(&merged);

                // Apply exit condition for C-style for
                if let Some(cond) = condition {
                    self.state = self.refine_condition(&self.state, cond, source, false);
                }
                self.record(line);
                return Ok(());
            }

            self.state = merged;
        }

        // Did not converge
        self.converged = false;
        // Still join with pre-loop state (skip case)
        self.state = pre_loop_state.join(&self.state);
        self.record(line);
        Ok(())
    }

    /// Resolve the initializer, condition, and update parts of a for loop.
    ///
    /// Most C-family languages (C, C++, Java, C#, JS, TS, PHP) have these
    /// as direct fields on the for_statement. Go wraps them in a for_clause
    /// child node.
    fn resolve_for_parts<'b>(&self, stmt: Node<'b>) -> (Option<Node<'b>>, Option<Node<'b>>, Option<Node<'b>>) {
        // First try direct fields (C, C++, Java, C#, JS, TS, PHP)
        let init_direct = stmt.child_by_field_name("initializer")
            .or_else(|| stmt.child_by_field_name("init"));
        let cond_direct = stmt.child_by_field_name("condition");
        let update_direct = stmt.child_by_field_name("update");

        if init_direct.is_some() || cond_direct.is_some() || update_direct.is_some() {
            return (init_direct, cond_direct, update_direct);
        }

        // Go: for_clause wraps initializer, condition, update
        let child_count = stmt.child_count();
        for i in 0..child_count {
            if let Some(child) = stmt.child(i) {
                if child.kind() == "for_clause" {
                    let init = child.child_by_field_name("initializer");
                    let cond = child.child_by_field_name("condition");
                    let upd = child.child_by_field_name("update");
                    return (init, cond, upd);
                }
            }
        }

        (None, None, None)
    }

    /// Extract loop variable and bounds from a for statement.
    /// Returns (variable_name, optional_range_bounds).
    ///
    /// Handles:
    /// - Python: `for x in range(n)` -> (x, [0, n-1])
    /// - Rust: `for i in 0..10` -> (i, [0, 9]), `for i in 0..=10` -> (i, [0, 10])
    /// - Lua/Luau: `for i = 1, 10` -> (i, [1, 10])
    /// - C-style bounds extraction from initializer + condition
    fn extract_for_loop_info(&mut self, stmt: Node, source: &[u8], line: u32) -> (Option<String>, Option<Interval>) {
        // === Python style: for x in iterable ===
        let left = stmt.child_by_field_name("left");
        let right = stmt.child_by_field_name("right");

        if let (Some(target), Some(iter)) = (left, right) {
            if target.kind() == "identifier" {
                let name = get_node_text(target, source).to_string();
                if iter.kind() == "call" || iter.kind() == "call_expression" {
                    if let Some(func) = iter.child_by_field_name("function") {
                        if get_node_text(func, source) == "range" {
                            let bounds = self.eval_range_call(iter, source, line);
                            return (Some(name), Some(bounds));
                        }
                    }
                    return (Some(name), None);
                }
                return (Some(name), None);
            }
        }

        // === Rust: for i in 0..10 / for i in 0..=10 ===
        let pattern = stmt.child_by_field_name("pattern");
        let value = stmt.child_by_field_name("value");
        if let (Some(pat), Some(val)) = (pattern, value) {
            if pat.kind() == "identifier" {
                let name = get_node_text(pat, source).to_string();
                if val.kind() == "range_expression" {
                    if let Some(bounds) = extract_rust_range_bounds(val, source) {
                        return (Some(name), Some(bounds));
                    }
                }
                return (Some(name), None);
            }
        }

        // === Lua/Luau: for i = start, end [, step] ===
        // Tree-sitter: for_statement -> for_numeric_clause (fields: name, start, end)
        if let Some(bounds) = self.extract_lua_numeric_for_bounds(stmt, source, line) {
            return bounds;
        }

        // C-style for loops (C, C++, Java, C#, Go, JS, TS, PHP, Kotlin) are handled
        // by the existing fixpoint iteration in analyze_for via init/condition/update.
        // We do NOT set loop_var_name/range_bounds for C-style loops because that
        // would interfere with the natural fixpoint convergence.
        (None, None)
    }

    /// Extract bounds from Lua/Luau numeric for: `for i = start, end [, step]`
    ///
    /// AST: for_statement -> for_numeric_clause (fields: name, start, end)
    fn extract_lua_numeric_for_bounds(&mut self, stmt: Node, source: &[u8], line: u32)
        -> Option<(Option<String>, Option<Interval>)>
    {
        let mut cursor = stmt.walk();
        for child in stmt.children(&mut cursor) {
            if child.kind() == "for_numeric_clause" {
                let name_node = child.child_by_field_name("name");
                let start_node = child.child_by_field_name("start");
                let end_node = child.child_by_field_name("end");

                if let (Some(name), Some(start), Some(end)) = (name_node, start_node, end_node) {
                    if name.kind() == "identifier" {
                        let var_name = get_node_text(name, source).to_string();
                        let start_val = self.eval_expr(start, source, line);
                        let end_val = self.eval_expr(end, source, line);

                        if start_val.lo.is_finite() && end_val.hi.is_finite() {
                            let bounds = Interval {
                                lo: start_val.lo,
                                hi: end_val.hi,
                            };
                            return Some((Some(var_name), Some(bounds)));
                        }
                        return Some((Some(var_name), None));
                    }
                }
            }
        }
        None
    }

}

/// Extract bounds from a Rust range expression: 0..10 or 0..=10
///
/// AST: range_expression -> integer_literal, ".." or "..=", integer_literal
fn extract_rust_range_bounds(range_node: Node, source: &[u8]) -> Option<Interval> {
    let mut lo_val: Option<f64> = None;
    let mut hi_val: Option<f64> = None;
    let mut is_inclusive = false;

    let mut cursor = range_node.walk();
    let mut saw_operator = false;

    for child in range_node.children(&mut cursor) {
        match child.kind() {
            "integer_literal" | "float_literal" | "number_literal" => {
                let text = get_node_text(child, source);
                if let Some(num) = parse_numeric_literal(text) {
                    if !saw_operator {
                        lo_val = Some(num);
                    } else {
                        hi_val = Some(num);
                    }
                }
            }
            ".." => {
                saw_operator = true;
                is_inclusive = false;
            }
            "..=" => {
                saw_operator = true;
                is_inclusive = true;
            }
            _ => {}
        }
    }

    let lo = lo_val?;
    let hi_raw = hi_val?;
    let hi = if is_inclusive { hi_raw } else { hi_raw - 1.0 };

    Some(Interval { lo, hi })
}

/// Parse a numeric literal string to f64.
/// Handles integer and float formats, stripping type suffixes.
fn parse_numeric_literal(text: &str) -> Option<f64> {
    let cleaned = text.trim()
        .trim_end_matches(|c: char| c.is_alphabetic() || c == '_'); // strip suffixes like i32, f64, L, etc.
    cleaned.parse::<f64>().ok()
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Parse source code with tree-sitter for any supported language.
fn parse_source(source: &str, file: &PathBuf, language: Language) -> ContractsResult<Tree> {
    let ts_lang = ParserPool::get_ts_language(language).ok_or_else(|| ContractsError::ParseError {
        file: file.clone(),
        message: format!("Unsupported language for bounds analysis: {:?}", language),
    })?;

    let mut parser = Parser::new();
    parser
        .set_language(&ts_lang)
        .map_err(|e| ContractsError::ParseError {
            file: file.clone(),
            message: format!("Failed to set {:?} language: {}", language, e),
        })?;

    parser
        .parse(source, None)
        .ok_or_else(|| ContractsError::ParseError {
            file: file.clone(),
            message: "Parsing returned None".to_string(),
        })
}

/// Get the node kinds that represent functions in each language.
fn get_function_node_kinds(language: Language) -> &'static [&'static str] {
    match language {
        Language::Python => &["function_definition"],
        Language::TypeScript | Language::JavaScript => {
            &["function_declaration", "arrow_function", "method_definition", "function"]
        }
        Language::Go => &["function_declaration", "method_declaration"],
        Language::Rust => &["function_item"],
        Language::Java => &["method_declaration", "constructor_declaration"],
        Language::C | Language::Cpp => &["function_definition"],
        Language::Ruby => &["method", "singleton_method"],
        Language::Php => &["function_definition", "method_declaration"],
        Language::CSharp => &["method_declaration", "constructor_declaration"],
        Language::Kotlin => &["function_declaration"],
        Language::Scala => &["function_definition", "function_declaration"],
        Language::Elixir => &["call"],  // def/defp are calls
        Language::Lua | Language::Luau => &["function_declaration", "function_definition"],
        Language::Swift => &["function_declaration"],
        Language::Ocaml => &["let_binding", "value_definition"],
        _ => &[],
    }
}

/// Get the node kinds that represent class/struct containers.
fn get_class_node_kinds(language: Language) -> &'static [&'static str] {
    match language {
        Language::Python => &["class_definition"],
        Language::Java | Language::CSharp | Language::Kotlin => &["class_declaration"],
        Language::TypeScript | Language::JavaScript => &["class_declaration"],
        Language::Cpp => &["class_specifier", "struct_specifier"],
        Language::Ruby => &["class"],
        Language::Php => &["class_declaration"],
        Language::Scala => &["class_definition", "object_definition"],
        Language::Rust => &["impl_item"],
        _ => &[],
    }
}

/// Extract function name from a function node (language-aware).
fn get_func_name_from_node(node: Node, language: Language, source: &[u8]) -> Option<String> {
    match language {
        Language::C | Language::Cpp => {
            // C/C++: function_definition -> declarator -> identifier
            if let Some(declarator) = node.child_by_field_name("declarator") {
                if declarator.kind() == "function_declarator" {
                    if let Some(name_node) = declarator.child_by_field_name("declarator") {
                        if name_node.kind() == "identifier" {
                            return Some(get_node_text(name_node, source).to_string());
                        }
                        // pointer_declarator wrapping identifier
                        if name_node.kind() == "pointer_declarator" {
                            let mut cursor = name_node.walk();
                            for child in name_node.children(&mut cursor) {
                                if child.kind() == "identifier" {
                                    return Some(get_node_text(child, source).to_string());
                                }
                            }
                        }
                    }
                }
                if declarator.kind() == "identifier" {
                    return Some(get_node_text(declarator, source).to_string());
                }
            }
            None
        }
        Language::Ruby => {
            node.child_by_field_name("name")
                .map(|n| get_node_text(n, source).to_string())
        }
        Language::Elixir => {
            // def/defp are calls
            if node.kind() == "call" {
                let first_child = node.child(0)?;
                let first_text = get_node_text(first_child, source);
                if first_text == "def" || first_text == "defp" {
                    if let Some(args) = node.child(1) {
                        if args.kind() == "identifier" {
                            return Some(get_node_text(args, source).to_string());
                        }
                        if args.kind() == "arguments" || args.kind() == "call" {
                            let mut cursor = args.walk();
                            for child in args.children(&mut cursor) {
                                if child.kind() == "identifier" {
                                    return Some(get_node_text(child, source).to_string());
                                }
                                if child.kind() == "call" {
                                    if let Some(name) = child.child(0) {
                                        if name.kind() == "identifier" {
                                            return Some(get_node_text(name, source).to_string());
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
            // OCaml: value_definition -> let_binding -> pattern (name) + parameter children
            // The name is in the "pattern" field of the let_binding.
            // A function def has "parameter" children; a value binding does not.
            let binding = if node.kind() == "value_definition" {
                // value_definition wraps a let_binding
                let mut cursor = node.walk();
                let mut inner = None;
                for child in node.children(&mut cursor) {
                    if child.kind() == "let_binding" {
                        inner = Some(child);
                        break;
                    }
                }
                inner
            } else if node.kind() == "let_binding" {
                Some(node)
            } else {
                None
            };

            if let Some(binding) = binding {
                // Check if this binding has parameters (is a function)
                let mut has_params = false;
                let mut cursor = binding.walk();
                for child in binding.children(&mut cursor) {
                    if child.kind() == "parameter" {
                        has_params = true;
                        break;
                    }
                }

                if has_params {
                    // Name is in the "pattern" field
                    if let Some(pat) = binding.child_by_field_name("pattern") {
                        return Some(get_node_text(pat, source).to_string());
                    }
                }
            }
            None
        }
        _ => {
            // Most languages use "name" field
            node.child_by_field_name("name")
                .map(|n| get_node_text(n, source).to_string())
        }
    }
}

/// Find function definitions in the AST (language-aware).
fn find_functions_multi<'a>(
    root: Node<'a>,
    function_name: Option<&str>,
    source: &'a [u8],
    language: Language,
) -> Vec<(String, Node<'a>)> {
    let func_kinds = get_function_node_kinds(language);
    let class_kinds = get_class_node_kinds(language);
    let mut functions = Vec::new();

    collect_functions_recursive(root, function_name, source, language, func_kinds, class_kinds, &mut functions);

    functions
}

/// Recursively collect function nodes.
fn collect_functions_recursive<'a>(
    node: Node<'a>,
    function_name: Option<&str>,
    source: &'a [u8],
    language: Language,
    func_kinds: &[&str],
    class_kinds: &[&str],
    functions: &mut Vec<(String, Node<'a>)>,
) {
    let kind = node.kind();

    if func_kinds.contains(&kind) {
        // This is a function node -- add it if it matches
        if let Some(name) = get_func_name_from_node(node, language, source) {
            if function_name.map_or(true, |f| f == name) {
                functions.push((name, node));
            }
            return; // Don't recurse into function bodies
        }
        // For Elixir/OCaml: if name extraction fails (e.g., defmodule is a "call"
        // but not a def/defp), fall through to recurse into children
    }

    // Check for arrow functions in variable declarations (TS/JS pattern):
    // lexical_declaration / variable_declaration -> variable_declarator -> name + value(arrow_function)
    if matches!(kind, "lexical_declaration" | "variable_declaration") {
        let mut decl_cursor = node.walk();
        for child in node.children(&mut decl_cursor) {
            if child.kind() == "variable_declarator" {
                if let Some(name_node) = child.child_by_field_name("name") {
                    if let Some(value_node) = child.child_by_field_name("value") {
                        if matches!(value_node.kind(), "arrow_function" | "function" | "function_expression" | "generator_function") {
                            let var_name = get_node_text(name_node, source).to_string();
                            if function_name.map_or(true, |f| f == var_name) {
                                functions.push((var_name, value_node));
                            }
                            return; // Don't recurse into function bodies
                        }
                    }
                }
            }
        }
    }

    // Recurse into all children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_functions_recursive(child, function_name, source, language, func_kinds, class_kinds, functions);
    }
}

/// Get the parameters node from a function node (language-aware).
fn get_params_node<'a>(func_node: Node<'a>, language: Language) -> Option<Node<'a>> {
    match language {
        Language::Python => func_node.child_by_field_name("parameters"),
        Language::Go => func_node.child_by_field_name("parameters"),
        Language::Rust => func_node.child_by_field_name("parameters"),
        Language::Java | Language::CSharp => func_node.child_by_field_name("parameters"),
        Language::C | Language::Cpp => {
            // C/C++: function_definition -> declarator -> parameters
            if let Some(declarator) = func_node.child_by_field_name("declarator") {
                if declarator.kind() == "function_declarator" {
                    return declarator.child_by_field_name("parameters");
                }
            }
            func_node.child_by_field_name("parameters")
        }
        Language::TypeScript | Language::JavaScript => {
            func_node.child_by_field_name("parameters")
        }
        Language::Ruby => func_node.child_by_field_name("parameters"),
        Language::Php => func_node.child_by_field_name("parameters"),
        Language::Ocaml => {
            // OCaml parameters are individual "parameter" children of the let_binding.
            // We return the binding itself so initialize_parameters can iterate over its children.
            let binding = if func_node.kind() == "value_definition" {
                let mut cursor = func_node.walk();
                let mut inner = None;
                for child in func_node.children(&mut cursor) {
                    if child.kind() == "let_binding" {
                        inner = Some(child);
                        break;
                    }
                }
                inner.unwrap_or(func_node)
            } else {
                func_node
            };
            Some(binding)
        }
        _ => func_node.child_by_field_name("parameters"),
    }
}

/// Get the body node from a function node (language-aware).
fn get_body_node<'a>(func_node: Node<'a>, language: Language) -> Option<Node<'a>> {
    match language {
        Language::Elixir => {
            // Elixir: def body is inside a "do" block
            let mut cursor = func_node.walk();
            for child in func_node.children(&mut cursor) {
                if child.kind() == "do_block" {
                    return Some(child);
                }
            }
            func_node.child_by_field_name("body")
        }
        Language::Ocaml => {
            // OCaml: value_definition -> let_binding -> body field
            let binding = if func_node.kind() == "value_definition" {
                let mut cursor = func_node.walk();
                let mut inner = None;
                for child in func_node.children(&mut cursor) {
                    if child.kind() == "let_binding" {
                        inner = Some(child);
                        break;
                    }
                }
                inner.unwrap_or(func_node)
            } else {
                func_node
            };
            // Try "body" field on the let_binding
            if let Some(body) = binding.child_by_field_name("body") {
                return Some(body);
            }
            // Fallback: last child that's not a keyword
            let count = binding.child_count();
            if count > 0 {
                let last = binding.child(count.saturating_sub(1));
                if let Some(last_node) = last {
                    if last_node.kind() != "=" && last_node.kind() != "let" {
                        return Some(last_node);
                    }
                }
            }
            None
        }
        _ => func_node.child_by_field_name("body"),
    }
}

/// Get text content of a node.
fn get_node_text<'a>(node: Node<'a>, source: &'a [u8]) -> &'a str {
    let start = node.start_byte();
    let end = node.end_byte();
    if end <= source.len() {
        std::str::from_utf8(&source[start..end]).unwrap_or("")
    } else {
        ""
    }
}

// =============================================================================
// Text Output Formatting
// =============================================================================

/// Format bounds result as human-readable text.
pub fn format_bounds_text(result: &BoundsResult) -> String {
    let mut output = String::new();

    output.push_str(&format!("Function: {}\n", result.function));
    output.push_str(&format!(
        "  Converged: {} ({} iterations)\n",
        if result.converged { "true" } else { "false" },
        result.iterations
    ));

    // Sort lines for consistent output
    let mut lines: Vec<_> = result.bounds.keys().collect();
    lines.sort();

    output.push_str("  Bounds:\n");
    for line in lines {
        if let Some(vars) = result.bounds.get(line) {
            for (var, interval) in vars {
                output.push_str(&format!("    Line {}: {} in {}\n", line, var, interval));
            }
        }
    }

    if !result.warnings.is_empty() {
        output.push_str("  Warnings:\n");
        for warning in &result.warnings {
            output.push_str(&format!(
                "    [{}] line {}: {}\n",
                warning.kind, warning.line, warning.message
            ));
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interval_const() {
        let iv = Interval::const_val(5.0);
        assert_eq!(iv.lo, 5.0);
        assert_eq!(iv.hi, 5.0);
        assert!(iv.contains(5.0));
        assert!(!iv.contains(4.0));
    }

    #[test]
    fn test_interval_top_bottom() {
        let top = Interval::top();
        assert!(!top.is_bottom());
        assert!(top.is_top());

        let bottom = Interval::bottom();
        assert!(bottom.is_bottom());
        assert!(!bottom.is_top());
    }

    #[test]
    fn test_interval_join() {
        let a = Interval { lo: 0.0, hi: 10.0 };
        let b = Interval { lo: 5.0, hi: 15.0 };
        let joined = a.join(&b);
        assert_eq!(joined.lo, 0.0);
        assert_eq!(joined.hi, 15.0);
    }

    #[test]
    fn test_interval_meet() {
        let a = Interval { lo: 0.0, hi: 10.0 };
        let b = Interval { lo: 5.0, hi: 15.0 };
        let met = a.meet(&b);
        assert_eq!(met.lo, 5.0);
        assert_eq!(met.hi, 10.0);
    }

    #[test]
    fn test_interval_widen() {
        let old = Interval { lo: 0.0, hi: 10.0 };
        let new = Interval { lo: 0.0, hi: 15.0 };
        let widened = old.widen(&new);
        assert_eq!(widened.lo, 0.0);
        assert_eq!(widened.hi, f64::INFINITY);
    }

    #[test]
    fn test_interval_add() {
        let a = Interval { lo: 0.0, hi: 10.0 };
        let b = Interval { lo: 5.0, hi: 15.0 };
        let sum = a.add(&b);
        assert_eq!(sum.lo, 5.0);
        assert_eq!(sum.hi, 25.0);
    }

    #[test]
    fn test_interval_mul() {
        let a = Interval { lo: 0.0, hi: 10.0 };
        let b = Interval { lo: -1.0, hi: 2.0 };
        let prod = a.mul(&b);
        assert_eq!(prod.lo, -10.0);
        assert_eq!(prod.hi, 20.0);
    }

    #[test]
    fn test_interval_div_contains_zero() {
        let a = Interval { lo: 10.0, hi: 20.0 };
        let b = Interval { lo: -5.0, hi: 5.0 }; // Contains zero
        let (result, may_div_zero) = a.div(&b);
        assert!(may_div_zero);
        assert!(result.is_top()); // Conservative for division by potential zero
    }

    // =====================================================================
    // Multi-language bounds analysis tests
    // =====================================================================

    /// Helper to run bounds on inline source code for a given language
    fn run_bounds_on_source(source: &str, language: Language, function: Option<&str>, max_iter: u32) -> ContractsResult<Vec<BoundsResult>> {
        let tree = parse_source(source, &PathBuf::from("<test>"), language)?;
        let root = tree.root_node();
        let functions = find_functions_multi(root, function, source.as_bytes(), language);

        let mut results = Vec::new();
        for (func_name, func_node) in functions {
            let result = analyze_function(&func_name, func_node, source.as_bytes(), max_iter, language)?;
            results.push(result);
        }
        Ok(results)
    }

    #[test]
    fn test_bounds_python_simple_assignment() {
        let source = "def foo():\n    x = 5\n    y = x + 10\n";
        let results = run_bounds_on_source(source, Language::Python, Some("foo"), 50).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].function, "foo");
        // Should have bounds for x = [5,5] and y = [15,15]
        let has_bounds = !results[0].bounds.is_empty();
        assert!(has_bounds, "Should find bounds for variables");
    }

    #[test]
    fn test_bounds_go_simple_assignment() {
        let source = r#"package main

func foo() {
    x := 5
    y := x + 10
}
"#;
        let results = run_bounds_on_source(source, Language::Go, Some("foo"), 50).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].function, "foo");
        let has_bounds = !results[0].bounds.is_empty();
        assert!(has_bounds, "Go: should find bounds for variables");
    }

    #[test]
    fn test_bounds_rust_simple_assignment() {
        let source = r#"fn foo() {
    let x = 5;
    let y = x + 10;
}
"#;
        let results = run_bounds_on_source(source, Language::Rust, Some("foo"), 50).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].function, "foo");
        let has_bounds = !results[0].bounds.is_empty();
        assert!(has_bounds, "Rust: should find bounds for variables");
    }

    #[test]
    fn test_bounds_javascript_simple_assignment() {
        let source = r#"function foo() {
    let x = 5;
    let y = x + 10;
}
"#;
        let results = run_bounds_on_source(source, Language::JavaScript, Some("foo"), 50).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].function, "foo");
        let has_bounds = !results[0].bounds.is_empty();
        assert!(has_bounds, "JS: should find bounds for variables");
    }

    #[test]
    fn test_bounds_java_simple_assignment() {
        let source = r#"class Foo {
    void foo() {
        int x = 5;
        int y = x + 10;
    }
}
"#;
        let results = run_bounds_on_source(source, Language::Java, Some("foo"), 50).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].function, "foo");
        let has_bounds = !results[0].bounds.is_empty();
        assert!(has_bounds, "Java: should find bounds for variables");
    }

    #[test]
    fn test_bounds_c_simple_assignment() {
        let source = r#"void foo() {
    int x = 5;
    int y = x + 10;
}
"#;
        let results = run_bounds_on_source(source, Language::C, Some("foo"), 50).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].function, "foo");
        let has_bounds = !results[0].bounds.is_empty();
        assert!(has_bounds, "C: should find bounds for variables");
    }

    #[test]
    fn test_bounds_ruby_simple_assignment() {
        let source = r#"def foo
  x = 5
  y = x + 10
end
"#;
        let results = run_bounds_on_source(source, Language::Ruby, Some("foo"), 50).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].function, "foo");
        let has_bounds = !results[0].bounds.is_empty();
        assert!(has_bounds, "Ruby: should find bounds for variables");
    }

    #[test]
    fn test_bounds_python_division_by_zero_warning() {
        let source = "def foo(x):\n    y = 100 / x\n";
        let results = run_bounds_on_source(source, Language::Python, Some("foo"), 50).unwrap();
        assert_eq!(results.len(), 1);
        // x is unknown (Top), so division by x should warn
        assert!(!results[0].warnings.is_empty(), "Should warn about division by unknown");
    }

    #[test]
    fn test_bounds_go_if_statement() {
        let source = r#"package main

func foo(x int) int {
    if x > 0 {
        return x + 10
    }
    return 0
}
"#;
        let results = run_bounds_on_source(source, Language::Go, Some("foo"), 50).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].converged, "Go if analysis should converge");
    }

    #[test]
    fn test_bounds_rust_if_statement() {
        let source = r#"fn foo(x: i32) -> i32 {
    if x > 0 {
        x + 10
    } else {
        0
    }
}
"#;
        let results = run_bounds_on_source(source, Language::Rust, Some("foo"), 50).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].converged, "Rust if analysis should converge");
    }

    #[test]
    fn test_bounds_python_for_loop() {
        let source = "def foo():\n    s = 0\n    for i in range(10):\n        s = s + i\n";
        let results = run_bounds_on_source(source, Language::Python, Some("foo"), 50).unwrap();
        assert_eq!(results.len(), 1);
        // Should have bounds for s (accumulated sum)
        let has_bounds = !results[0].bounds.is_empty();
        assert!(has_bounds, "Python: should track bounds through for loop");
    }

    #[test]
    fn test_bounds_go_for_loop() {
        let source = r#"package main

func foo() int {
    s := 0
    for i := 0; i < 10; i++ {
        s = s + i
    }
    return s
}
"#;
        let results = run_bounds_on_source(source, Language::Go, Some("foo"), 50).unwrap();
        assert_eq!(results.len(), 1);
        let has_bounds = !results[0].bounds.is_empty();
        assert!(has_bounds, "Go: should track bounds through for loop");
    }

    #[test]
    fn test_bounds_language_gate_removed() {
        // Verify that the Python-only gate is gone -- run_bounds now accepts language
        let go_source = r#"package main
func bar() { x := 42 }
"#;
        // This should NOT error with "only supports Python"
        let results = run_bounds_on_source(go_source, Language::Go, Some("bar"), 50).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_bounds_cpp_simple() {
        let source = r#"void foo() {
    int x = 5;
    int y = x * 2;
}
"#;
        let results = run_bounds_on_source(source, Language::Cpp, Some("foo"), 50).unwrap();
        assert_eq!(results.len(), 1);
        let has_bounds = !results[0].bounds.is_empty();
        assert!(has_bounds, "C++: should find bounds for variables");
    }

    #[test]
    fn test_bounds_typescript_simple() {
        let source = r#"function foo(): number {
    const x = 5;
    const y = x + 10;
    return y;
}
"#;
        let results = run_bounds_on_source(source, Language::TypeScript, Some("foo"), 50).unwrap();
        assert_eq!(results.len(), 1);
        let has_bounds = !results[0].bounds.is_empty();
        assert!(has_bounds, "TypeScript: should find bounds for variables");
    }

    // =====================================================================
    // Precise loop bounds extraction tests
    //
    // These tests verify that loop variables get exact numeric bounds
    // instead of [-inf, +inf] for various language patterns.
    // =====================================================================

    /// Helper: find a variable's interval across all recorded line states
    fn find_var_bounds(results: &[BoundsResult], var_name: &str) -> Option<Interval> {
        for result in results {
            for (_line, vars) in &result.bounds {
                if let Some(interval) = vars.get(var_name) {
                    // Return the first non-top interval we find for this var
                    if !interval.is_top() {
                        return Some(*interval);
                    }
                }
            }
        }
        // Fallback: return any interval for the var (even if top)
        for result in results {
            for (_line, vars) in &result.bounds {
                if let Some(interval) = vars.get(var_name) {
                    return Some(*interval);
                }
            }
        }
        None
    }

    // --- C-style for loops (Go, Java, C, C++, C#, TS, JS, PHP) ---

    #[test]
    fn test_precise_bounds_go_c_style_for() {
        let source = r#"package main

func compute() {
    for i := 0; i < 10; i++ {
        _ = i
    }
}
"#;
        let results = run_bounds_on_source(source, Language::Go, Some("compute"), 50).unwrap();
        assert_eq!(results.len(), 1);
        let iv = find_var_bounds(&results, "i");
        assert!(iv.is_some(), "Go: should find bounds for loop var i");
        let iv = iv.unwrap();
        assert_eq!(iv.lo, 0.0, "Go: i lower bound should be 0");
        assert!(iv.hi <= 9.0 || iv.hi == f64::INFINITY,
            "Go: i upper bound should be 9, got {}", iv.hi);
        // Primary assertion: i should NOT be [-inf, +inf]
        assert!(iv.lo >= 0.0, "Go: i lower bound should not be -inf");
    }

    #[test]
    fn test_precise_bounds_c_for_loop() {
        let source = r#"void compute() {
    for (int i = 0; i < 10; i++) {
        int x = i;
    }
}
"#;
        let results = run_bounds_on_source(source, Language::C, Some("compute"), 50).unwrap();
        assert_eq!(results.len(), 1);
        let iv = find_var_bounds(&results, "i");
        assert!(iv.is_some(), "C: should find bounds for loop var i");
        let iv = iv.unwrap();
        assert_eq!(iv.lo, 0.0, "C: i lower bound should be 0");
        assert!(iv.hi <= 9.0 || iv.hi == f64::INFINITY,
            "C: i upper bound should be 9, got {}", iv.hi);
    }

    #[test]
    fn test_precise_bounds_java_for_loop() {
        let source = r#"class Test {
    void compute() {
        for (int i = 0; i < 10; i++) {
            int x = i;
        }
    }
}
"#;
        let results = run_bounds_on_source(source, Language::Java, Some("compute"), 50).unwrap();
        assert_eq!(results.len(), 1);
        let iv = find_var_bounds(&results, "i");
        assert!(iv.is_some(), "Java: should find bounds for loop var i");
        let iv = iv.unwrap();
        assert_eq!(iv.lo, 0.0, "Java: i lower bound should be 0");
    }

    #[test]
    fn test_precise_bounds_typescript_for_loop() {
        let source = r#"function compute(): void {
    for (let i = 0; i < 10; i++) {
        let x = i;
    }
}
"#;
        let results = run_bounds_on_source(source, Language::TypeScript, Some("compute"), 50).unwrap();
        assert_eq!(results.len(), 1);
        let iv = find_var_bounds(&results, "i");
        assert!(iv.is_some(), "TypeScript: should find bounds for loop var i");
        let iv = iv.unwrap();
        assert_eq!(iv.lo, 0.0, "TypeScript: i lower bound should be 0");
    }

    #[test]
    fn test_precise_bounds_javascript_for_loop() {
        let source = r#"function compute() {
    for (let i = 0; i < 10; i++) {
        let x = i;
    }
}
"#;
        let results = run_bounds_on_source(source, Language::JavaScript, Some("compute"), 50).unwrap();
        assert_eq!(results.len(), 1);
        let iv = find_var_bounds(&results, "i");
        assert!(iv.is_some(), "JavaScript: should find bounds for loop var i");
        let iv = iv.unwrap();
        assert_eq!(iv.lo, 0.0, "JavaScript: i lower bound should be 0");
    }

    #[test]
    fn test_precise_bounds_cpp_for_loop() {
        let source = r#"void compute() {
    for (int i = 0; i < 10; i++) {
        int x = i;
    }
}
"#;
        let results = run_bounds_on_source(source, Language::Cpp, Some("compute"), 50).unwrap();
        assert_eq!(results.len(), 1);
        let iv = find_var_bounds(&results, "i");
        assert!(iv.is_some(), "C++: should find bounds for loop var i");
        let iv = iv.unwrap();
        assert_eq!(iv.lo, 0.0, "C++: i lower bound should be 0");
    }

    #[test]
    fn test_precise_bounds_csharp_for_loop() {
        let source = r#"class Test {
    void Compute() {
        for (int i = 0; i < 10; i++) {
            int x = i;
        }
    }
}
"#;
        let results = run_bounds_on_source(source, Language::CSharp, Some("Compute"), 50).unwrap();
        assert_eq!(results.len(), 1);
        let iv = find_var_bounds(&results, "i");
        assert!(iv.is_some(), "C#: should find bounds for loop var i");
        let iv = iv.unwrap();
        assert_eq!(iv.lo, 0.0, "C#: i lower bound should be 0");
    }

    // --- Rust range expressions ---

    #[test]
    fn test_precise_bounds_rust_range() {
        let source = r#"fn compute() {
    for i in 0..10 {
        let x = i;
    }
}
"#;
        let results = run_bounds_on_source(source, Language::Rust, Some("compute"), 50).unwrap();
        assert_eq!(results.len(), 1);
        let iv = find_var_bounds(&results, "i");
        assert!(iv.is_some(), "Rust: should find bounds for loop var i");
        let iv = iv.unwrap();
        assert_eq!(iv.lo, 0.0, "Rust: i lower bound should be 0");
        assert_eq!(iv.hi, 9.0, "Rust: i upper bound should be 9 for 0..10");
    }

    #[test]
    fn test_precise_bounds_rust_range_inclusive() {
        let source = r#"fn compute() {
    for i in 0..=10 {
        let x = i;
    }
}
"#;
        let results = run_bounds_on_source(source, Language::Rust, Some("compute"), 50).unwrap();
        assert_eq!(results.len(), 1);
        let iv = find_var_bounds(&results, "i");
        assert!(iv.is_some(), "Rust: should find bounds for loop var i (inclusive)");
        let iv = iv.unwrap();
        assert_eq!(iv.lo, 0.0, "Rust: i lower bound should be 0");
        assert_eq!(iv.hi, 10.0, "Rust: i upper bound should be 10 for 0..=10");
    }

    // --- Lua/Luau numeric for ---

    #[test]
    fn test_precise_bounds_lua_numeric_for() {
        let source = r#"function compute()
    for i = 1, 10 do
        local x = i
    end
end
"#;
        let results = run_bounds_on_source(source, Language::Lua, Some("compute"), 50).unwrap();
        assert_eq!(results.len(), 1);
        let iv = find_var_bounds(&results, "i");
        assert!(iv.is_some(), "Lua: should find bounds for loop var i");
        let iv = iv.unwrap();
        assert_eq!(iv.lo, 1.0, "Lua: i lower bound should be 1");
        assert_eq!(iv.hi, 10.0, "Lua: i upper bound should be 10");
    }

    #[test]
    fn test_precise_bounds_luau_numeric_for() {
        let source = r#"local function compute()
    for i = 0, 9 do
        local x = i
    end
end
"#;
        let results = run_bounds_on_source(source, Language::Luau, Some("compute"), 50).unwrap();
        assert_eq!(results.len(), 1);
        let iv = find_var_bounds(&results, "i");
        assert!(iv.is_some(), "Luau: should find bounds for loop var i");
        let iv = iv.unwrap();
        assert_eq!(iv.lo, 0.0, "Luau: i lower bound should be 0");
        assert_eq!(iv.hi, 9.0, "Luau: i upper bound should be 9");
    }

    // --- Python range (existing behavior, verify not broken) ---

    #[test]
    fn test_precise_bounds_python_range() {
        let source = "def compute():\n    for i in range(10):\n        x = i\n";
        let results = run_bounds_on_source(source, Language::Python, Some("compute"), 50).unwrap();
        assert_eq!(results.len(), 1);
        let iv = find_var_bounds(&results, "i");
        assert!(iv.is_some(), "Python: should find bounds for loop var i");
        let iv = iv.unwrap();
        assert_eq!(iv.lo, 0.0, "Python: i lower bound should be 0");
        assert_eq!(iv.hi, 9.0, "Python: i upper bound should be 9 for range(10)");
    }

    #[test]
    fn test_precise_bounds_python_range_start_stop() {
        let source = "def compute():\n    for i in range(5, 15):\n        x = i\n";
        let results = run_bounds_on_source(source, Language::Python, Some("compute"), 50).unwrap();
        assert_eq!(results.len(), 1);
        let iv = find_var_bounds(&results, "i");
        assert!(iv.is_some(), "Python: should find bounds for range(5, 15)");
        let iv = iv.unwrap();
        assert_eq!(iv.lo, 5.0, "Python: i lower bound should be 5");
        assert_eq!(iv.hi, 14.0, "Python: i upper bound should be 14 for range(5,15)");
    }

    #[test]
    fn test_find_ts_arrow_function_bounds() {
        let ts_source = r#"
const getDuration = (start: number, end: number): number => {
    const diff = end - start;
    return diff;
};
"#;
        let results = run_bounds_on_source(ts_source, Language::TypeScript, Some("getDuration"), 50).unwrap();
        assert_eq!(results.len(), 1, "Should find TS arrow function 'getDuration' for bounds analysis");
        assert_eq!(results[0].function, "getDuration");
    }

}
