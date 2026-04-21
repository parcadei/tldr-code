//! GVN Engine - Core Value Numbering Implementation
//!
//! Implements hash-based Global Value Numbering with:
//! - Commutativity awareness (a + b == b + a)
//! - Alias propagation (x = expr; y = x implies y has same VN as expr)
//! - Call conservatism (every call gets unique VN)
//! - Depth limiting (prevent stack overflow on deeply nested expressions)
//!
//! # Behavioral Contracts (from spec.md)
//!
//! - BC-GVN-1: Commutativity normalization
//! - BC-GVN-2: Alias propagation
//! - BC-GVN-3: Sequential analysis
//! - BC-GVN-4: Function call conservatism
//! - BC-GVN-5: Depth limiting (MAX_DEPTH = 10)

use std::collections::HashMap;
use tree_sitter::Node;

use super::hash_key::{is_commutative, normalize_binop, HashKey};
use super::types::{ExpressionRef, GVNEquivalence, GVNReport, Redundancy};
use crate::ast::parser::parse;
use crate::types::Language;

/// Maximum recursion depth for hashing nested expressions.
/// Expressions deeper than this get unique value numbers (BC-GVN-5).
const MAX_DEPTH: usize = 10;

/// GVN Engine for computing value numbers on Python AST expressions.
///
/// The engine maintains state for:
/// - Value number assignment (monotonic counter)
/// - Hash key to value number mapping
/// - Variable to value number mapping (for alias propagation)
/// - Expression tracking for reporting
pub struct GVNEngine {
    /// Monotonic counter for fresh value numbers
    next_vn: usize,
    /// Hash key -> value number mapping
    hash_to_vn: HashMap<HashKey, usize>,
    /// Variable name -> current value number (for alias propagation)
    var_vn: HashMap<String, usize>,
    /// All numbered expressions
    expressions: Vec<ExpressionRef>,
    /// Value number -> first occurrence (for redundancy detection)
    vn_first: HashMap<usize, ExpressionRef>,
    /// Track which value numbers are from commutative operations
    vn_is_commutative: HashMap<usize, bool>,
    /// Current recursion depth for hash_node
    depth: usize,
    /// Source code for extracting expression text
    source: String,
}

impl GVNEngine {
    /// Create a new GVN engine with the given source code.
    ///
    /// # Arguments
    /// * `source` - The source code being analyzed
    pub fn new(source: &str) -> Self {
        Self {
            next_vn: 0,
            hash_to_vn: HashMap::new(),
            var_vn: HashMap::new(),
            expressions: Vec::new(),
            vn_first: HashMap::new(),
            vn_is_commutative: HashMap::new(),
            depth: 0,
            source: source.to_string(),
        }
    }

    /// Generate a fresh (unique) value number.
    pub fn fresh_vn(&mut self) -> usize {
        let vn = self.next_vn;
        self.next_vn += 1;
        vn
    }

    /// Get the text for a tree-sitter node from the source.
    pub fn get_node_text(&self, node: &Node) -> String {
        node.utf8_text(self.source.as_bytes())
            .unwrap_or("")
            .to_string()
    }

    /// Hash a tree-sitter expression node to a HashKey.
    ///
    /// Returns (hash_key, is_commutative) where is_commutative indicates
    /// if the expression involves a commutative operation at the top level.
    ///
    /// # Behavioral Contracts
    /// - BC-GVN-4: Calls always get unique hash keys
    /// - BC-GVN-5: Depth > MAX_DEPTH gets unique hash key
    pub fn hash_node(&mut self, node: &Node) -> (HashKey, bool) {
        // BC-GVN-5: Depth limiting
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            self.depth -= 1;
            let id = self.fresh_vn();
            return (HashKey::Unique { id }, false);
        }

        let result = self.hash_node_inner(node);
        self.depth -= 1;
        result
    }

    /// Inner implementation of hash_node (called after depth check).
    fn hash_node_inner(&mut self, node: &Node) -> (HashKey, bool) {
        let kind = node.kind();

        match kind {
            // === Constants ===
            "integer" | "float" | "string" | "true" | "false" | "none" => {
                let text = self.get_node_text(node);
                let type_name = match kind {
                    "integer" => "int",
                    "float" => "float",
                    "string" => "str",
                    "true" | "false" => "bool",
                    "none" => "NoneType",
                    _ => "unknown",
                };
                (
                    HashKey::Const {
                        type_name: type_name.to_string(),
                        repr: text,
                    },
                    false,
                )
            }

            // === Identifiers (Names) ===
            "identifier" => {
                let name = self.get_node_text(node);
                // Check if we have a known value number for this variable
                if let Some(&vn) = self.var_vn.get(&name) {
                    (HashKey::VarVN { vn }, false)
                } else {
                    (HashKey::Name { name }, false)
                }
            }

            // === Binary Operations ===
            "binary_operator" => self.hash_binary_operator(node),

            // === Unary Operations ===
            "unary_operator" => self.hash_unary_operator(node),

            // === Boolean Operations (and/or) ===
            "boolean_operator" => self.hash_boolean_operator(node),

            // === Comparison Operations ===
            "comparison_operator" => self.hash_comparison(node),

            // === Function Calls - Always Unique (BC-GVN-4) ===
            "call" => {
                let id = self.fresh_vn();
                (HashKey::Call { unique_id: id }, false)
            }

            // === Attribute Access ===
            "attribute" => self.hash_attribute(node),

            // === Subscript Access ===
            "subscript" => self.hash_subscript(node),

            // === Parenthesized expressions ===
            "parenthesized_expression" => {
                // Unwrap the inner expression
                if let Some(inner) = node
                    .child_by_field_name("expression")
                    .or_else(|| node.named_child(0))
                {
                    self.hash_node_inner(&inner)
                } else {
                    let id = self.fresh_vn();
                    (HashKey::Unique { id }, false)
                }
            }

            // === Fallback: Unique hash key ===
            _ => {
                let id = self.fresh_vn();
                (HashKey::Unique { id }, false)
            }
        }
    }

    /// Hash a binary operator node.
    fn hash_binary_operator(&mut self, node: &Node) -> (HashKey, bool) {
        // Get left operand
        let left_node = match node.child_by_field_name("left") {
            Some(n) => n,
            None => {
                let id = self.fresh_vn();
                return (HashKey::Unique { id }, false);
            }
        };

        // Get operator
        let op_node = match node.child_by_field_name("operator") {
            Some(n) => n,
            None => {
                // Try to find operator by looking at children
                let mut op = None;
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        let k = child.kind();
                        if matches!(
                            k,
                            "+" | "-"
                                | "*"
                                | "/"
                                | "//"
                                | "%"
                                | "**"
                                | "|"
                                | "&"
                                | "^"
                                | "<<"
                                | ">>"
                                | "@"
                        ) {
                            op = Some(k.to_string());
                            break;
                        }
                    }
                }
                match op {
                    Some(o) => {
                        // Convert symbol to name
                        let op_name = match o.as_str() {
                            "+" => "Add",
                            "-" => "Sub",
                            "*" => "Mult",
                            "/" => "Div",
                            "//" => "FloorDiv",
                            "%" => "Mod",
                            "**" => "Pow",
                            "|" => "BitOr",
                            "&" => "BitAnd",
                            "^" => "BitXor",
                            "<<" => "LShift",
                            ">>" => "RShift",
                            "@" => "MatMult",
                            _ => &o,
                        };
                        return self.build_binop_hash(&left_node, op_name, node);
                    }
                    None => {
                        let id = self.fresh_vn();
                        return (HashKey::Unique { id }, false);
                    }
                }
            }
        };

        let op_text = self.get_node_text(&op_node);
        let op_name = match op_text.as_str() {
            "+" => "Add",
            "-" => "Sub",
            "*" => "Mult",
            "/" => "Div",
            "//" => "FloorDiv",
            "%" => "Mod",
            "**" => "Pow",
            "|" => "BitOr",
            "&" => "BitAnd",
            "^" => "BitXor",
            "<<" => "LShift",
            ">>" => "RShift",
            "@" => "MatMult",
            _ => &op_text,
        };

        self.build_binop_hash(&left_node, op_name, node)
    }

    /// Build a BinOp hash key from left operand, operator name, and parent node.
    fn build_binop_hash(
        &mut self,
        left_node: &Node,
        op_name: &str,
        parent: &Node,
    ) -> (HashKey, bool) {
        // Get right operand
        let right_node = match parent.child_by_field_name("right") {
            Some(n) => n,
            None => {
                let id = self.fresh_vn();
                return (HashKey::Unique { id }, false);
            }
        };

        // Recursively hash operands
        let (left_key, _) = self.hash_node(left_node);
        let (right_key, _) = self.hash_node(&right_node);

        // Use normalize_binop for commutative normalization
        let commutative = is_commutative(op_name);
        let hash_key = normalize_binop(op_name, left_key, right_key);

        (hash_key, commutative)
    }

    /// Hash a unary operator node.
    fn hash_unary_operator(&mut self, node: &Node) -> (HashKey, bool) {
        // Get operand
        let operand_node = match node
            .child_by_field_name("argument")
            .or_else(|| node.child_by_field_name("operand"))
            .or_else(|| {
                // Find first named child that's not the operator
                for i in 0..node.named_child_count() {
                    if let Some(child) = node.named_child(i) {
                        if !matches!(child.kind(), "-" | "+" | "~" | "not") {
                            return Some(child);
                        }
                    }
                }
                None
            }) {
            Some(n) => n,
            None => {
                let id = self.fresh_vn();
                return (HashKey::Unique { id }, false);
            }
        };

        // Get operator
        let op = {
            let mut found_op = None;
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    let k = child.kind();
                    if matches!(k, "-" | "+" | "~" | "not") {
                        found_op = Some(match k {
                            "-" => "USub",
                            "+" => "UAdd",
                            "~" => "Invert",
                            "not" => "Not",
                            _ => k,
                        });
                        break;
                    }
                }
            }
            match found_op {
                Some(o) => o.to_string(),
                None => {
                    let id = self.fresh_vn();
                    return (HashKey::Unique { id }, false);
                }
            }
        };

        let (operand_key, _) = self.hash_node(&operand_node);

        (
            HashKey::UnaryOp {
                op,
                operand: Box::new(operand_key),
            },
            false,
        )
    }

    /// Hash a boolean operator node (and/or).
    fn hash_boolean_operator(&mut self, node: &Node) -> (HashKey, bool) {
        let mut operands = Vec::new();
        let mut op_name = String::new();

        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                let kind = child.kind();
                if kind == "and" || kind == "or" {
                    op_name = kind.to_string();
                } else if child.is_named() {
                    let (key, _) = self.hash_node(&child);
                    operands.push(key);
                }
            }
        }

        if operands.is_empty() || op_name.is_empty() {
            let id = self.fresh_vn();
            return (HashKey::Unique { id }, false);
        }

        (
            HashKey::BoolOp {
                op: op_name,
                operands,
            },
            false,
        )
    }

    /// Hash a comparison operator node.
    fn hash_comparison(&mut self, node: &Node) -> (HashKey, bool) {
        let mut parts = Vec::new();

        // Collect all parts of the comparison (operands and operators)
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                let text = self.get_node_text(&child);
                if !text.is_empty() {
                    parts.push(text);
                }
            }
        }

        if parts.is_empty() {
            let id = self.fresh_vn();
            return (HashKey::Unique { id }, false);
        }

        (HashKey::Compare { parts }, false)
    }

    /// Hash an attribute access node.
    fn hash_attribute(&mut self, node: &Node) -> (HashKey, bool) {
        let value_node = match node
            .child_by_field_name("object")
            .or_else(|| node.child_by_field_name("value"))
        {
            Some(n) => n,
            None => {
                let id = self.fresh_vn();
                return (HashKey::Unique { id }, false);
            }
        };

        let attr_node = match node.child_by_field_name("attribute") {
            Some(n) => n,
            None => {
                let id = self.fresh_vn();
                return (HashKey::Unique { id }, false);
            }
        };

        let (value_key, _) = self.hash_node(&value_node);
        let attr = self.get_node_text(&attr_node);

        (
            HashKey::Attribute {
                value: Box::new(value_key),
                attr,
            },
            false,
        )
    }

    /// Hash a subscript access node.
    fn hash_subscript(&mut self, node: &Node) -> (HashKey, bool) {
        let value_node = match node.child_by_field_name("value") {
            Some(n) => n,
            None => {
                let id = self.fresh_vn();
                return (HashKey::Unique { id }, false);
            }
        };

        let slice_node = match node.child_by_field_name("subscript") {
            Some(n) => n,
            None => {
                let id = self.fresh_vn();
                return (HashKey::Unique { id }, false);
            }
        };

        let (value_key, _) = self.hash_node(&value_node);
        let (slice_key, _) = self.hash_node(&slice_node);

        (
            HashKey::Subscript {
                value: Box::new(value_key),
                slice: Box::new(slice_key),
            },
            false,
        )
    }

    /// Assign a value number to an expression.
    ///
    /// # Arguments
    /// * `node` - The expression AST node
    /// * `line` - The line number (1-based)
    ///
    /// # Returns
    /// The assigned value number
    ///
    /// # Behavioral Contracts
    /// - BC-GVN-2: For Name nodes with known VN, return directly (alias propagation)
    pub fn number_expression(&mut self, node: &Node, line: usize) -> usize {
        let text = self.get_node_text(node);

        // BC-GVN-2: Alias propagation for identifiers
        if node.kind() == "identifier" {
            if let Some(&vn) = self.var_vn.get(&text) {
                // Record this reference
                let expr_ref = ExpressionRef::new(&text, line, vn);
                self.expressions.push(expr_ref);
                return vn;
            }
        }

        // Hash the node
        let (hash_key, is_comm) = self.hash_node(node);

        // Look up or create value number
        let vn = if let Some(&existing_vn) = self.hash_to_vn.get(&hash_key) {
            existing_vn
        } else {
            let new_vn = self.fresh_vn();
            self.hash_to_vn.insert(hash_key, new_vn);
            new_vn
        };

        // Record expression reference
        let expr_ref = ExpressionRef::new(&text, line, vn);

        // Track first occurrence
        self.vn_first
            .entry(vn)
            .or_insert_with(|| expr_ref.clone());

        // Track commutativity
        if is_comm {
            self.vn_is_commutative.insert(vn, true);
        }

        self.expressions.push(expr_ref);
        vn
    }

    /// Record that a variable has a specific value number.
    ///
    /// Used for alias propagation (BC-GVN-2).
    ///
    /// # Arguments
    /// * `name` - The variable name
    /// * `vn` - The value number to assign
    pub fn assign_variable(&mut self, name: &str, vn: usize) {
        self.var_vn.insert(name.to_string(), vn);
    }

    /// Remove a variable from tracking.
    ///
    /// Called when a variable is reassigned to invalidate alias propagation.
    ///
    /// # Arguments
    /// * `name` - The variable name to remove
    pub fn kill_variable(&mut self, name: &str) {
        self.var_vn.remove(name);
    }

    /// Get all recorded expressions.
    pub fn expressions(&self) -> &[ExpressionRef] {
        &self.expressions
    }

    /// Get the first occurrence for a value number.
    pub fn get_first(&self, vn: usize) -> Option<&ExpressionRef> {
        self.vn_first.get(&vn)
    }

    /// Check if a value number is from a commutative operation.
    pub fn is_commutative_vn(&self, vn: usize) -> bool {
        self.vn_is_commutative.get(&vn).copied().unwrap_or(false)
    }

    /// Get the current count of unique value numbers.
    pub fn unique_count(&self) -> usize {
        self.next_vn
    }

    /// Get the variable to VN mapping (for debugging/testing).
    pub fn var_vn_map(&self) -> &HashMap<String, usize> {
        &self.var_vn
    }

    // =========================================================================
    // P3: Statement Walking (BC-GVN-3)
    // =========================================================================

    /// Walk a statement node and number its expressions.
    ///
    /// Handles statement types sequentially for proper alias propagation:
    /// - Assign: number RHS, assign VN to LHS targets
    /// - AugAssign: number value, kill target, assign new VN
    /// - AnnAssign: if value exists, number it and assign
    /// - Expr: number the expression
    /// - Return: number the value if present
    /// - Control flow: walk body, orelse, etc. recursively
    pub fn walk_stmt(&mut self, stmt: &Node) {
        let kind = stmt.kind();
        let line = stmt.start_position().row + 1;

        match kind {
            // === Expression statement containing an assignment ===
            "expression_statement" => {
                // Check if first child is an assignment
                if let Some(inner) = stmt.named_child(0) {
                    if inner.kind() == "assignment" {
                        self.handle_assignment(&inner, line);
                    } else if inner.kind() == "augmented_assignment" {
                        self.handle_aug_assignment(&inner, line);
                    } else {
                        // Standalone expression (e.g., call)
                        let inner_line = inner.start_position().row + 1;
                        self.number_expression(&inner, inner_line);
                    }
                }
            }

            // === Direct assignment (shouldn't happen in Python, but just in case) ===
            "assignment" => {
                self.handle_assignment(stmt, line);
            }

            // === Augmented assignment: x += expr ===
            "augmented_assignment" => {
                self.handle_aug_assignment(stmt, line);
            }

            // === Return statement ===
            "return_statement" => {
                // Number the return value if present
                for i in 0..stmt.named_child_count() {
                    if let Some(child) = stmt.named_child(i) {
                        if child.kind() != "return" {
                            let child_line = child.start_position().row + 1;
                            self.number_expression(&child, child_line);
                        }
                    }
                }
            }

            // === If statement ===
            "if_statement" => {
                // Walk condition
                if let Some(cond) = stmt.child_by_field_name("condition") {
                    let cond_line = cond.start_position().row + 1;
                    self.number_expression(&cond, cond_line);
                }
                // Walk consequence (body)
                if let Some(body) = stmt.child_by_field_name("consequence") {
                    self.walk_block(&body);
                }
                // Walk alternative (else/elif)
                if let Some(alt) = stmt.child_by_field_name("alternative") {
                    self.walk_block_or_stmt(&alt);
                }
            }

            // === For loop ===
            "for_statement" => {
                // Walk the iterable
                if let Some(right) = stmt.child_by_field_name("right") {
                    let right_line = right.start_position().row + 1;
                    self.number_expression(&right, right_line);
                }
                // Walk body
                if let Some(body) = stmt.child_by_field_name("body") {
                    self.walk_block(&body);
                }
                // Walk else clause if present
                if let Some(alt) = stmt.child_by_field_name("alternative") {
                    self.walk_block(&alt);
                }
            }

            // === While loop ===
            "while_statement" => {
                // Walk condition
                if let Some(cond) = stmt.child_by_field_name("condition") {
                    let cond_line = cond.start_position().row + 1;
                    self.number_expression(&cond, cond_line);
                }
                // Walk body
                if let Some(body) = stmt.child_by_field_name("body") {
                    self.walk_block(&body);
                }
                // Walk else clause if present
                if let Some(alt) = stmt.child_by_field_name("alternative") {
                    self.walk_block(&alt);
                }
            }

            // === With statement ===
            "with_statement" => {
                // Walk items
                for i in 0..stmt.named_child_count() {
                    if let Some(child) = stmt.named_child(i) {
                        if child.kind() == "with_clause" {
                            self.walk_with_clause(&child);
                        } else if child.kind() == "block" {
                            self.walk_block(&child);
                        }
                    }
                }
            }

            // === Try statement ===
            "try_statement" => {
                // Walk body
                if let Some(body) = stmt.child_by_field_name("body") {
                    self.walk_block(&body);
                }
                // Walk except handlers
                for i in 0..stmt.named_child_count() {
                    if let Some(child) = stmt.named_child(i) {
                        match child.kind() {
                            "except_clause" | "except_group_clause" => {
                                self.walk_except_clause(&child);
                            }
                            "else_clause" => {
                                if let Some(body) = child.child_by_field_name("body") {
                                    self.walk_block(&body);
                                }
                            }
                            "finally_clause" => {
                                if let Some(body) = child.child_by_field_name("body") {
                                    self.walk_block(&body);
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }

            // === Function definitions: skip (analyze separately) ===
            "function_definition" | "async_function_definition" => {
                // Skip - these should be analyzed as separate functions
            }

            // === Class definition: skip ===
            "class_definition" => {
                // Skip
            }

            // === Pass/Break/Continue: no expressions ===
            "pass_statement" | "break_statement" | "continue_statement" => {
                // Nothing to do
            }

            // === Assert statement ===
            "assert_statement" => {
                for i in 0..stmt.named_child_count() {
                    if let Some(child) = stmt.named_child(i) {
                        let child_line = child.start_position().row + 1;
                        self.number_expression(&child, child_line);
                    }
                }
            }

            // === Raise statement ===
            "raise_statement" => {
                for i in 0..stmt.named_child_count() {
                    if let Some(child) = stmt.named_child(i) {
                        if child.kind() != "from" {
                            let child_line = child.start_position().row + 1;
                            self.number_expression(&child, child_line);
                        }
                    }
                }
            }

            // === Import statements: no expressions ===
            "import_statement" | "import_from_statement" => {
                // Nothing to do
            }

            // === Global/nonlocal declarations: no expressions ===
            "global_statement" | "nonlocal_statement" => {
                // Nothing to do
            }

            // === Delete statement ===
            "delete_statement" => {
                // Nothing to number
            }

            // === Match statement (Python 3.10+) ===
            "match_statement" => {
                // Walk subject
                if let Some(subject) = stmt.child_by_field_name("subject") {
                    let subj_line = subject.start_position().row + 1;
                    self.number_expression(&subject, subj_line);
                }
                // Walk cases
                for i in 0..stmt.named_child_count() {
                    if let Some(child) = stmt.named_child(i) {
                        if child.kind() == "case_clause" {
                            if let Some(body) = child.child_by_field_name("consequence") {
                                self.walk_block(&body);
                            }
                        }
                    }
                }
            }

            // === Default: walk children looking for statements ===
            _ => {
                // Walk named children
                for i in 0..stmt.named_child_count() {
                    if let Some(child) = stmt.named_child(i) {
                        if self.is_statement_kind(child.kind()) {
                            self.walk_stmt(&child);
                        }
                    }
                }
            }
        }
    }

    /// Handle a regular assignment statement.
    fn handle_assignment(&mut self, stmt: &Node, _line: usize) {
        // Get the RHS (right side)
        if let Some(right) = stmt.child_by_field_name("right") {
            let right_line = right.start_position().row + 1;
            let vn = self.number_expression(&right, right_line);

            // Assign VN to each LHS target
            if let Some(left) = stmt.child_by_field_name("left") {
                self.assign_targets(&left, vn);
            }
        }
    }

    /// Handle an augmented assignment (e.g., x += expr).
    fn handle_aug_assignment(&mut self, stmt: &Node, _line: usize) {
        // Get the value (RHS)
        if let Some(right) = stmt.child_by_field_name("right") {
            let right_line = right.start_position().row + 1;
            self.number_expression(&right, right_line);
        }

        // Kill and reassign the target
        if let Some(left) = stmt.child_by_field_name("left") {
            let target_name = self.get_node_text(&left);
            self.kill_variable(&target_name);

            // Aug assign creates a new value (we don't track the operation precisely)
            let new_vn = self.fresh_vn();
            self.assign_variable(&target_name, new_vn);
        }
    }

    /// Assign value number to target(s) in an assignment.
    fn assign_targets(&mut self, target: &Node, vn: usize) {
        let kind = target.kind();

        match kind {
            "identifier" => {
                let name = self.get_node_text(target);
                self.assign_variable(&name, vn);
            }
            "pattern_list" | "tuple_pattern" | "list_pattern" => {
                // Tuple/list unpacking - kill all targets (imprecise but safe)
                for i in 0..target.named_child_count() {
                    if let Some(child) = target.named_child(i) {
                        if child.kind() == "identifier" {
                            let name = self.get_node_text(&child);
                            self.kill_variable(&name);
                        }
                    }
                }
            }
            "tuple" | "list" => {
                // e.g., (a, b) = expr
                for i in 0..target.named_child_count() {
                    if let Some(child) = target.named_child(i) {
                        if child.kind() == "identifier" {
                            let name = self.get_node_text(&child);
                            self.kill_variable(&name);
                        }
                    }
                }
            }
            "attribute" | "subscript" => {
                // x.attr = ... or x[i] = ... - don't track these
            }
            _ => {
                // Single target
                let name = self.get_node_text(target);
                if !name.is_empty()
                    && name
                        .chars()
                        .next()
                        .map(|c| c.is_alphabetic() || c == '_')
                        .unwrap_or(false)
                {
                    self.assign_variable(&name, vn);
                }
            }
        }
    }

    /// Walk a block (compound statement body).
    fn walk_block(&mut self, block: &Node) {
        for i in 0..block.named_child_count() {
            if let Some(child) = block.named_child(i) {
                self.walk_stmt(&child);
            }
        }
    }

    /// Walk a block or a single statement (for else clauses).
    fn walk_block_or_stmt(&mut self, node: &Node) {
        if node.kind() == "block" {
            self.walk_block(node);
        } else if node.kind() == "elif_clause" || node.kind() == "else_clause" {
            // Walk condition if present (elif)
            if let Some(cond) = node.child_by_field_name("condition") {
                let cond_line = cond.start_position().row + 1;
                self.number_expression(&cond, cond_line);
            }
            // Walk body
            if let Some(body) = node.child_by_field_name("consequence") {
                self.walk_block(&body);
            }
            // Walk further alternative
            if let Some(alt) = node.child_by_field_name("alternative") {
                self.walk_block_or_stmt(&alt);
            }
        } else {
            self.walk_stmt(node);
        }
    }

    /// Walk a with clause.
    fn walk_with_clause(&mut self, clause: &Node) {
        for i in 0..clause.named_child_count() {
            if let Some(item) = clause.named_child(i) {
                if item.kind() == "with_item" {
                    if let Some(value) = item.child_by_field_name("value") {
                        let value_line = value.start_position().row + 1;
                        self.number_expression(&value, value_line);
                    }
                }
            }
        }
    }

    /// Walk an except clause.
    fn walk_except_clause(&mut self, clause: &Node) {
        // Walk body
        for i in 0..clause.named_child_count() {
            if let Some(child) = clause.named_child(i) {
                if child.kind() == "block" {
                    self.walk_block(&child);
                }
            }
        }
    }

    /// Check if a kind is a statement kind.
    fn is_statement_kind(&self, kind: &str) -> bool {
        matches!(
            kind,
            "expression_statement"
                | "assignment"
                | "augmented_assignment"
                | "return_statement"
                | "if_statement"
                | "for_statement"
                | "while_statement"
                | "with_statement"
                | "try_statement"
                | "function_definition"
                | "async_function_definition"
                | "class_definition"
                | "pass_statement"
                | "break_statement"
                | "continue_statement"
                | "assert_statement"
                | "raise_statement"
                | "import_statement"
                | "import_from_statement"
                | "global_statement"
                | "nonlocal_statement"
                | "delete_statement"
                | "match_statement"
        )
    }

    /// Walk a list of statements (function body).
    pub fn walk_stmts(&mut self, body: &Node) {
        for i in 0..body.named_child_count() {
            if let Some(child) = body.named_child(i) {
                self.walk_stmt(&child);
            }
        }
    }

    // =========================================================================
    // P3: Report Building (BC-GVN-6)
    // =========================================================================

    /// Build the GVN report for the analyzed function.
    ///
    /// Groups expressions by value number and creates:
    /// - Equivalence classes for VNs with 2+ expressions
    /// - Redundancy records (first = original, rest = redundant)
    pub fn build_report(&self, func_name: &str) -> GVNReport {
        let mut report = GVNReport::new(func_name);

        // Group expressions by value number
        let mut vn_to_exprs: HashMap<usize, Vec<ExpressionRef>> = HashMap::new();
        for expr in &self.expressions {
            vn_to_exprs
                .entry(expr.value_number)
                .or_default()
                .push(expr.clone());
        }

        // Collect unique value numbers actually used
        let unique_vns: std::collections::HashSet<usize> =
            self.expressions.iter().map(|e| e.value_number).collect();

        report.total_expressions = self.expressions.len();
        report.unique_values = unique_vns.len();

        // Build equivalence classes and redundancies
        for (vn, exprs) in vn_to_exprs {
            if exprs.len() >= 2 {
                // Determine reason
                let reason = if self.is_commutative_vn(vn) {
                    "commutativity".to_string()
                } else {
                    "identical expression".to_string()
                };

                // Create equivalence class
                let equiv = GVNEquivalence::new(vn, exprs.clone(), &reason);
                report.equivalences.push(equiv);

                // Create redundancy records (first is original, rest are redundant)
                let mut sorted_exprs = exprs;
                sorted_exprs.sort_by_key(|e| e.line);

                if let Some(original) = sorted_exprs.first() {
                    for redundant in sorted_exprs.iter().skip(1) {
                        let redund = Redundancy::new(original.clone(), redundant.clone(), &reason);
                        report.redundancies.push(redund);
                    }
                }
            }
        }

        report
    }
}

// =============================================================================
// P4: Public API - compute_gvn
// =============================================================================

/// Compute GVN analysis for Python source code.
///
/// If `function_name` is Some, analyze only that function.
/// If `function_name` is None, analyze all top-level functions.
///
/// # Arguments
/// * `source` - Python source code to analyze
/// * `function_name` - Optional function name to filter
///
/// # Returns
/// A vector of GVNReport, one per analyzed function.
///
/// # Behavioral Contracts
/// - BC-GVN-1: Commutativity normalization
/// - BC-GVN-2: Alias propagation
/// - BC-GVN-3: Sequential analysis
/// - BC-GVN-4: Function call conservatism
/// - BC-GVN-5: Depth limiting
/// - BC-GVN-6: Redundancy detection
pub fn compute_gvn(source: &str, function_name: Option<&str>) -> Vec<GVNReport> {
    // Handle empty source
    if source.trim().is_empty() {
        return vec![];
    }

    // Parse the source
    let tree = match parse(source, Language::Python) {
        Ok(t) => t,
        Err(_) => return vec![],
    };

    let root = tree.root_node();
    let source_bytes = source.as_bytes();
    let mut reports = Vec::new();

    // Find all function definitions
    let mut functions = Vec::new();
    find_functions(root, source_bytes, &mut functions);

    // Filter by function_name if provided
    let filtered: Vec<_> = if let Some(name) = function_name {
        functions
            .into_iter()
            .filter(|(fn_name, _)| fn_name == name)
            .collect()
    } else {
        functions
    };

    // Analyze each function
    for (fn_name, fn_node) in filtered {
        let mut engine = GVNEngine::new(source);

        // Find and walk the function body
        if let Some(body) = fn_node.child_by_field_name("body") {
            engine.walk_stmts(&body);
        }

        let report = engine.build_report(&fn_name);
        reports.push(report);
    }

    reports
}

/// Find all function definitions in the tree.
fn find_functions<'a>(node: Node<'a>, source: &[u8], results: &mut Vec<(String, Node<'a>)>) {
    let kind = node.kind();

    if kind == "function_definition" || kind == "async_function_definition" {
        // Get the function name
        if let Some(name_node) = node.child_by_field_name("name") {
            let name = name_node.utf8_text(source).unwrap_or("");
            results.push((name.to_string(), node));
        }
    }

    // Recurse into module-level children (but not into function bodies)
    if kind == "module" {
        for i in 0..node.named_child_count() {
            if let Some(child) = node.named_child(i) {
                find_functions(child, source, results);
            }
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::parser::parse;
    use crate::types::Language;

    /// Helper to find a node by kind in the tree
    fn find_node_by_kind<'a>(
        node: tree_sitter::Node<'a>,
        kind: &str,
    ) -> Option<tree_sitter::Node<'a>> {
        if node.kind() == kind {
            return Some(node);
        }
        for i in 0..node.named_child_count() {
            if let Some(child) = node.named_child(i) {
                if let Some(found) = find_node_by_kind(child, kind) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// Helper to find all nodes by kind
    fn find_all_nodes_by_kind<'a>(
        node: tree_sitter::Node<'a>,
        kind: &str,
        results: &mut Vec<tree_sitter::Node<'a>>,
    ) {
        if node.kind() == kind {
            results.push(node);
        }
        for i in 0..node.named_child_count() {
            if let Some(child) = node.named_child(i) {
                find_all_nodes_by_kind(child, kind, results);
            }
        }
    }

    // =========================================================================
    // test_hash_node_constant
    // =========================================================================

    #[test]
    fn test_hash_node_constant_integer() {
        let source = "42";
        let tree = parse(source, Language::Python).unwrap();
        let mut engine = GVNEngine::new(source);

        let root = tree.root_node();
        let int_node = find_node_by_kind(root, "integer").expect("Should find integer node");

        let (key, is_comm) = engine.hash_node(&int_node);

        assert!(!is_comm, "Constants are not commutative");
        match key {
            HashKey::Const { type_name, repr } => {
                assert_eq!(type_name, "int");
                assert_eq!(repr, "42");
            }
            _ => panic!("Expected Const hash key, got {:?}", key),
        }
    }

    #[test]
    fn test_hash_node_constant_string() {
        let source = r#""hello""#;
        let tree = parse(source, Language::Python).unwrap();
        let mut engine = GVNEngine::new(source);

        let root = tree.root_node();
        let str_node = find_node_by_kind(root, "string").expect("Should find string node");

        let (key, is_comm) = engine.hash_node(&str_node);

        assert!(!is_comm);
        match key {
            HashKey::Const { type_name, repr } => {
                assert_eq!(type_name, "str");
                assert_eq!(repr, r#""hello""#);
            }
            _ => panic!("Expected Const hash key, got {:?}", key),
        }
    }

    // =========================================================================
    // test_hash_node_binop_commutative
    // =========================================================================

    #[test]
    fn test_hash_node_binop_commutative_add() {
        let source = "x + y";
        let tree = parse(source, Language::Python).unwrap();
        let mut engine = GVNEngine::new(source);

        let root = tree.root_node();
        let binop =
            find_node_by_kind(root, "binary_operator").expect("Should find binary_operator");

        let (key, is_comm) = engine.hash_node(&binop);

        assert!(is_comm, "Add is commutative");
        match &key {
            HashKey::BinOp {
                op, commutative, ..
            } => {
                assert_eq!(op, "Add");
                assert!(*commutative);
            }
            _ => panic!("Expected BinOp hash key, got {:?}", key),
        }
    }

    #[test]
    fn test_hash_node_binop_commutative_normalization() {
        // x + y and y + x should produce the same hash key
        let source1 = "x + y";
        let source2 = "y + x";

        let tree1 = parse(source1, Language::Python).unwrap();
        let tree2 = parse(source2, Language::Python).unwrap();

        let mut engine1 = GVNEngine::new(source1);
        let mut engine2 = GVNEngine::new(source2);

        let binop1 = find_node_by_kind(tree1.root_node(), "binary_operator").unwrap();
        let binop2 = find_node_by_kind(tree2.root_node(), "binary_operator").unwrap();

        let (key1, _) = engine1.hash_node(&binop1);
        let (key2, _) = engine2.hash_node(&binop2);

        assert_eq!(key1, key2, "x + y and y + x should hash to the same key");
    }

    #[test]
    fn test_hash_node_binop_non_commutative() {
        // x - y and y - x should produce different hash keys
        let source1 = "x - y";
        let source2 = "y - x";

        let tree1 = parse(source1, Language::Python).unwrap();
        let tree2 = parse(source2, Language::Python).unwrap();

        let mut engine1 = GVNEngine::new(source1);
        let mut engine2 = GVNEngine::new(source2);

        let binop1 = find_node_by_kind(tree1.root_node(), "binary_operator").unwrap();
        let binop2 = find_node_by_kind(tree2.root_node(), "binary_operator").unwrap();

        let (key1, is_comm1) = engine1.hash_node(&binop1);
        let (key2, is_comm2) = engine2.hash_node(&binop2);

        assert!(!is_comm1, "Sub is not commutative");
        assert!(!is_comm2, "Sub is not commutative");
        assert_ne!(key1, key2, "x - y and y - x should hash to different keys");
    }

    // =========================================================================
    // test_hash_node_call_unique
    // =========================================================================

    #[test]
    fn test_hash_node_call_unique() {
        let source = "foo()\nfoo()";
        let tree = parse(source, Language::Python).unwrap();
        let mut engine = GVNEngine::new(source);

        let root = tree.root_node();
        let mut calls = Vec::new();
        find_all_nodes_by_kind(root, "call", &mut calls);

        assert_eq!(calls.len(), 2, "Should find 2 call nodes");

        let (key1, _) = engine.hash_node(&calls[0]);
        let (key2, _) = engine.hash_node(&calls[1]);

        assert_ne!(
            key1, key2,
            "Each call should get a unique hash key (BC-GVN-4)"
        );

        // Both should be Call variants
        match (&key1, &key2) {
            (HashKey::Call { unique_id: id1 }, HashKey::Call { unique_id: id2 }) => {
                assert_ne!(id1, id2);
            }
            _ => panic!("Expected Call hash keys"),
        }
    }

    // =========================================================================
    // test_number_expression_basic
    // =========================================================================

    #[test]
    fn test_number_expression_basic() {
        let source = "x + y";
        let tree = parse(source, Language::Python).unwrap();
        let mut engine = GVNEngine::new(source);

        let binop = find_node_by_kind(tree.root_node(), "binary_operator").unwrap();

        let vn = engine.number_expression(&binop, 1);

        // First expression should get VN
        assert!(vn < 100, "Should get a reasonable VN");

        // Expression should be recorded
        assert_eq!(engine.expressions().len(), 1);
        assert_eq!(engine.expressions()[0].text, "x + y");
        assert_eq!(engine.expressions()[0].line, 1);
        assert_eq!(engine.expressions()[0].value_number, vn);
    }

    #[test]
    fn test_number_expression_same_expression_same_vn() {
        // Two identical expressions should get the same VN
        let source = "x + y\nx + y";
        let tree = parse(source, Language::Python).unwrap();
        let mut engine = GVNEngine::new(source);

        let mut binops = Vec::new();
        find_all_nodes_by_kind(tree.root_node(), "binary_operator", &mut binops);

        assert_eq!(binops.len(), 2, "Should find 2 binary operators");

        let vn1 = engine.number_expression(&binops[0], 1);
        let vn2 = engine.number_expression(&binops[1], 2);

        assert_eq!(vn1, vn2, "Identical expressions should get same VN");
    }

    // =========================================================================
    // test_alias_propagation
    // =========================================================================

    #[test]
    fn test_alias_propagation() {
        // If we assign a VN to a variable, using that variable should return the same VN
        let source = "x";
        let tree = parse(source, Language::Python).unwrap();
        let mut engine = GVNEngine::new(source);

        let id_node = find_node_by_kind(tree.root_node(), "identifier").unwrap();

        // Without assignment, x gets a Name hash key
        let (key_before, _) = engine.hash_node(&id_node);
        match &key_before {
            HashKey::Name { name } => assert_eq!(name, "x"),
            _ => panic!("Expected Name hash key before assignment"),
        }

        // Assign VN 42 to x
        engine.assign_variable("x", 42);

        // Now x should use VarVN
        let (key_after, _) = engine.hash_node(&id_node);
        match &key_after {
            HashKey::VarVN { vn } => assert_eq!(*vn, 42),
            _ => panic!(
                "Expected VarVN hash key after assignment, got {:?}",
                key_after
            ),
        }
    }

    #[test]
    fn test_alias_propagation_number_expression() {
        // Test that number_expression returns the same VN for aliased variables
        let source = "x";
        let tree = parse(source, Language::Python).unwrap();
        let mut engine = GVNEngine::new(source);

        let id_node = find_node_by_kind(tree.root_node(), "identifier").unwrap();

        // Assign VN 42 to x
        engine.assign_variable("x", 42);

        // number_expression should return 42
        let vn = engine.number_expression(&id_node, 1);
        assert_eq!(vn, 42, "Aliased variable should return assigned VN");
    }

    #[test]
    fn test_kill_variable() {
        let source = "x";
        let tree = parse(source, Language::Python).unwrap();
        let mut engine = GVNEngine::new(source);

        let id_node = find_node_by_kind(tree.root_node(), "identifier").unwrap();

        // Assign then kill
        engine.assign_variable("x", 42);
        engine.kill_variable("x");

        // Should now use Name hash key again
        let (key, _) = engine.hash_node(&id_node);
        match &key {
            HashKey::Name { name } => assert_eq!(name, "x"),
            _ => panic!("Expected Name hash key after kill"),
        }
    }
}
