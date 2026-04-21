//! Fragment extraction for clone detection v2.
//!
//! Uses tree-sitter queries per language to find function/method boundaries.
//! Provides real line numbers from tree-sitter (BUG-1 fix).
//! Enforces min_lines (BUG-2 fix).
//! Populates preview from source lines (BUG-5 fix).
//! Falls back to sliding window if tree-sitter yields < 2 fragments.

use std::collections::HashSet;
use std::path::PathBuf;

use tree_sitter::Node;

use super::{
    apply_normalization, categorize_token, hash_token, is_comment_node, NormalizationMode,
    NormalizedToken, RollingHash,
};
use crate::ast::parser::parse_file;

use super::tokenize::FileTokens;

/// Fragment data extracted from a file.
#[derive(Debug, Clone)]
pub struct FragmentData {
    /// Index of the source file in the tokenized file list.
    pub file_idx: usize,
    /// Path of the source file.
    pub file: PathBuf,
    /// First line of the fragment (1-indexed, tree-sitter based).
    pub start_line: usize, // 1-indexed, from tree-sitter
    /// Last line of the fragment (1-indexed, inclusive).
    pub end_line: usize,   // 1-indexed, from tree-sitter
    /// Raw tokens extracted for the fragment.
    pub raw_tokens: Vec<NormalizedToken>,
    /// Normalized tokens for clone matching.
    pub normalized_tokens: Vec<NormalizedToken>,
    /// Rolling hash of the raw token sequence.
    pub raw_hash: u64,
    /// Rolling hash of the normalized token sequence.
    pub normalized_hash: u64,
    /// Preview snippet extracted from source lines.
    pub preview: String,
    /// Function/method name associated with the fragment, when available.
    pub function_name: Option<String>,
}

/// Extract syntactic fragments from a file using tree-sitter queries.
///
/// Returns fragments aligned to function/method boundaries (REQ-1).
/// Falls back to sliding window if < 2 fragments extracted.
pub fn extract_fragments_from_file(
    file_tokens: &FileTokens,
    file_idx: usize,
    min_tokens: usize,
    min_lines: usize,
    normalization: NormalizationMode,
) -> Vec<FragmentData> {
    let language = super::filter::get_language_from_path(&file_tokens.file).unwrap_or("unknown");

    // Parse the file to get the AST for fragment boundary detection
    let tree = match parse_file(&file_tokens.file) {
        Ok((tree, _source, _lang)) => tree,
        Err(_) => return vec![],
    };

    let root = tree.root_node();
    let source = &file_tokens.source;
    let source_bytes = source.as_bytes();

    // Collect function/method node boundaries from tree-sitter
    let mut func_nodes: Vec<FuncNodeInfo> = Vec::new();
    collect_function_nodes(&root, source_bytes, language, &mut func_nodes, 0);

    // Deduplicate by (start_line, end_line)
    let mut seen: HashSet<(usize, usize)> = HashSet::new();
    func_nodes.retain(|n| seen.insert((n.start_line, n.end_line)));

    // Convert function nodes to fragments
    let mut fragments: Vec<FragmentData> = Vec::new();

    for func_node in &func_nodes {
        let start_line = func_node.start_line;
        let end_line = func_node.end_line;
        let line_count = end_line - start_line + 1;

        // REQ-6, BUG-2: Enforce min_lines
        if line_count < min_lines {
            continue;
        }

        // Extract raw tokens for this function's range
        let raw_tokens = extract_tokens_for_range(
            source_bytes,
            language,
            func_node.byte_start,
            func_node.byte_end,
            &tree,
        );

        // Check min_tokens
        if raw_tokens.len() < min_tokens {
            continue;
        }

        // Compute normalized tokens
        let normalized_tokens = apply_normalization(raw_tokens.clone(), normalization);

        // Compute hashes
        let raw_hash = compute_sequence_hash(&raw_tokens);
        let normalized_hash = compute_sequence_hash(&normalized_tokens);

        // REQ-4, BUG-5: Extract preview from source lines
        let preview = extract_preview(source, start_line, end_line);

        fragments.push(FragmentData {
            file_idx,
            file: file_tokens.file.clone(),
            start_line,
            end_line,
            raw_tokens,
            normalized_tokens,
            raw_hash,
            normalized_hash,
            preview,
            function_name: func_node.name.clone(),
        });
    }

    // Sliding window fallback: if tree-sitter yields < 2 fragments
    if fragments.len() < 2 && !file_tokens.raw_tokens.is_empty() {
        let window_fragments =
            extract_sliding_window(file_tokens, file_idx, min_tokens, min_lines, normalization);
        if window_fragments.len() >= fragments.len() {
            return window_fragments;
        }
    }

    fragments
}

/// Info about a function/method node from tree-sitter.
struct FuncNodeInfo {
    start_line: usize,
    end_line: usize,
    byte_start: usize,
    byte_end: usize,
    name: Option<String>,
}

/// Recursively collect function/method nodes from the AST.
fn collect_function_nodes(
    node: &Node,
    source: &[u8],
    language: &str,
    result: &mut Vec<FuncNodeInfo>,
    depth: usize,
) {
    let kind = node.kind();

    let is_function_node = match language {
        "python" => kind == "function_definition" && depth <= 1,
        "typescript" | "javascript" => {
            matches!(
                kind,
                "function_declaration" | "method_definition" | "arrow_function"
            )
        }
        "go" => matches!(kind, "function_declaration" | "method_declaration"),
        "rust" => kind == "function_item",
        "java" => matches!(kind, "method_declaration" | "constructor_declaration"),
        "c" | "cpp" => kind == "function_definition",
        "csharp" => matches!(kind, "method_declaration" | "constructor_declaration"),
        "ruby" => matches!(kind, "method" | "singleton_method"),
        "php" => matches!(kind, "function_definition" | "method_declaration"),
        "swift" => matches!(kind, "function_declaration" | "init_declaration"),
        "kotlin" => kind == "function_declaration",
        "scala" => matches!(kind, "function_definition" | "function_declaration"),
        "lua" | "luau" => matches!(
            kind,
            "function_declaration" | "function_definition" | "local_function"
        ),
        "elixir" => {
            // Elixir def/defp are represented as `call` nodes
            if kind == "call" {
                // Check if the first child is "def" or "defp"
                if let Some(first) = node.child(0) {
                    if let Ok(text) = first.utf8_text(source) {
                        text == "def" || text == "defp"
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            }
        }
        "ocaml" => matches!(kind, "let_binding" | "value_definition"),
        _ => false,
    };

    if is_function_node {
        let start_line = node.start_position().row + 1;
        let end_line = node.end_position().row + 1;
        let byte_start = node.start_byte();
        let byte_end = node.end_byte();

        // Extract function name from the 'name' child node
        let name = extract_function_name(node, source, language);

        result.push(FuncNodeInfo {
            start_line,
            end_line,
            byte_start,
            byte_end,
            name,
        });

        // Don't recurse into function bodies — prevents nested extraction
        // (arrow functions inside methods, anonymous functions inside Go funcs, etc.)
        return;
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_function_nodes(&child, source, language, result, depth + 1);
    }
}

/// Extract the function name from a tree-sitter node.
fn extract_function_name(node: &Node, source: &[u8], language: &str) -> Option<String> {
    match language {
        "c" | "cpp" => {
            // C/C++: function_definition -> declarator -> function_declarator -> identifier
            if let Some(declarator) = node.child_by_field_name("declarator") {
                return extract_c_declarator_name(&declarator, source);
            }
            None
        }
        "elixir" => {
            // Elixir: call node where first child is "def"/"defp"
            // Second child (arguments) contains the function name
            if node.kind() == "call" {
                if let Some(args) = node.child(1) {
                    if args.kind() == "identifier" {
                        return args.utf8_text(source).ok().map(|s| s.to_string());
                    }
                    // arguments or call node
                    let mut cursor = args.walk();
                    for child in args.children(&mut cursor) {
                        if child.kind() == "identifier" {
                            return child.utf8_text(source).ok().map(|s| s.to_string());
                        }
                        if child.kind() == "call" {
                            if let Some(name) = child.child(0) {
                                if name.kind() == "identifier" {
                                    return name.utf8_text(source).ok().map(|s| s.to_string());
                                }
                            }
                        }
                    }
                }
            }
            None
        }
        "lua" | "luau" => {
            // Lua: function_declaration has "name" field, local_function has identifier child
            if let Some(name_node) = node.child_by_field_name("name") {
                return name_node.utf8_text(source).ok().map(|s| s.to_string());
            }
            // Fallback: search for identifier child
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if matches!(child.kind(), "identifier" | "dot_index_expression") {
                    return child.utf8_text(source).ok().map(|s| s.to_string());
                }
            }
            None
        }
        "ocaml" => {
            // OCaml: let_binding has "pattern" field, value_definition wraps let_binding
            if node.kind() == "value_definition" {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "let_binding" {
                        return child
                            .child_by_field_name("pattern")
                            .and_then(|n| n.utf8_text(source).ok().map(|s| s.to_string()));
                    }
                }
                None
            } else {
                node.child_by_field_name("pattern")
                    .and_then(|n| n.utf8_text(source).ok().map(|s| s.to_string()))
            }
        }
        "swift" => {
            // Swift: init_declaration has no "name" field -- the name is "init"
            if node.kind() == "init_declaration" {
                return Some("init".to_string());
            }
            node.child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok().map(|s| s.to_string()))
        }
        // Most languages (python, rust, go, typescript, javascript, java,
        // csharp, ruby, php, kotlin, scala) use "name" field
        _ => node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok().map(|s| s.to_string())),
    }
}

/// Recursively extract function name from C/C++ declarator chain.
/// Handles: function_declarator, pointer_declarator, identifier, etc.
fn extract_c_declarator_name(declarator: &Node, source: &[u8]) -> Option<String> {
    match declarator.kind() {
        "identifier" | "field_identifier" => {
            declarator.utf8_text(source).ok().map(|s| s.to_string())
        }
        "function_declarator" | "pointer_declarator" | "parenthesized_declarator" => {
            // Look for nested declarator field, or first identifier child
            if let Some(inner) = declarator.child_by_field_name("declarator") {
                return extract_c_declarator_name(&inner, source);
            }
            let mut cursor = declarator.walk();
            for child in declarator.children(&mut cursor) {
                if let Some(name) = extract_c_declarator_name(&child, source) {
                    return Some(name);
                }
            }
            None
        }
        _ => None,
    }
}

/// Extract tokens for a specific byte range within the tree, walking the AST.
fn extract_tokens_for_range(
    source: &[u8],
    language: &str,
    byte_start: usize,
    byte_end: usize,
    tree: &tree_sitter::Tree,
) -> Vec<NormalizedToken> {
    let mut tokens = Vec::new();
    let root = tree.root_node();
    extract_tokens_in_range_recursive(&root, source, language, byte_start, byte_end, &mut tokens);
    tokens
}

/// Recursively extract tokens that fall within [byte_start, byte_end).
fn extract_tokens_in_range_recursive(
    node: &Node,
    source: &[u8],
    language: &str,
    range_start: usize,
    range_end: usize,
    tokens: &mut Vec<NormalizedToken>,
) {
    // Skip nodes entirely outside our range
    if node.end_byte() <= range_start || node.start_byte() >= range_end {
        return;
    }

    let kind = node.kind();

    // Skip comments
    if is_comment_node(kind, language) {
        return;
    }

    // Skip imports
    if is_import_node(kind, language) {
        return;
    }

    // Skip decorators
    if is_decorator_node(kind, language) {
        return;
    }

    // Leaf node or special capture
    let should_capture = node.child_count() == 0 || should_capture_as_token(kind, language);

    if should_capture {
        // Only include if within range
        if node.start_byte() >= range_start && node.end_byte() <= range_end {
            if let Ok(text) = node.utf8_text(source) {
                let text = text.trim();
                if !text.is_empty() && !text.chars().all(|c| c.is_whitespace()) {
                    let category = categorize_token(kind, language);
                    tokens.push(NormalizedToken {
                        value: text.to_string(),
                        original: text.to_string(),
                        category,
                    });
                }
            }
        }
    }

    // Recurse into children
    if node.child_count() > 0 && !should_capture_as_token(kind, language) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            extract_tokens_in_range_recursive(
                &child,
                source,
                language,
                range_start,
                range_end,
                tokens,
            );
        }
    }
}

fn is_import_node(kind: &str, language: &str) -> bool {
    match language {
        "python" => matches!(kind, "import_statement" | "import_from_statement"),
        "typescript" | "javascript" => matches!(kind, "import_statement" | "import_declaration"),
        "go" => matches!(kind, "import_declaration" | "import_spec"),
        "rust" => matches!(kind, "use_declaration"),
        "java" => matches!(kind, "import_declaration"),
        "c" | "cpp" => kind == "preproc_include",
        "csharp" => kind == "using_directive",
        "scala" => kind == "import_declaration",
        "swift" => kind == "import_declaration",
        "kotlin" => kind == "import_header",
        "php" => kind == "namespace_use_declaration",
        "ocaml" => kind == "open_statement",
        _ => false,
    }
}

fn is_decorator_node(kind: &str, language: &str) -> bool {
    match language {
        "python" => kind == "decorator",
        "typescript" | "javascript" => kind == "decorator",
        "java" => matches!(kind, "marker_annotation" | "annotation"),
        "rust" => matches!(kind, "attribute_item" | "inner_attribute_item"),
        "csharp" => kind == "attribute_list",
        "kotlin" => kind == "annotation",
        "php" => kind == "attribute_list",
        "swift" => kind == "attribute",
        "elixir" => kind == "unary_operator",
        _ => false,
    }
}

fn should_capture_as_token(kind: &str, language: &str) -> bool {
    match language {
        "rust" => kind == "macro_invocation",
        _ => false,
    }
}

/// Compute hash for a token sequence using RollingHash.
fn compute_sequence_hash(tokens: &[NormalizedToken]) -> u64 {
    if tokens.is_empty() {
        return 0;
    }
    let mut hasher = RollingHash::new(tokens.len());
    for token in tokens {
        hasher.push(hash_token(token));
    }
    hasher.current()
}

/// Extract preview from source lines (REQ-4, BUG-5 fix).
/// Truncated to 100 chars with "..." suffix by CloneFragment::with_preview.
fn extract_preview(source: &str, start_line: usize, end_line: usize) -> String {
    let lines: Vec<&str> = source.lines().collect();
    let start_idx = start_line.saturating_sub(1);
    let end_idx = end_line.min(lines.len());
    if start_idx >= lines.len() {
        return String::new();
    }
    lines[start_idx..end_idx].join("\n")
}

/// Sliding window fallback when tree-sitter yields < 2 fragments.
fn extract_sliding_window(
    file_tokens: &FileTokens,
    file_idx: usize,
    min_tokens: usize,
    min_lines: usize,
    normalization: NormalizationMode,
) -> Vec<FragmentData> {
    let tokens = &file_tokens.raw_tokens;
    if tokens.len() < min_tokens {
        return vec![];
    }

    let source = &file_tokens.source;
    let source_lines: Vec<&str> = source.lines().collect();
    let total_lines = source_lines.len();

    // Single fragment for the whole file (similar to v1's small file handling)
    let normalized = apply_normalization(tokens.clone(), normalization);
    let raw_hash = compute_sequence_hash(tokens);
    let normalized_hash = compute_sequence_hash(&normalized);

    // Use actual line count for the whole file
    let start_line = 1;
    let end_line = total_lines.max(1);
    let line_count = end_line - start_line + 1;

    if line_count < min_lines {
        return vec![];
    }

    let preview = extract_preview(source, start_line, end_line);

    vec![FragmentData {
        file_idx,
        file: file_tokens.file.clone(),
        start_line,
        end_line,
        raw_tokens: tokens.clone(),
        normalized_tokens: normalized,
        raw_hash,
        normalized_hash,
        preview,
        function_name: None,
    }]
}
