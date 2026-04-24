//! CFG extraction from source code
//!
//! Extracts control flow graphs from functions using tree-sitter parsing.
//!
//! # Algorithm
//! 1. Parse source with tree-sitter
//! 2. Find function by name
//! 3. Build basic blocks (maximal sequences without branches)
//! 4. Connect blocks via edges based on control structures
//! 5. Compute cyclomatic complexity: E - N + 2
//!
//! # Block Boundaries (M7 documentation)
//! A new block starts at:
//! - Function entry
//! - Target of a branch (if/else/elif)
//! - Loop header (for/while)
//! - After a branch rejoins
//! - Exception handler entry (except/catch)
//! - Return statements

use std::collections::HashMap;
use std::path::Path;

use tree_sitter::{Node, Tree};

use crate::ast::function_finder::{find_function_node, get_function_body, get_function_name};
use crate::ast::parser::parse;
use crate::types::{BlockType, CfgBlock, CfgEdge, CfgInfo, EdgeType, Language};
use crate::TldrResult;

/// Maximum recursion depth for nested structures (M24 mitigation)
const MAX_NESTING_DEPTH: usize = 50;

/// Extract CFG for a function from source code or file path
///
/// # Arguments
/// * `source_or_path` - Either source code string or path to a file
/// * `function_name` - Name of the function to extract CFG for
/// * `language` - Programming language
///
/// # Returns
/// * `Ok(CfgInfo)` - CFG with blocks, edges, and metrics
/// * Empty CFG if function not found (per spec Section 2.3.1)
///
/// # Example
/// ```ignore
/// use tldr_core::cfg::get_cfg_context;
/// use tldr_core::Language;
///
/// let cfg = get_cfg_context("def foo(): pass", "foo", Language::Python)?;
/// assert_eq!(cfg.function, "foo");
/// ```
pub fn get_cfg_context(
    source_or_path: &str,
    function_name: &str,
    language: Language,
) -> TldrResult<CfgInfo> {
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

    // Extract CFG from the parsed tree
    extract_cfg_from_tree(&tree, &source, function_name, language)
}

/// Extract CFG from a parsed tree
fn extract_cfg_from_tree(
    tree: &Tree,
    source: &str,
    function_name: &str,
    language: Language,
) -> TldrResult<CfgInfo> {
    let root = tree.root_node();

    // Find the function node
    let func_node = find_function_node(root, function_name, language, source);

    match func_node {
        Some(node) => build_cfg_for_function(node, function_name, source, language, 0),
        None => {
            // Return empty CFG when function not found (per spec)
            Ok(CfgInfo {
                function: function_name.to_string(),
                blocks: Vec::new(),
                edges: Vec::new(),
                entry_block: 0,
                exit_blocks: Vec::new(),
                cyclomatic_complexity: 0,
                nested_functions: HashMap::new(),
            })
        }
    }
}

/// Check if a node kind represents a control flow construct that should be
/// processed as a statement rather than iterated over as a block container.
fn is_control_flow_node(kind: &str) -> bool {
    matches!(
        kind,
        "if_statement"
            | "if_expression"
            | "for_statement"
            | "for_in_statement"
            | "for_expression"
            | "while_statement"
            | "while_expression"
            | "loop_expression"
            | "try_statement"
            | "try_expression"
            | "match_expression"
            | "return_statement"
            | "return_expression"
            | "break_statement"
            | "break_expression"
            | "continue_statement"
            | "continue_expression"
    )
}

/// Find a child node by its kind (for languages like OCaml that use node types
/// instead of field names for certain children like then_clause, else_clause, do_clause)
fn find_child_by_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            if cursor.node().kind() == kind {
                return Some(cursor.node());
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

/// Build CFG for a function node
fn build_cfg_for_function(
    func_node: Node,
    function_name: &str,
    source: &str,
    language: Language,
    depth: usize,
) -> TldrResult<CfgInfo> {
    // M24: Depth limit to prevent infinite loops
    if depth > MAX_NESTING_DEPTH {
        return Ok(CfgInfo {
            function: function_name.to_string(),
            blocks: Vec::new(),
            edges: Vec::new(),
            entry_block: 0,
            exit_blocks: Vec::new(),
            cyclomatic_complexity: 0,
            nested_functions: HashMap::new(),
        });
    }

    let mut builder = CfgBuilder::new(function_name.to_string(), source, language);

    // Get the function body
    let body_node = get_function_body(func_node, language);

    if let Some(body) = body_node {
        builder.process_block(body, depth)?;
    }

    builder.finalize()
}

/// Builder for constructing CFG
struct CfgBuilder<'a> {
    function_name: String,
    source: &'a str,
    language: Language,
    blocks: Vec<CfgBlock>,
    edges: Vec<CfgEdge>,
    nested_functions: HashMap<String, CfgInfo>,
    current_block_id: usize,
    exit_blocks: Vec<usize>,
}

impl<'a> CfgBuilder<'a> {
    fn new(function_name: String, source: &'a str, language: Language) -> Self {
        // Create entry block
        let entry_block = CfgBlock {
            id: 0,
            block_type: BlockType::Entry,
            lines: (0, 0),
            calls: Vec::new(),
        };

        Self {
            function_name,
            source,
            language,
            blocks: vec![entry_block],
            edges: Vec::new(),
            nested_functions: HashMap::new(),
            current_block_id: 0,
            exit_blocks: Vec::new(),
        }
    }

    /// Create a new basic block
    fn new_block(&mut self, block_type: BlockType, start_line: u32, end_line: u32) -> usize {
        let id = self.blocks.len();
        self.blocks.push(CfgBlock {
            id,
            block_type,
            lines: (start_line, end_line),
            calls: Vec::new(),
        });
        id
    }

    /// Add an edge between blocks
    fn add_edge(&mut self, from: usize, to: usize, edge_type: EdgeType, condition: Option<String>) {
        self.edges.push(CfgEdge {
            from,
            to,
            edge_type,
            condition,
        });
    }

    /// Process a block of statements
    fn process_block(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        if depth > MAX_NESTING_DEPTH {
            return Ok(());
        }

        // Update entry block line range
        if self.blocks[0].lines == (0, 0) {
            self.blocks[0].lines = (
                node.start_position().row as u32 + 1,
                node.start_position().row as u32 + 1,
            );
        }

        // If the node itself is a control flow construct (common in expression-oriented
        // languages like OCaml where the function body IS the expression, not a block
        // containing statements), process it directly as a statement.
        if is_control_flow_node(node.kind()) {
            return self.process_statement(node, depth);
        }

        let mut cursor = node.walk();
        if cursor.goto_first_child() {
            loop {
                let child = cursor.node();
                self.process_statement(child, depth)?;
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }

        Ok(())
    }

    /// Process a single statement
    fn process_statement(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        let kind = node.kind();
        let start_line = node.start_position().row as u32 + 1;
        let end_line = node.end_position().row as u32 + 1;

        match kind {
            // Control flow statements
            "if_statement" | "if_expression" => self.process_if_statement(node, depth)?,
            "for_statement" | "for_in_statement" | "for_expression" => {
                self.process_for_loop(node, depth)?
            }
            "while_statement" | "while_expression" => self.process_while_loop(node, depth)?,
            "loop_expression" => self.process_loop_expression(node, depth)?,
            "try_statement" => self.process_try_statement(node, depth)?,
            // tree-sitter produces "try_expression" for BOTH OCaml try/with AND
            // Rust's ? operator. Distinguish by checking for "expression" field
            // (present in OCaml, absent in Rust ?).
            "try_expression" => {
                if node.child_by_field_name("expression").is_some() {
                    // OCaml: try <expression> with <match_case>...
                    self.process_try_statement(node, depth)?
                } else {
                    // Rust: <expr>? — hidden branch on Result/Option
                    self.process_question_mark(node, depth)?
                }
            }
            "match_expression" => self.process_match_expression(node, depth)?,
            // Rust uses _expression variants (return/break/continue are expressions)
            "return_statement" | "return_expression" => {
                self.process_return_statement(node, start_line, end_line)?
            }
            "break_statement" | "break_expression" => {
                self.process_break_statement(node, start_line, end_line)?
            }
            "continue_statement" | "continue_expression" => {
                self.process_continue_statement(node, start_line, end_line)?
            }

            // Nested function definitions
            "function_definition" | "function_declaration" | "arrow_function" => {
                self.process_nested_function(node, depth)?;
            }

            // Expression statements and let declarations — in expression-oriented
            // languages (Rust, OCaml) the child of an expression_statement or the
            // value of a let_declaration may itself be a control flow node (match,
            // if, try_expression/?, etc.). Scan immediate children to find them.
            "expression_statement" | "let_declaration" => {
                let mut found_cf = false;
                let mut cursor = node.walk();
                if cursor.goto_first_child() {
                    loop {
                        let child = cursor.node();
                        if is_control_flow_node(child.kind()) || child.kind() == "try_expression" {
                            self.process_statement(child, depth)?;
                            found_cf = true;
                            // Don't break — there may be multiple CF nodes
                            // e.g. `let x = a?; let y = b?;` in a let chain
                        }
                        if !cursor.goto_next_sibling() {
                            break;
                        }
                    }
                }
                if !found_cf {
                    self.process_expression(node, start_line, end_line)?;
                }
            }

            // Bare call expressions
            "call_expression" | "call" => {
                self.process_expression(node, start_line, end_line)?;
            }

            // Container expressions that may contain control flow (OCaml let-in bindings,
            // semicolon-separated sequences, value definitions, Rust blocks inside
            // match arms or other contexts) - recurse into children
            "sequence_expression" | "let_expression" | "value_definition" | "block" => {
                self.process_block(node, depth)?;
            }

            // Other statements - just update current block
            _ => {
                self.update_current_block_lines(start_line, end_line);
                // Check for function calls in the statement
                self.extract_calls_from_node(node);
            }
        }

        Ok(())
    }

    /// Process an if statement (handles both `if_statement` and OCaml `if_expression`)
    fn process_if_statement(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        let start_line = node.start_position().row as u32 + 1;
        let end_line = node.end_position().row as u32 + 1;

        // Get the condition
        let condition = node.child_by_field_name("condition").map(|n| {
            n.utf8_text(self.source.as_bytes())
                .unwrap_or("")
                .to_string()
        });

        // Create branch block
        let branch_block = self.new_block(BlockType::Branch, start_line, start_line);

        // Connect current block to branch
        self.add_edge(
            self.current_block_id,
            branch_block,
            EdgeType::Unconditional,
            None,
        );

        // Create blocks for then and else branches
        // Standard languages use "consequence"/"alternative" field names
        // OCaml uses then_clause/else_clause child node types (no field names)
        let consequence = node
            .child_by_field_name("consequence")
            .or_else(|| find_child_by_kind(node, "then_clause"));
        let alternative = node
            .child_by_field_name("alternative")
            .or_else(|| find_child_by_kind(node, "else_clause"));

        // Create join block for after the if
        let join_block = self.new_block(BlockType::Body, end_line, end_line);

        // Process then branch
        if let Some(then_node) = consequence {
            let then_start = then_node.start_position().row as u32 + 1;
            let then_end = then_node.end_position().row as u32 + 1;
            let then_block = self.new_block(BlockType::Body, then_start, then_end);

            self.add_edge(branch_block, then_block, EdgeType::True, condition.clone());

            self.current_block_id = then_block;
            self.process_block(then_node, depth + 1)?;

            // Connect to join (unless we returned)
            if !self.exit_blocks.contains(&self.current_block_id) {
                self.add_edge(
                    self.current_block_id,
                    join_block,
                    EdgeType::Unconditional,
                    None,
                );
            }
        }

        // Process else branch
        if let Some(else_node) = alternative {
            let else_start = else_node.start_position().row as u32 + 1;
            let else_end = else_node.end_position().row as u32 + 1;
            let else_block = self.new_block(BlockType::Body, else_start, else_end);

            self.add_edge(branch_block, else_block, EdgeType::False, None);

            self.current_block_id = else_block;

            // Handle elif by processing as nested if
            self.process_block(else_node, depth + 1)?;

            if !self.exit_blocks.contains(&self.current_block_id) {
                self.add_edge(
                    self.current_block_id,
                    join_block,
                    EdgeType::Unconditional,
                    None,
                );
            }
        } else {
            // No else - false edge goes directly to join
            self.add_edge(branch_block, join_block, EdgeType::False, None);
        }

        self.current_block_id = join_block;
        Ok(())
    }

    /// Process a for loop (handles `for_statement`, `for_in_statement`, OCaml `for_expression`)
    fn process_for_loop(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        let start_line = node.start_position().row as u32 + 1;
        let end_line = node.end_position().row as u32 + 1;

        // Create loop header
        let header_block = self.new_block(BlockType::LoopHeader, start_line, start_line);

        // Connect current to header
        self.add_edge(
            self.current_block_id,
            header_block,
            EdgeType::Unconditional,
            None,
        );

        // Create loop body block
        // Standard languages use "body" field; OCaml uses do_clause child
        let body = node
            .child_by_field_name("body")
            .or_else(|| find_child_by_kind(node, "do_clause"));
        let exit_block = self.new_block(BlockType::Body, end_line, end_line);

        if let Some(body_node) = body {
            let body_start = body_node.start_position().row as u32 + 1;
            let body_end = body_node.end_position().row as u32 + 1;
            let body_block = self.new_block(BlockType::LoopBody, body_start, body_end);

            // True edge: enter loop
            self.add_edge(header_block, body_block, EdgeType::True, None);

            // False edge: exit loop
            self.add_edge(header_block, exit_block, EdgeType::False, None);

            self.current_block_id = body_block;
            self.process_block(body_node, depth + 1)?;

            // Back edge to header
            if !self.exit_blocks.contains(&self.current_block_id) {
                self.add_edge(
                    self.current_block_id,
                    header_block,
                    EdgeType::BackEdge,
                    None,
                );
            }
        } else {
            self.add_edge(header_block, exit_block, EdgeType::False, None);
        }

        self.current_block_id = exit_block;
        Ok(())
    }

    /// Process a while loop (handles `while_statement` and OCaml `while_expression`)
    fn process_while_loop(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        let start_line = node.start_position().row as u32 + 1;
        let end_line = node.end_position().row as u32 + 1;

        // Get condition
        let condition = node.child_by_field_name("condition").map(|n| {
            n.utf8_text(self.source.as_bytes())
                .unwrap_or("")
                .to_string()
        });

        // Create loop header
        let header_block = self.new_block(BlockType::LoopHeader, start_line, start_line);

        // Connect current to header
        self.add_edge(
            self.current_block_id,
            header_block,
            EdgeType::Unconditional,
            None,
        );

        // Create exit block
        let exit_block = self.new_block(BlockType::Body, end_line, end_line);

        // Process body
        // Standard languages use "body" field; OCaml uses do_clause child
        let body = node
            .child_by_field_name("body")
            .or_else(|| find_child_by_kind(node, "do_clause"));
        if let Some(body_node) = body {
            let body_start = body_node.start_position().row as u32 + 1;
            let body_end = body_node.end_position().row as u32 + 1;
            let body_block = self.new_block(BlockType::LoopBody, body_start, body_end);

            self.add_edge(header_block, body_block, EdgeType::True, condition);
            self.add_edge(header_block, exit_block, EdgeType::False, None);

            self.current_block_id = body_block;
            self.process_block(body_node, depth + 1)?;

            // Back edge
            if !self.exit_blocks.contains(&self.current_block_id) {
                self.add_edge(
                    self.current_block_id,
                    header_block,
                    EdgeType::BackEdge,
                    None,
                );
            }
        } else {
            self.add_edge(header_block, exit_block, EdgeType::False, None);
        }

        self.current_block_id = exit_block;
        Ok(())
    }

    /// Process Rust `loop { ... }` (infinite loop, exits only via break)
    fn process_loop_expression(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        let start_line = node.start_position().row as u32 + 1;
        let end_line = node.end_position().row as u32 + 1;

        // Create loop header
        let header_block = self.new_block(BlockType::LoopHeader, start_line, start_line);

        // Connect current block to header
        self.add_edge(
            self.current_block_id,
            header_block,
            EdgeType::Unconditional,
            None,
        );

        // Create exit block (reached via break)
        let exit_block = self.new_block(BlockType::Body, end_line, end_line);

        // Process body
        if let Some(body_node) = node.child_by_field_name("body") {
            let body_start = body_node.start_position().row as u32 + 1;
            let body_end = body_node.end_position().row as u32 + 1;
            let body_block = self.new_block(BlockType::LoopBody, body_start, body_end);

            // Unconditional entry into body (infinite loop — no condition)
            self.add_edge(header_block, body_block, EdgeType::True, None);

            self.current_block_id = body_block;
            self.process_block(body_node, depth + 1)?;

            // Back edge to header
            if !self.exit_blocks.contains(&self.current_block_id) {
                self.add_edge(
                    self.current_block_id,
                    header_block,
                    EdgeType::BackEdge,
                    None,
                );
            }
        }

        // The exit block is reached only via break statements inside the loop body.
        // Connect header→exit as False edge so the exit block is reachable in the CFG
        // even when break statements are not explicitly modeled as edges to exit_block.
        self.add_edge(header_block, exit_block, EdgeType::False, None);

        self.current_block_id = exit_block;
        Ok(())
    }

    /// Process Rust `?` operator (try_expression).
    ///
    /// The `?` creates a hidden branch in the control flow:
    /// - Ok(val)/Some(val) → unwrap and continue to next statement
    /// - Err(e)/None → early return from the function
    ///
    /// This is semantically equivalent to:
    /// ```ignore
    /// match expr {
    ///     Ok(val) => val,      // True edge → continue
    ///     Err(e) => return Err(e), // False edge → exit
    /// }
    /// ```
    fn process_question_mark(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        if depth > MAX_NESTING_DEPTH {
            return Ok(());
        }

        let start_line = node.start_position().row as u32 + 1;
        let end_line = node.end_position().row as u32 + 1;

        // Process the inner expression (the part before ?) which may contain
        // nested ? operators or function calls
        let mut cursor = node.walk();
        if cursor.goto_first_child() {
            loop {
                let child = cursor.node();
                if child.kind() == "try_expression" {
                    // Nested ? — recurse
                    self.process_question_mark(child, depth + 1)?;
                } else if child.kind() != "?" {
                    // Process inner expression for calls etc.
                    self.extract_calls_from_node(child);
                }
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }

        // Create branch block for the ? check
        let branch_block = self.new_block(BlockType::Branch, start_line, end_line);
        self.add_edge(
            self.current_block_id,
            branch_block,
            EdgeType::Unconditional,
            None,
        );

        // True edge: Ok/Some → continue to next statement
        let ok_block = self.new_block(BlockType::Body, start_line, end_line);
        self.add_edge(
            branch_block,
            ok_block,
            EdgeType::True,
            Some("Ok/Some".to_string()),
        );

        // False edge: Err/None → early return
        let err_block = self.new_block(BlockType::Return, start_line, end_line);
        self.add_edge(
            branch_block,
            err_block,
            EdgeType::False,
            Some("Err/None".to_string()),
        );

        // Err path exits the function
        let exit_block = self.new_block(BlockType::Exit, end_line, end_line);
        self.add_edge(err_block, exit_block, EdgeType::Unconditional, None);
        self.exit_blocks.push(exit_block);

        // Continue on the Ok path
        self.current_block_id = ok_block;
        Ok(())
    }

    /// Process try/except statement (handles `try_statement` and OCaml `try_expression`)
    fn process_try_statement(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        let start_line = node.start_position().row as u32 + 1;
        let end_line = node.end_position().row as u32 + 1;

        // Create try block
        let try_block = self.new_block(BlockType::Body, start_line, start_line);
        self.add_edge(
            self.current_block_id,
            try_block,
            EdgeType::Unconditional,
            None,
        );

        // Create exit block
        let exit_block = self.new_block(BlockType::Body, end_line, end_line);

        // For OCaml try_expression, the body is the "expression" field
        // and exception handlers are match_case children
        if let Some(expr_body) = node.child_by_field_name("expression") {
            // OCaml-style: try <expression> with <match_case>...
            self.current_block_id = try_block;
            self.process_statement(expr_body, depth + 1)?;
            if !self.exit_blocks.contains(&self.current_block_id) {
                self.add_edge(
                    self.current_block_id,
                    exit_block,
                    EdgeType::Unconditional,
                    None,
                );
            }

            // Process match_case children as exception handlers
            let mut cursor = node.walk();
            if cursor.goto_first_child() {
                loop {
                    let child = cursor.node();
                    if child.kind() == "match_case" {
                        let except_start = child.start_position().row as u32 + 1;
                        let except_end = child.end_position().row as u32 + 1;
                        let except_block =
                            self.new_block(BlockType::Body, except_start, except_end);

                        self.add_edge(
                            try_block,
                            except_block,
                            EdgeType::Unconditional,
                            Some("exception".to_string()),
                        );

                        self.current_block_id = except_block;
                        self.process_block(child, depth + 1)?;

                        if !self.exit_blocks.contains(&self.current_block_id) {
                            self.add_edge(
                                self.current_block_id,
                                exit_block,
                                EdgeType::Unconditional,
                                None,
                            );
                        }
                    }
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
            }
        } else {
            // Standard try/except/catch/finally pattern
            let mut cursor = node.walk();
            if cursor.goto_first_child() {
                loop {
                    let child = cursor.node();
                    match child.kind() {
                        "block" => {
                            // Try block body
                            self.current_block_id = try_block;
                            self.process_block(child, depth + 1)?;
                            if !self.exit_blocks.contains(&self.current_block_id) {
                                self.add_edge(
                                    self.current_block_id,
                                    exit_block,
                                    EdgeType::Unconditional,
                                    None,
                                );
                            }
                        }
                        "except_clause" | "catch_clause" => {
                            let except_start = child.start_position().row as u32 + 1;
                            let except_end = child.end_position().row as u32 + 1;
                            let except_block =
                                self.new_block(BlockType::Body, except_start, except_end);

                            // Exception edge from try block
                            self.add_edge(
                                try_block,
                                except_block,
                                EdgeType::Unconditional,
                                Some("exception".to_string()),
                            );

                            self.current_block_id = except_block;
                            self.process_block(child, depth + 1)?;

                            if !self.exit_blocks.contains(&self.current_block_id) {
                                self.add_edge(
                                    self.current_block_id,
                                    exit_block,
                                    EdgeType::Unconditional,
                                    None,
                                );
                            }
                        }
                        "finally_clause" => {
                            let finally_start = child.start_position().row as u32 + 1;
                            let finally_end = child.end_position().row as u32 + 1;
                            let finally_block =
                                self.new_block(BlockType::Body, finally_start, finally_end);

                            // Finally is always executed
                            self.add_edge(exit_block, finally_block, EdgeType::Unconditional, None);

                            self.current_block_id = finally_block;
                            self.process_block(child, depth + 1)?;

                            // Update exit block to be after finally
                            let new_exit =
                                self.new_block(BlockType::Body, finally_end, finally_end);
                            if !self.exit_blocks.contains(&self.current_block_id) {
                                self.add_edge(
                                    self.current_block_id,
                                    new_exit,
                                    EdgeType::Unconditional,
                                    None,
                                );
                            }
                            self.current_block_id = new_exit;
                            return Ok(());
                        }
                        _ => {}
                    }
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
            }
        }

        self.current_block_id = exit_block;
        Ok(())
    }

    /// Process a match/switch expression (OCaml `match_expression`, Rust `match_expression`)
    fn process_match_expression(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        let start_line = node.start_position().row as u32 + 1;
        let end_line = node.end_position().row as u32 + 1;

        // Get the scrutinee expression.
        // OCaml uses "expression" field; Rust uses "value" field.
        let scrutinee = node
            .child_by_field_name("expression")
            .or_else(|| node.child_by_field_name("value"))
            .map(|n| {
                n.utf8_text(self.source.as_bytes())
                    .unwrap_or("")
                    .to_string()
            });

        // Create branch block for the match
        let branch_block = self.new_block(BlockType::Branch, start_line, start_line);

        // Connect current block to branch
        self.add_edge(
            self.current_block_id,
            branch_block,
            EdgeType::Unconditional,
            None,
        );

        // Create join block for after the match
        let join_block = self.new_block(BlockType::Body, end_line, end_line);

        // Find the container of match arms/cases.
        // OCaml: match_case children are direct children of match_expression.
        // Rust: match_arm children are inside a match_block child (via "body" field).
        let arms_parent = node.child_by_field_name("body").unwrap_or(node);

        // Process each match_case/match_arm child as a separate branch
        let mut cursor = arms_parent.walk();
        let mut case_count = 0;
        if cursor.goto_first_child() {
            loop {
                let child = cursor.node();
                if child.kind() == "match_case" || child.kind() == "match_arm" {
                    let case_start = child.start_position().row as u32 + 1;
                    let case_end = child.end_position().row as u32 + 1;
                    let case_block = self.new_block(BlockType::Body, case_start, case_end);

                    let edge_type = if case_count == 0 {
                        EdgeType::True
                    } else {
                        EdgeType::False
                    };
                    self.add_edge(branch_block, case_block, edge_type, scrutinee.clone());

                    self.current_block_id = case_block;
                    self.process_block(child, depth + 1)?;

                    if !self.exit_blocks.contains(&self.current_block_id) {
                        self.add_edge(
                            self.current_block_id,
                            join_block,
                            EdgeType::Unconditional,
                            None,
                        );
                    }
                    case_count += 1;
                }
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }

        // If no cases were found, connect branch directly to join
        if case_count == 0 {
            self.add_edge(branch_block, join_block, EdgeType::Unconditional, None);
        }

        self.current_block_id = join_block;
        Ok(())
    }

    /// Process return statement
    fn process_return_statement(
        &mut self,
        node: Node,
        start_line: u32,
        end_line: u32,
    ) -> TldrResult<()> {
        // Create return block
        let return_block = self.new_block(BlockType::Return, start_line, end_line);

        self.add_edge(
            self.current_block_id,
            return_block,
            EdgeType::Unconditional,
            None,
        );

        // Check for function calls in return expression
        self.extract_calls_from_node(node);

        // Create exit block
        let exit_block = self.new_block(BlockType::Exit, end_line, end_line);
        self.add_edge(return_block, exit_block, EdgeType::Unconditional, None);

        self.exit_blocks.push(exit_block);
        self.current_block_id = return_block;

        Ok(())
    }

    /// Process break statement
    fn process_break_statement(
        &mut self,
        _node: Node,
        start_line: u32,
        end_line: u32,
    ) -> TldrResult<()> {
        let break_block = self.new_block(BlockType::Body, start_line, end_line);

        self.add_edge(self.current_block_id, break_block, EdgeType::Break, None);

        self.current_block_id = break_block;
        Ok(())
    }

    /// Process continue statement
    fn process_continue_statement(
        &mut self,
        _node: Node,
        start_line: u32,
        end_line: u32,
    ) -> TldrResult<()> {
        let continue_block = self.new_block(BlockType::Body, start_line, end_line);

        self.add_edge(
            self.current_block_id,
            continue_block,
            EdgeType::Continue,
            None,
        );

        self.current_block_id = continue_block;
        Ok(())
    }

    /// Process nested function definition
    fn process_nested_function(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        if let Some(name) = get_function_name(node, self.language, self.source) {
            let nested_cfg =
                build_cfg_for_function(node, &name, self.source, self.language, depth + 1)?;
            self.nested_functions.insert(name, nested_cfg);
        }
        Ok(())
    }

    /// Process expression statement
    fn process_expression(&mut self, node: Node, start_line: u32, end_line: u32) -> TldrResult<()> {
        self.update_current_block_lines(start_line, end_line);
        self.extract_calls_from_node(node);
        Ok(())
    }

    /// Update current block line range
    fn update_current_block_lines(&mut self, start_line: u32, end_line: u32) {
        if let Some(block) = self.blocks.get_mut(self.current_block_id) {
            if block.lines.0 == 0 {
                block.lines.0 = start_line;
            }
            block.lines.1 = end_line.max(block.lines.1);
        }
    }

    /// Extract function calls from a node
    fn extract_calls_from_node(&mut self, node: Node) {
        let mut cursor = node.walk();
        let mut stack = vec![node];

        while let Some(current) = stack.pop() {
            // Check if this is a function call
            if current.kind() == "call" || current.kind() == "call_expression" {
                if let Some(callee) = self.get_callee_name(current) {
                    if let Some(block) = self.blocks.get_mut(self.current_block_id) {
                        if !block.calls.contains(&callee) {
                            block.calls.push(callee);
                        }
                    }
                }
            }

            // Add children
            cursor.reset(current);
            if cursor.goto_first_child() {
                loop {
                    stack.push(cursor.node());
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
    }

    /// Get the name of the function being called
    fn get_callee_name(&self, call_node: Node) -> Option<String> {
        let func_node = call_node
            .child_by_field_name("function")
            .or_else(|| call_node.child(0))?;

        match func_node.kind() {
            "identifier" => Some(
                func_node
                    .utf8_text(self.source.as_bytes())
                    .ok()?
                    .to_string(),
            ),
            "attribute" | "member_expression" => {
                // Get the attribute/property name
                func_node
                    .child_by_field_name("attribute")
                    .or_else(|| func_node.child_by_field_name("property"))
                    .and_then(|n| {
                        n.utf8_text(self.source.as_bytes())
                            .ok()
                            .map(|s| s.to_string())
                    })
            }
            _ => Some(
                func_node
                    .utf8_text(self.source.as_bytes())
                    .ok()?
                    .to_string(),
            ),
        }
    }

    /// Finalize the CFG and compute metrics
    fn finalize(mut self) -> TldrResult<CfgInfo> {
        // Ensure we have at least an entry and exit
        if self.blocks.is_empty() {
            self.blocks.push(CfgBlock {
                id: 0,
                block_type: BlockType::Entry,
                lines: (1, 1),
                calls: Vec::new(),
            });
        }

        // If no explicit exit blocks, the last block is an exit
        if self.exit_blocks.is_empty() && !self.blocks.is_empty() {
            let last_id = self.blocks.len() - 1;
            self.exit_blocks.push(last_id);

            // Add exit block if not present
            if self.blocks[last_id].block_type != BlockType::Exit {
                let exit_block = self.new_block(
                    BlockType::Exit,
                    self.blocks[last_id].lines.1,
                    self.blocks[last_id].lines.1,
                );
                self.add_edge(last_id, exit_block, EdgeType::Unconditional, None);
                self.exit_blocks = vec![exit_block];
            }
        } else if !self.exit_blocks.is_empty() {
            // There are explicit exit blocks (from return/break/? etc.) but the
            // normal fall-through path may also need an exit. If the current block
            // isn't already an exit block, connect it to a new exit.
            let current = self.current_block_id;
            if !self.exit_blocks.contains(&current)
                && self.blocks.get(current).is_some_and(|b| {
                    b.block_type != BlockType::Exit && b.block_type != BlockType::Return
                })
            {
                let exit_block = self.new_block(
                    BlockType::Exit,
                    self.blocks[current].lines.1,
                    self.blocks[current].lines.1,
                );
                self.add_edge(current, exit_block, EdgeType::Unconditional, None);
                self.exit_blocks.push(exit_block);
            }
        }

        // Calculate cyclomatic complexity using two methods, take the max:
        // 1. Edge formula: E - N + 2P (P = connected components, usually 1)
        //    This can undercount when multiple exit nodes inflate N without
        //    proportional edges (e.g., Rust ? operator creates separate exits).
        // 2. Decision count: number of branch/loop-header nodes + 1
        //    This is McCabe's original definition and handles all cases correctly.
        let e = self.edges.len() as u32;
        let n = self.blocks.len() as u32;
        let edge_formula = if n > 0 { e.saturating_sub(n) + 2 } else { 1 };
        let decision_points = self
            .blocks
            .iter()
            .filter(|b| matches!(b.block_type, BlockType::Branch | BlockType::LoopHeader))
            .count() as u32;
        let cyclomatic = edge_formula.max(decision_points + 1);

        Ok(CfgInfo {
            function: self.function_name,
            blocks: self.blocks,
            edges: self.edges,
            entry_block: 0,
            exit_blocks: self.exit_blocks,
            cyclomatic_complexity: cyclomatic,
            nested_functions: self.nested_functions,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_function() {
        let source = r#"
def simple():
    x = 1
    return x
"#;
        let cfg = get_cfg_context(source, "simple", Language::Python).unwrap();
        assert_eq!(cfg.function, "simple");
        assert!(!cfg.blocks.is_empty());
        assert!(!cfg.exit_blocks.is_empty());
    }

    #[test]
    fn test_if_statement() {
        let source = r#"
def with_if(x):
    if x > 0:
        return 1
    else:
        return -1
"#;
        let cfg = get_cfg_context(source, "with_if", Language::Python).unwrap();
        assert!(cfg.cyclomatic_complexity >= 2);

        // Should have true and false edges
        let has_true = cfg.edges.iter().any(|e| e.edge_type == EdgeType::True);
        let has_false = cfg.edges.iter().any(|e| e.edge_type == EdgeType::False);
        assert!(has_true);
        assert!(has_false);
    }

    #[test]
    fn test_for_loop() {
        let source = r#"
def with_loop():
    total = 0
    for i in range(10):
        total += i
    return total
"#;
        let cfg = get_cfg_context(source, "with_loop", Language::Python).unwrap();

        // Should have loop header
        let has_loop = cfg
            .blocks
            .iter()
            .any(|b| b.block_type == BlockType::LoopHeader);
        assert!(has_loop);

        // Should have back edge
        let has_back = cfg.edges.iter().any(|e| e.edge_type == EdgeType::BackEdge);
        assert!(has_back);
    }

    #[test]
    fn test_function_not_found() {
        let source = "def foo(): pass";
        let cfg = get_cfg_context(source, "nonexistent", Language::Python).unwrap();
        assert!(cfg.blocks.is_empty());
        assert_eq!(cfg.cyclomatic_complexity, 0);
    }

    #[test]
    fn test_ocaml_if_expression() {
        let source = r#"
let compute x =
  if x > 0 then
    x + 1
  else
    x - 1
"#;
        let cfg = get_cfg_context(source, "compute", Language::Ocaml).unwrap();
        // OCaml if_expression should create branch blocks with true/false edges
        assert!(
            cfg.blocks.len() > 2,
            "OCaml if should create multiple blocks: got {}",
            cfg.blocks.len()
        );
        assert!(
            cfg.cyclomatic_complexity >= 2,
            "OCaml if should increase cyclomatic complexity: got {}",
            cfg.cyclomatic_complexity
        );
    }

    #[test]
    fn test_ocaml_match_expression() {
        let source = r#"
let classify x =
  match x with
  | 0 -> "zero"
  | _ -> "other"
"#;
        let cfg = get_cfg_context(source, "classify", Language::Ocaml).unwrap();
        // match_expression should create branch blocks
        assert!(
            cfg.blocks.len() > 2,
            "OCaml match should create multiple blocks: got {}",
            cfg.blocks.len()
        );
    }

    #[test]
    fn test_ocaml_while_expression() {
        let source = r#"
let loop_while x =
  let i = ref 0 in
  while !i < x do
    i := !i + 1
  done
"#;
        let cfg = get_cfg_context(source, "loop_while", Language::Ocaml).unwrap();
        // while_expression should create loop header with back edge
        let has_loop = cfg
            .blocks
            .iter()
            .any(|b| b.block_type == BlockType::LoopHeader);
        assert!(has_loop, "OCaml while should create a loop header block");
    }

    #[test]
    fn test_ocaml_for_expression() {
        let source = r#"
let loop_for n =
  for i = 1 to n do
    print_int i
  done
"#;
        let cfg = get_cfg_context(source, "loop_for", Language::Ocaml).unwrap();
        // for_expression should create loop header with back edge
        let has_loop = cfg
            .blocks
            .iter()
            .any(|b| b.block_type == BlockType::LoopHeader);
        assert!(has_loop, "OCaml for should create a loop header block");
    }

    #[test]
    fn test_ocaml_try_expression() {
        let source = r#"
let safe_read filename =
  try
    let chan = open_in filename in
    close_in chan
  with End_of_file ->
    ()
"#;
        let cfg = get_cfg_context(source, "safe_read", Language::Ocaml).unwrap();
        // try_expression should create multiple blocks for try body and exception handler
        assert!(
            cfg.blocks.len() > 2,
            "OCaml try should create multiple blocks: got {}",
            cfg.blocks.len()
        );
    }

    #[test]
    fn test_function_calls_tracked() {
        let source = r#"
def caller():
    foo()
    bar()
    return baz()
"#;
        let cfg = get_cfg_context(source, "caller", Language::Python).unwrap();

        // Collect all calls from all blocks
        let all_calls: Vec<&String> = cfg.blocks.iter().flat_map(|b| b.calls.iter()).collect();

        assert!(all_calls.contains(&&"foo".to_string()));
        assert!(all_calls.contains(&&"bar".to_string()));
        assert!(all_calls.contains(&&"baz".to_string()));
    }

    // =========================================================================
    // Rust-specific CFG tests (Phase 0)
    // =========================================================================

    #[test]
    fn test_rust_match_expression() {
        let source = r#"
fn classify(x: i32) -> &'static str {
    match x {
        0 => "zero",
        1 => "one",
        2 => "two",
        _ => "other",
    }
}
"#;
        let cfg = get_cfg_context(source, "classify", Language::Rust).unwrap();
        // 4 match arms → branch + 4 arm blocks + join = at least 6 blocks
        // (plus entry block)
        assert!(
            cfg.blocks.len() >= 6,
            "Rust match with 4 arms should produce >= 6 blocks, got {}",
            cfg.blocks.len()
        );
        // Should have true/false edges from branch to arms
        let has_true = cfg.edges.iter().any(|e| e.edge_type == EdgeType::True);
        let has_false = cfg.edges.iter().any(|e| e.edge_type == EdgeType::False);
        assert!(has_true, "match should have True edge to first arm");
        assert!(has_false, "match should have False edges to other arms");
    }

    #[test]
    fn test_rust_if_let_expression() {
        let source = r#"
fn maybe_inc(val: Option<i32>) -> i32 {
    if let Some(x) = val {
        x + 1
    } else {
        0
    }
}
"#;
        let cfg = get_cfg_context(source, "maybe_inc", Language::Rust).unwrap();
        // if-let with else should create branch + then + else + join blocks
        assert!(
            cfg.blocks.len() > 2,
            "Rust if-let should create multiple blocks, got {}",
            cfg.blocks.len()
        );
        assert!(
            cfg.cyclomatic_complexity >= 2,
            "Rust if-let should increase cyclomatic complexity, got {}",
            cfg.cyclomatic_complexity
        );
        let has_true = cfg.edges.iter().any(|e| e.edge_type == EdgeType::True);
        let has_false = cfg.edges.iter().any(|e| e.edge_type == EdgeType::False);
        assert!(has_true, "if-let should have True edge");
        assert!(has_false, "if-let should have False edge");
    }

    #[test]
    fn test_rust_while_let_expression() {
        let source = r#"
fn drain_sum(items: &mut Vec<Option<i32>>) -> i32 {
    let mut sum = 0;
    while let Some(Some(x)) = items.pop() {
        sum += x;
    }
    sum
}
"#;
        let cfg = get_cfg_context(source, "drain_sum", Language::Rust).unwrap();
        // while-let should create loop header with back edge
        let has_loop = cfg
            .blocks
            .iter()
            .any(|b| b.block_type == BlockType::LoopHeader);
        assert!(has_loop, "Rust while-let should create a loop header block");
        let has_back = cfg.edges.iter().any(|e| e.edge_type == EdgeType::BackEdge);
        assert!(has_back, "Rust while-let should have a back edge");
    }

    #[test]
    fn test_rust_loop_expression() {
        let source = r#"
fn count_up() -> i32 {
    let mut i = 0;
    loop {
        i += 1;
        if i > 10 {
            break;
        }
    }
    i
}
"#;
        let cfg = get_cfg_context(source, "count_up", Language::Rust).unwrap();
        // loop {} should create a loop header with a back edge
        let has_loop = cfg
            .blocks
            .iter()
            .any(|b| b.block_type == BlockType::LoopHeader);
        assert!(has_loop, "Rust loop should create a loop header block");
        let has_back = cfg.edges.iter().any(|e| e.edge_type == EdgeType::BackEdge);
        assert!(has_back, "Rust loop should have a back edge");
    }

    /// Rust `?` operator (try_expression) should create a branch in the CFG:
    /// success → continue, error → early return.
    #[test]
    fn test_rust_try_expression() {
        let source = r#"
fn parse_add(a: &str, b: &str) -> Result<i32, std::num::ParseIntError> {
    let x = a.parse::<i32>()?;
    let y = b.parse::<i32>()?;
    Ok(x + y)
}
"#;
        let cfg = get_cfg_context(source, "parse_add", Language::Rust).unwrap();
        // Each ? creates a branch (ok → continue, err → return)
        // With 2 ? operators we should have at least 2 additional branch points
        assert!(
            cfg.cyclomatic_complexity >= 3,
            "Two ? operators should give complexity >= 3, got {}",
            cfg.cyclomatic_complexity
        );
        // Should have true/false edges from the ? branches
        let true_edges = cfg
            .edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::True)
            .count();
        assert!(
            true_edges >= 2,
            "Two ? operators should produce >= 2 True edges, got {}",
            true_edges
        );
    }

    /// Rust ? operator in a chain: `foo()?.bar()?` should create 2 branches
    #[test]
    fn test_rust_try_expression_chained() {
        let source = r#"
fn chained(s: &str) -> Result<String, Box<dyn std::error::Error>> {
    let val = s.parse::<i32>()?.to_string();
    Ok(val)
}
"#;
        let cfg = get_cfg_context(source, "chained", Language::Rust).unwrap();
        assert!(
            cfg.cyclomatic_complexity >= 2,
            "Chained ? should give complexity >= 2, got {}",
            cfg.cyclomatic_complexity
        );
    }

    /// Rust if-let should extract let_condition as the condition text
    #[test]
    fn test_rust_if_let_condition_extraction() {
        let source = r#"
fn check(val: Option<i32>) -> i32 {
    if let Some(x) = val {
        x + 1
    } else {
        0
    }
}
"#;
        let cfg = get_cfg_context(source, "check", Language::Rust).unwrap();
        // Verify branch block has condition info from the let_condition
        let branch_edges_with_condition: Vec<_> =
            cfg.edges.iter().filter(|e| e.condition.is_some()).collect();
        assert!(
            !branch_edges_with_condition.is_empty(),
            "if-let should have edges with condition info, edges: {:?}",
            cfg.edges
        );
    }

    /// Match with nested control flow inside arms should track inner blocks
    #[test]
    fn test_rust_match_with_nested_control_flow() {
        let source = r#"
fn nested_match(x: i32) -> i32 {
    match x {
        0 => {
            if x == 0 {
                return 42;
            }
            0
        }
        _ => x,
    }
}
"#;
        let cfg = get_cfg_context(source, "nested_match", Language::Rust).unwrap();
        // match (1 branch) + if inside arm (1 branch) = complexity >= 3
        assert!(
            cfg.cyclomatic_complexity >= 3,
            "match with nested if should have complexity >= 3, got {}",
            cfg.cyclomatic_complexity
        );
        // Should have return block from the inner `return 42`
        let has_return = cfg.blocks.iter().any(|b| b.block_type == BlockType::Return);
        assert!(
            has_return,
            "nested return inside match arm should create Return block"
        );
    }

    #[test]
    fn test_rust_for_expression() {
        let source = r#"
fn sum_items(items: &[i32]) -> i32 {
    let mut sum = 0;
    for x in items {
        sum += x;
    }
    sum
}
"#;
        let cfg = get_cfg_context(source, "sum_items", Language::Rust).unwrap();
        // for-in should create loop header with back edge
        let has_loop = cfg
            .blocks
            .iter()
            .any(|b| b.block_type == BlockType::LoopHeader);
        assert!(has_loop, "Rust for should create a loop header block");
        let has_back = cfg.edges.iter().any(|e| e.edge_type == EdgeType::BackEdge);
        assert!(has_back, "Rust for should have a back edge");
    }
}
