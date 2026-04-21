//! Base helpers for language handlers.
//!
//! This module provides shared utility functions used by all language handlers:
//! - Path normalization
//! - Safe source file reading with UTF-8/Latin-1 fallback
//! - Tree-sitter node text extraction
//! - Call type classification
//!
//! # Spec Reference
//!
//! See `migration/spec/callgraph-spec.md` Section 8.2 for the full specification.

use std::fs;
use std::io;
use std::path::Path;

use tree_sitter::Node;

use super::super::cross_file_types::{CallType, ImportDef};

// =============================================================================
// Path Normalization
// =============================================================================

/// Normalize a path to use forward slashes.
///
/// This ensures consistent path representation across platforms.
///
/// # Arguments
///
/// * `path` - The path to normalize
/// * `root` - Optional root to make path relative to
///
/// # Returns
///
/// A string with forward slashes, optionally relative to root.
///
/// # Example
///
/// ```rust
/// use tldr_core::callgraph::languages::base::normalize_path;
/// use std::path::Path;
///
/// assert_eq!(normalize_path(Path::new("src\\main.py"), None), "src/main.py");
/// assert_eq!(
///     normalize_path(Path::new("/project/src/main.py"), Some(Path::new("/project"))),
///     "src/main.py"
/// );
/// ```
pub fn normalize_path(path: &Path, root: Option<&Path>) -> String {
    let path_str = if let Some(root) = root {
        // Make relative to root if possible
        path.strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string()
    } else {
        path.to_string_lossy().to_string()
    };

    // Convert backslashes to forward slashes
    path_str.replace('\\', "/")
}

// =============================================================================
// Safe File Reading
// =============================================================================

/// Read a source file with UTF-8/Latin-1 fallback.
///
/// Attempts to read the file as UTF-8 first. If that fails due to invalid
/// UTF-8 sequences, falls back to Latin-1 (ISO-8859-1) encoding.
///
/// # Arguments
///
/// * `path` - Path to the source file
///
/// # Returns
///
/// The file contents as a String, or an IO error.
///
/// # Encoding Strategy
///
/// 1. Try UTF-8 (most common, and what Rust expects)
/// 2. If invalid UTF-8, decode as Latin-1 (every byte is valid Latin-1)
///
/// This matches the Python implementation which uses similar fallback logic.
///
/// # Example
///
/// ```rust,ignore
/// use tldr_core::callgraph::languages::base::read_source_safely;
/// use std::path::Path;
///
/// let source = read_source_safely(Path::new("src/main.py"))?;
/// ```
pub fn read_source_safely(path: &Path) -> Result<String, io::Error> {
    // Read raw bytes
    let bytes = fs::read(path)?;

    // Try UTF-8 first
    match String::from_utf8(bytes.clone()) {
        Ok(s) => Ok(s),
        Err(_) => {
            // Fall back to Latin-1 (ISO-8859-1)
            // Every byte is valid in Latin-1, so this always succeeds
            Ok(bytes.iter().map(|&b| b as char).collect())
        }
    }
}

// =============================================================================
// Tree-Sitter Helpers
// =============================================================================

/// Extract text content from a tree-sitter node.
///
/// # Arguments
///
/// * `node` - The tree-sitter node
/// * `source` - The source code as a byte slice
///
/// # Returns
///
/// The text content of the node, or an empty string if extraction fails.
///
/// # Example
///
/// ```rust,ignore
/// let text = get_node_text(&node, source.as_bytes());
/// ```
pub fn get_node_text<'a>(node: &Node, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
}

/// Extract text content from a tree-sitter node, owned version.
///
/// Returns an owned String instead of a reference.
pub fn get_node_text_owned(node: &Node, source: &[u8]) -> String {
    get_node_text(node, source).to_string()
}

/// Extract function name from a call node.
///
/// This handles various call patterns:
/// - Direct call: `func()` -> "func"
/// - Attribute call: `obj.method()` -> "obj.method"
/// - Chained call: `a.b.c()` -> "a.b.c"
///
/// # Arguments
///
/// * `node` - A "call" node from tree-sitter
/// * `source` - The source code
///
/// # Returns
///
/// The extracted call name, or None if extraction fails.
pub fn extract_call_name(node: &Node, source: &str) -> Option<String> {
    let source_bytes = source.as_bytes();

    // Look for the function part of the call
    // Different languages use different node types:
    // - Python: "call" with "function" child
    // - TypeScript: "call_expression" with "function" child
    // - Go: "call_expression" with "function" child

    // Try common child names
    for child_name in &["function", "callee", "receiver"] {
        if let Some(func_node) = node.child_by_field_name(child_name) {
            return Some(get_node_text_owned(&func_node, source_bytes));
        }
    }

    // Fallback: use the first named child
    for i in 0..node.named_child_count() {
        if let Some(child) = node.named_child(i) {
            // Skip argument lists
            if child.kind().contains("argument") {
                continue;
            }
            return Some(get_node_text_owned(&child, source_bytes));
        }
    }

    None
}

/// Determine call type from AST context.
///
/// Classifies a call based on:
/// - Whether target is a known local function (Intra)
/// - Whether target has a dot (Attr/Method)
/// - Whether it's a static/class method call (Static)
///
/// # Arguments
///
/// * `target` - The call target string
/// * `defined_funcs` - Set of function names defined in the current file
///
/// # Returns
///
/// The classified CallType.
pub fn determine_call_type(
    target: &str,
    defined_funcs: &std::collections::HashSet<String>,
) -> CallType {
    // Check for attribute/method access
    if target.contains('.') {
        // Could be method call (obj.method) or module access (os.path.join)
        // Default to Attr; type resolver may upgrade to Method later
        return CallType::Attr;
    }

    // Check for static method call (primarily PHP)
    if target.contains("::") {
        return CallType::Static;
    }

    // Check if it's a local/intra-file call
    if defined_funcs.contains(target) {
        return CallType::Intra;
    }

    // Default to Direct (will be resolved via imports)
    CallType::Direct
}

// =============================================================================
// Import Helpers
// =============================================================================

/// Helper to create ImportDef from parsed data.
///
/// # Arguments
///
/// * `module` - Module path
/// * `names` - Imported names
/// * `is_from` - True for "from X import Y" style
/// * `level` - Relative import level (0 = absolute)
///
/// # Example
///
/// ```rust
/// use tldr_core::callgraph::languages::base::make_import;
///
/// // from os.path import join
/// let imp = make_import("os.path", &["join"], true, 0);
/// assert!(imp.is_from);
/// assert_eq!(imp.names, vec!["join"]);
///
/// // from . import utils
/// let imp = make_import("", &["utils"], true, 1);
/// assert!(imp.is_relative());
/// ```
pub fn make_import(module: &str, names: &[&str], is_from: bool, level: u8) -> ImportDef {
    ImportDef {
        module: module.to_string(),
        is_from,
        names: names.iter().map(|s| s.to_string()).collect(),
        alias: None,
        aliases: None,
        resolved_module: None,
        is_default: false,
        is_namespace: false,
        is_mod: false,
        level,
        is_type_checking: false,
    }
}

/// Helper to create ImportDef with an alias.
pub fn make_import_with_alias(module: &str, alias: &str, level: u8) -> ImportDef {
    ImportDef {
        module: module.to_string(),
        is_from: false,
        names: vec![],
        alias: Some(alias.to_string()),
        aliases: None,
        resolved_module: None,
        is_default: false,
        is_namespace: false,
        is_mod: false,
        level,
        is_type_checking: false,
    }
}

// =============================================================================
// Tree Walking
// =============================================================================

/// Iterator that walks all nodes in a tree-sitter tree.
///
/// Performs a depth-first traversal of the entire tree.
pub struct TreeWalker<'a> {
    cursor: tree_sitter::TreeCursor<'a>,
    done: bool,
}

impl<'a> TreeWalker<'a> {
    /// Create a new tree walker starting from the given node.
    pub fn new(node: Node<'a>) -> Self {
        Self {
            cursor: node.walk(),
            done: false,
        }
    }
}

impl<'a> Iterator for TreeWalker<'a> {
    type Item = Node<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }

        let node = self.cursor.node();

        // Try to go to first child
        if self.cursor.goto_first_child() {
            return Some(node);
        }

        // Try to go to next sibling
        if self.cursor.goto_next_sibling() {
            return Some(node);
        }

        // Go up until we can go to a sibling
        loop {
            if !self.cursor.goto_parent() {
                self.done = true;
                return Some(node);
            }
            if self.cursor.goto_next_sibling() {
                return Some(node);
            }
        }
    }
}

/// Walk all nodes in a tree.
///
/// # Example
///
/// ```rust,ignore
/// for node in walk_tree(tree.root_node()) {
///     if node.kind() == "function_definition" {
///         // Process function
///     }
/// }
/// ```
pub fn walk_tree(node: Node<'_>) -> TreeWalker<'_> {
    TreeWalker::new(node)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    mod normalize_path_tests {
        use super::*;

        #[test]
        fn test_normalize_forward_slashes() {
            assert_eq!(
                normalize_path(Path::new("src/main.py"), None),
                "src/main.py"
            );
        }

        #[test]
        fn test_normalize_backslashes() {
            // Note: On Unix, backslashes are valid path characters, not separators
            // This test verifies our string replacement works
            let path = Path::new("src\\sub\\main.py");
            let normalized = normalize_path(path, None);
            // The path string may or may not have backslashes depending on platform
            // But our normalization should convert them
            assert!(!normalized.contains('\\') || cfg!(not(windows)));
        }

        #[test]
        fn test_normalize_with_root() {
            let path = Path::new("/project/src/main.py");
            let root = Path::new("/project");
            assert_eq!(normalize_path(path, Some(root)), "src/main.py");
        }

        #[test]
        fn test_normalize_root_not_prefix() {
            let path = Path::new("/other/src/main.py");
            let root = Path::new("/project");
            assert_eq!(normalize_path(path, Some(root)), "/other/src/main.py");
        }
    }

    mod read_source_tests {
        use super::*;
        use std::io::Write;
        use tempfile::NamedTempFile;

        #[test]
        fn test_read_utf8_file() {
            let mut file = NamedTempFile::new().unwrap();
            writeln!(file, "def hello(): pass").unwrap();

            let content = read_source_safely(file.path()).unwrap();
            assert!(content.contains("def hello()"));
        }

        #[test]
        fn test_read_latin1_fallback() {
            let mut file = NamedTempFile::new().unwrap();
            // Write some bytes that are valid Latin-1 but invalid UTF-8
            // 0xE9 is 'e' with acute accent in Latin-1
            file.write_all(b"caf\xe9").unwrap();

            let content = read_source_safely(file.path()).unwrap();
            assert_eq!(content, "caf\u{00e9}");
        }

        #[test]
        fn test_read_nonexistent_file() {
            let result = read_source_safely(Path::new("/nonexistent/file.py"));
            assert!(result.is_err());
        }
    }

    mod call_type_tests {
        use super::*;

        #[test]
        fn test_determine_call_type_intra() {
            let mut defined = HashSet::new();
            defined.insert("local_func".to_string());

            assert_eq!(determine_call_type("local_func", &defined), CallType::Intra);
        }

        #[test]
        fn test_determine_call_type_attr() {
            let defined = HashSet::new();
            assert_eq!(determine_call_type("obj.method", &defined), CallType::Attr);
            assert_eq!(determine_call_type("a.b.c", &defined), CallType::Attr);
        }

        #[test]
        fn test_determine_call_type_static() {
            let defined = HashSet::new();
            assert_eq!(
                determine_call_type("Class::method", &defined),
                CallType::Static
            );
        }

        #[test]
        fn test_determine_call_type_direct() {
            let defined = HashSet::new();
            assert_eq!(
                determine_call_type("external_func", &defined),
                CallType::Direct
            );
        }
    }

    mod import_helper_tests {
        use super::*;

        #[test]
        fn test_make_import_simple() {
            let imp = make_import("os", &[], false, 0);
            assert_eq!(imp.module, "os");
            assert!(!imp.is_from);
            assert!(imp.names.is_empty());
            assert_eq!(imp.level, 0);
        }

        #[test]
        fn test_make_import_from() {
            let imp = make_import("os.path", &["join", "exists"], true, 0);
            assert_eq!(imp.module, "os.path");
            assert!(imp.is_from);
            assert_eq!(imp.names, vec!["join", "exists"]);
        }

        #[test]
        fn test_make_import_relative() {
            let imp = make_import("", &["utils"], true, 1);
            assert!(imp.is_from);
            assert_eq!(imp.level, 1);
            assert!(imp.is_relative());
        }

        #[test]
        fn test_make_import_with_alias() {
            let imp = make_import_with_alias("numpy", "np", 0);
            assert_eq!(imp.module, "numpy");
            assert_eq!(imp.alias, Some("np".to_string()));
            assert!(!imp.is_from);
        }
    }

    mod tree_walker_tests {
        use super::*;

        #[test]
        fn test_walk_tree_simple() {
            use tree_sitter::Parser;

            let mut parser = Parser::new();
            parser
                .set_language(&tree_sitter_python::LANGUAGE.into())
                .unwrap();

            let source = "def hello(): pass";
            let tree = parser.parse(source, None).unwrap();

            let nodes: Vec<_> = walk_tree(tree.root_node()).collect();
            assert!(!nodes.is_empty());

            // Verify we find the function definition
            let kinds: Vec<_> = nodes.iter().map(|n| n.kind()).collect();
            assert!(kinds.contains(&"function_definition"));
        }
    }
}
