//! Tokenization pipeline for clone detection v2.
//!
//! Wraps existing tokenization but preserves source text for preview extraction.
//! Skips comment nodes, import/use statement nodes, and decorator/annotation nodes.

use std::path::{Path, PathBuf};

use tree_sitter::Node;

use super::{categorize_token, has_parse_errors, is_comment_node, NormalizedToken};
use crate::ast::parser::parse_file;

/// Per-file tokenization result. Stores source for preview extraction.
pub struct FileTokens {
    /// Path of the tokenized file.
    pub file: PathBuf,
    /// Full file source text.
    pub source: String,
    /// Raw tokens extracted from the file.
    pub raw_tokens: Vec<NormalizedToken>,
}

/// Tokenize a single file. Returns raw tokens from tree-sitter AST walk.
/// Skips comment nodes, import/use statement nodes, decorator/annotation nodes.
pub fn tokenize_file_v2(path: &Path) -> anyhow::Result<FileTokens> {
    let (tree, source, _detected_lang) = parse_file(path)?;

    // Check for parse errors
    if has_parse_errors(&tree) {
        return Err(anyhow::anyhow!(
            "File {} has parse errors, skipping",
            path.display()
        ));
    }

    let language = super::filter::get_language_from_path(path).unwrap_or("unknown");

    let mut tokens = Vec::new();
    let root = tree.root_node();
    extract_tokens_v2(&root, source.as_bytes(), language, &mut tokens);

    Ok(FileTokens {
        file: path.to_path_buf(),
        source,
        raw_tokens: tokens,
    })
}

/// Recursively extract tokens from a node, skipping comments,
/// imports/use statements, and decorators/annotations.
fn extract_tokens_v2(
    node: &Node,
    source: &[u8],
    language: &str,
    tokens: &mut Vec<NormalizedToken>,
) {
    let kind = node.kind();

    // Skip comment nodes
    if is_comment_node(kind, language) {
        return;
    }

    // Skip import/use statements entirely
    if is_import_node(kind, language) {
        return;
    }

    // Skip decorator/annotation nodes
    if is_decorator_node(kind, language) {
        return;
    }

    // Handle special cases
    match language {
        "python" => {
            if kind == "interpolation" {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    extract_tokens_v2(&child, source, language, tokens);
                }
                return;
            }
        }
        "typescript" | "javascript" => {
            if kind == "template_substitution" {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    extract_tokens_v2(&child, source, language, tokens);
                }
                return;
            }
        }
        _ => {}
    }

    // Capture leaf nodes as tokens
    if node.child_count() == 0 || should_capture_as_token(kind, language) {
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

    // Recurse into children for non-leaf nodes
    if node.child_count() > 0 && !should_capture_as_token(kind, language) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            extract_tokens_v2(&child, source, language, tokens);
        }
    }
}

/// Check if a node kind is an import/use statement
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

/// Check if a node kind is a decorator/annotation
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

/// Check if a node kind should be captured as a single token
fn should_capture_as_token(kind: &str, language: &str) -> bool {
    match language {
        "rust" => kind == "macro_invocation",
        _ => false,
    }
}
