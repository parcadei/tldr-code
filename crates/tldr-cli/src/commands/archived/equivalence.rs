//! Equivalence Command - GVN-based Redundancy Detection
//!
//! The equivalence command uses Global Value Numbering (GVN) to detect
//! redundant expressions in code. It identifies expressions that compute
//! the same value, including handling commutative operators.
//!
//! P3: Multi-language support via ast_utils helpers for node kind lookups.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use anyhow::Result;
use clap::Args;
use tree_sitter::Node;

use super::error::RemainingError;
use super::types::{ExpressionRef, GVNEquivalence, GVNReport, GVNSummary, Redundancy};

use crate::output::{OutputFormat, OutputWriter};
use tldr_core::ast::parser::parse;
use tldr_core::security::ast_utils;
use tldr_core::types::Language;

/// Detect redundant expressions using Global Value Numbering.
#[derive(Debug, Clone, Args)]
pub struct EquivalenceArgs {
    /// Source file to analyze
    pub file: PathBuf,

    /// Specific function to analyze (optional, analyzes all if not specified)
    pub function: Option<String>,

    /// Output file (stdout if not specified)
    #[arg(long, short = 'o')]
    pub output: Option<PathBuf>,

    /// Language override (auto-detected from file extension if not specified)
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,
}

const COMMUTATIVE_OPS: &[&str] = &["+", "*", "==", "!=", "and", "or", "&", "|", "^", "&&", "||"];
const MAX_EXPR_DEPTH: u32 = 50;

fn node_text<'a>(node: Node, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
}

fn get_line_number(node: Node) -> u32 {
    node.start_position().row as u32 + 1
}

/// Check if an operator string represents a commutative operation.
fn is_op_commutative(op: &str) -> bool {
    COMMUTATIVE_OPS.contains(&op)
}

#[derive(Debug)]
pub struct GVNEngine {
    value_numbers: HashMap<u64, u32>,
    expressions: HashMap<u32, Vec<ExpressionRef>>,
    variable_values: HashMap<String, u32>,
    next_vn: u32,
    language: Language,
}

impl GVNEngine {
    pub fn new(language: Language) -> Self {
        Self {
            value_numbers: HashMap::new(),
            expressions: HashMap::new(),
            variable_values: HashMap::new(),
            next_vn: 1,
            language,
        }
    }

    pub fn get_or_create_vn(&mut self, hash: u64) -> u32 {
        if let Some(&vn) = self.value_numbers.get(&hash) {
            vn
        } else {
            let vn = self.next_vn;
            self.next_vn += 1;
            self.value_numbers.insert(hash, vn);
            vn
        }
    }

    pub fn record_expression(&mut self, expr_ref: ExpressionRef) {
        let vn = expr_ref.value_number;
        self.expressions.entry(vn).or_default().push(expr_ref);
    }

    pub fn propagate_through_assignment(&mut self, var: &str, vn: u32) {
        self.variable_values.insert(var.to_string(), vn);
    }

    /// Hash a tree-sitter expression node, using ast_utils helpers for
    /// language-aware node kind matching (P3).
    pub fn hash_expression(&self, node: Node, source: &[u8], depth: u32) -> u64 {
        if depth > MAX_EXPR_DEPTH {
            return u64::MAX; // Sentinel: avoid collision with legitimate hash values
        }

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        let kind = node.kind();
        // Hash the canonical category, not the raw kind (so different languages
        // with different node kind names produce comparable hashes for the same
        // semantic category).

        let lang = self.language;

        // Binary expression (arithmetic/bitwise)
        if ast_utils::binary_expression_node_kinds(lang).contains(&kind) {
            return self.hash_binary_expr(node, source, depth, &mut hasher);
        }

        // Identifier / variable reference
        if ast_utils::identifier_node_kinds(lang).contains(&kind) {
            "identifier".hash(&mut hasher);
            let name = node_text(node, source);
            if let Some(vn) = self.variable_values.get(name) {
                "propagated_vn".hash(&mut hasher);
                vn.hash(&mut hasher);
            } else {
                name.hash(&mut hasher);
            }
            return hasher.finish();
        }

        // Literal (integer, float, string, etc.)
        if ast_utils::literal_node_kinds(lang).contains(&kind) {
            "literal".hash(&mut hasher);
            let text = node_text(node, source);
            text.hash(&mut hasher);
            return hasher.finish();
        }

        // Boolean literals (true/false/nil/null/none/undefined)
        if is_boolean_or_null_literal(kind, lang) {
            "literal".hash(&mut hasher);
            let text = node_text(node, source);
            text.hash(&mut hasher);
            return hasher.finish();
        }

        // Unary expression
        if ast_utils::unary_expression_node_kinds(lang).contains(&kind) {
            "unary".hash(&mut hasher);
            return self.hash_unary_expr(node, source, depth, &mut hasher);
        }

        // Call expression
        if ast_utils::call_node_kinds(lang).contains(&kind) {
            return self.hash_call_expr(node, source, depth, &mut hasher);
        }

        // Parenthesized expression - unwrap
        if ast_utils::parenthesized_expression_node_kinds(lang).contains(&kind) {
            // Try to find the inner expression
            if let Some(inner) = node.child_by_field_name("expression")
                .or_else(|| node.named_child(0))
                .or_else(|| node.child(1))
            {
                return self.hash_expression(inner, source, depth + 1);
            }
        }

        // Field access / attribute access
        if is_field_access_kind(kind, lang) {
            "attribute".hash(&mut hasher);
            return self.hash_field_access(node, source, depth, &mut hasher);
        }

        // Subscript access
        if is_subscript_kind(kind, lang) {
            "subscript".hash(&mut hasher);
            return self.hash_subscript(node, source, depth, &mut hasher);
        }

        // Collection literals (list, tuple, set, dictionary, array)
        if is_collection_kind(kind) {
            kind.hash(&mut hasher);
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    let child_hash = self.hash_expression(child, source, depth + 1);
                    child_hash.hash(&mut hasher);
                }
            }
            return hasher.finish();
        }

        // Fallback: hash the text content
        kind.hash(&mut hasher);
        let text = node_text(node, source);
        text.hash(&mut hasher);
        hasher.finish()
    }

    /// Hash a binary expression node.
    /// Handles operator extraction for both Python (field-based) and other languages.
    fn hash_binary_expr(&self, node: Node, source: &[u8], depth: u32, hasher: &mut std::collections::hash_map::DefaultHasher) -> u64 {
        "binary".hash(hasher);

        // Try field-based extraction first (Python: left/operator/right fields)
        if let (Some(left), Some(right)) = (
            node.child_by_field_name("left"),
            node.child_by_field_name("right"),
        ) {
            let op_text = if let Some(op) = node.child_by_field_name("operator") {
                node_text(op, source).to_string()
            } else {
                // Find the operator child between left and right
                extract_operator_from_children(node, source, &left, &right)
            };

            let left_hash = self.hash_expression(left, source, depth + 1);
            let right_hash = self.hash_expression(right, source, depth + 1);

            if is_op_commutative(&op_text) {
                let (min_hash, max_hash) = if left_hash <= right_hash {
                    (left_hash, right_hash)
                } else {
                    (right_hash, left_hash)
                };
                op_text.hash(hasher);
                min_hash.hash(hasher);
                max_hash.hash(hasher);
            } else {
                op_text.hash(hasher);
                left_hash.hash(hasher);
                right_hash.hash(hasher);
            }
            return hasher.finish();
        }

        // Fallback: positional children (child(0)=left, child(1)=op, child(2)=right)
        // Common for Ruby "binary" and some other grammars
        if node.child_count() >= 3 {
            if let (Some(left), Some(op_node), Some(right)) = (node.child(0), node.child(1), node.child(2)) {
                let op_text = node_text(op_node, source).to_string();
                let left_hash = self.hash_expression(left, source, depth + 1);
                let right_hash = self.hash_expression(right, source, depth + 1);

                if is_op_commutative(&op_text) {
                    let (min_hash, max_hash) = if left_hash <= right_hash {
                        (left_hash, right_hash)
                    } else {
                        (right_hash, left_hash)
                    };
                    op_text.hash(hasher);
                    min_hash.hash(hasher);
                    max_hash.hash(hasher);
                } else {
                    op_text.hash(hasher);
                    left_hash.hash(hasher);
                    right_hash.hash(hasher);
                }
                return hasher.finish();
            }
        }

        // Last resort: hash all children
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let child_hash = self.hash_expression(child, source, depth + 1);
            child_hash.hash(hasher);
        }
        hasher.finish()
    }

    /// Hash a unary expression node.
    fn hash_unary_expr(&self, node: Node, source: &[u8], depth: u32, hasher: &mut std::collections::hash_map::DefaultHasher) -> u64 {
        // Try field-based (Python: operator + argument)
        if let Some(arg) = node.child_by_field_name("argument")
            .or_else(|| node.child_by_field_name("operand"))
        {
            if let Some(op) = node.child_by_field_name("operator").or_else(|| node.child(0)) {
                let op_text = node_text(op, source);
                op_text.hash(hasher);
            }
            let arg_hash = self.hash_expression(arg, source, depth + 1);
            arg_hash.hash(hasher);
            return hasher.finish();
        }

        // Positional: first unnamed child is operator, first named child is operand
        let mut op_found = false;
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if !child.is_named() && !op_found {
                    let op_text = node_text(child, source);
                    op_text.hash(hasher);
                    op_found = true;
                } else if child.is_named() {
                    let child_hash = self.hash_expression(child, source, depth + 1);
                    child_hash.hash(hasher);
                    return hasher.finish();
                }
            }
        }

        hasher.finish()
    }

    /// Hash a call expression.
    fn hash_call_expr(&self, node: Node, source: &[u8], depth: u32, hasher: &mut std::collections::hash_map::DefaultHasher) -> u64 {
        "call".hash(hasher);

        // Try Python-style: function + arguments fields
        if let Some(func) = node.child_by_field_name("function") {
            let func_hash = self.hash_expression(func, source, depth + 1);
            func_hash.hash(hasher);

            if let Some(args) = node.child_by_field_name("arguments") {
                let mut cursor = args.walk();
                for arg in args.children(&mut cursor) {
                    if arg.is_named() {
                        let arg_hash = self.hash_expression(arg, source, depth + 1);
                        arg_hash.hash(hasher);
                    }
                }
            }
            return hasher.finish();
        }

        // Try other patterns: first child is the function/callee
        if let Some(func) = node.child_by_field_name("name")
            .or_else(|| node.child_by_field_name("callee"))
            .or_else(|| node.named_child(0))
        {
            let func_hash = self.hash_expression(func, source, depth + 1);
            func_hash.hash(hasher);
        }

        // Hash arguments if present
        if let Some(args) = node.child_by_field_name("arguments")
            .or_else(|| node.child_by_field_name("argument_list"))
        {
            let mut cursor = args.walk();
            for arg in args.children(&mut cursor) {
                if arg.is_named() {
                    let arg_hash = self.hash_expression(arg, source, depth + 1);
                    arg_hash.hash(hasher);
                }
            }
        }

        hasher.finish()
    }

    /// Hash a field/attribute access node.
    fn hash_field_access(&self, node: Node, source: &[u8], depth: u32, hasher: &mut std::collections::hash_map::DefaultHasher) -> u64 {
        // Try Python-style: object + attribute fields
        if let Some(obj) = node.child_by_field_name("object")
            .or_else(|| node.child_by_field_name("value"))
            .or_else(|| node.child_by_field_name("operand"))
            .or_else(|| node.child_by_field_name("argument"))
            .or_else(|| node.named_child(0))
        {
            let obj_hash = self.hash_expression(obj, source, depth + 1);
            obj_hash.hash(hasher);
        }

        if let Some(attr) = node.child_by_field_name("attribute")
            .or_else(|| node.child_by_field_name("property"))
            .or_else(|| node.child_by_field_name("field"))
            .or_else(|| node.child_by_field_name("name"))
        {
            let attr_text = node_text(attr, source);
            attr_text.hash(hasher);
        }

        hasher.finish()
    }

    /// Hash a subscript/index access node.
    fn hash_subscript(&self, node: Node, source: &[u8], depth: u32, hasher: &mut std::collections::hash_map::DefaultHasher) -> u64 {
        if let Some(value) = node.child_by_field_name("value")
            .or_else(|| node.child_by_field_name("object"))
            .or_else(|| node.named_child(0))
        {
            let value_hash = self.hash_expression(value, source, depth + 1);
            value_hash.hash(hasher);
        }

        if let Some(subscript) = node.child_by_field_name("subscript")
            .or_else(|| node.child_by_field_name("index"))
            .or_else(|| node.named_child(1))
        {
            let subscript_hash = self.hash_expression(subscript, source, depth + 1);
            subscript_hash.hash(hasher);
        }

        hasher.finish()
    }

    pub fn analyze_function(&mut self, func_node: Node, source: &[u8]) {
        self.collect_expressions(func_node, source);
    }

    fn collect_expressions(&mut self, node: Node, source: &[u8]) {
        let kind = node.kind();
        let lang = self.language;

        // Handle assignment statements
        if is_assignment_kind(kind, lang) {
            self.handle_assignment_node(node, source);
        }

        // Track non-assignment expressions
        if is_expression_kind(kind, lang) && !is_in_assignment(node, lang) {
            let hash = self.hash_expression(node, source, 0);
            let vn = self.get_or_create_vn(hash);
            let expr_text = node_text(node, source).trim().to_string();
            if !expr_text.is_empty() && is_interesting_expression(kind, &expr_text, lang) {
                let expr_ref = ExpressionRef {
                    text: expr_text,
                    line: get_line_number(node),
                    value_number: vn,
                };
                self.record_expression(expr_ref);
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.collect_expressions(child, source);
        }
    }

    /// Handle assignment nodes for variable-value propagation.
    /// Language-aware: detects different assignment patterns per language.
    fn handle_assignment_node(&mut self, node: Node, source: &[u8]) {
        let kind = node.kind();

        // Python: assignment with left/right fields
        if kind == "assignment" || kind == "augmented_assignment" {
            if let Some(left) = node.child_by_field_name("left") {
                if is_identifier_kind(left.kind(), self.language) {
                    let var_name = node_text(left, source).to_string();
                    if let Some(right) = node.child_by_field_name("right") {
                        let hash = self.hash_expression(right, source, 0);
                        let vn = self.get_or_create_vn(hash);
                        let expr_text = node_text(right, source).trim().to_string();
                        if !expr_text.is_empty() {
                            let expr_ref = ExpressionRef {
                                text: expr_text,
                                line: get_line_number(right),
                                value_number: vn,
                            };
                            self.record_expression(expr_ref);
                        }
                        self.propagate_through_assignment(&var_name, vn);
                    }
                }
            }
            return;
        }

        // C-family: variable_declaration / let_declaration / short_var_declaration etc.
        // These typically have a declarator child containing the name and value
        match kind {
            "variable_declaration" | "lexical_declaration" => {
                // JS/TS: const x = expr; / let x = expr;
                // has "variable_declarator" children
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "variable_declarator" {
                        self.handle_declarator(child, source);
                    }
                }
            }
            "local_variable_declaration" => {
                // Java: int x = expr;
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "variable_declarator" {
                        self.handle_declarator(child, source);
                    }
                }
            }
            "declaration" => {
                // C/C++: int x = a + b;
                // has "init_declarator" children
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "init_declarator" {
                        self.handle_declarator(child, source);
                    }
                }
            }
            "let_declaration" => {
                // Rust: let x = expr;
                if let Some(pat) = node.child_by_field_name("pattern") {
                    if let Some(value) = node.child_by_field_name("value") {
                        let var_name = node_text(pat, source).to_string();
                        let hash = self.hash_expression(value, source, 0);
                        let vn = self.get_or_create_vn(hash);
                        let expr_text = node_text(value, source).trim().to_string();
                        if !expr_text.is_empty() {
                            let expr_ref = ExpressionRef {
                                text: expr_text,
                                line: get_line_number(value),
                                value_number: vn,
                            };
                            self.record_expression(expr_ref);
                        }
                        self.propagate_through_assignment(&var_name, vn);
                    }
                }
            }
            "short_var_declaration" => {
                // Go: x := expr
                if let Some(left) = node.child_by_field_name("left") {
                    if let Some(right) = node.child_by_field_name("right") {
                        // left is expression_list, get first identifier
                        let var_name = if left.kind() == "expression_list" {
                            left.named_child(0).map(|c| node_text(c, source).to_string())
                        } else {
                            Some(node_text(left, source).to_string())
                        };
                        if let Some(var_name) = var_name {
                            // right is expression_list, get first expression
                            let right_expr = if right.kind() == "expression_list" {
                                right.named_child(0)
                            } else {
                                Some(right)
                            };
                            if let Some(right_expr) = right_expr {
                                let hash = self.hash_expression(right_expr, source, 0);
                                let vn = self.get_or_create_vn(hash);
                                let expr_text = node_text(right_expr, source).trim().to_string();
                                if !expr_text.is_empty() {
                                    let expr_ref = ExpressionRef {
                                        text: expr_text,
                                        line: get_line_number(right_expr),
                                        value_number: vn,
                                    };
                                    self.record_expression(expr_ref);
                                }
                                self.propagate_through_assignment(&var_name, vn);
                            }
                        }
                    }
                }
            }
            "assignment_statement" => {
                // Go: x = expr (also Lua)
                if let Some(left) = node.child_by_field_name("left") {
                    if let Some(right) = node.child_by_field_name("right") {
                        let var_name = if left.kind() == "expression_list" {
                            left.named_child(0).map(|c| node_text(c, source).to_string())
                        } else {
                            Some(node_text(left, source).to_string())
                        };
                        if let Some(var_name) = var_name {
                            let right_expr = if right.kind() == "expression_list" {
                                right.named_child(0)
                            } else {
                                Some(right)
                            };
                            if let Some(right_expr) = right_expr {
                                let hash = self.hash_expression(right_expr, source, 0);
                                let vn = self.get_or_create_vn(hash);
                                let expr_text = node_text(right_expr, source).trim().to_string();
                                if !expr_text.is_empty() {
                                    let expr_ref = ExpressionRef {
                                        text: expr_text,
                                        line: get_line_number(right_expr),
                                        value_number: vn,
                                    };
                                    self.record_expression(expr_ref);
                                }
                                self.propagate_through_assignment(&var_name, vn);
                            }
                        }
                    }
                }
            }
            "assignment_expression" => {
                // Java/C/C++/C#/PHP: x = expr (also augmented)
                if let Some(left) = node.child_by_field_name("left") {
                    if is_identifier_kind(left.kind(), self.language) {
                        let var_name = node_text(left, source).to_string();
                        if let Some(right) = node.child_by_field_name("right") {
                            let hash = self.hash_expression(right, source, 0);
                            let vn = self.get_or_create_vn(hash);
                            let expr_text = node_text(right, source).trim().to_string();
                            if !expr_text.is_empty() {
                                let expr_ref = ExpressionRef {
                                    text: expr_text,
                                    line: get_line_number(right),
                                    value_number: vn,
                                };
                                self.record_expression(expr_ref);
                            }
                            self.propagate_through_assignment(&var_name, vn);
                        }
                    }
                }
            }
            "property_declaration" => {
                // Kotlin: val x = expr
                self.handle_declarator(node, source);
            }
            "val_definition" | "var_definition" => {
                // Scala: val x = expr
                self.handle_declarator(node, source);
            }
            "variable_declaration" => {
                // Lua: local x = expr
                self.handle_lua_var_declaration(node, source);
            }
            "match_operator" => {
                // Elixir: x = expr
                self.handle_match_operator(node, source);
            }
            "let_binding" | "value_definition" => {
                // OCaml: let x = expr
                self.handle_declarator(node, source);
            }
            _ => {}
        }
    }

    /// Handle a variable declarator node (name = value pattern).
    fn handle_declarator(&mut self, node: Node, source: &[u8]) {
        // Try name + value fields
        let name_node = node.child_by_field_name("name")
            .or_else(|| node.child_by_field_name("pattern"));
        let value_node = node.child_by_field_name("value")
            .or_else(|| node.child_by_field_name("initializer"));

        if let (Some(name), Some(value)) = (name_node, value_node) {
            let var_name = node_text(name, source).to_string();
            let hash = self.hash_expression(value, source, 0);
            let vn = self.get_or_create_vn(hash);
            let expr_text = node_text(value, source).trim().to_string();
            if !expr_text.is_empty() {
                let expr_ref = ExpressionRef {
                    text: expr_text,
                    line: get_line_number(value),
                    value_number: vn,
                };
                self.record_expression(expr_ref);
            }
            self.propagate_through_assignment(&var_name, vn);
        }
    }

    /// Handle Lua variable declaration (local x = expr).
    fn handle_lua_var_declaration(&mut self, node: Node, source: &[u8]) {
        // Lua: variable_declaration has assignment_statement or variable_list children
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "assignment_statement" {
                self.handle_assignment_node(child, source);
            }
        }
    }

    /// Handle Elixir match operator (x = expr).
    fn handle_match_operator(&mut self, node: Node, source: &[u8]) {
        if let (Some(left), Some(right)) = (node.child_by_field_name("left").or_else(|| node.child(0)),
                                              node.child_by_field_name("right").or_else(|| node.child(2))) {
            if is_identifier_kind(left.kind(), self.language) {
                let var_name = node_text(left, source).to_string();
                let hash = self.hash_expression(right, source, 0);
                let vn = self.get_or_create_vn(hash);
                let expr_text = node_text(right, source).trim().to_string();
                if !expr_text.is_empty() {
                    let expr_ref = ExpressionRef {
                        text: expr_text,
                        line: get_line_number(right),
                        value_number: vn,
                    };
                    self.record_expression(expr_ref);
                }
                self.propagate_through_assignment(&var_name, vn);
            }
        }
    }

    pub fn find_redundancies(&self) -> Vec<Redundancy> {
        let mut redundancies = Vec::new();
        for (vn, exprs) in &self.expressions {
            if exprs.len() > 1 {
                let original = &exprs[0];
                for redundant in exprs.iter().skip(1) {
                    redundancies.push(Redundancy {
                        original: original.clone(),
                        redundant: redundant.clone(),
                        reason: if original.text == redundant.text {
                            "exact duplicate".to_string()
                        } else {
                            format!("equivalent to '{}' (same value number {})", original.text, vn)
                        },
                    });
                }
            }
        }
        redundancies.sort_by_key(|r| r.redundant.line);
        redundancies
    }

    pub fn build_equivalences(&self) -> Vec<GVNEquivalence> {
        let mut equivalences = Vec::new();
        for (vn, exprs) in &self.expressions {
            if exprs.len() > 1 {
                let texts: Vec<_> = exprs.iter().map(|e| e.text.as_str()).collect();
                let all_same = texts.iter().all(|t| *t == texts[0]);
                let reason = if all_same {
                    "exact duplicate expressions".to_string()
                } else {
                    "commutative/propagated equivalence".to_string()
                };
                equivalences.push(GVNEquivalence {
                    value_number: *vn,
                    expressions: exprs.clone(),
                    reason,
                });
            }
        }
        equivalences.sort_by_key(|e| e.value_number);
        equivalences
    }

    pub fn compute_summary(&self) -> GVNSummary {
        let total_expressions: u32 = self.expressions.values().map(|v| v.len() as u32).sum();
        let unique_values = self.expressions.len() as u32;
        let compression_ratio = if total_expressions > 0 {
            unique_values as f64 / total_expressions as f64
        } else {
            1.0
        };
        GVNSummary { total_expressions, unique_values, compression_ratio }
    }
}

impl Default for GVNEngine {
    fn default() -> Self { Self::new(Language::Python) }
}

// =============================================================================
// Language-aware helper functions
// =============================================================================

/// Check if a node kind represents an identifier in the given language.
fn is_identifier_kind(kind: &str, lang: Language) -> bool {
    ast_utils::identifier_node_kinds(lang).contains(&kind)
}

/// Check if a node kind represents an expression worth tracking.
fn is_expression_kind(kind: &str, lang: Language) -> bool {
    ast_utils::binary_expression_node_kinds(lang).contains(&kind)
        || ast_utils::unary_expression_node_kinds(lang).contains(&kind)
        || ast_utils::call_node_kinds(lang).contains(&kind)
        || is_field_access_kind(kind, lang)
        || is_subscript_kind(kind, lang)
        || is_collection_kind(kind)
        || ast_utils::parenthesized_expression_node_kinds(lang).contains(&kind)
        || ast_utils::boolean_expression_node_kinds(lang).contains(&kind)
        || ast_utils::comparison_node_kinds(lang).contains(&kind)
}

/// Check if a node kind is an assignment statement.
fn is_assignment_kind(kind: &str, lang: Language) -> bool {
    ast_utils::assignment_node_kinds(lang).contains(&kind)
}

/// Check if a node kind represents a field/attribute access.
fn is_field_access_kind(kind: &str, lang: Language) -> bool {
    ast_utils::field_access_info(lang).iter().any(|p| p.node_kind == kind)
}

/// Check if a node kind represents a subscript/index access.
fn is_subscript_kind(kind: &str, _lang: Language) -> bool {
    matches!(kind,
        "subscript" | "subscript_expression" | "index_expression" |
        "element_access_expression" | "array_access" | "computed_member_expression" |
        "member_access_expression"
    )
}

/// Check if a node kind represents a collection literal.
fn is_collection_kind(kind: &str) -> bool {
    matches!(kind,
        "list" | "tuple" | "set" | "dictionary" | "array" | "array_literal" |
        "object" | "map_literal" | "set_literal" | "list_literal" |
        "array_expression" | "array_creation_expression"
    )
}

/// Check if a node kind represents a boolean or null literal.
fn is_boolean_or_null_literal(kind: &str, _lang: Language) -> bool {
    matches!(kind,
        "true" | "false" | "none" | "null" | "nil" | "undefined" |
        "boolean_literal" | "null_literal" | "boolean"
    )
}

fn is_in_assignment(node: Node, lang: Language) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        if is_assignment_kind(parent.kind(), lang) {
            // Check if node is the right-hand side of the assignment
            if let Some(right) = parent.child_by_field_name("right")
                .or_else(|| parent.child_by_field_name("value"))
                .or_else(|| parent.child_by_field_name("initializer"))
            {
                if is_ancestor_of(right, node) { return true; }
            }
        }
        current = parent.parent();
    }
    false
}

fn is_ancestor_of(ancestor: Node, descendant: Node) -> bool {
    let mut current = Some(descendant);
    while let Some(node) = current {
        if node.id() == ancestor.id() { return true; }
        current = node.parent();
    }
    false
}

fn is_interesting_expression(kind: &str, text: &str, lang: Language) -> bool {
    if ast_utils::identifier_node_kinds(lang).contains(&kind) { return false; }
    if ast_utils::literal_node_kinds(lang).contains(&kind) { return false; }
    if is_boolean_or_null_literal(kind, lang) { return false; }
    text.len() >= 3
}

/// Extract operator text from children of a binary expression node.
/// Used when the grammar doesn't have a named "operator" field.
fn extract_operator_from_children(node: Node, source: &[u8], left: &Node, right: &Node) -> String {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            // The operator is typically between left and right children
            if child.start_byte() > left.end_byte() && child.end_byte() <= right.start_byte() {
                let text = node_text(child, source);
                if !text.is_empty() && !child.is_named() {
                    return text.to_string();
                }
            }
        }
    }
    // Last resort: try to find it from unnamed children
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if !child.is_named() {
                let text = node_text(child, source).trim();
                if is_operator_text(text) {
                    return text.to_string();
                }
            }
        }
    }
    String::new()
}

/// Check if text looks like an operator.
fn is_operator_text(text: &str) -> bool {
    matches!(text,
        "+" | "-" | "*" | "/" | "%" | "**" | "//" |
        "&" | "|" | "^" | "<<" | ">>" |
        "==" | "!=" | "<" | ">" | "<=" | ">=" |
        "and" | "or" | "not" | "&&" | "||" | "!" |
        ".." | "..=" | "..< " | "..."
    )
}

// =============================================================================
// Function finding (multi-language)
// =============================================================================

struct FunctionInfo { name: String, start_byte: usize, end_byte: usize, #[allow(dead_code)] line: u32 }

/// Get the node kinds that represent functions for each language
fn get_function_node_kinds(language: Language) -> &'static [&'static str] {
    ast_utils::function_node_kinds(language)
}

/// Extract the function name from a function definition node.
/// Handles language-specific patterns for name extraction.
fn extract_function_name(node: Node, source: &[u8], language: Language) -> Option<String> {
    // Try direct "name" field first (Python, Go, Rust, Kotlin, Swift, Scala, PHP)
    if let Some(name_node) = node.child_by_field_name("name") {
        return Some(node_text(name_node, source).to_string());
    }

    match language {
        Language::C | Language::Cpp => {
            // C/C++: function_definition -> declarator -> [function_]declarator -> identifier
            if let Some(decl) = node.child_by_field_name("declarator") {
                return extract_c_function_name(decl, source);
            }
        }
        Language::Ruby => {
            // Ruby "method": first named child is often the name identifier
            for i in 0..node.named_child_count() {
                if let Some(child) = node.named_child(i) {
                    if child.kind() == "identifier" {
                        return Some(node_text(child, source).to_string());
                    }
                }
            }
        }
        Language::Elixir => {
            // Elixir: def/defp are call nodes: call -> identifier("def") -> arguments -> [name, ...]
            // The function is a "call" with the callee being "def" or "defp"
            if node.kind() == "call" {
                // First child should be identifier (def/defp)
                if let Some(target) = node.named_child(0) {
                    let target_text = node_text(target, source);
                    if target_text == "def" || target_text == "defp" || target_text == "defmodule" {
                        // Second named child is typically the actual function call
                        if let Some(call) = node.named_child(1) {
                            // Could be a "call" node (def compute(a, b)) or identifier
                            if call.kind() == "call" {
                                if let Some(func_name) = call.named_child(0) {
                                    return Some(node_text(func_name, source).to_string());
                                }
                            } else if call.kind() == "identifier" || call.kind() == "atom" {
                                return Some(node_text(call, source).to_string());
                            }
                        }
                    }
                }
            }
        }
        Language::Ocaml => {
            // OCaml: let_binding / value_definition
            // Pattern: let <name> <args> = <body>
            if let Some(pattern) = node.child_by_field_name("pattern") {
                return Some(node_text(pattern, source).to_string());
            }
            // Try positional: first named child after "let" keyword
            for i in 0..node.named_child_count() {
                if let Some(child) = node.named_child(i) {
                    let ck = child.kind();
                    if ck == "value_name" || ck == "identifier" || ck == "value_path" {
                        return Some(node_text(child, source).to_string());
                    }
                }
            }
        }
        Language::Lua | Language::Luau => {
            // Lua: function_declaration has name field, but local_function might not
            // local_function: "local" "function" <name> <params> <body>
            if node.kind() == "local_function" || node.kind() == "function_declaration" {
                for i in 0..node.named_child_count() {
                    if let Some(child) = node.named_child(i) {
                        if child.kind() == "identifier" || child.kind() == "dot_index_expression" {
                            return Some(node_text(child, source).to_string());
                        }
                    }
                }
            }
        }
        _ => {}
    }

    // Fallback: try any child that is an identifier
    for i in 0..node.named_child_count() {
        if let Some(child) = node.named_child(i) {
            if ast_utils::identifier_node_kinds(language).contains(&child.kind()) {
                return Some(node_text(child, source).to_string());
            }
        }
    }

    None
}

/// Extract function name from C/C++ declarator chain.
fn extract_c_function_name(decl: Node, source: &[u8]) -> Option<String> {
    let kind = decl.kind();

    // Direct identifier
    if kind == "identifier" {
        return Some(node_text(decl, source).to_string());
    }

    // function_declarator: has a "declarator" field which is the name
    if kind == "function_declarator" {
        if let Some(inner_decl) = decl.child_by_field_name("declarator") {
            return extract_c_function_name(inner_decl, source);
        }
    }

    // pointer_declarator: *name
    if kind == "pointer_declarator" {
        if let Some(inner_decl) = decl.child_by_field_name("declarator") {
            return extract_c_function_name(inner_decl, source);
        }
    }

    // parenthesized_declarator: (name)
    if kind == "parenthesized_declarator" {
        for i in 0..decl.named_child_count() {
            if let Some(child) = decl.named_child(i) {
                if let Some(name) = extract_c_function_name(child, source) {
                    return Some(name);
                }
            }
        }
    }

    // Recurse into named children
    for i in 0..decl.named_child_count() {
        if let Some(child) = decl.named_child(i) {
            if let Some(name) = extract_c_function_name(child, source) {
                return Some(name);
            }
        }
    }

    None
}

fn find_functions(tree: &tree_sitter::Tree, source: &[u8], language: Language) -> Vec<FunctionInfo> {
    let mut functions = Vec::new();
    let func_kinds = get_function_node_kinds(language);

    fn collect_functions(node: Node, source: &[u8], functions: &mut Vec<FunctionInfo>, func_kinds: &[&str], language: Language) {
        if func_kinds.contains(&node.kind()) {
            if let Some(name) = extract_function_name(node, source, language) {
                functions.push(FunctionInfo {
                    name,
                    start_byte: node.start_byte(),
                    end_byte: node.end_byte(),
                    line: get_line_number(node),
                });
            }
        }
        // Check for arrow functions in variable declarations (TS/JS pattern):
        // lexical_declaration / variable_declaration -> variable_declarator -> name + value(arrow_function)
        if matches!(node.kind(), "lexical_declaration" | "variable_declaration") {
            let mut decl_cursor = node.walk();
            for child in node.children(&mut decl_cursor) {
                if child.kind() == "variable_declarator" {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        if let Some(value_node) = child.child_by_field_name("value") {
                            if matches!(value_node.kind(), "arrow_function" | "function" | "function_expression" | "generator_function") {
                                let var_name = node_text(name_node, source).to_string();
                                functions.push(FunctionInfo {
                                    name: var_name,
                                    start_byte: value_node.start_byte(),
                                    end_byte: value_node.end_byte(),
                                    line: get_line_number(value_node),
                                });
                            }
                        }
                    }
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) { collect_functions(child, source, functions, func_kinds, language); }
    }
    collect_functions(tree.root_node(), source, &mut functions, func_kinds, language);

    // For Elixir: also search for "call" nodes that are def/defp
    if language == Language::Elixir && functions.is_empty() {
        fn find_elixir_defs(node: Node, source: &[u8], functions: &mut Vec<FunctionInfo>) {
            if node.kind() == "call" {
                if let Some(target) = node.named_child(0) {
                    let target_text = node_text(target, source);
                    if target_text == "def" || target_text == "defp" {
                        if let Some(name) = extract_function_name(node, source, Language::Elixir) {
                            functions.push(FunctionInfo {
                                name,
                                start_byte: node.start_byte(),
                                end_byte: node.end_byte(),
                                line: get_line_number(node),
                            });
                        }
                    }
                }
            }
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                find_elixir_defs(child, source, functions);
            }
        }
        find_elixir_defs(tree.root_node(), source, &mut functions);
    }

    functions
}

fn find_node_by_range(root: Node, start: usize, end: usize) -> Option<Node> {
    if root.start_byte() == start && root.end_byte() == end { return Some(root); }
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if let Some(found) = find_node_by_range(child, start, end) { return Some(found); }
    }
    None
}

fn analyze_single_function(func_info: &FunctionInfo, tree: &tree_sitter::Tree, source: &[u8], language: Language) -> GVNReport {
    let mut engine = GVNEngine::new(language);
    if let Some(func_node) = find_node_by_range(tree.root_node(), func_info.start_byte, func_info.end_byte) {
        engine.analyze_function(func_node, source);
    }
    GVNReport {
        function: func_info.name.clone(),
        equivalences: engine.build_equivalences(),
        redundancies: engine.find_redundancies(),
        summary: engine.compute_summary(),
    }
}

fn format_gvn_text(report: &GVNReport) -> String {
    let mut output = format!("=== GVN Analysis: {} ===\n\nSummary:\n  Total expressions: {}\n  Unique values: {}\n  Compression ratio: {:.2}\n\n", report.function, report.summary.total_expressions, report.summary.unique_values, report.summary.compression_ratio);
    if !report.equivalences.is_empty() {
        output.push_str("Equivalence Classes:\n");
        for eq in &report.equivalences {
            output.push_str(&format!("  Value #{} ({}):\n", eq.value_number, eq.reason));
            for expr in &eq.expressions { output.push_str(&format!("    Line {}: {}\n", expr.line, expr.text)); }
        }
        output.push('\n');
    }
    if !report.redundancies.is_empty() {
        output.push_str("Redundant Expressions:\n");
        for red in &report.redundancies {
            output.push_str(&format!("  Line {}: '{}' is redundant\n    Original at line {}: '{}'\n    Reason: {}\n", red.redundant.line, red.redundant.text, red.original.line, red.original.text, red.reason));
        }
    } else {
        output.push_str("No redundant expressions found.\n");
    }
    output
}

fn format_reports_text(reports: &[GVNReport]) -> String {
    reports.iter().map(format_gvn_text).collect::<Vec<_>>().join("\n")
}

impl EquivalenceArgs {
    pub fn run(&self, format: OutputFormat, _quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, false);
        writer.progress(&format!("Analyzing expressions in {}...", self.file.display()));

        if !self.file.exists() { return Err(RemainingError::file_not_found(&self.file).into()); }

        // Detect language from --lang flag or file path
        let language = self.lang.or_else(|| Language::from_path(&self.file)).ok_or_else(|| {
            RemainingError::parse_error(&self.file, "Could not detect language from file extension. Use --lang to specify.".to_string())
        })?;

        let source = std::fs::read_to_string(&self.file).map_err(|e| RemainingError::parse_error(&self.file, e.to_string()))?;
        let source_bytes = source.as_bytes();

        let tree = parse(&source, language).map_err(|e| RemainingError::parse_error(&self.file, format!("Failed to parse file: {}", e)))?;

        let functions = find_functions(&tree, source_bytes, language);
        if functions.is_empty() { return Err(RemainingError::parse_error(&self.file, "No functions found in file".to_string()).into()); }

        if let Some(ref func_name) = self.function {
            let func = functions.iter().find(|f| f.name == *func_name).ok_or_else(|| RemainingError::symbol_not_found(func_name.clone(), &self.file))?;
            let report = analyze_single_function(func, &tree, source_bytes, language);
            match format {
                OutputFormat::Json => { println!("{}", serde_json::to_string_pretty(&report)?); }
                OutputFormat::Text => { println!("{}", format_gvn_text(&report)); }
                _ => { println!("{}", serde_json::to_string_pretty(&report)?); }
            }
        } else {
            let reports: Vec<GVNReport> = functions.iter().map(|f| analyze_single_function(f, &tree, source_bytes, language)).collect();
            match format {
                OutputFormat::Json => { println!("{}", serde_json::to_string_pretty(&reports)?); }
                OutputFormat::Text => { println!("{}", format_reports_text(&reports)); }
                _ => { println!("{}", serde_json::to_string_pretty(&reports)?); }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tldr_core::ast::parser::parse;
    use tldr_core::types::Language;

    #[test]
    fn test_find_ts_arrow_function_gvn() {
        let ts_source = r#"
const getDuration = (start: Date, end: Date): number => {
    const diff = end.getTime() - start.getTime();
    return diff;
};

function regularFunc(x: number): number {
    return x * 2;
}

export const processItems = (items: string[]) => {
    return items.map(i => i.trim());
};
"#;
        let language = Language::TypeScript;
        let tree = parse(ts_source, language).unwrap();
        let source_bytes = ts_source.as_bytes();

        let functions = find_functions(&tree, source_bytes, language);

        let names: Vec<&str> = functions.iter().map(|f| f.name.as_str()).collect();

        // Regular function should always work
        assert!(names.contains(&"regularFunc"), "Should find regular TS function, got: {:?}", names);

        // Arrow function assigned to const should also work
        assert!(names.contains(&"getDuration"), "Should find TS arrow function 'getDuration', got: {:?}", names);

        // Exported arrow function should also work
        assert!(names.contains(&"processItems"), "Should find exported TS arrow function 'processItems', got: {:?}", names);
    }
}
