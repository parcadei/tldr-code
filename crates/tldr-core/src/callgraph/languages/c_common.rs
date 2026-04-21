//! Shared C/C++ utilities for call graph analysis.
//!
//! This module extracts common code used by both the C and C++ language handlers,
//! including `#include` parsing and tree-sitter source parsing.

use tree_sitter::{Node, Parser, Tree};

use super::base::{get_node_text, walk_tree};
use super::ParseError;
use crate::callgraph::cross_file_types::ImportDef;

/// Parse a single `#include` directive node into an `ImportDef`.
///
/// Handles both local (`#include "file.h"`) and system (`#include <file.h>`) includes.
/// Returns `None` if the node does not contain a recognizable include path.
pub(crate) fn parse_include(node: &Node, source: &[u8]) -> Option<ImportDef> {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            match child.kind() {
                "string_literal" => {
                    // Local include: #include "file.h"
                    let text = get_node_text(&child, source);
                    let module = text.trim_matches('"').to_string();
                    let mut imp = ImportDef::simple_import(module);
                    imp.is_namespace = false; // Mark as local include
                    return Some(imp);
                }
                "system_lib_string" => {
                    // System include: #include <file.h>
                    let text = get_node_text(&child, source);
                    let module = text.trim_matches(|c| c == '<' || c == '>').to_string();
                    let mut imp = ImportDef::simple_import(module);
                    imp.is_namespace = true; // Mark as system include
                    return Some(imp);
                }
                _ => {}
            }
        }
    }
    None
}

/// Parse all `#include` directives from a parsed tree.
///
/// Walks the tree looking for `preproc_include` nodes and delegates to
/// [`parse_include`] for each one.
pub(crate) fn parse_preproc_imports(tree: &Tree, source: &str) -> Vec<ImportDef> {
    let source_bytes = source.as_bytes();
    let mut imports = Vec::new();

    for node in walk_tree(tree.root_node()) {
        if node.kind() == "preproc_include" {
            if let Some(imp) = parse_include(&node, source_bytes) {
                imports.push(imp);
            }
        }
    }

    imports
}

/// Parse source code into a tree-sitter `Tree` using the given language.
///
/// # Arguments
///
/// * `source` - The source code to parse.
/// * `ts_language` - The tree-sitter language grammar to use.
/// * `lang_name` - A human-readable language name for error messages (e.g., "C" or "C++").
pub(crate) fn parse_source_with_language(
    source: &str,
    ts_language: tree_sitter::Language,
    lang_name: &str,
) -> Result<Tree, ParseError> {
    let mut parser = Parser::new();
    parser
        .set_language(&ts_language)
        .map_err(|e| ParseError::ParseFailed {
            file: std::path::PathBuf::new(),
            message: format!("Failed to set {} language: {}", lang_name, e),
        })?;

    parser
        .parse(source, None)
        .ok_or_else(|| ParseError::ParseFailed {
            file: std::path::PathBuf::new(),
            message: "Parser returned None".to_string(),
        })
}
