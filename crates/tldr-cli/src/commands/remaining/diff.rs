//! Diff command - AST-aware structural diff
//!
//! Compares two source files at the AST level, detecting:
//! - Insert: new function/class/method
//! - Delete: removed function/class/method
//! - Update: modified body
//! - Move: same content, different location
//! - Rename: same body, different name
//!
//! # Example
//!
//! ```bash
//! tldr diff old.py new.py
//! tldr diff old.py new.py --semantic-only
//! tldr diff old.py new.py --format text
//! ```

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use clap::Args;
use regex::Regex;
use tree_sitter::Node;

use tldr_core::ast::function_finder::{get_function_name, get_function_node_kinds};
use tldr_core::ast::parser::ParserPool;
use tldr_core::callgraph::languages::LanguageRegistry;
use tldr_core::types::Language;

use super::error::RemainingError;
use super::types::{
    ASTChange, ArchChangeType, ArchDiffSummary, ArchLevelChange, BaseChanges, ChangeType,
    DiffGranularity, DiffReport, DiffSummary, FileLevelChange, ImportEdge, ImportGraphSummary,
    Location, ModuleLevelChange, NodeKind,
};
use crate::output::OutputFormat;

// =============================================================================
// Constants
// =============================================================================

/// Similarity threshold for detecting renames (0.0-1.0)
const RENAME_SIMILARITY_THRESHOLD: f64 = 0.8;

// =============================================================================
// CLI Arguments
// =============================================================================

/// AST-aware structural diff between two files
///
/// Compares two source files at the AST level, detecting structural changes
/// like inserted, deleted, updated, moved, and renamed functions/classes.
///
/// # Example
///
/// ```bash
/// tldr diff old.py new.py
/// tldr diff old.py new.py --semantic-only
/// ```
#[derive(Debug, Args)]
pub struct DiffArgs {
    /// First file (or directory for L6/L7/L8) to compare
    pub file_a: PathBuf,

    /// Second file (or directory for L6/L7/L8) to compare
    pub file_b: PathBuf,

    /// Diff granularity level
    #[arg(long, short = 'g', default_value = "function")]
    pub granularity: DiffGranularity,

    /// Exclude formatting-only changes (comments, whitespace)
    #[arg(long)]
    pub semantic_only: bool,

    /// Output file (optional, stdout if not specified)
    #[arg(long, short = 'O')]
    pub output: Option<PathBuf>,
}

// =============================================================================
// Extracted Function Info
// =============================================================================

/// Information about an extracted function/class/method
#[derive(Debug, Clone)]
struct ExtractedNode {
    /// Name of the function/class
    name: String,
    /// Kind of node
    kind: NodeKind,
    /// Line number (1-indexed)
    line: u32,
    /// End line number (1-indexed)
    end_line: u32,
    /// Column
    column: u32,
    /// Full source text (body)
    body: String,
    /// Normalized body (whitespace-insensitive)
    normalized_body: String,
    /// Parameters (for functions)
    params: String,
    /// Whether this is a method (inside a class)
    is_method: bool,
}

impl ExtractedNode {
    fn new(
        name: impl Into<String>,
        kind: NodeKind,
        line: u32,
        end_line: u32,
        column: u32,
        body: impl Into<String>,
    ) -> Self {
        let body_str: String = body.into();
        let normalized = normalize_body(&body_str);
        Self {
            name: name.into(),
            kind,
            line,
            end_line,
            column,
            body: body_str,
            normalized_body: normalized,
            params: String::new(),
            is_method: false,
        }
    }

    fn with_params(mut self, params: impl Into<String>) -> Self {
        self.params = params.into();
        self
    }

    fn with_method_kind(mut self) -> Self {
        self.is_method = true;
        if self.kind == NodeKind::Function {
            self.kind = NodeKind::Method;
        }
        self
    }
}

/// Normalize body for comparison (remove whitespace variations and comments)
/// For rename detection, we skip the first line (function/class signature)
/// and only compare the actual body content.
fn normalize_body(body: &str) -> String {
    body.lines()
        .skip(1) // Skip signature line (def foo(): or class Bar:)
        .map(|line| {
            // Strip inline comments (simple approach: truncate at #)
            let stripped = if let Some(pos) = line.find('#') {
                // Make sure it's not inside a string
                // Simple heuristic: if there's a # before any quote, strip it
                let before_hash = &line[..pos];
                let single_quotes = before_hash.matches('\'').count();
                let double_quotes = before_hash.matches('"').count();
                // If quotes are balanced (even count), it's a real comment
                if single_quotes % 2 == 0 && double_quotes % 2 == 0 {
                    &line[..pos]
                } else {
                    line
                }
            } else {
                line
            };
            stripped.trim()
        })
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

// =============================================================================
// Implementation
// =============================================================================

impl DiffArgs {
    /// Run the diff command and return the structured report.
    ///
    /// This is the internal workhorse: it dispatches to the appropriate
    /// algorithm based on `self.granularity` and returns a `DiffReport`
    /// without any output formatting.
    pub fn run_to_report(&self) -> Result<DiffReport> {
        // Validate paths exist
        if !self.file_a.exists() {
            return Err(RemainingError::file_not_found(&self.file_a).into());
        }
        if !self.file_b.exists() {
            return Err(RemainingError::file_not_found(&self.file_b).into());
        }

        match self.granularity {
            DiffGranularity::File => {
                // L6: directory-level structural fingerprint diff
                if !self.file_a.is_dir() || !self.file_b.is_dir() {
                    bail!("File-level (L6) diff requires directories, not individual files");
                }
                run_file_level_diff(&self.file_a, &self.file_b)
            }
            DiffGranularity::Module => {
                // L7: module-level import graph diff
                if !self.file_a.is_dir() || !self.file_b.is_dir() {
                    bail!("Module-level (L7) diff requires directories, not individual files");
                }
                run_module_level_diff(&self.file_a, &self.file_b)
            }
            DiffGranularity::Architecture => {
                // L8: architecture-level diff
                if !self.file_a.is_dir() || !self.file_b.is_dir() {
                    bail!(
                        "Architecture-level (L8) diff requires directories, not individual files"
                    );
                }
                run_arch_level_diff(&self.file_a, &self.file_b)
            }
            DiffGranularity::Class => {
                // L5: class-level diff (supports both files and directories)
                if self.file_a.is_dir() && self.file_b.is_dir() {
                    run_class_diff_directory(&self.file_a, &self.file_b, self.semantic_only)
                } else {
                    run_class_diff(&self.file_a, &self.file_b, self.semantic_only)
                }
            }
            DiffGranularity::Statement => {
                // L3: statement-level diff (Zhang-Shasha tree edit distance)
                self.run_statement_level_diff()
            }
            DiffGranularity::Token => {
                // L1: token-level diff using difftastic graph-based algorithm
                self.run_token_level_diff()
            }
            DiffGranularity::Expression => {
                // L2: expression-level diff (stub -- uses L1 until Phase 6)
                self.run_expression_level_diff()
            }
            _ => {
                // L4 and below: function-level diff (original behavior)
                self.run_function_level_diff()
            }
        }
    }

    /// Run the diff command with output formatting.
    pub fn run(&self, format: OutputFormat) -> Result<()> {
        let report = self.run_to_report()?;

        // Output
        match format {
            OutputFormat::Json => {
                let json = serde_json::to_string_pretty(&report)?;
                if let Some(ref output_path) = self.output {
                    fs::write(output_path, &json)?;
                } else {
                    println!("{}", json);
                }
            }
            OutputFormat::Text => {
                let text = format_diff_text(&report);
                if let Some(ref output_path) = self.output {
                    fs::write(output_path, &text)?;
                } else {
                    println!("{}", text);
                }
            }
            OutputFormat::Sarif | OutputFormat::Compact | OutputFormat::Dot => {
                // Other formats not supported for diff, fall back to JSON
                let json = serde_json::to_string_pretty(&report)?;
                println!("{}", json);
            }
        }

        Ok(())
    }

    /// Original L4 function-level diff implementation.
    fn run_function_level_diff(&self) -> Result<DiffReport> {
        // Detect language from file_a extension
        let lang = Language::from_path(&self.file_a).ok_or_else(|| {
            let ext = self
                .file_a
                .extension()
                .map(|e| e.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            RemainingError::parse_error(&self.file_a, format!("Unsupported language: .{}", ext))
        })?;

        // Read file contents
        let source_a = fs::read_to_string(&self.file_a)?;
        let source_b = fs::read_to_string(&self.file_b)?;

        // Parse both files using language-aware parser
        let pool = ParserPool::new();
        let tree_a = pool.parse(&source_a, lang).map_err(|e| {
            RemainingError::parse_error(&self.file_a, format!("Failed to parse file: {}", e))
        })?;
        let tree_b = pool.parse(&source_b, lang).map_err(|e| {
            RemainingError::parse_error(&self.file_b, format!("Failed to parse file: {}", e))
        })?;

        // Extract nodes from both files
        let nodes_a = extract_nodes(tree_a.root_node(), source_a.as_bytes(), lang);
        let nodes_b = extract_nodes(tree_b.root_node(), source_b.as_bytes(), lang);

        // Detect changes
        let changes = detect_changes(
            &nodes_a,
            &nodes_b,
            &self.file_a,
            &self.file_b,
            self.semantic_only,
        );

        // Build summary
        let mut summary = DiffSummary::default();
        for change in &changes {
            summary.total_changes += 1;
            if change.change_type != ChangeType::Format {
                summary.semantic_changes += 1;
            }
            match change.change_type {
                ChangeType::Insert => summary.inserts += 1,
                ChangeType::Delete => summary.deletes += 1,
                ChangeType::Update => summary.updates += 1,
                ChangeType::Move => summary.moves += 1,
                ChangeType::Rename => summary.renames += 1,
                ChangeType::Format => summary.formats += 1,
                ChangeType::Extract => summary.extracts += 1,
                ChangeType::Inline => {}
            }
        }

        // Build report
        let report = DiffReport {
            file_a: self.file_a.display().to_string(),
            file_b: self.file_b.display().to_string(),
            identical: changes.is_empty(),
            changes,
            summary: Some(summary),
            granularity: self.granularity,
            file_changes: None,
            module_changes: None,
            import_graph_summary: None,
            arch_changes: None,
            arch_summary: None,
        };

        Ok(report)
    }

    /// L1 Token-level diff using difftastic's graph-based algorithm.
    ///
    /// Pipeline:
    /// 1. Read files and detect language
    /// 2. Parse with tree-sitter
    /// 3. Convert to difftastic Syntax trees
    /// 4. Run unchanged marking, Dijkstra graph diff, slider fixup
    /// 5. Convert ChangeMap to DiffReport via changemap_to_report
    fn run_token_level_diff(&self) -> Result<DiffReport> {
        use super::difftastic;
        use typed_arena::Arena;

        // Detect language from file_a extension
        let lang = Language::from_path(&self.file_a).ok_or_else(|| {
            let ext = self
                .file_a
                .extension()
                .map(|e| e.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            RemainingError::parse_error(&self.file_a, format!("Unsupported language: .{}", ext))
        })?;

        // Read file contents
        let lhs_src = fs::read_to_string(&self.file_a)?;
        let rhs_src = fs::read_to_string(&self.file_b)?;

        // Get language config for difftastic tree-sitter conversion
        let config = difftastic::lang_config::LangConfig::for_language(lang.as_str());

        // Parse both files using existing tree-sitter infrastructure
        let pool = ParserPool::new();
        let lhs_tree = pool.parse(&lhs_src, lang).map_err(|e| {
            RemainingError::parse_error(&self.file_a, format!("Failed to parse file: {}", e))
        })?;
        let rhs_tree = pool.parse(&rhs_src, lang).map_err(|e| {
            RemainingError::parse_error(&self.file_b, format!("Failed to parse file: {}", e))
        })?;

        // Convert tree-sitter trees to difftastic Syntax trees
        let arena = Arena::new();
        let (lhs_nodes, rhs_nodes) = difftastic::ts_to_syntax::prepare_syntax_trees(
            &arena, &lhs_src, &rhs_src, &lhs_tree, &rhs_tree, &config,
        );

        // Run diff pipeline
        let mut change_map = difftastic::changes::ChangeMap::default();

        // Phase 1: Mark unchanged nodes (structural matching)
        let chunks = difftastic::unchanged::mark_unchanged(&lhs_nodes, &rhs_nodes, &mut change_map);

        // Phase 2: Run Dijkstra graph diff on each changed chunk
        for (lhs_chunk, rhs_chunk) in &chunks {
            match (lhs_chunk.first(), rhs_chunk.first()) {
                (Some(lhs_first), Some(rhs_first)) => {
                    if difftastic::dijkstra::mark_syntax(
                        Some(*lhs_first),
                        Some(*rhs_first),
                        &mut change_map,
                        difftastic::dijkstra::DEFAULT_GRAPH_LIMIT,
                    )
                    .is_err()
                    {
                        // Graph limit exceeded -- mark all nodes as Novel
                        for node in lhs_chunk {
                            difftastic::changes::insert_deep_novel(node, &mut change_map);
                        }
                        for node in rhs_chunk {
                            difftastic::changes::insert_deep_novel(node, &mut change_map);
                        }
                    }
                }
                (Some(_), None) => {
                    // LHS has nodes, RHS is empty -- all LHS nodes are Novel (deleted)
                    for node in lhs_chunk {
                        difftastic::changes::insert_deep_novel(node, &mut change_map);
                    }
                }
                (None, Some(_)) => {
                    // RHS has nodes, LHS is empty -- all RHS nodes are Novel (inserted)
                    for node in rhs_chunk {
                        difftastic::changes::insert_deep_novel(node, &mut change_map);
                    }
                }
                (None, None) => {
                    // Both sides empty -- nothing to do
                }
            }
        }

        // Phase 3: Fix sliders for better alignment
        difftastic::sliders::fix_all_sliders(&lhs_nodes, &mut change_map);
        difftastic::sliders::fix_all_sliders(&rhs_nodes, &mut change_map);

        // Convert to DiffReport
        let fa = self.file_a.display().to_string();
        let fb = self.file_b.display().to_string();
        Ok(difftastic::changemap_to_report::changemap_to_l1_report(
            &lhs_nodes,
            &rhs_nodes,
            &change_map,
            &fa,
            &fb,
        ))
    }

    /// L2 Expression-level diff using difftastic with expression grouping.
    ///
    /// Same diff pipeline as L1 (unchanged marking, Dijkstra, slider fixup)
    /// but converts the ChangeMap via `changemap_to_l2_report`, which groups
    /// token changes under their nearest `Syntax::List` parent.
    fn run_expression_level_diff(&self) -> Result<DiffReport> {
        use super::difftastic;
        use typed_arena::Arena;

        // Detect language from file_a extension
        let lang = Language::from_path(&self.file_a).ok_or_else(|| {
            let ext = self
                .file_a
                .extension()
                .map(|e| e.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            RemainingError::parse_error(&self.file_a, format!("Unsupported language: .{}", ext))
        })?;

        // Read file contents
        let lhs_src = fs::read_to_string(&self.file_a)?;
        let rhs_src = fs::read_to_string(&self.file_b)?;

        // Get language config for difftastic tree-sitter conversion
        let config = difftastic::lang_config::LangConfig::for_language(lang.as_str());

        // Parse both files using existing tree-sitter infrastructure
        let pool = ParserPool::new();
        let lhs_tree = pool.parse(&lhs_src, lang).map_err(|e| {
            RemainingError::parse_error(&self.file_a, format!("Failed to parse file: {}", e))
        })?;
        let rhs_tree = pool.parse(&rhs_src, lang).map_err(|e| {
            RemainingError::parse_error(&self.file_b, format!("Failed to parse file: {}", e))
        })?;

        // Convert tree-sitter trees to difftastic Syntax trees
        let arena = Arena::new();
        let (lhs_nodes, rhs_nodes) = difftastic::ts_to_syntax::prepare_syntax_trees(
            &arena, &lhs_src, &rhs_src, &lhs_tree, &rhs_tree, &config,
        );

        // Run diff pipeline
        let mut change_map = difftastic::changes::ChangeMap::default();

        // Phase 1: Mark unchanged nodes (structural matching)
        let chunks = difftastic::unchanged::mark_unchanged(&lhs_nodes, &rhs_nodes, &mut change_map);

        // Phase 2: Run Dijkstra graph diff on each changed chunk
        for (lhs_chunk, rhs_chunk) in &chunks {
            match (lhs_chunk.first(), rhs_chunk.first()) {
                (Some(lhs_first), Some(rhs_first)) => {
                    if difftastic::dijkstra::mark_syntax(
                        Some(*lhs_first),
                        Some(*rhs_first),
                        &mut change_map,
                        difftastic::dijkstra::DEFAULT_GRAPH_LIMIT,
                    )
                    .is_err()
                    {
                        for node in lhs_chunk {
                            difftastic::changes::insert_deep_novel(node, &mut change_map);
                        }
                        for node in rhs_chunk {
                            difftastic::changes::insert_deep_novel(node, &mut change_map);
                        }
                    }
                }
                (Some(_), None) => {
                    for node in lhs_chunk {
                        difftastic::changes::insert_deep_novel(node, &mut change_map);
                    }
                }
                (None, Some(_)) => {
                    for node in rhs_chunk {
                        difftastic::changes::insert_deep_novel(node, &mut change_map);
                    }
                }
                (None, None) => {}
            }
        }

        // Phase 3: Fix sliders for better alignment
        difftastic::sliders::fix_all_sliders(&lhs_nodes, &mut change_map);
        difftastic::sliders::fix_all_sliders(&rhs_nodes, &mut change_map);

        // Convert to DiffReport using L2 expression grouping
        let fa = self.file_a.display().to_string();
        let fb = self.file_b.display().to_string();
        Ok(difftastic::changemap_to_report::changemap_to_l2_report(
            &lhs_nodes,
            &rhs_nodes,
            &change_map,
            &fa,
            &fb,
        ))
    }
}

// =============================================================================
// Tree-sitter Parsing
// =============================================================================

/// Get text for a node from source
fn node_text<'a>(node: Node, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
}

/// Get the class-like node kinds for each language
fn get_class_node_kinds(language: Language) -> &'static [&'static str] {
    match language {
        Language::Python => &["class_definition"],
        Language::TypeScript | Language::JavaScript => &["class_declaration", "class"],
        Language::Go => &["type_declaration"],
        Language::Rust => &["struct_item", "enum_item", "impl_item"],
        Language::Java => &[
            "class_declaration",
            "interface_declaration",
            "enum_declaration",
        ],
        Language::C => &["struct_specifier", "enum_specifier"],
        Language::Cpp => &["class_specifier", "struct_specifier", "enum_specifier"],
        Language::Ruby => &["class", "module"],
        Language::Php => &["class_declaration", "interface_declaration"],
        Language::CSharp => &[
            "class_declaration",
            "interface_declaration",
            "struct_declaration",
        ],
        Language::Kotlin => &["class_declaration", "object_declaration"],
        Language::Scala => &["class_definition", "object_definition", "trait_definition"],
        Language::Swift => &[
            "class_declaration",
            "struct_declaration",
            "protocol_declaration",
        ],
        Language::Elixir => &["call"],         // defmodule is a call
        Language::Lua | Language::Luau => &[], // Lua has no class syntax
        Language::Ocaml => &["module_definition", "type_definition"],
    }
}

/// Get the node kinds that represent class body containers for method extraction
fn get_class_body_kinds(language: Language) -> &'static [&'static str] {
    match language {
        Language::Python => &["block"],
        Language::TypeScript | Language::JavaScript => &["class_body"],
        Language::Go => &[], // Go methods are not nested in type declarations
        Language::Rust => &["declaration_list"], // impl_item body
        Language::Java => &["class_body"],
        Language::C | Language::Cpp => &["field_declaration_list"],
        Language::Ruby => &["body_statement"],
        Language::Php => &["declaration_list"],
        Language::CSharp => &["declaration_list"],
        Language::Kotlin => &["class_body"],
        Language::Scala => &["template_body"],
        Language::Swift => &["class_body"],
        Language::Elixir => &["do_block"],
        Language::Lua | Language::Luau => &[],
        Language::Ocaml => &[],
    }
}

// =============================================================================
// Node Extraction
// =============================================================================

/// Extract all functions, classes, and methods from AST
fn extract_nodes(root: Node, source: &[u8], lang: Language) -> Vec<ExtractedNode> {
    let mut nodes = Vec::new();
    let kinds = NodeKindSets {
        func: get_function_node_kinds(lang),
        class: get_class_node_kinds(lang),
        body: get_class_body_kinds(lang),
    };
    extract_nodes_recursive(root, source, &mut nodes, false, lang, &kinds);
    nodes
}

struct NodeKindSets<'a> {
    func: &'a [&'a str],
    class: &'a [&'a str],
    body: &'a [&'a str],
}

fn extract_nodes_recursive(
    node: Node,
    source: &[u8],
    nodes: &mut Vec<ExtractedNode>,
    in_class: bool,
    lang: Language,
    kinds: &NodeKindSets<'_>,
) {
    let kind = node.kind();

    // OCaml-specific: function-kinds are `value_definition` AND
    // `let_binding`. The tree-sitter shape is:
    //   value_definition -> let_binding -> pattern: <name>
    // Plus, `let_binding` ALSO appears nested inside expressions
    // (`let _ = expr in body`), where it is NOT a function definition.
    // VAL-018: filter to top-level value_definition only, and require a
    // parameter (mirrors `extract_ocaml_functions` in
    // crates/tldr-core/src/ast/extractor.rs:1132). Skip nested
    // let_bindings inside function bodies and anonymous `_` bindings.
    if lang == Language::Ocaml && kind == "value_definition" {
        for child in node.children(&mut node.walk()) {
            if child.kind() == "let_binding" && ocaml_let_binding_is_function(child) {
                if let Some(extracted) = extract_function_node(child, source, in_class, lang) {
                    // Skip anonymous `_` patterns and `()` unit bindings.
                    if extracted.name != "_" && extracted.name != "()" && !extracted.name.is_empty()
                    {
                        nodes.push(extracted);
                    }
                }
            }
        }
        // Don't recurse — we've already extracted the function. Inner
        // let-bindings (e.g. `let _ = helper () in ...`) are body
        // expressions, not functions.
        return;
    }
    if lang == Language::Ocaml && kind == "let_binding" {
        // Bare let_binding outside a value_definition: only valid as a
        // top-level definition without a wrapping value_definition,
        // which is not the canonical form. Don't extract; recurse normally.
        // (Tree-sitter usually wraps top-level lets in value_definition.)
        for child in node.children(&mut node.walk()) {
            extract_nodes_recursive(child, source, nodes, in_class, lang, kinds);
        }
        return;
    }

    // Check if this is a function node
    if kinds.func.contains(&kind) {
        if let Some(extracted) = extract_function_node(node, source, in_class, lang) {
            nodes.push(extracted);
        }
    }
    // Check if this is a class node
    else if kinds.class.contains(&kind) {
        if let Some(extracted) = extract_class_node(node, source, lang) {
            nodes.push(extracted);
        }
        // Extract methods inside the class body
        for child in node.children(&mut node.walk()) {
            if kinds.body.contains(&child.kind()) {
                extract_nodes_recursive(child, source, nodes, true, lang, kinds);
            }
        }
        return; // Don't recurse further - we handled the body
    }

    // Recurse into children
    for child in node.children(&mut node.walk()) {
        extract_nodes_recursive(child, source, nodes, in_class, lang, kinds);
    }
}

/// True if an OCaml `let_binding` node has at least one `parameter`
/// child — i.e. it's a function definition rather than a value binding.
/// Mirrors `ocaml_binding_has_params_simple` in
/// `crates/tldr-core/src/ast/extractor.rs:1158`.
fn ocaml_let_binding_is_function(node: Node) -> bool {
    for child in node.children(&mut node.walk()) {
        if child.kind() == "parameter" {
            return true;
        }
    }
    false
}

fn extract_function_node(
    node: Node,
    source: &[u8],
    is_method: bool,
    lang: Language,
) -> Option<ExtractedNode> {
    // Use language-aware name extraction from function_finder
    let source_str = std::str::from_utf8(source).unwrap_or("");
    let func_name = get_function_name(node, lang, source_str)?;

    // Try to extract parameters (varies by language but most use "parameters" or "formal_parameters")
    let params = node
        .child_by_field_name("parameters")
        .or_else(|| node.child_by_field_name("formal_parameters"))
        .map(|p| node_text(p, source).to_string())
        .unwrap_or_default();

    let line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;
    let column = node.start_position().column as u32;
    let body = node_text(node, source).to_string();

    let mut extracted =
        ExtractedNode::new(func_name, NodeKind::Function, line, end_line, column, body)
            .with_params(params);

    if is_method {
        extracted = extracted.with_method_kind();
    }

    Some(extracted)
}

fn extract_class_node(node: Node, source: &[u8], lang: Language) -> Option<ExtractedNode> {
    // Get class name - most languages use "name" field
    let class_name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, source).to_string())
        .or_else(|| {
            // Fallback: search for first identifier child
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier"
                    || child.kind() == "type_identifier"
                    || child.kind() == "constant"
                {
                    return Some(node_text(child, source).to_string());
                }
            }
            None
        })?;

    // Skip empty names
    if class_name.is_empty() {
        return None;
    }

    // For Elixir defmodule, filter to only actual module definitions
    if lang == Language::Elixir && node.kind() == "call" {
        let first_child = node.child(0)?;
        let first_text = node_text(first_child, source);
        if first_text != "defmodule" {
            return None;
        }
        // Module name is in the arguments
        if let Some(args) = node.child(1) {
            let name = node_text(args, source).to_string();
            if !name.is_empty() {
                let line = node.start_position().row as u32 + 1;
                let end_line = node.end_position().row as u32 + 1;
                let column = node.start_position().column as u32;
                let body = node_text(node, source).to_string();
                return Some(ExtractedNode::new(
                    name,
                    NodeKind::Class,
                    line,
                    end_line,
                    column,
                    body,
                ));
            }
        }
        return None;
    }

    let line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;
    let column = node.start_position().column as u32;
    let body = node_text(node, source).to_string();

    Some(ExtractedNode::new(
        class_name,
        NodeKind::Class,
        line,
        end_line,
        column,
        body,
    ))
}

// =============================================================================
// Change Detection
// =============================================================================

/// Detect changes between two sets of nodes
fn detect_changes(
    nodes_a: &[ExtractedNode],
    nodes_b: &[ExtractedNode],
    file_a: &Path,
    file_b: &Path,
    semantic_only: bool,
) -> Vec<ASTChange> {
    let mut changes = Vec::new();

    // Build lookup maps by name
    let _map_a: HashMap<&str, &ExtractedNode> =
        nodes_a.iter().map(|n| (n.name.as_str(), n)).collect();
    let map_b: HashMap<&str, &ExtractedNode> =
        nodes_b.iter().map(|n| (n.name.as_str(), n)).collect();

    // Track which nodes have been matched
    let mut matched_a: Vec<bool> = vec![false; nodes_a.len()];
    let mut matched_b: Vec<bool> = vec![false; nodes_b.len()];

    // First pass: exact name matches
    for (i, node_a) in nodes_a.iter().enumerate() {
        let _ = node_a.end_line;
        if let Some(&node_b) = map_b.get(node_a.name.as_str()) {
            // Same name exists in both files
            matched_a[i] = true;
            if let Some(j) = nodes_b.iter().position(|n| n.name == node_a.name) {
                matched_b[j] = true;
            }

            // Check if body changed
            if node_a.normalized_body != node_b.normalized_body {
                // It's an update
                changes.push(ASTChange {
                    change_type: ChangeType::Update,
                    node_kind: node_a.kind,
                    name: Some(node_a.name.clone()),
                    old_location: Some(Location::with_column(
                        file_a.display().to_string(),
                        node_a.line,
                        node_a.column,
                    )),
                    new_location: Some(Location::with_column(
                        file_b.display().to_string(),
                        node_b.line,
                        node_b.column,
                    )),
                    old_text: Some(node_a.body.clone()),
                    new_text: Some(node_b.body.clone()),
                    similarity: Some(compute_similarity(
                        &node_a.normalized_body,
                        &node_b.normalized_body,
                    )),
                    children: None,
                    base_changes: None,
                });
            } else if node_a.line != node_b.line && !semantic_only {
                // Same content but moved - only report if not semantic_only
                changes.push(ASTChange {
                    change_type: ChangeType::Move,
                    node_kind: node_a.kind,
                    name: Some(node_a.name.clone()),
                    old_location: Some(Location::with_column(
                        file_a.display().to_string(),
                        node_a.line,
                        node_a.column,
                    )),
                    new_location: Some(Location::with_column(
                        file_b.display().to_string(),
                        node_b.line,
                        node_b.column,
                    )),
                    old_text: None,
                    new_text: None,
                    similarity: Some(1.0),
                    children: None,
                    base_changes: None,
                });
            }
        }
    }

    // Collect unmatched nodes
    let unmatched_a: Vec<(usize, &ExtractedNode)> = nodes_a
        .iter()
        .enumerate()
        .filter(|(i, _)| !matched_a[*i])
        .collect();
    let unmatched_b: Vec<(usize, &ExtractedNode)> = nodes_b
        .iter()
        .enumerate()
        .filter(|(i, _)| !matched_b[*i])
        .collect();

    // Second pass: detect renames (same body, different name)
    let mut used_b: Vec<bool> = vec![false; unmatched_b.len()];

    for (_, node_a) in &unmatched_a {
        let mut best_match: Option<(usize, f64)> = None;

        for (j, (_, node_b)) in unmatched_b.iter().enumerate() {
            if used_b[j] {
                continue;
            }
            if node_a.kind != node_b.kind {
                continue;
            }

            let similarity = compute_similarity(&node_a.normalized_body, &node_b.normalized_body);
            if similarity >= RENAME_SIMILARITY_THRESHOLD
                && (best_match.is_none() || similarity > best_match.unwrap().1)
            {
                best_match = Some((j, similarity));
            }
        }

        if let Some((j, similarity)) = best_match {
            let (_, node_b) = unmatched_b[j];
            used_b[j] = true;

            // Mark as renamed
            changes.push(ASTChange {
                change_type: ChangeType::Rename,
                node_kind: node_a.kind,
                name: Some(node_a.name.clone()),
                old_location: Some(Location::with_column(
                    file_a.display().to_string(),
                    node_a.line,
                    node_a.column,
                )),
                new_location: Some(Location::with_column(
                    file_b.display().to_string(),
                    node_b.line,
                    node_b.column,
                )),
                old_text: Some(node_a.name.clone()),
                new_text: Some(node_b.name.clone()),
                similarity: Some(similarity),
                children: None,
                base_changes: None,
            });
        }
    }

    // Remaining unmatched in A are deletes
    for (_, node_a) in &unmatched_a {
        // Check if already matched as rename
        let is_renamed = changes
            .iter()
            .any(|c| c.change_type == ChangeType::Rename && c.name.as_ref() == Some(&node_a.name));
        if !is_renamed {
            changes.push(ASTChange {
                change_type: ChangeType::Delete,
                node_kind: node_a.kind,
                name: Some(node_a.name.clone()),
                old_location: Some(Location::with_column(
                    file_a.display().to_string(),
                    node_a.line,
                    node_a.column,
                )),
                new_location: None,
                old_text: None,
                new_text: None,
                similarity: None,
                children: None,
                base_changes: None,
            });
        }
    }

    // Remaining unmatched in B are inserts
    for (j, (_, node_b)) in unmatched_b.iter().enumerate() {
        if !used_b[j] {
            changes.push(ASTChange {
                change_type: ChangeType::Insert,
                node_kind: node_b.kind,
                name: Some(node_b.name.clone()),
                old_location: None,
                new_location: Some(Location::with_column(
                    file_b.display().to_string(),
                    node_b.line,
                    node_b.column,
                )),
                old_text: None,
                new_text: None,
                similarity: None,
                children: None,
                base_changes: None,
            });
        }
    }

    // Sort changes: deletes, renames, updates, inserts
    changes.sort_by_key(|c| match c.change_type {
        ChangeType::Delete => 0,
        ChangeType::Rename => 1,
        ChangeType::Update => 2,
        ChangeType::Move => 3,
        ChangeType::Insert => 4,
        _ => 5,
    });

    changes
}

// =============================================================================
// Similarity Computation
// =============================================================================

/// Compute similarity between two strings using Jaccard on lines,
/// with a character-level fallback for short/single-line bodies.
fn compute_similarity(a: &str, b: &str) -> f64 {
    if a == b {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }

    // Jaccard similarity on lines
    let lines_a: std::collections::HashSet<&str> = a.lines().collect();
    let lines_b: std::collections::HashSet<&str> = b.lines().collect();

    let intersection = lines_a.intersection(&lines_b).count();
    let union = lines_a.union(&lines_b).count();

    let line_sim = if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    };

    // For short bodies (few lines), also compute character-level similarity
    // to avoid 0.0 when a single line was slightly modified
    if line_sim == 0.0 && lines_a.len() <= 2 && lines_b.len() <= 2 {
        return char_jaccard_similarity(a, b);
    }

    line_sim
}

/// Character-level Jaccard similarity (bigrams).
fn char_jaccard_similarity(a: &str, b: &str) -> f64 {
    if a.len() < 2 || b.len() < 2 {
        return if a == b { 1.0 } else { 0.0 };
    }

    let bigrams_a: std::collections::HashSet<&[u8]> = a.as_bytes().windows(2).collect();
    let bigrams_b: std::collections::HashSet<&[u8]> = b.as_bytes().windows(2).collect();

    let intersection = bigrams_a.intersection(&bigrams_b).count();
    let union = bigrams_a.union(&bigrams_b).count();

    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

// =============================================================================
// Text Formatting
// =============================================================================

/// Format diff report as human-readable text
fn format_diff_text(report: &DiffReport) -> String {
    let mut out = String::new();

    out.push_str("Diff Report\n");
    out.push_str("===========\n\n");
    out.push_str(&format!("File A: {}\n", report.file_a));
    out.push_str(&format!("File B: {}\n", report.file_b));
    out.push_str(&format!("Identical: {}\n\n", report.identical));

    if report.identical {
        out.push_str("No structural changes detected.\n");
        return out;
    }

    out.push_str("Changes:\n");
    out.push_str("--------\n");

    for change in &report.changes {
        let change_type = match change.change_type {
            ChangeType::Insert => "+",
            ChangeType::Delete => "-",
            ChangeType::Update => "~",
            ChangeType::Move => ">",
            ChangeType::Rename => "R",
            ChangeType::Format => "F",
            ChangeType::Extract => "E",
            ChangeType::Inline => "I",
        };

        let kind = match change.node_kind {
            NodeKind::Function => "function",
            NodeKind::Class => "class",
            NodeKind::Method => "method",
            NodeKind::Field => "field",
            NodeKind::Statement => "statement",
            NodeKind::Expression => "expression",
            NodeKind::Block => "block",
        };

        let name = change.name.as_deref().unwrap_or("<unknown>");

        match change.change_type {
            ChangeType::Insert => {
                if let Some(ref loc) = change.new_location {
                    out.push_str(&format!(
                        "  {} {} {} at {}:{}\n",
                        change_type, kind, name, loc.file, loc.line
                    ));
                }
            }
            ChangeType::Delete => {
                if let Some(ref loc) = change.old_location {
                    out.push_str(&format!(
                        "  {} {} {} at {}:{}\n",
                        change_type, kind, name, loc.file, loc.line
                    ));
                }
            }
            ChangeType::Update | ChangeType::Move => {
                if let (Some(ref old), Some(ref new)) = (&change.old_location, &change.new_location)
                {
                    out.push_str(&format!(
                        "  {} {} {} from {}:{} to {}:{}\n",
                        change_type, kind, name, old.file, old.line, new.file, new.line
                    ));
                }
            }
            ChangeType::Rename => {
                let old_name = change.old_text.as_deref().unwrap_or(name);
                let new_name = change.new_text.as_deref().unwrap_or(name);
                out.push_str(&format!(
                    "  {} {} {} -> {}\n",
                    change_type, kind, old_name, new_name
                ));
            }
            _ => {
                out.push_str(&format!("  {} {} {}\n", change_type, kind, name));
            }
        }
    }

    if let Some(ref summary) = report.summary {
        out.push_str("\nSummary:\n");
        out.push_str("--------\n");
        out.push_str(&format!("  Total changes: {}\n", summary.total_changes));
        out.push_str(&format!(
            "  Semantic changes: {}\n",
            summary.semantic_changes
        ));
        out.push_str(&format!("  Inserts: {}\n", summary.inserts));
        out.push_str(&format!("  Deletes: {}\n", summary.deletes));
        out.push_str(&format!("  Updates: {}\n", summary.updates));
        out.push_str(&format!("  Renames: {}\n", summary.renames));
        out.push_str(&format!("  Moves: {}\n", summary.moves));
    }

    // L6: File-level structural changes
    if let Some(ref file_changes) = report.file_changes {
        out.push_str("\nFile-Level Changes:\n");
        out.push_str("-------------------\n");
        for fc in file_changes {
            let change_type = match fc.change_type {
                ChangeType::Insert => "+",
                ChangeType::Delete => "-",
                ChangeType::Update => "~",
                _ => "?",
            };
            out.push_str(&format!("  {} {}\n", change_type, fc.relative_path));
            if let Some(ref sigs) = fc.signature_changes {
                for sig in sigs {
                    out.push_str(&format!("      changed: {}\n", sig));
                }
            }
        }
    }

    // L7: Module-level changes
    if let Some(ref module_changes) = report.module_changes {
        out.push_str("\nModule-Level Changes:\n");
        out.push_str("---------------------\n");
        for mc in module_changes {
            let change_type = match mc.change_type {
                ChangeType::Insert => "+",
                ChangeType::Delete => "-",
                ChangeType::Update => "~",
                _ => "?",
            };
            out.push_str(&format!("  {} {}\n", change_type, mc.module_path));
            for edge in &mc.imports_added {
                let names = if edge.imported_names.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", edge.imported_names.join(", "))
                };
                out.push_str(&format!("      + import {}{}\n", edge.target_module, names));
            }
            for edge in &mc.imports_removed {
                let names = if edge.imported_names.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", edge.imported_names.join(", "))
                };
                out.push_str(&format!("      - import {}{}\n", edge.target_module, names));
            }
        }
    }

    // L7: Import graph summary
    if let Some(ref igs) = report.import_graph_summary {
        out.push_str("\nImport Graph Summary:\n");
        out.push_str("---------------------\n");
        out.push_str(&format!("  Edges in A: {}\n", igs.total_edges_a));
        out.push_str(&format!("  Edges in B: {}\n", igs.total_edges_b));
        out.push_str(&format!("  Edges added: {}\n", igs.edges_added));
        out.push_str(&format!("  Edges removed: {}\n", igs.edges_removed));
        out.push_str(&format!(
            "  Modules with import changes: {}\n",
            igs.modules_with_import_changes
        ));
    }

    // L8: Architecture-level changes
    if let Some(ref arch_changes) = report.arch_changes {
        out.push_str("\nArchitecture-Level Changes:\n");
        out.push_str("---------------------------\n");
        for ac in arch_changes {
            let change_label = match ac.change_type {
                ArchChangeType::LayerMigration => "migration",
                ArchChangeType::Added => "added",
                ArchChangeType::Removed => "removed",
                ArchChangeType::CompositionChanged => "composition changed",
                ArchChangeType::CycleIntroduced => "cycle introduced",
                ArchChangeType::CycleResolved => "cycle resolved",
            };
            out.push_str(&format!("  [{}] {}\n", change_label, ac.directory));
            if let (Some(ref old), Some(ref new)) = (&ac.old_layer, &ac.new_layer) {
                out.push_str(&format!("      {} -> {}\n", old, new));
            } else if let Some(ref new) = ac.new_layer {
                out.push_str(&format!("      -> {}\n", new));
            } else if let Some(ref old) = ac.old_layer {
                out.push_str(&format!("      {} ->\n", old));
            }
            if !ac.migrated_functions.is_empty() {
                out.push_str(&format!(
                    "      migrated: {}\n",
                    ac.migrated_functions.join(", ")
                ));
            }
        }
    }

    // L8: Architecture diff summary
    if let Some(ref arch_summary) = report.arch_summary {
        out.push_str("\nArchitecture Summary:\n");
        out.push_str("---------------------\n");
        out.push_str(&format!(
            "  Layer migrations: {}\n",
            arch_summary.layer_migrations
        ));
        out.push_str(&format!(
            "  Directories added: {}\n",
            arch_summary.directories_added
        ));
        out.push_str(&format!(
            "  Directories removed: {}\n",
            arch_summary.directories_removed
        ));
        out.push_str(&format!(
            "  Cycles introduced: {}\n",
            arch_summary.cycles_introduced
        ));
        out.push_str(&format!(
            "  Cycles resolved: {}\n",
            arch_summary.cycles_resolved
        ));
        out.push_str(&format!(
            "  Stability score: {}\n",
            arch_summary.stability_score
        ));
    }

    out
}

// =============================================================================
// Statement-Level Diff (L3) - Zhang-Shasha Tree Edit Distance
// =============================================================================

/// Statement node kinds per language for tree extraction.
fn get_statement_node_kinds(lang: Language) -> &'static [&'static str] {
    match lang {
        Language::Python => &[
            "return_statement",
            "if_statement",
            "for_statement",
            "while_statement",
            "expression_statement",
            "assert_statement",
            "raise_statement",
            "try_statement",
            "with_statement",
            "assignment",
            "augmented_assignment",
            "delete_statement",
            "pass_statement",
            "break_statement",
            "continue_statement",
        ],
        Language::TypeScript | Language::JavaScript => &[
            "return_statement",
            "if_statement",
            "for_statement",
            "for_in_statement",
            "while_statement",
            "do_statement",
            "expression_statement",
            "variable_declaration",
            "lexical_declaration",
            "throw_statement",
            "try_statement",
            "switch_statement",
            "break_statement",
            "continue_statement",
        ],
        Language::Go => &[
            "return_statement",
            "if_statement",
            "for_statement",
            "expression_statement",
            "short_var_declaration",
            "var_declaration",
            "assignment_statement",
            "go_statement",
            "defer_statement",
            "select_statement",
            "switch_statement",
        ],
        Language::Rust => &[
            "let_declaration",
            "expression_statement",
            "return_expression",
            "if_expression",
            "for_expression",
            "while_expression",
            "loop_expression",
            "match_expression",
        ],
        Language::Java => &[
            "return_statement",
            "if_statement",
            "for_statement",
            "enhanced_for_statement",
            "while_statement",
            "do_statement",
            "expression_statement",
            "local_variable_declaration",
            "throw_statement",
            "try_statement",
            "switch_expression",
        ],
        Language::C | Language::Cpp => &[
            "return_statement",
            "if_statement",
            "for_statement",
            "while_statement",
            "do_statement",
            "expression_statement",
            "declaration",
            "switch_statement",
        ],
        Language::Ruby => &[
            "return",
            "if",
            "unless",
            "for",
            "while",
            "until",
            "assignment",
            "call",
            "begin",
        ],
        Language::Php => &[
            "return_statement",
            "if_statement",
            "for_statement",
            "foreach_statement",
            "while_statement",
            "expression_statement",
            "echo_statement",
            "throw_expression",
            "try_statement",
        ],
        Language::CSharp => &[
            "return_statement",
            "if_statement",
            "for_statement",
            "foreach_statement",
            "while_statement",
            "expression_statement",
            "local_declaration_statement",
            "throw_statement",
            "try_statement",
        ],
        Language::Kotlin => &[
            "property_declaration",
            "assignment",
            "if_expression",
            "for_statement",
            "while_statement",
            "do_while_statement",
            "return_expression",
            "throw_expression",
            "try_expression",
        ],
        Language::Scala => &[
            "val_definition",
            "var_definition",
            "if_expression",
            "for_expression",
            "while_expression",
            "return_expression",
            "throw_expression",
            "try_expression",
            "call_expression",
        ],
        Language::Swift => &[
            "value_binding_pattern",
            "if_statement",
            "for_in_statement",
            "while_statement",
            "return_statement",
            "throw_statement",
            "guard_statement",
            "switch_statement",
        ],
        Language::Elixir => &["call", "if", "case", "cond"],
        Language::Lua | Language::Luau => &[
            "return_statement",
            "if_statement",
            "for_statement",
            "while_statement",
            "variable_declaration",
            "assignment_statement",
            "function_call",
        ],
        Language::Ocaml => &[
            "let_binding",
            "if_expression",
            "match_expression",
            "application",
        ],
    }
}

/// A labeled tree node for the Zhang-Shasha tree edit distance algorithm.
#[derive(Debug, Clone)]
struct LabeledTreeNode {
    /// Node label: "node_kind:significant_text"
    label: String,
    /// Children (ordered)
    children: Vec<LabeledTreeNode>,
    /// Source line number (1-indexed) for mapping back to locations
    line: u32,
}

/// Flattened node in postorder for Zhang-Shasha.
#[derive(Debug, Clone)]
struct PostorderNode {
    label: String,
    line: u32,
    /// Index of leftmost leaf descendant in the postorder array
    leftmost_leaf: usize,
}

/// Edit operation from Zhang-Shasha.
#[derive(Debug, Clone)]
enum EditOp {
    /// Delete node from tree A (index in postorder of A)
    Delete { index_a: usize },
    /// Insert node from tree B (index in postorder of B)
    Insert { index_b: usize },
    /// Relabel (update) node A[i] -> B[j]
    Relabel { index_a: usize, index_b: usize },
}

/// Build a labeled tree from a tree-sitter function body node.
///
/// Walks the AST and picks out statement-level nodes, building an ordered
/// tree where each statement is a node and nested statements (e.g., inside
/// if-bodies) become children.
fn build_labeled_tree(node: Node, source: &[u8], statement_kinds: &[&str]) -> LabeledTreeNode {
    let label = build_node_label(node, source);
    let line = node.start_position().row as u32 + 1;

    let mut children = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if statement_kinds.contains(&child.kind()) {
            // This child is a statement node - add it and recurse into its body
            children.push(build_labeled_tree(child, source, statement_kinds));
        } else {
            // Not a statement node - look deeper for nested statements
            let nested = collect_nested_statements(child, source, statement_kinds);
            children.extend(nested);
        }
    }

    LabeledTreeNode {
        label,
        children,
        line,
    }
}

/// Collect statement nodes from non-statement intermediate nodes.
fn collect_nested_statements(
    node: Node,
    source: &[u8],
    statement_kinds: &[&str],
) -> Vec<LabeledTreeNode> {
    let mut result = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if statement_kinds.contains(&child.kind()) {
            result.push(build_labeled_tree(child, source, statement_kinds));
        } else {
            result.extend(collect_nested_statements(child, source, statement_kinds));
        }
    }
    result
}

/// Build a label string for a tree-sitter node.
///
/// Format: "node_kind:significant_tokens" where significant tokens
/// are identifiers and operators (not whitespace or delimiters).
fn build_node_label(node: Node, source: &[u8]) -> String {
    let kind = node.kind();
    let text = node.utf8_text(source).unwrap_or("");

    // Extract significant tokens: identifiers, operators, literals
    // We take just the first line for conciseness and strip whitespace
    let first_line = text.lines().next().unwrap_or("").trim();

    // Truncate to avoid huge labels
    let significant = if first_line.len() > 120 {
        &first_line[..120]
    } else {
        first_line
    };

    format!("{}:{}", kind, significant)
}

/// Extract statement-level subtree from a function body node.
///
/// Finds the function body (block node) and builds a labeled tree
/// from the statements within it.
fn extract_statement_tree(
    func_node: Node,
    source: &[u8],
    lang: Language,
    statement_kinds: &[&str],
) -> LabeledTreeNode {
    // Find the function body node
    let body_node = find_function_body(func_node, lang);

    match body_node {
        Some(body) => {
            // Build a root node representing the function body
            let mut children = Vec::new();
            let mut cursor = body.walk();
            for child in body.children(&mut cursor) {
                if statement_kinds.contains(&child.kind()) {
                    children.push(build_labeled_tree(child, source, statement_kinds));
                } else {
                    children.extend(collect_nested_statements(child, source, statement_kinds));
                }
            }

            LabeledTreeNode {
                label: format!("body:{}", func_node.kind()),
                children,
                line: body.start_position().row as u32 + 1,
            }
        }
        None => {
            // Fallback: treat the entire function node as the body
            build_labeled_tree(func_node, source, statement_kinds)
        }
    }
}

/// Find the body/block node within a function definition.
fn find_function_body(func_node: Node, lang: Language) -> Option<Node> {
    // Try common field names
    if let Some(body) = func_node.child_by_field_name("body") {
        return Some(body);
    }
    if let Some(body) = func_node.child_by_field_name("block") {
        return Some(body);
    }

    // Language-specific body detection
    let body_kinds = match lang {
        Language::Python => &["block"][..],
        Language::TypeScript | Language::JavaScript => &["statement_block"],
        Language::Go => &["block"],
        Language::Rust => &["block"],
        Language::Java => &["block"],
        Language::C | Language::Cpp => &["compound_statement"],
        Language::Ruby => &["body_statement"],
        Language::Php => &["compound_statement"],
        Language::CSharp => &["block"],
        Language::Kotlin => &["function_body"],
        Language::Scala => &["block", "indented_block"],
        Language::Swift => &["function_body"],
        Language::Elixir => &["do_block"],
        Language::Lua | Language::Luau => &["block"],
        Language::Ocaml => &["let_binding"],
    };

    let mut cursor = func_node.walk();
    let found = func_node
        .children(&mut cursor)
        .find(|&child| body_kinds.contains(&child.kind()));
    found
}

/// Count total nodes in a labeled tree.
fn count_tree_nodes(tree: &LabeledTreeNode) -> usize {
    1 + tree.children.iter().map(count_tree_nodes).sum::<usize>()
}

// =============================================================================
// Zhang-Shasha Tree Edit Distance
// =============================================================================

/// Flatten a labeled tree into postorder traversal, computing leftmost leaf descendants.
fn flatten_postorder(tree: &LabeledTreeNode) -> Vec<PostorderNode> {
    let mut nodes = Vec::new();
    flatten_postorder_recursive(tree, &mut nodes);
    nodes
}

fn flatten_postorder_recursive(tree: &LabeledTreeNode, nodes: &mut Vec<PostorderNode>) -> usize {
    if tree.children.is_empty() {
        // Leaf node: leftmost leaf is itself
        let idx = nodes.len();
        nodes.push(PostorderNode {
            label: tree.label.clone(),
            line: tree.line,
            leftmost_leaf: idx,
        });
        return idx;
    }

    // Process children first (postorder)
    let mut first_child_leftmost = usize::MAX;
    for (i, child) in tree.children.iter().enumerate() {
        let child_leftmost = flatten_postorder_recursive(child, nodes);
        if i == 0 {
            first_child_leftmost = child_leftmost;
        }
    }

    // Now add this node
    nodes.push(PostorderNode {
        label: tree.label.clone(),
        line: tree.line,
        leftmost_leaf: first_child_leftmost,
    });

    // The leftmost leaf of this node is the leftmost leaf of its first child
    first_child_leftmost
}

/// Compute keyroots from a postorder traversal.
///
/// A keyroot is a node whose leftmost-leaf is different from its parent's
/// leftmost-leaf, OR the root node. In practice, we collect the rightmost
/// node at each unique leftmost-leaf value.
fn compute_keyroots(nodes: &[PostorderNode]) -> Vec<usize> {
    let n = nodes.len();
    if n == 0 {
        return Vec::new();
    }

    // For each unique leftmost leaf value, keep the highest index (rightmost occurrence)
    let mut lr_map: HashMap<usize, usize> = HashMap::new();
    for (i, node) in nodes.iter().enumerate() {
        lr_map.insert(node.leftmost_leaf, i);
    }

    let mut keyroots: Vec<usize> = lr_map.into_values().collect();
    keyroots.sort();
    keyroots
}

/// Run the Zhang-Shasha tree edit distance algorithm.
///
/// Returns the edit operations (edit script).
///
/// Costs: Delete = 1, Insert = 1, Relabel = 0 (same label) or 1 (different label).
fn zhang_shasha(nodes_a: &[PostorderNode], nodes_b: &[PostorderNode]) -> Vec<EditOp> {
    let na = nodes_a.len();
    let nb = nodes_b.len();

    if na == 0 && nb == 0 {
        return Vec::new();
    }
    if na == 0 {
        // All inserts
        return (0..nb).map(|j| EditOp::Insert { index_b: j }).collect();
    }
    if nb == 0 {
        // All deletes
        return (0..na).map(|i| EditOp::Delete { index_a: i }).collect();
    }

    let keyroots_a = compute_keyroots(nodes_a);
    let keyroots_b = compute_keyroots(nodes_b);

    // Tree distance matrix (1-indexed, 0 means empty tree)
    let mut td = vec![vec![0usize; nb + 1]; na + 1];
    // Track operations: 0=relabel/match, 1=delete, 2=insert, 3=tree-match
    let mut td_ops = vec![vec![0u8; nb + 1]; na + 1];

    for &kr_a in &keyroots_a {
        for &kr_b in &keyroots_b {
            let la = nodes_a[kr_a].leftmost_leaf;
            let lb = nodes_b[kr_b].leftmost_leaf;

            let rows = kr_a - la + 2;
            let cols = kr_b - lb + 2;
            let mut fd = vec![vec![0usize; cols]; rows];

            // Base cases
            for i in 1..rows {
                fd[i][0] = fd[i - 1][0] + 1;
            }
            for j in 1..cols {
                fd[0][j] = fd[0][j - 1] + 1;
            }

            for i in 1..rows {
                for j in 1..cols {
                    let idx_a = la + i - 1;
                    let idx_b = lb + j - 1;

                    let cost_relabel = if nodes_a[idx_a].label == nodes_b[idx_b].label {
                        0
                    } else {
                        1
                    };

                    if nodes_a[idx_a].leftmost_leaf == la && nodes_b[idx_b].leftmost_leaf == lb {
                        let delete = fd[i - 1][j] + 1;
                        let insert = fd[i][j - 1] + 1;
                        let relabel = fd[i - 1][j - 1] + cost_relabel;

                        if relabel <= delete && relabel <= insert {
                            fd[i][j] = relabel;
                            td[idx_a + 1][idx_b + 1] = relabel;
                            td_ops[idx_a + 1][idx_b + 1] = if cost_relabel == 0 { 0 } else { 3 };
                        } else if delete <= insert {
                            fd[i][j] = delete;
                            td[idx_a + 1][idx_b + 1] = delete;
                            td_ops[idx_a + 1][idx_b + 1] = 1;
                        } else {
                            fd[i][j] = insert;
                            td[idx_a + 1][idx_b + 1] = insert;
                            td_ops[idx_a + 1][idx_b + 1] = 2;
                        }
                    } else {
                        let p = nodes_a[idx_a].leftmost_leaf - la;
                        let q = nodes_b[idx_b].leftmost_leaf - lb;

                        let delete = fd[i - 1][j] + 1;
                        let insert = fd[i][j - 1] + 1;
                        let tree_match = fd[p][q] + td[idx_a + 1][idx_b + 1];

                        if tree_match <= delete && tree_match <= insert {
                            fd[i][j] = tree_match;
                        } else if delete <= insert {
                            fd[i][j] = delete;
                        } else {
                            fd[i][j] = insert;
                        }
                    }
                }
            }
        }
    }

    // Extract edit script using sequence alignment on postorder nodes
    // guided by the tree distance computation
    let mut ops = Vec::new();
    derive_edit_ops_dp(nodes_a, nodes_b, &mut ops);
    ops
}

/// Derive edit operations using DP on the postorder sequences.
///
/// This produces the edit script by sequence-aligning the postorder
/// traversals, which captures the essential edit operations.
fn derive_edit_ops_dp(nodes_a: &[PostorderNode], nodes_b: &[PostorderNode], ops: &mut Vec<EditOp>) {
    let na = nodes_a.len();
    let nb = nodes_b.len();

    let mut dp = vec![vec![0usize; nb + 1]; na + 1];
    let mut choice = vec![vec![0u8; nb + 1]; na + 1];

    for i in 1..=na {
        dp[i][0] = i;
        choice[i][0] = 1;
    }
    for j in 1..=nb {
        dp[0][j] = j;
        choice[0][j] = 2;
    }

    for i in 1..=na {
        for j in 1..=nb {
            let cost = if nodes_a[i - 1].label == nodes_b[j - 1].label {
                0
            } else {
                1
            };

            let del = dp[i - 1][j] + 1;
            let ins = dp[i][j - 1] + 1;
            let sub = dp[i - 1][j - 1] + cost;

            if sub <= del && sub <= ins {
                dp[i][j] = sub;
                choice[i][j] = if cost == 0 { 0 } else { 3 };
            } else if del <= ins {
                dp[i][j] = del;
                choice[i][j] = 1;
            } else {
                dp[i][j] = ins;
                choice[i][j] = 2;
            }
        }
    }

    // Backtrack
    let mut i = na;
    let mut j = nb;
    let mut rev_ops = Vec::new();

    while i > 0 || j > 0 {
        if i > 0 && j > 0 && (choice[i][j] == 0 || choice[i][j] == 3) {
            if choice[i][j] == 3 {
                rev_ops.push(EditOp::Relabel {
                    index_a: i - 1,
                    index_b: j - 1,
                });
            }
            i -= 1;
            j -= 1;
        } else if i > 0 && (j == 0 || choice[i][j] == 1) {
            rev_ops.push(EditOp::Delete { index_a: i - 1 });
            i -= 1;
        } else if j > 0 {
            rev_ops.push(EditOp::Insert { index_b: j - 1 });
            j -= 1;
        }
    }

    rev_ops.reverse();
    ops.extend(rev_ops);
}

/// Convert Zhang-Shasha edit operations into ASTChange records.
fn edit_ops_to_ast_changes(
    ops: &[EditOp],
    nodes_a: &[PostorderNode],
    nodes_b: &[PostorderNode],
    file_a: &Path,
    file_b: &Path,
) -> Vec<ASTChange> {
    let mut changes = Vec::new();

    for op in ops {
        match op {
            EditOp::Delete { index_a } => {
                let node = &nodes_a[*index_a];
                let stmt_kind = node.label.split(':').next().unwrap_or("statement");
                changes.push(ASTChange {
                    change_type: ChangeType::Delete,
                    node_kind: NodeKind::Statement,
                    name: Some(stmt_kind.to_string()),
                    old_location: Some(Location::new(file_a.display().to_string(), node.line)),
                    new_location: None,
                    old_text: Some(node.label.clone()),
                    new_text: None,
                    similarity: None,
                    children: None,
                    base_changes: None,
                });
            }
            EditOp::Insert { index_b } => {
                let node = &nodes_b[*index_b];
                let stmt_kind = node.label.split(':').next().unwrap_or("statement");
                changes.push(ASTChange {
                    change_type: ChangeType::Insert,
                    node_kind: NodeKind::Statement,
                    name: Some(stmt_kind.to_string()),
                    old_location: None,
                    new_location: Some(Location::new(file_b.display().to_string(), node.line)),
                    old_text: None,
                    new_text: Some(node.label.clone()),
                    similarity: None,
                    children: None,
                    base_changes: None,
                });
            }
            EditOp::Relabel { index_a, index_b } => {
                let node_a = &nodes_a[*index_a];
                let node_b = &nodes_b[*index_b];
                let stmt_kind = node_a.label.split(':').next().unwrap_or("statement");
                changes.push(ASTChange {
                    change_type: ChangeType::Update,
                    node_kind: NodeKind::Statement,
                    name: Some(stmt_kind.to_string()),
                    old_location: Some(Location::new(file_a.display().to_string(), node_a.line)),
                    new_location: Some(Location::new(file_b.display().to_string(), node_b.line)),
                    old_text: Some(node_a.label.clone()),
                    new_text: Some(node_b.label.clone()),
                    similarity: None,
                    children: None,
                    base_changes: None,
                });
            }
        }
    }

    changes
}

/// Maximum number of statements before falling back to L4-style Jaccard.
const STATEMENT_FALLBACK_THRESHOLD: usize = 200;

impl DiffArgs {
    /// L3 Statement-level diff: Zhang-Shasha tree edit distance within matched functions.
    ///
    /// Algorithm:
    /// 1. Parse both files and extract functions (reusing L4 infrastructure)
    /// 2. Match functions by name
    /// 3. For each matched pair with different bodies:
    ///    a. Extract statement subtrees from tree-sitter AST
    ///    b. Build labeled trees from statement nodes
    ///    c. Run Zhang-Shasha tree edit distance
    ///    d. Convert edit script to ASTChange children
    /// 4. For unmatched functions: report as function-level Insert/Delete
    fn run_statement_level_diff(&self) -> Result<DiffReport> {
        // Detect language
        let lang = Language::from_path(&self.file_a).ok_or_else(|| {
            let ext = self
                .file_a
                .extension()
                .map(|e| e.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            RemainingError::parse_error(&self.file_a, format!("Unsupported language: .{}", ext))
        })?;

        // Read file contents
        let source_a = fs::read_to_string(&self.file_a)?;
        let source_b = fs::read_to_string(&self.file_b)?;

        // Parse both files
        let pool = ParserPool::new();
        let tree_a = pool.parse(&source_a, lang).map_err(|e| {
            RemainingError::parse_error(&self.file_a, format!("Failed to parse: {}", e))
        })?;
        let tree_b = pool.parse(&source_b, lang).map_err(|e| {
            RemainingError::parse_error(&self.file_b, format!("Failed to parse: {}", e))
        })?;

        // Extract function nodes (reuse L4 infrastructure)
        let funcs_a = extract_nodes(tree_a.root_node(), source_a.as_bytes(), lang);
        let funcs_b = extract_nodes(tree_b.root_node(), source_b.as_bytes(), lang);

        let statement_kinds = get_statement_node_kinds(lang);

        // Build name lookup maps
        let map_b: HashMap<&str, (usize, &ExtractedNode)> = funcs_b
            .iter()
            .enumerate()
            .map(|(i, n)| (n.name.as_str(), (i, n)))
            .collect();

        let mut matched_a: Vec<bool> = vec![false; funcs_a.len()];
        let mut matched_b: Vec<bool> = vec![false; funcs_b.len()];
        let mut changes = Vec::new();

        // Pass 1: Match functions by name and compute statement-level diffs
        for (i, func_a) in funcs_a.iter().enumerate() {
            if let Some(&(j, func_b)) = map_b.get(func_a.name.as_str()) {
                matched_a[i] = true;
                matched_b[j] = true;

                // Check if bodies differ
                if func_a.normalized_body != func_b.normalized_body {
                    // Find the function nodes in the parsed trees
                    let func_node_a =
                        find_function_node_by_line(tree_a.root_node(), func_a.line, lang);
                    let func_node_b =
                        find_function_node_by_line(tree_b.root_node(), func_b.line, lang);

                    let stmt_children = match (func_node_a, func_node_b) {
                        (Some(node_a), Some(node_b)) => {
                            // Build statement trees
                            let tree_a_stmts = extract_statement_tree(
                                node_a,
                                source_a.as_bytes(),
                                lang,
                                statement_kinds,
                            );
                            let tree_b_stmts = extract_statement_tree(
                                node_b,
                                source_b.as_bytes(),
                                lang,
                                statement_kinds,
                            );

                            let count_a = count_tree_nodes(&tree_a_stmts);
                            let count_b = count_tree_nodes(&tree_b_stmts);

                            // Check fallback threshold
                            if count_a > STATEMENT_FALLBACK_THRESHOLD
                                || count_b > STATEMENT_FALLBACK_THRESHOLD
                            {
                                // Fall back to L4-style (no statement children)
                                None
                            } else {
                                // Flatten to postorder and run Zhang-Shasha
                                let po_a = flatten_postorder(&tree_a_stmts);
                                let po_b = flatten_postorder(&tree_b_stmts);

                                let edit_ops = zhang_shasha(&po_a, &po_b);

                                if edit_ops.is_empty() {
                                    None
                                } else {
                                    let stmt_changes = edit_ops_to_ast_changes(
                                        &edit_ops,
                                        &po_a,
                                        &po_b,
                                        &self.file_a,
                                        &self.file_b,
                                    );
                                    if stmt_changes.is_empty() {
                                        None
                                    } else {
                                        Some(stmt_changes)
                                    }
                                }
                            }
                        }
                        _ => None,
                    };

                    changes.push(ASTChange {
                        change_type: ChangeType::Update,
                        node_kind: func_a.kind,
                        name: Some(func_a.name.clone()),
                        old_location: Some(Location::with_column(
                            self.file_a.display().to_string(),
                            func_a.line,
                            func_a.column,
                        )),
                        new_location: Some(Location::with_column(
                            self.file_b.display().to_string(),
                            func_b.line,
                            func_b.column,
                        )),
                        old_text: Some(func_a.body.clone()),
                        new_text: Some(func_b.body.clone()),
                        similarity: Some(compute_similarity(
                            &func_a.normalized_body,
                            &func_b.normalized_body,
                        )),
                        children: stmt_children,
                        base_changes: None,
                    });
                }
            }
        }

        // Pass 2: Detect renames among unmatched functions
        let unmatched_a: Vec<(usize, &ExtractedNode)> = funcs_a
            .iter()
            .enumerate()
            .filter(|(i, _)| !matched_a[*i])
            .collect();
        let unmatched_b: Vec<(usize, &ExtractedNode)> = funcs_b
            .iter()
            .enumerate()
            .filter(|(i, _)| !matched_b[*i])
            .collect();

        let mut used_b = vec![false; unmatched_b.len()];

        for (_, func_a) in &unmatched_a {
            let mut best_match: Option<(usize, f64)> = None;
            for (j, (_, func_b)) in unmatched_b.iter().enumerate() {
                if used_b[j] || func_a.kind != func_b.kind {
                    continue;
                }
                let sim = compute_similarity(&func_a.normalized_body, &func_b.normalized_body);
                if sim >= RENAME_SIMILARITY_THRESHOLD
                    && (best_match.is_none() || sim > best_match.unwrap().1)
                {
                    best_match = Some((j, sim));
                }
            }

            if let Some((j, sim)) = best_match {
                let (_, func_b) = unmatched_b[j];
                used_b[j] = true;
                changes.push(ASTChange {
                    change_type: ChangeType::Rename,
                    node_kind: func_a.kind,
                    name: Some(func_a.name.clone()),
                    old_location: Some(Location::with_column(
                        self.file_a.display().to_string(),
                        func_a.line,
                        func_a.column,
                    )),
                    new_location: Some(Location::with_column(
                        self.file_b.display().to_string(),
                        func_b.line,
                        func_b.column,
                    )),
                    old_text: Some(func_a.name.clone()),
                    new_text: Some(func_b.name.clone()),
                    similarity: Some(sim),
                    children: None,
                    base_changes: None,
                });
            }
        }

        // Pass 3: Remaining unmatched in A are Deletes
        for (_, func_a) in &unmatched_a {
            let is_renamed = changes.iter().any(|c| {
                c.change_type == ChangeType::Rename && c.name.as_ref() == Some(&func_a.name)
            });
            if !is_renamed {
                changes.push(ASTChange {
                    change_type: ChangeType::Delete,
                    node_kind: func_a.kind,
                    name: Some(func_a.name.clone()),
                    old_location: Some(Location::with_column(
                        self.file_a.display().to_string(),
                        func_a.line,
                        func_a.column,
                    )),
                    new_location: None,
                    old_text: None,
                    new_text: None,
                    similarity: None,
                    children: None,
                    base_changes: None,
                });
            }
        }

        // Pass 4: Remaining unmatched in B are Inserts
        for (j, (_, func_b)) in unmatched_b.iter().enumerate() {
            if !used_b[j] {
                changes.push(ASTChange {
                    change_type: ChangeType::Insert,
                    node_kind: func_b.kind,
                    name: Some(func_b.name.clone()),
                    old_location: None,
                    new_location: Some(Location::with_column(
                        self.file_b.display().to_string(),
                        func_b.line,
                        func_b.column,
                    )),
                    old_text: None,
                    new_text: None,
                    similarity: None,
                    children: None,
                    base_changes: None,
                });
            }
        }

        // Build summary
        let mut summary = DiffSummary::default();
        for change in &changes {
            summary.total_changes += 1;
            if change.change_type != ChangeType::Format {
                summary.semantic_changes += 1;
            }
            match change.change_type {
                ChangeType::Insert => summary.inserts += 1,
                ChangeType::Delete => summary.deletes += 1,
                ChangeType::Update => summary.updates += 1,
                ChangeType::Move => summary.moves += 1,
                ChangeType::Rename => summary.renames += 1,
                ChangeType::Format => summary.formats += 1,
                ChangeType::Extract => summary.extracts += 1,
                ChangeType::Inline => {}
            }
        }

        // Sort changes
        changes.sort_by_key(|c| match c.change_type {
            ChangeType::Delete => 0,
            ChangeType::Rename => 1,
            ChangeType::Update => 2,
            ChangeType::Move => 3,
            ChangeType::Insert => 4,
            _ => 5,
        });

        Ok(DiffReport {
            file_a: self.file_a.display().to_string(),
            file_b: self.file_b.display().to_string(),
            identical: changes.is_empty(),
            changes,
            summary: Some(summary),
            granularity: DiffGranularity::Statement,
            file_changes: None,
            module_changes: None,
            import_graph_summary: None,
            arch_changes: None,
            arch_summary: None,
        })
    }
}

/// Find a function tree-sitter node by its start line number.
fn find_function_node_by_line(root: Node, target_line: u32, lang: Language) -> Option<Node> {
    let func_kinds = get_function_node_kinds(lang);
    find_function_node_recursive(root, target_line, func_kinds)
}

fn find_function_node_recursive<'a>(
    node: Node<'a>,
    target_line: u32,
    func_kinds: &[&str],
) -> Option<Node<'a>> {
    let line = node.start_position().row as u32 + 1;

    if func_kinds.contains(&node.kind()) && line == target_line {
        return Some(node);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = find_function_node_recursive(child, target_line, func_kinds) {
            return Some(found);
        }
    }

    None
}

// =============================================================================
// Class-Level Diff (L5)
// =============================================================================

/// Information about a class extracted from AST for class-level diffing.
#[derive(Debug, Clone)]
struct ClassNode {
    /// Class name
    name: String,
    /// Line number (1-indexed)
    line: u32,
    /// End line number (1-indexed)
    end_line: u32,
    /// Column
    column: u32,
    /// Full source text
    body: String,
    /// Normalized body for comparison
    normalized_body: String,
    /// Methods within this class
    methods: Vec<ExtractedNode>,
    /// Class-level fields (assignments in class body)
    fields: Vec<FieldNode>,
    /// Base classes
    bases: Vec<String>,
}

/// A class-level field (class variable assignment).
#[derive(Debug, Clone)]
struct FieldNode {
    /// Field name
    name: String,
    /// Line number
    line: u32,
    /// Column
    column: u32,
    /// Full text of the assignment
    body: String,
    /// Normalized body
    normalized_body: String,
}

/// Run a class-level diff between two files.
///
/// This is the L5 diff algorithm. It extracts classes from both files,
/// matches them by name, and then diffs their members (methods, fields, bases).
pub fn run_class_diff(file_a: &Path, file_b: &Path, semantic_only: bool) -> Result<DiffReport> {
    // Validate files exist
    if !file_a.exists() {
        return Err(RemainingError::file_not_found(file_a).into());
    }
    if !file_b.exists() {
        return Err(RemainingError::file_not_found(file_b).into());
    }

    // Detect language from file_a extension
    let lang = Language::from_path(file_a).ok_or_else(|| {
        let ext = file_a
            .extension()
            .map(|e| e.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());
        RemainingError::parse_error(file_a, format!("Unsupported language: .{}", ext))
    })?;

    // Read file contents
    let source_a = fs::read_to_string(file_a)?;
    let source_b = fs::read_to_string(file_b)?;

    // Parse both files
    let pool = ParserPool::new();
    let tree_a = pool
        .parse(&source_a, lang)
        .map_err(|e| RemainingError::parse_error(file_a, format!("Failed to parse file: {}", e)))?;
    let tree_b = pool
        .parse(&source_b, lang)
        .map_err(|e| RemainingError::parse_error(file_b, format!("Failed to parse file: {}", e)))?;

    // Extract class information from both files
    let classes_a = extract_class_nodes(tree_a.root_node(), source_a.as_bytes(), lang);
    let classes_b = extract_class_nodes(tree_b.root_node(), source_b.as_bytes(), lang);

    // Detect class-level changes
    let changes = detect_class_changes(&classes_a, &classes_b, file_a, file_b, semantic_only);

    // Build summary
    let mut summary = DiffSummary::default();
    for change in &changes {
        summary.total_changes += 1;
        if change.change_type != ChangeType::Format {
            summary.semantic_changes += 1;
        }
        match change.change_type {
            ChangeType::Insert => summary.inserts += 1,
            ChangeType::Delete => summary.deletes += 1,
            ChangeType::Update => summary.updates += 1,
            ChangeType::Move => summary.moves += 1,
            ChangeType::Rename => summary.renames += 1,
            ChangeType::Format => summary.formats += 1,
            ChangeType::Extract => summary.extracts += 1,
            ChangeType::Inline => {}
        }
    }

    let report = DiffReport {
        file_a: file_a.display().to_string(),
        file_b: file_b.display().to_string(),
        identical: changes.is_empty(),
        changes,
        summary: Some(summary),
        granularity: DiffGranularity::Class,
        file_changes: None,
        module_changes: None,
        import_graph_summary: None,
        arch_changes: None,
        arch_summary: None,
    };

    Ok(report)
}

/// Run class-level diff across two directories, pairing files by relative path.
/// Skips files with unsupported language extensions.
fn run_class_diff_directory(dir_a: &Path, dir_b: &Path, semantic_only: bool) -> Result<DiffReport> {
    let files_a = collect_source_files(dir_a)?;
    let files_b = collect_source_files(dir_b)?;

    let map_a: HashMap<&str, &PathBuf> = files_a.iter().map(|(rel, p)| (rel.as_str(), p)).collect();
    let map_b: HashMap<&str, &PathBuf> = files_b.iter().map(|(rel, p)| (rel.as_str(), p)).collect();

    let all_paths: BTreeSet<&str> = map_a.keys().chain(map_b.keys()).copied().collect();

    let mut all_changes = Vec::new();

    for rel_path in all_paths {
        match (map_a.get(rel_path), map_b.get(rel_path)) {
            (Some(path_a), Some(path_b)) => {
                // File exists in both -- run class diff, skip on language error
                match run_class_diff(path_a, path_b, semantic_only) {
                    Ok(sub_report) => all_changes.extend(sub_report.changes),
                    Err(_) => continue, // unsupported language, skip
                }
            }
            (None, Some(_)) | (Some(_), None) => {
                // Added or removed file -- skip at class level (L6 handles file-level adds/removes)
                continue;
            }
            (None, None) => unreachable!(),
        }
    }

    let mut summary = DiffSummary::default();
    for change in &all_changes {
        summary.total_changes += 1;
        if change.change_type != ChangeType::Format {
            summary.semantic_changes += 1;
        }
        match change.change_type {
            ChangeType::Insert => summary.inserts += 1,
            ChangeType::Delete => summary.deletes += 1,
            ChangeType::Update => summary.updates += 1,
            ChangeType::Move => summary.moves += 1,
            ChangeType::Rename => summary.renames += 1,
            ChangeType::Format => summary.formats += 1,
            ChangeType::Extract => summary.extracts += 1,
            ChangeType::Inline => {}
        }
    }

    Ok(DiffReport {
        file_a: dir_a.display().to_string(),
        file_b: dir_b.display().to_string(),
        identical: all_changes.is_empty(),
        changes: all_changes,
        summary: Some(summary),
        granularity: DiffGranularity::Class,
        file_changes: None,
        module_changes: None,
        import_graph_summary: None,
        arch_changes: None,
        arch_summary: None,
    })
}

/// Extract class nodes with their members from the AST.
fn extract_class_nodes(root: Node, source: &[u8], lang: Language) -> Vec<ClassNode> {
    let mut classes = Vec::new();
    let class_kinds = get_class_node_kinds(lang);
    let func_kinds = get_function_node_kinds(lang);
    let body_kinds = get_class_body_kinds(lang);

    extract_class_nodes_recursive(
        root,
        source,
        &mut classes,
        lang,
        func_kinds,
        class_kinds,
        body_kinds,
    );

    // Go: methods are declared at file level with receiver syntax, not inside the struct.
    // Scan root-level method_declaration nodes and associate them with their struct.
    if lang == Language::Go {
        associate_go_receiver_methods(root, source, lang, &mut classes);
    }

    classes
}

/// For Go, scan file-level `method_declaration` nodes, parse the receiver type,
/// and associate each method with the matching struct's ClassNode.
fn associate_go_receiver_methods(
    root: Node,
    source: &[u8],
    lang: Language,
    classes: &mut [ClassNode],
) {
    let source_str = std::str::from_utf8(source).unwrap_or("");
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() != "method_declaration" {
            continue;
        }
        // Extract receiver type name
        let receiver_type = match extract_go_receiver_type(child, source) {
            Some(name) => name,
            None => continue,
        };

        // Extract method name and build an ExtractedNode
        let method_name = match get_function_name(child, lang, source_str) {
            Some(name) => name,
            None => continue,
        };

        let params = child
            .child_by_field_name("parameters")
            .map(|p| node_text(p, source).to_string())
            .unwrap_or_default();

        let line = child.start_position().row as u32 + 1;
        let end_line = child.end_position().row as u32 + 1;
        let column = child.start_position().column as u32;
        let body = node_text(child, source).to_string();

        let extracted =
            ExtractedNode::new(method_name, NodeKind::Method, line, end_line, column, body)
                .with_params(params)
                .with_method_kind();

        // Associate with matching struct
        for class in classes.iter_mut() {
            if class.name == receiver_type {
                class.methods.push(extracted);
                break;
            }
        }
    }
}

/// Extract the receiver type name from a Go method_declaration node.
///
/// Handles both pointer receivers `(f *Foo)` and value receivers `(f Foo)`.
/// Returns the bare type name (e.g., "Foo") without the pointer `*`.
fn extract_go_receiver_type(method_node: Node, source: &[u8]) -> Option<String> {
    // method_declaration -> receiver: parameter_list -> parameter_declaration -> type
    let receiver = method_node.child_by_field_name("receiver")?;
    let mut recv_cursor = receiver.walk();
    for recv_child in receiver.children(&mut recv_cursor) {
        if recv_child.kind() == "parameter_declaration" {
            if let Some(type_node) = recv_child.child_by_field_name("type") {
                return extract_go_type_identifier(type_node, source);
            }
        }
    }
    None
}

/// Recursively extract the type_identifier from a Go type node,
/// handling pointer_type wrappers.
fn extract_go_type_identifier(type_node: Node, source: &[u8]) -> Option<String> {
    match type_node.kind() {
        "type_identifier" => Some(node_text(type_node, source).to_string()),
        "pointer_type" => {
            // pointer_type has a single named child which is the underlying type
            let mut cursor = type_node.walk();
            for child in type_node.children(&mut cursor) {
                if child.is_named() {
                    return extract_go_type_identifier(child, source);
                }
            }
            None
        }
        _ => None,
    }
}

fn extract_class_nodes_recursive(
    node: Node,
    source: &[u8],
    classes: &mut Vec<ClassNode>,
    lang: Language,
    func_kinds: &[&str],
    class_kinds: &[&str],
    body_kinds: &[&str],
) {
    let kind = node.kind();

    if class_kinds.contains(&kind) {
        if let Some(class_node) = build_class_node(node, source, lang, func_kinds, body_kinds) {
            classes.push(class_node);
        }
        return; // Don't recurse into class children for nested classes at this level
    }

    for child in node.children(&mut node.walk()) {
        extract_class_nodes_recursive(
            child,
            source,
            classes,
            lang,
            func_kinds,
            class_kinds,
            body_kinds,
        );
    }
}

/// Build a ClassNode from a tree-sitter class node.
fn build_class_node(
    node: Node,
    source: &[u8],
    lang: Language,
    func_kinds: &[&str],
    body_kinds: &[&str],
) -> Option<ClassNode> {
    // Get class name
    let class_name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, source).to_string())
        .or_else(|| {
            // Go: type_declaration has no "name" field; the name is in
            // the child type_spec node's "name" field.
            if lang == Language::Go && node.kind() == "type_declaration" {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "type_spec" {
                        if let Some(name_node) = child.child_by_field_name("name") {
                            return Some(node_text(name_node, source).to_string());
                        }
                    }
                }
            }
            // Fallback: search for first identifier child
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier"
                    || child.kind() == "type_identifier"
                    || child.kind() == "constant"
                {
                    return Some(node_text(child, source).to_string());
                }
            }
            None
        })?;

    if class_name.is_empty() {
        return None;
    }

    let line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;
    let column = node.start_position().column as u32;
    let body = node_text(node, source).to_string();
    let normalized_body = normalize_body(&body);

    // Extract base classes
    let bases = extract_bases(node, source, lang);

    // Extract methods and fields from class body
    let mut methods = Vec::new();
    let mut fields = Vec::new();

    for child in node.children(&mut node.walk()) {
        if body_kinds.contains(&child.kind()) {
            extract_class_members(child, source, lang, func_kinds, &mut methods, &mut fields);
        }
    }

    Some(ClassNode {
        name: class_name,
        line,
        end_line,
        column,
        body,
        normalized_body,
        methods,
        fields,
        bases,
    })
}

/// Extract base classes from a class definition node.
fn extract_bases(node: Node, source: &[u8], lang: Language) -> Vec<String> {
    let mut bases = Vec::new();

    match lang {
        Language::Python => {
            // Python: class Foo(Base1, Base2):
            // Look for argument_list or superclasses
            if let Some(superclasses) = node.child_by_field_name("superclasses") {
                for child in superclasses.children(&mut superclasses.walk()) {
                    let text = node_text(child, source).trim().to_string();
                    if !text.is_empty() && text != "(" && text != ")" && text != "," {
                        bases.push(text);
                    }
                }
            }
        }
        _ => {
            // For other languages, base extraction would be different
            // For now, only Python is fully supported for class-level diff
        }
    }

    bases
}

/// Extract methods and fields from a class body.
fn extract_class_members(
    body_node: Node,
    source: &[u8],
    lang: Language,
    func_kinds: &[&str],
    methods: &mut Vec<ExtractedNode>,
    fields: &mut Vec<FieldNode>,
) {
    for child in body_node.children(&mut body_node.walk()) {
        let kind = child.kind();

        // Extract methods
        if func_kinds.contains(&kind) {
            let source_str = std::str::from_utf8(source).unwrap_or("");
            if let Some(func_name) = get_function_name(child, lang, source_str) {
                let params = child
                    .child_by_field_name("parameters")
                    .or_else(|| child.child_by_field_name("formal_parameters"))
                    .map(|p| node_text(p, source).to_string())
                    .unwrap_or_default();

                let line = child.start_position().row as u32 + 1;
                let end_line = child.end_position().row as u32 + 1;
                let column = child.start_position().column as u32;
                let body = node_text(child, source).to_string();

                let extracted =
                    ExtractedNode::new(func_name, NodeKind::Method, line, end_line, column, body)
                        .with_params(params)
                        .with_method_kind();

                methods.push(extracted);
            }
        }
        // Extract fields (Python: expression_statement with assignment)
        else if kind == "expression_statement" {
            if let Some(field) = extract_field_from_statement(child, source, lang) {
                fields.push(field);
            }
        }
    }
}

/// Extract a field from a statement node (e.g., `timeout = 30`).
fn extract_field_from_statement(node: Node, source: &[u8], _lang: Language) -> Option<FieldNode> {
    // Look for assignment in this expression_statement
    for child in node.children(&mut node.walk()) {
        if child.kind() == "assignment" {
            // Get the left side (field name)
            if let Some(left) = child.child_by_field_name("left") {
                let name = node_text(left, source).trim().to_string();
                if !name.is_empty() && !name.contains('.') {
                    // Skip `self.x = ...` (those are instance vars, not class fields)
                    let line = node.start_position().row as u32 + 1;
                    let column = node.start_position().column as u32;
                    let body = node_text(node, source).to_string();
                    let normalized_body = body.trim().to_string();

                    return Some(FieldNode {
                        name,
                        line,
                        column,
                        body,
                        normalized_body,
                    });
                }
            }
        }
    }
    None
}

/// Detect changes between two sets of class nodes.
fn detect_class_changes(
    classes_a: &[ClassNode],
    classes_b: &[ClassNode],
    file_a: &Path,
    file_b: &Path,
    _semantic_only: bool,
) -> Vec<ASTChange> {
    let mut changes = Vec::new();

    // Build lookup maps by name
    let map_b: HashMap<&str, &ClassNode> = classes_b.iter().map(|c| (c.name.as_str(), c)).collect();

    // Track which classes have been matched
    let mut matched_a: Vec<bool> = vec![false; classes_a.len()];
    let mut matched_b: Vec<bool> = vec![false; classes_b.len()];

    // First pass: exact name matches
    for (i, class_a) in classes_a.iter().enumerate() {
        let _ = class_a.end_line;
        let _ = &class_a.body;
        let _ = &class_a.normalized_body;
        if let Some(&class_b) = map_b.get(class_a.name.as_str()) {
            matched_a[i] = true;
            if let Some(j) = classes_b.iter().position(|c| c.name == class_a.name) {
                matched_b[j] = true;
            }

            // Diff the matched pair
            if let Some(change) = diff_class_pair(class_a, class_b, file_a, file_b) {
                changes.push(change);
            }
        }
    }

    // Collect unmatched classes
    let unmatched_a: Vec<(usize, &ClassNode)> = classes_a
        .iter()
        .enumerate()
        .filter(|(i, _)| !matched_a[*i])
        .collect();
    let unmatched_b: Vec<(usize, &ClassNode)> = classes_b
        .iter()
        .enumerate()
        .filter(|(i, _)| !matched_b[*i])
        .collect();

    // Second pass: detect renames (same member signatures, different name)
    let mut used_b: Vec<bool> = vec![false; unmatched_b.len()];

    for (_, class_a) in &unmatched_a {
        let mut best_match: Option<(usize, f64)> = None;

        for (j, (_, class_b)) in unmatched_b.iter().enumerate() {
            if used_b[j] {
                continue;
            }

            let similarity = compute_class_similarity(class_a, class_b);
            if similarity >= RENAME_SIMILARITY_THRESHOLD
                && (best_match.is_none() || similarity > best_match.unwrap().1)
            {
                best_match = Some((j, similarity));
            }
        }

        if let Some((j, similarity)) = best_match {
            let (_, class_b) = unmatched_b[j];
            used_b[j] = true;

            changes.push(ASTChange {
                change_type: ChangeType::Rename,
                node_kind: NodeKind::Class,
                name: Some(class_a.name.clone()),
                old_location: Some(Location::with_column(
                    file_a.display().to_string(),
                    class_a.line,
                    class_a.column,
                )),
                new_location: Some(Location::with_column(
                    file_b.display().to_string(),
                    class_b.line,
                    class_b.column,
                )),
                old_text: Some(class_a.name.clone()),
                new_text: Some(class_b.name.clone()),
                similarity: Some(similarity),
                children: None,
                base_changes: None,
            });
        }
    }

    // Remaining unmatched in A are deletes
    for (_, class_a) in &unmatched_a {
        let is_renamed = changes
            .iter()
            .any(|c| c.change_type == ChangeType::Rename && c.name.as_ref() == Some(&class_a.name));
        if !is_renamed {
            changes.push(ASTChange {
                change_type: ChangeType::Delete,
                node_kind: NodeKind::Class,
                name: Some(class_a.name.clone()),
                old_location: Some(Location::with_column(
                    file_a.display().to_string(),
                    class_a.line,
                    class_a.column,
                )),
                new_location: None,
                old_text: None,
                new_text: None,
                similarity: None,
                children: None,
                base_changes: None,
            });
        }
    }

    // Remaining unmatched in B are inserts
    for (j, (_, class_b)) in unmatched_b.iter().enumerate() {
        if !used_b[j] {
            changes.push(ASTChange {
                change_type: ChangeType::Insert,
                node_kind: NodeKind::Class,
                name: Some(class_b.name.clone()),
                old_location: None,
                new_location: Some(Location::with_column(
                    file_b.display().to_string(),
                    class_b.line,
                    class_b.column,
                )),
                old_text: None,
                new_text: None,
                similarity: None,
                children: None,
                base_changes: None,
            });
        }
    }

    // Sort changes: deletes first, then renames, updates, inserts
    changes.sort_by_key(|c| match c.change_type {
        ChangeType::Delete => 0,
        ChangeType::Rename => 1,
        ChangeType::Update => 2,
        ChangeType::Move => 3,
        ChangeType::Insert => 4,
        _ => 5,
    });

    changes
}

/// Diff two matched classes and produce an ASTChange if they differ.
fn diff_class_pair(
    class_a: &ClassNode,
    class_b: &ClassNode,
    file_a: &Path,
    file_b: &Path,
) -> Option<ASTChange> {
    let mut children = Vec::new();
    let mut has_changes = false;

    // 1. Diff methods
    diff_methods(
        &class_a.methods,
        &class_b.methods,
        file_a,
        file_b,
        &mut children,
    );

    // 2. Diff fields
    diff_fields(
        &class_a.fields,
        &class_b.fields,
        file_a,
        file_b,
        &mut children,
    );

    // 3. Diff base classes
    let base_changes = diff_bases(&class_a.bases, &class_b.bases);

    if !children.is_empty() {
        has_changes = true;
    }
    if base_changes.is_some() {
        has_changes = true;
    }

    if !has_changes {
        return None; // Classes are identical
    }

    Some(ASTChange {
        change_type: ChangeType::Update,
        node_kind: NodeKind::Class,
        name: Some(class_a.name.clone()),
        old_location: Some(Location::with_column(
            file_a.display().to_string(),
            class_a.line,
            class_a.column,
        )),
        new_location: Some(Location::with_column(
            file_b.display().to_string(),
            class_b.line,
            class_b.column,
        )),
        old_text: None,
        new_text: None,
        similarity: None,
        children: if children.is_empty() {
            None
        } else {
            Some(children)
        },
        base_changes,
    })
}

/// Diff methods between two matched classes.
fn diff_methods(
    methods_a: &[ExtractedNode],
    methods_b: &[ExtractedNode],
    file_a: &Path,
    file_b: &Path,
    children: &mut Vec<ASTChange>,
) {
    let map_b: HashMap<&str, &ExtractedNode> =
        methods_b.iter().map(|m| (m.name.as_str(), m)).collect();

    let mut matched_a: Vec<bool> = vec![false; methods_a.len()];
    let mut matched_b: Vec<bool> = vec![false; methods_b.len()];

    // Exact name match
    for (i, method_a) in methods_a.iter().enumerate() {
        if let Some(&method_b) = map_b.get(method_a.name.as_str()) {
            matched_a[i] = true;
            if let Some(j) = methods_b.iter().position(|m| m.name == method_a.name) {
                matched_b[j] = true;
            }

            // Check if body changed
            if method_a.normalized_body != method_b.normalized_body {
                children.push(ASTChange {
                    change_type: ChangeType::Update,
                    node_kind: NodeKind::Method,
                    name: Some(method_a.name.clone()),
                    old_location: Some(Location::with_column(
                        file_a.display().to_string(),
                        method_a.line,
                        method_a.column,
                    )),
                    new_location: Some(Location::with_column(
                        file_b.display().to_string(),
                        method_b.line,
                        method_b.column,
                    )),
                    old_text: None,
                    new_text: None,
                    similarity: Some(compute_similarity(
                        &method_a.normalized_body,
                        &method_b.normalized_body,
                    )),
                    children: None,
                    base_changes: None,
                });
            }
        }
    }

    // Collect unmatched
    let unmatched_a: Vec<&ExtractedNode> = methods_a
        .iter()
        .enumerate()
        .filter(|(i, _)| !matched_a[*i])
        .map(|(_, m)| m)
        .collect();
    let unmatched_b: Vec<&ExtractedNode> = methods_b
        .iter()
        .enumerate()
        .filter(|(i, _)| !matched_b[*i])
        .map(|(_, m)| m)
        .collect();

    // Rename detection among unmatched methods
    let mut used_b: Vec<bool> = vec![false; unmatched_b.len()];

    for method_a in &unmatched_a {
        let mut best_match: Option<(usize, f64)> = None;

        for (j, method_b) in unmatched_b.iter().enumerate() {
            if used_b[j] {
                continue;
            }
            let similarity =
                compute_similarity(&method_a.normalized_body, &method_b.normalized_body);
            if similarity >= RENAME_SIMILARITY_THRESHOLD
                && (best_match.is_none() || similarity > best_match.unwrap().1)
            {
                best_match = Some((j, similarity));
            }
        }

        if let Some((j, similarity)) = best_match {
            let method_b = unmatched_b[j];
            used_b[j] = true;

            children.push(ASTChange {
                change_type: ChangeType::Rename,
                node_kind: NodeKind::Method,
                name: Some(method_a.name.clone()),
                old_location: Some(Location::with_column(
                    file_a.display().to_string(),
                    method_a.line,
                    method_a.column,
                )),
                new_location: Some(Location::with_column(
                    file_b.display().to_string(),
                    method_b.line,
                    method_b.column,
                )),
                old_text: Some(method_a.name.clone()),
                new_text: Some(method_b.name.clone()),
                similarity: Some(similarity),
                children: None,
                base_changes: None,
            });
        }
    }

    // Remaining unmatched in A are deletes
    for method_a in &unmatched_a {
        let is_renamed = children.iter().any(|c| {
            c.change_type == ChangeType::Rename && c.name.as_ref() == Some(&method_a.name)
        });
        if !is_renamed {
            children.push(ASTChange {
                change_type: ChangeType::Delete,
                node_kind: NodeKind::Method,
                name: Some(method_a.name.clone()),
                old_location: Some(Location::with_column(
                    file_a.display().to_string(),
                    method_a.line,
                    method_a.column,
                )),
                new_location: None,
                old_text: None,
                new_text: None,
                similarity: None,
                children: None,
                base_changes: None,
            });
        }
    }

    // Remaining unmatched in B are inserts
    for (j, method_b) in unmatched_b.iter().enumerate() {
        if !used_b[j] {
            children.push(ASTChange {
                change_type: ChangeType::Insert,
                node_kind: NodeKind::Method,
                name: Some(method_b.name.clone()),
                old_location: None,
                new_location: Some(Location::with_column(
                    file_b.display().to_string(),
                    method_b.line,
                    method_b.column,
                )),
                old_text: None,
                new_text: None,
                similarity: None,
                children: None,
                base_changes: None,
            });
        }
    }
}

/// Diff fields between two matched classes.
fn diff_fields(
    fields_a: &[FieldNode],
    fields_b: &[FieldNode],
    file_a: &Path,
    file_b: &Path,
    children: &mut Vec<ASTChange>,
) {
    let map_b: HashMap<&str, &FieldNode> = fields_b.iter().map(|f| (f.name.as_str(), f)).collect();

    let mut matched_a: Vec<bool> = vec![false; fields_a.len()];
    let mut matched_b: Vec<bool> = vec![false; fields_b.len()];

    // Exact name match
    for (i, field_a) in fields_a.iter().enumerate() {
        if let Some(&field_b) = map_b.get(field_a.name.as_str()) {
            matched_a[i] = true;
            if let Some(j) = fields_b.iter().position(|f| f.name == field_a.name) {
                matched_b[j] = true;
            }

            // Check if value changed
            if field_a.normalized_body != field_b.normalized_body {
                children.push(ASTChange {
                    change_type: ChangeType::Update,
                    node_kind: NodeKind::Field,
                    name: Some(field_a.name.clone()),
                    old_location: Some(Location::with_column(
                        file_a.display().to_string(),
                        field_a.line,
                        field_a.column,
                    )),
                    new_location: Some(Location::with_column(
                        file_b.display().to_string(),
                        field_b.line,
                        field_b.column,
                    )),
                    old_text: Some(field_a.body.trim().to_string()),
                    new_text: Some(field_b.body.trim().to_string()),
                    similarity: None,
                    children: None,
                    base_changes: None,
                });
            }
        }
    }

    // Remaining unmatched in A are deletes
    for (i, field_a) in fields_a.iter().enumerate() {
        if !matched_a[i] {
            children.push(ASTChange {
                change_type: ChangeType::Delete,
                node_kind: NodeKind::Field,
                name: Some(field_a.name.clone()),
                old_location: Some(Location::with_column(
                    file_a.display().to_string(),
                    field_a.line,
                    field_a.column,
                )),
                new_location: None,
                old_text: None,
                new_text: None,
                similarity: None,
                children: None,
                base_changes: None,
            });
        }
    }

    // Remaining unmatched in B are inserts
    for (j, field_b) in fields_b.iter().enumerate() {
        if !matched_b[j] {
            children.push(ASTChange {
                change_type: ChangeType::Insert,
                node_kind: NodeKind::Field,
                name: Some(field_b.name.clone()),
                old_location: None,
                new_location: Some(Location::with_column(
                    file_b.display().to_string(),
                    field_b.line,
                    field_b.column,
                )),
                old_text: None,
                new_text: None,
                similarity: None,
                children: None,
                base_changes: None,
            });
        }
    }
}

/// Diff base classes between two matched classes.
fn diff_bases(bases_a: &[String], bases_b: &[String]) -> Option<BaseChanges> {
    let set_a: std::collections::HashSet<&String> = bases_a.iter().collect();
    let set_b: std::collections::HashSet<&String> = bases_b.iter().collect();

    let added: Vec<String> = set_b.difference(&set_a).map(|s| (*s).clone()).collect();
    let removed: Vec<String> = set_a.difference(&set_b).map(|s| (*s).clone()).collect();

    if added.is_empty() && removed.is_empty() {
        None
    } else {
        Some(BaseChanges { added, removed })
    }
}

/// Compute similarity between two classes based on their member signatures.
fn compute_class_similarity(class_a: &ClassNode, class_b: &ClassNode) -> f64 {
    // Collect method names + normalized bodies
    let method_sigs_a: std::collections::HashSet<String> = class_a
        .methods
        .iter()
        .map(|m| format!("{}:{}", m.name, m.normalized_body))
        .collect();
    let method_sigs_b: std::collections::HashSet<String> = class_b
        .methods
        .iter()
        .map(|m| format!("{}:{}", m.name, m.normalized_body))
        .collect();

    let field_sigs_a: std::collections::HashSet<String> = class_a
        .fields
        .iter()
        .map(|f| f.normalized_body.clone())
        .collect();
    let field_sigs_b: std::collections::HashSet<String> = class_b
        .fields
        .iter()
        .map(|f| f.normalized_body.clone())
        .collect();

    // Combined Jaccard similarity
    let all_a: std::collections::HashSet<&String> =
        method_sigs_a.iter().chain(field_sigs_a.iter()).collect();
    let all_b: std::collections::HashSet<&String> =
        method_sigs_b.iter().chain(field_sigs_b.iter()).collect();

    if all_a.is_empty() && all_b.is_empty() {
        // Both empty classes - consider identical
        return 1.0;
    }

    let intersection = all_a.intersection(&all_b).count();
    let union = all_a.union(&all_b).count();

    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

// =============================================================================
// L6: File-Level Diff
// =============================================================================

/// Recognized source file extensions for directory walking.
const SOURCE_EXTENSIONS: &[&str] = &[
    "py", "rs", "ts", "tsx", "js", "jsx", "go", "java", "c", "h", "cpp", "hpp", "cc", "cxx", "rb",
    "php", "cs", "kt", "scala", "swift", "ex", "exs", "lua", "ml", "mli", "luau",
];

/// Walk a directory and collect source files with their relative paths.
fn collect_source_files(root: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut files = Vec::new();
    collect_source_files_recursive(root, root, &mut files)?;
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(files)
}

fn collect_source_files_recursive(
    root: &Path,
    current: &Path,
    files: &mut Vec<(String, PathBuf)>,
) -> Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_source_files_recursive(root, &path, files)?;
        } else if path.is_file() {
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if SOURCE_EXTENSIONS.contains(&ext) {
                    let rel = path
                        .strip_prefix(root)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .replace('\\', "/");
                    files.push((rel, path));
                }
            }
        }
    }
    Ok(())
}

/// Compute a structural fingerprint for a source file.
///
/// The fingerprint is a hash of the sorted list of function/class signatures
/// extracted via tree-sitter. Two files with the same structural definitions
/// (regardless of whitespace/comments) produce the same fingerprint.
fn compute_structural_fingerprint(path: &Path) -> Result<(u64, Vec<String>)> {
    let lang = match Language::from_path(path) {
        Some(l) => l,
        None => {
            // Fallback: hash the raw content for unsupported languages
            let content = fs::read_to_string(path)?;
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            content.hash(&mut hasher);
            return Ok((hasher.finish(), vec![]));
        }
    };

    let source = fs::read_to_string(path)?;
    let pool = ParserPool::new();
    let tree = match pool.parse(&source, lang) {
        Ok(t) => t,
        Err(_) => {
            // Parse failure: hash raw content
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            source.hash(&mut hasher);
            return Ok((hasher.finish(), vec![]));
        }
    };

    let nodes = extract_nodes(tree.root_node(), source.as_bytes(), lang);

    // Build sorted list of signatures: "kind:name(params)|body_hash"
    // We include a hash of the normalized body so that body-only changes
    // (same name/params but different implementation) alter the fingerprint.
    let mut signatures: Vec<String> = nodes
        .iter()
        .map(|n| {
            let kind = match n.kind {
                NodeKind::Function => "fn",
                NodeKind::Class => "class",
                NodeKind::Method => "method",
                NodeKind::Field => "field",
                _ => "other",
            };
            let sig = if n.params.is_empty() {
                format!("{}:{}", kind, n.name)
            } else {
                format!("{}:{}({})", kind, n.name, n.params)
            };
            // Append a body hash so body-only changes are detected
            let mut body_hasher = std::collections::hash_map::DefaultHasher::new();
            n.normalized_body.hash(&mut body_hasher);
            format!("{}|{}", sig, body_hasher.finish())
        })
        .collect();
    signatures.sort();

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for sig in &signatures {
        sig.hash(&mut hasher);
    }
    let fingerprint = hasher.finish();

    Ok((fingerprint, signatures))
}

/// Run L6 file-level diff between two directories.
fn run_file_level_diff(dir_a: &Path, dir_b: &Path) -> Result<DiffReport> {
    let files_a = collect_source_files(dir_a)?;
    let files_b = collect_source_files(dir_b)?;

    // Build maps: relative_path -> full_path
    let map_a: HashMap<&str, &PathBuf> = files_a.iter().map(|(rel, p)| (rel.as_str(), p)).collect();
    let map_b: HashMap<&str, &PathBuf> = files_b.iter().map(|(rel, p)| (rel.as_str(), p)).collect();

    let all_paths: BTreeSet<&str> = map_a.keys().chain(map_b.keys()).copied().collect();

    let mut file_changes = Vec::new();
    let mut has_any_change = false;

    for rel_path in all_paths {
        match (map_a.get(rel_path), map_b.get(rel_path)) {
            (Some(path_a), Some(path_b)) => {
                // File exists in both directories
                let (fp_a, sigs_a) = compute_structural_fingerprint(path_a)?;
                let (fp_b, sigs_b) = compute_structural_fingerprint(path_b)?;

                if fp_a == fp_b {
                    // Identical structure - skip or include as no-change
                    // (tests filter these out anyway)
                } else {
                    has_any_change = true;
                    // Find which signatures differ
                    let set_a: HashSet<&String> = sigs_a.iter().collect();
                    let set_b: HashSet<&String> = sigs_b.iter().collect();
                    let changed: Vec<String> = set_a
                        .symmetric_difference(&set_b)
                        .map(|s| (*s).clone())
                        .collect();

                    file_changes.push(FileLevelChange {
                        relative_path: rel_path.to_string(),
                        change_type: ChangeType::Update,
                        old_fingerprint: Some(fp_a),
                        new_fingerprint: Some(fp_b),
                        signature_changes: if changed.is_empty() {
                            None
                        } else {
                            Some(changed)
                        },
                    });
                }
            }
            (None, Some(path_b)) => {
                // Added file
                has_any_change = true;
                let (fp_b, _) = compute_structural_fingerprint(path_b)?;
                file_changes.push(FileLevelChange {
                    relative_path: rel_path.to_string(),
                    change_type: ChangeType::Insert,
                    old_fingerprint: None,
                    new_fingerprint: Some(fp_b),
                    signature_changes: None,
                });
            }
            (Some(path_a), None) => {
                // Removed file
                has_any_change = true;
                let (fp_a, _) = compute_structural_fingerprint(path_a)?;
                file_changes.push(FileLevelChange {
                    relative_path: rel_path.to_string(),
                    change_type: ChangeType::Delete,
                    old_fingerprint: Some(fp_a),
                    new_fingerprint: None,
                    signature_changes: None,
                });
            }
            (None, None) => unreachable!(),
        }
    }

    Ok(DiffReport {
        file_a: dir_a.display().to_string(),
        file_b: dir_b.display().to_string(),
        identical: !has_any_change,
        changes: Vec::new(),
        summary: None,
        granularity: DiffGranularity::File,
        file_changes: Some(file_changes),
        module_changes: None,
        import_graph_summary: None,
        arch_changes: None,
        arch_summary: None,
    })
}

// =============================================================================
// L7: Module-Level Diff
// =============================================================================

/// An import edge used internally during graph building.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct InternalImportEdge {
    source_file: String,
    target_module: String,
    imported_names: Vec<String>,
}

/// Parse Python import statements from a file using regex.
///
/// Recognizes:
/// - `from X import Y, Z`
/// - `import X`
fn parse_python_imports(source: &str, relative_path: &str) -> Vec<InternalImportEdge> {
    let mut edges = Vec::new();

    // Match "from X import Y, Z"
    let from_re = Regex::new(r"(?m)^(?:\s*)from\s+([\w.]+)\s+import\s+(.+)$").unwrap();
    for cap in from_re.captures_iter(source) {
        let target = cap[1].to_string();
        let names_str = &cap[2];
        let names: Vec<String> = names_str
            .split(',')
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty())
            .collect();
        edges.push(InternalImportEdge {
            source_file: relative_path.to_string(),
            target_module: target,
            imported_names: names,
        });
    }

    // Match "import X" (but not "from X import Y" which is already handled)
    let import_re = Regex::new(r"(?m)^(?:\s*)import\s+([\w.]+)$").unwrap();
    for cap in import_re.captures_iter(source) {
        let target = cap[1].to_string();
        edges.push(InternalImportEdge {
            source_file: relative_path.to_string(),
            target_module: target,
            imported_names: vec![],
        });
    }

    edges
}

/// Parse imports for a single file using CallGraphLanguageSupport.
///
/// Returns `Some(edges)` if a handler could parse the file, `None` otherwise.
/// On handler parse failure for Python files, falls back to regex parsing.
fn parse_file_imports(
    registry: &LanguageRegistry,
    source: &str,
    full_path: &Path,
    rel_path: &str,
) -> Vec<InternalImportEdge> {
    let ext = match full_path.extension().and_then(|e| e.to_str()) {
        Some(e) => format!(".{}", e),
        None => return Vec::new(),
    };

    let is_python = ext == ".py" || ext == ".pyi";

    // Try the language handler from the registry
    if let Some(handler) = registry.get_by_extension(&ext) {
        if let Ok(import_defs) = handler.parse_imports(source, full_path) {
            return import_defs
                .into_iter()
                .map(|def| InternalImportEdge {
                    source_file: rel_path.to_string(),
                    target_module: def.module,
                    imported_names: def.names,
                })
                .collect();
        }
    }

    // Fallback: regex-based parsing for Python files only
    if is_python {
        return parse_python_imports(source, rel_path);
    }

    Vec::new()
}

/// Build import graph for all source files in a directory.
///
/// Uses `CallGraphLanguageSupport::parse_imports()` from tldr-core for
/// multi-language support (Python, TypeScript, Go, Rust, Java, C#, etc.).
/// Falls back to regex-based `parse_python_imports()` for Python files
/// when the core API fails, and skips import parsing for files whose
/// language is unsupported or whose handler returns an error.
fn build_import_graph(root: &Path) -> Result<Vec<InternalImportEdge>> {
    let files = collect_source_files(root)?;
    let registry = LanguageRegistry::with_defaults();
    let mut all_edges = Vec::new();

    for (rel_path, full_path) in &files {
        let source = fs::read_to_string(full_path)?;
        let edges = parse_file_imports(&registry, &source, full_path, rel_path);
        all_edges.extend(edges);
    }

    Ok(all_edges)
}

/// Convert an internal edge to the public ImportEdge type.
fn to_public_edge(edge: &InternalImportEdge) -> ImportEdge {
    ImportEdge {
        source_file: edge.source_file.clone(),
        target_module: edge.target_module.clone(),
        imported_names: edge.imported_names.clone(),
    }
}

/// Create a comparable key for an import edge (for set operations).
fn edge_key(edge: &InternalImportEdge) -> String {
    format!(
        "{}->{}:{}",
        edge.source_file,
        edge.target_module,
        edge.imported_names.join(",")
    )
}

/// Run L7 module-level diff between two directories.
fn run_module_level_diff(dir_a: &Path, dir_b: &Path) -> Result<DiffReport> {
    // Build import graphs
    let edges_a = build_import_graph(dir_a)?;
    let edges_b = build_import_graph(dir_b)?;

    // Build edge key sets for comparison
    let keys_a: HashSet<String> = edges_a.iter().map(edge_key).collect();
    let keys_b: HashSet<String> = edges_b.iter().map(edge_key).collect();

    // Edges added (in B but not in A)
    let added_keys: HashSet<&String> = keys_b.difference(&keys_a).collect();
    let removed_keys: HashSet<&String> = keys_a.difference(&keys_b).collect();

    // Get added/removed edges
    let added_edges: Vec<&InternalImportEdge> = edges_b
        .iter()
        .filter(|e| added_keys.contains(&edge_key(e)))
        .collect();
    let removed_edges: Vec<&InternalImportEdge> = edges_a
        .iter()
        .filter(|e| removed_keys.contains(&edge_key(e)))
        .collect();

    // Also run L6 file-level diff for context
    let files_a = collect_source_files(dir_a)?;
    let files_b = collect_source_files(dir_b)?;
    let map_a: HashMap<&str, &PathBuf> = files_a.iter().map(|(r, p)| (r.as_str(), p)).collect();
    let map_b: HashMap<&str, &PathBuf> = files_b.iter().map(|(r, p)| (r.as_str(), p)).collect();
    let all_paths: BTreeSet<&str> = map_a.keys().chain(map_b.keys()).copied().collect();

    // Build per-module changes
    let mut module_changes: Vec<ModuleLevelChange> = Vec::new();
    let mut modules_with_import_changes = 0usize;

    for rel_path in &all_paths {
        let in_a = map_a.contains_key(rel_path);
        let in_b = map_b.contains_key(rel_path);

        // Determine module change type
        let change_type = if !in_a && in_b {
            ChangeType::Insert
        } else if in_a && !in_b {
            ChangeType::Delete
        } else {
            ChangeType::Update
        };

        // Gather import changes for this module
        let mod_added: Vec<ImportEdge> = added_edges
            .iter()
            .filter(|e| e.source_file == *rel_path)
            .map(|e| to_public_edge(e))
            .collect();
        let mod_removed: Vec<ImportEdge> = removed_edges
            .iter()
            .filter(|e| e.source_file == *rel_path)
            .map(|e| to_public_edge(e))
            .collect();

        // Compute file-level change if both exist
        let file_change = if in_a && in_b {
            let path_a = map_a[rel_path];
            let path_b = map_b[rel_path];
            let (fp_a, sigs_a) = compute_structural_fingerprint(path_a)?;
            let (fp_b, sigs_b) = compute_structural_fingerprint(path_b)?;
            if fp_a != fp_b {
                let set_a: HashSet<&String> = sigs_a.iter().collect();
                let set_b: HashSet<&String> = sigs_b.iter().collect();
                let changed: Vec<String> = set_a
                    .symmetric_difference(&set_b)
                    .map(|s| (*s).clone())
                    .collect();
                Some(FileLevelChange {
                    relative_path: rel_path.to_string(),
                    change_type: ChangeType::Update,
                    old_fingerprint: Some(fp_a),
                    new_fingerprint: Some(fp_b),
                    signature_changes: if changed.is_empty() {
                        None
                    } else {
                        Some(changed)
                    },
                })
            } else {
                None
            }
        } else {
            None
        };

        // Only include modules with actual changes
        let has_import_changes = !mod_added.is_empty() || !mod_removed.is_empty();
        let has_file_change = file_change.is_some();
        let is_new_or_deleted =
            change_type == ChangeType::Insert || change_type == ChangeType::Delete;

        if has_import_changes || has_file_change || is_new_or_deleted {
            if has_import_changes {
                modules_with_import_changes += 1;
            }

            // For new modules, all their imports count as added
            let final_added = if change_type == ChangeType::Insert && mod_added.is_empty() {
                // Gather all imports for this new file
                edges_b
                    .iter()
                    .filter(|e| e.source_file == *rel_path)
                    .map(to_public_edge)
                    .collect()
            } else {
                mod_added
            };
            // For deleted modules, all their imports count as removed
            let final_removed = if change_type == ChangeType::Delete && mod_removed.is_empty() {
                edges_a
                    .iter()
                    .filter(|e| e.source_file == *rel_path)
                    .map(to_public_edge)
                    .collect()
            } else {
                mod_removed
            };

            // Recheck after expanding
            let has_expanded_imports = !final_added.is_empty() || !final_removed.is_empty();
            if has_expanded_imports && !has_import_changes {
                modules_with_import_changes += 1;
            }

            module_changes.push(ModuleLevelChange {
                module_path: rel_path.to_string(),
                change_type,
                imports_added: final_added,
                imports_removed: final_removed,
                file_change,
            });
        }
    }

    let summary = ImportGraphSummary {
        total_edges_a: edges_a.len(),
        total_edges_b: edges_b.len(),
        edges_added: added_keys.len(),
        edges_removed: removed_keys.len(),
        modules_with_import_changes,
    };

    let identical = module_changes.is_empty() && added_keys.is_empty() && removed_keys.is_empty();

    Ok(DiffReport {
        file_a: dir_a.display().to_string(),
        file_b: dir_b.display().to_string(),
        identical,
        changes: Vec::new(),
        summary: None,
        granularity: DiffGranularity::Module,
        file_changes: None,
        module_changes: Some(module_changes),
        import_graph_summary: Some(summary),
        arch_changes: None,
        arch_summary: None,
    })
}

// =============================================================================
// L8: Architecture-Level Diff
// =============================================================================

/// Classify a directory name into an architectural layer.
fn classify_directory_layer(dir_name: &str) -> String {
    let lower = dir_name.to_lowercase();
    match lower.as_str() {
        "api" | "routes" | "handlers" | "endpoints" | "views" | "controllers" => "api".to_string(),
        "core" | "models" | "domain" | "entities" => "core".to_string(),
        "utils" | "helpers" | "lib" | "common" | "shared" => "utility".to_string(),
        "middleware" | "interceptors" | "filters" => "middleware".to_string(),
        "services" | "service" => "service".to_string(),
        "tests" | "test" | "spec" | "specs" => "test".to_string(),
        "config" | "settings" | "conf" => "config".to_string(),
        "db" | "database" | "migrations" | "repositories" | "repo" => "data".to_string(),
        _ => "other".to_string(),
    }
}

/// Classify a directory using import-based fan-in/fan-out analysis.
///
/// For directories whose name doesn't match a known pattern ("other"),
/// we use the import graph to infer the architectural role:
/// - High fan-out + low fan-in  -> "entry" (entry points that depend on many modules)
/// - Low fan-out  + high fan-in -> "utility" (leaf modules imported by many)
/// - Balanced                   -> "service" (intermediate layer)
fn classify_by_import_flow(
    dir_name: &str,
    edges: &[InternalImportEdge],
    all_dirs: &HashSet<String>,
) -> String {
    // Count fan-out: how many distinct external directories does this dir import from?
    let fan_out: usize = edges
        .iter()
        .filter(|e| {
            e.source_file
                .split('/')
                .next()
                .map(|d| d == dir_name)
                .unwrap_or(false)
        })
        .filter(|e| {
            // Target module references a different top-level directory
            let target_first = e
                .target_module
                .split('/')
                .next()
                .or_else(|| e.target_module.split('.').next())
                .unwrap_or("");
            all_dirs.contains(target_first) && target_first != dir_name
        })
        .map(|e| e.target_module.clone())
        .collect::<HashSet<_>>()
        .len();

    // Count fan-in: how many edges from OTHER directories target files in this dir?
    let fan_in: usize = edges
        .iter()
        .filter(|e| {
            let source_dir = e.source_file.split('/').next().unwrap_or("");
            source_dir != dir_name
        })
        .filter(|e| {
            let target_first = e
                .target_module
                .split('/')
                .next()
                .or_else(|| e.target_module.split('.').next())
                .unwrap_or("");
            target_first == dir_name
        })
        .count();

    if fan_in == 0 && fan_out == 0 {
        return "other".to_string();
    }

    // Classify based on ratio
    if fan_out > 0 && fan_in == 0 {
        "entry".to_string()
    } else if fan_in > fan_out * 2 {
        "utility".to_string()
    } else if fan_out > fan_in * 2 {
        "entry".to_string()
    } else {
        "service".to_string()
    }
}

/// Collect top-level directories containing source files, classifying each
/// into an architectural layer.
///
/// Uses two-pass classification:
/// 1. Name-based heuristic (e.g., "api/" -> api, "utils/" -> utility)
/// 2. Import-based fan-in/fan-out analysis for "other" directories
fn collect_arch_directories(root: &Path) -> Result<HashMap<String, String>> {
    let mut dirs: HashMap<String, String> = HashMap::new();
    let files = collect_source_files(root)?;

    // Pass 1: classify by name
    for (rel_path, _) in &files {
        if let Some(first_dir) = rel_path.split('/').next() {
            if rel_path.contains('/') && !dirs.contains_key(first_dir) {
                let layer = classify_directory_layer(first_dir);
                dirs.insert(first_dir.to_string(), layer);
            }
        }
    }

    // Pass 2: for directories classified as "other", try import-based classification
    let other_dirs: Vec<String> = dirs
        .iter()
        .filter(|(_, layer)| *layer == "other")
        .map(|(name, _)| name.clone())
        .collect();

    if !other_dirs.is_empty() {
        // Build import graph to analyze import flow
        if let Ok(edges) = build_import_graph(root) {
            let all_dir_names: HashSet<String> = dirs.keys().cloned().collect();
            for dir_name in &other_dirs {
                let inferred = classify_by_import_flow(dir_name, &edges, &all_dir_names);
                if inferred != "other" {
                    dirs.insert(dir_name.clone(), inferred);
                }
            }
        }
    }

    Ok(dirs)
}

/// Run L8 architecture-level diff between two directories.
fn run_arch_level_diff(dir_a: &Path, dir_b: &Path) -> Result<DiffReport> {
    let dirs_a = collect_arch_directories(dir_a)?;
    let dirs_b = collect_arch_directories(dir_b)?;

    let all_dirs: BTreeSet<&str> = dirs_a
        .keys()
        .chain(dirs_b.keys())
        .map(|s| s.as_str())
        .collect();

    let mut arch_changes: Vec<ArchLevelChange> = Vec::new();
    let mut directories_added = 0usize;
    let mut directories_removed = 0usize;
    let mut layer_migrations = 0usize;
    let mut changed_dirs = 0usize;
    let total_dirs = all_dirs.len();

    for dir_name in &all_dirs {
        let in_a = dirs_a.get(*dir_name);
        let in_b = dirs_b.get(*dir_name);

        match (in_a, in_b) {
            (Some(layer_a), Some(layer_b)) => {
                if layer_a != layer_b {
                    // Layer migration
                    changed_dirs += 1;
                    layer_migrations += 1;
                    arch_changes.push(ArchLevelChange {
                        directory: dir_name.to_string(),
                        change_type: ArchChangeType::LayerMigration,
                        old_layer: Some(layer_a.clone()),
                        new_layer: Some(layer_b.clone()),
                        migrated_functions: Vec::new(),
                    });
                }
                // Same layer = no change (stable)
            }
            (None, Some(layer_b)) => {
                // Added directory
                changed_dirs += 1;
                directories_added += 1;
                arch_changes.push(ArchLevelChange {
                    directory: dir_name.to_string(),
                    change_type: ArchChangeType::Added,
                    old_layer: None,
                    new_layer: Some(layer_b.clone()),
                    migrated_functions: Vec::new(),
                });
            }
            (Some(layer_a), None) => {
                // Removed directory
                changed_dirs += 1;
                directories_removed += 1;
                arch_changes.push(ArchLevelChange {
                    directory: dir_name.to_string(),
                    change_type: ArchChangeType::Removed,
                    old_layer: Some(layer_a.clone()),
                    new_layer: None,
                    migrated_functions: Vec::new(),
                });
            }
            (None, None) => unreachable!(),
        }
    }

    let stability_score = if total_dirs == 0 {
        1.0
    } else {
        1.0 - (changed_dirs as f64 / total_dirs as f64)
    };

    let summary = ArchDiffSummary {
        layer_migrations,
        directories_added,
        directories_removed,
        cycles_introduced: 0,
        cycles_resolved: 0,
        stability_score,
    };

    let identical = arch_changes.is_empty();

    Ok(DiffReport {
        file_a: dir_a.display().to_string(),
        file_b: dir_b.display().to_string(),
        identical,
        changes: Vec::new(),
        summary: None,
        granularity: DiffGranularity::Architecture,
        file_changes: None,
        module_changes: None,
        import_graph_summary: None,
        arch_changes: Some(arch_changes),
        arch_summary: Some(summary),
    })
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_A: &str = r#"
def original_function(x):
    return x * 2

def renamed_later(a, b):
    return a + b

def will_be_deleted():
    return "goodbye"

class OriginalClass:
    def method_one(self):
        return 1
"#;

    const SAMPLE_B: &str = r#"
def original_function(x):
    # Modified implementation
    return x * 3

def better_name(a, b):
    return a + b

def new_function():
    return "hello"

class OriginalClass:
    def method_one(self):
        return 1

    def method_two(self):
        return 2
"#;

    /// Parse Python source for tests using the language-aware ParserPool
    fn parse_python(source: &str) -> tree_sitter::Tree {
        let pool = ParserPool::new();
        pool.parse(source, Language::Python).unwrap()
    }

    #[test]
    fn test_extract_nodes() {
        let tree = parse_python(SAMPLE_A);
        let nodes = extract_nodes(tree.root_node(), SAMPLE_A.as_bytes(), Language::Python);

        // Should find: original_function, renamed_later, will_be_deleted, OriginalClass, method_one
        assert!(
            nodes.len() >= 5,
            "Expected at least 5 nodes, got {}",
            nodes.len()
        );

        let names: Vec<&str> = nodes.iter().map(|n| n.name.as_str()).collect();
        assert!(names.contains(&"original_function"));
        assert!(names.contains(&"renamed_later"));
        assert!(names.contains(&"will_be_deleted"));
        assert!(names.contains(&"OriginalClass"));
        assert!(names.contains(&"method_one"));
    }

    #[test]
    fn test_detect_update() {
        let tree_a = parse_python(SAMPLE_A);
        let tree_b = parse_python(SAMPLE_B);

        let nodes_a = extract_nodes(tree_a.root_node(), SAMPLE_A.as_bytes(), Language::Python);
        let nodes_b = extract_nodes(tree_b.root_node(), SAMPLE_B.as_bytes(), Language::Python);

        let file_a = PathBuf::from("a.py");
        let file_b = PathBuf::from("b.py");
        let changes = detect_changes(&nodes_a, &nodes_b, &file_a, &file_b, false);

        // original_function should be detected as Update
        let updates: Vec<_> = changes
            .iter()
            .filter(|c| c.change_type == ChangeType::Update)
            .collect();
        assert!(!updates.is_empty(), "Should detect at least one update");
        assert!(
            updates
                .iter()
                .any(|c| c.name.as_deref() == Some("original_function")),
            "original_function should be marked as updated"
        );
    }

    #[test]
    fn test_detect_insert() {
        let tree_a = parse_python(SAMPLE_A);
        let tree_b = parse_python(SAMPLE_B);

        let nodes_a = extract_nodes(tree_a.root_node(), SAMPLE_A.as_bytes(), Language::Python);
        let nodes_b = extract_nodes(tree_b.root_node(), SAMPLE_B.as_bytes(), Language::Python);

        let file_a = PathBuf::from("a.py");
        let file_b = PathBuf::from("b.py");
        let changes = detect_changes(&nodes_a, &nodes_b, &file_a, &file_b, false);

        // new_function and method_two should be detected as Insert
        let inserts: Vec<_> = changes
            .iter()
            .filter(|c| c.change_type == ChangeType::Insert)
            .collect();
        assert!(!inserts.is_empty(), "Should detect insertions");
    }

    #[test]
    fn test_detect_delete() {
        let tree_a = parse_python(SAMPLE_A);
        let tree_b = parse_python(SAMPLE_B);

        let nodes_a = extract_nodes(tree_a.root_node(), SAMPLE_A.as_bytes(), Language::Python);
        let nodes_b = extract_nodes(tree_b.root_node(), SAMPLE_B.as_bytes(), Language::Python);

        let file_a = PathBuf::from("a.py");
        let file_b = PathBuf::from("b.py");
        let changes = detect_changes(&nodes_a, &nodes_b, &file_a, &file_b, false);

        // will_be_deleted should be detected as Delete
        let deletes: Vec<_> = changes
            .iter()
            .filter(|c| c.change_type == ChangeType::Delete)
            .collect();
        assert!(!deletes.is_empty(), "Should detect deletions");
        assert!(
            deletes
                .iter()
                .any(|c| c.name.as_deref() == Some("will_be_deleted")),
            "will_be_deleted should be marked as deleted"
        );
    }

    #[test]
    fn test_detect_rename() {
        let tree_a = parse_python(SAMPLE_A);
        let tree_b = parse_python(SAMPLE_B);

        let nodes_a = extract_nodes(tree_a.root_node(), SAMPLE_A.as_bytes(), Language::Python);
        let nodes_b = extract_nodes(tree_b.root_node(), SAMPLE_B.as_bytes(), Language::Python);

        let file_a = PathBuf::from("a.py");
        let file_b = PathBuf::from("b.py");
        let changes = detect_changes(&nodes_a, &nodes_b, &file_a, &file_b, false);

        // renamed_later -> better_name should be detected as Rename
        let renames: Vec<_> = changes
            .iter()
            .filter(|c| c.change_type == ChangeType::Rename)
            .collect();
        assert!(!renames.is_empty(), "Should detect renames");
    }

    #[test]
    fn test_identical_files() {
        let tree_a = parse_python(SAMPLE_A);
        let tree_b = parse_python(SAMPLE_A); // Same content

        let nodes_a = extract_nodes(tree_a.root_node(), SAMPLE_A.as_bytes(), Language::Python);
        let nodes_b = extract_nodes(tree_b.root_node(), SAMPLE_A.as_bytes(), Language::Python);

        let file_a = PathBuf::from("a.py");
        let file_b = PathBuf::from("b.py");
        let changes = detect_changes(&nodes_a, &nodes_b, &file_a, &file_b, true); // semantic_only

        assert!(
            changes.is_empty(),
            "Identical files should have no semantic changes"
        );
    }

    #[test]
    fn test_compute_similarity() {
        assert_eq!(compute_similarity("abc", "abc"), 1.0);
        assert_eq!(compute_similarity("", ""), 1.0); // two empty strings are equal
        assert!(compute_similarity("a\nb\nc", "a\nb\nd") >= 0.5); // Jaccard: 2/4 = 0.5
    }

    #[test]
    fn test_normalize_body() {
        // Test that normalize_body skips the signature line and strips comments
        let body = "def foo():\n    # pure comment line\n    return 1  # inline comment";
        let normalized = normalize_body(body);
        // Should skip "def foo():" (first line), filter "# pure comment line" (comment-only)
        // and strip "# inline comment" from the return line
        assert!(!normalized.contains('#'), "Comments should be removed");
        assert!(
            !normalized.contains("def foo"),
            "Signature should be skipped"
        );
        assert!(normalized.contains("return 1"), "Body should remain");
    }

    // =========================================================================
    // format_diff_text: L6-L8 rendering tests
    // =========================================================================

    #[test]
    fn test_format_diff_text_renders_file_changes() {
        let mut report = DiffReport::new("dir_a/", "dir_b/");
        report.identical = false;
        report.file_changes = Some(vec![
            FileLevelChange {
                relative_path: "src/main.py".to_string(),
                change_type: ChangeType::Update,
                old_fingerprint: Some(12345),
                new_fingerprint: Some(67890),
                signature_changes: Some(vec!["fn foo()".to_string()]),
            },
            FileLevelChange {
                relative_path: "src/new_module.py".to_string(),
                change_type: ChangeType::Insert,
                old_fingerprint: None,
                new_fingerprint: Some(11111),
                signature_changes: None,
            },
            FileLevelChange {
                relative_path: "src/removed.py".to_string(),
                change_type: ChangeType::Delete,
                old_fingerprint: Some(99999),
                new_fingerprint: None,
                signature_changes: None,
            },
        ]);

        let text = format_diff_text(&report);
        assert!(
            text.contains("File-Level Changes"),
            "Should have file-level section header"
        );
        assert!(text.contains("src/main.py"), "Should mention updated file");
        assert!(
            text.contains("src/new_module.py"),
            "Should mention added file"
        );
        assert!(
            text.contains("src/removed.py"),
            "Should mention removed file"
        );
    }

    #[test]
    fn test_format_diff_text_renders_module_changes() {
        let mut report = DiffReport::new("dir_a/", "dir_b/");
        report.identical = false;
        report.module_changes = Some(vec![ModuleLevelChange {
            module_path: "src/utils.py".to_string(),
            change_type: ChangeType::Update,
            imports_added: vec![ImportEdge {
                source_file: "src/utils.py".to_string(),
                target_module: "os.path".to_string(),
                imported_names: vec!["join".to_string()],
            }],
            imports_removed: vec![],
            file_change: None,
        }]);

        let text = format_diff_text(&report);
        assert!(
            text.contains("Module-Level Changes"),
            "Should have module-level section header"
        );
        assert!(
            text.contains("src/utils.py"),
            "Should mention the module path"
        );
        assert!(
            text.contains("os.path"),
            "Should mention added import target"
        );
    }

    #[test]
    fn test_format_diff_text_renders_import_graph_summary() {
        let mut report = DiffReport::new("dir_a/", "dir_b/");
        report.identical = false;
        report.import_graph_summary = Some(ImportGraphSummary {
            total_edges_a: 10,
            total_edges_b: 15,
            edges_added: 7,
            edges_removed: 2,
            modules_with_import_changes: 3,
        });

        let text = format_diff_text(&report);
        assert!(
            text.contains("Import Graph"),
            "Should have import graph section"
        );
        assert!(text.contains("7"), "Should show edges added");
        assert!(text.contains("2"), "Should show edges removed");
    }

    #[test]
    fn test_format_diff_text_renders_arch_changes() {
        let mut report = DiffReport::new("dir_a/", "dir_b/");
        report.identical = false;
        report.arch_changes = Some(vec![
            ArchLevelChange {
                directory: "src/api/".to_string(),
                change_type: ArchChangeType::LayerMigration,
                old_layer: Some("presentation".to_string()),
                new_layer: Some("business".to_string()),
                migrated_functions: vec!["handle_request".to_string()],
            },
            ArchLevelChange {
                directory: "src/new_service/".to_string(),
                change_type: ArchChangeType::Added,
                old_layer: None,
                new_layer: Some("service".to_string()),
                migrated_functions: vec![],
            },
        ]);

        let text = format_diff_text(&report);
        assert!(
            text.contains("Architecture-Level Changes"),
            "Should have arch section header"
        );
        assert!(
            text.contains("src/api/"),
            "Should mention migrated directory"
        );
        assert!(text.contains("presentation"), "Should show old layer");
        assert!(text.contains("business"), "Should show new layer");
        assert!(
            text.contains("src/new_service/"),
            "Should mention added directory"
        );
    }

    #[test]
    fn test_format_diff_text_renders_arch_summary() {
        let mut report = DiffReport::new("dir_a/", "dir_b/");
        report.identical = false;
        report.arch_summary = Some(ArchDiffSummary {
            layer_migrations: 2,
            directories_added: 1,
            directories_removed: 0,
            cycles_introduced: 1,
            cycles_resolved: 0,
            stability_score: 0.75,
        });

        let text = format_diff_text(&report);
        assert!(
            text.contains("Architecture Summary"),
            "Should have arch summary section"
        );
        assert!(text.contains("0.75"), "Should show stability score");
    }

    #[test]
    fn test_format_diff_text_identical_skips_higher_levels() {
        // When identical, format_diff_text returns early, so even if higher-level
        // fields were somehow set, they should not appear.
        let mut report = DiffReport::new("a.py", "b.py");
        report.identical = true;
        report.file_changes = Some(vec![FileLevelChange {
            relative_path: "should_not_appear.py".to_string(),
            change_type: ChangeType::Insert,
            old_fingerprint: None,
            new_fingerprint: Some(1),
            signature_changes: None,
        }]);

        let text = format_diff_text(&report);
        assert!(
            !text.contains("should_not_appear"),
            "Identical report should skip all change sections"
        );
        assert!(
            text.contains("No structural changes"),
            "Should show identical message"
        );
    }
}
