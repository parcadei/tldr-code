//! Adapter: convert tree-sitter parse trees to difftastic Syntax trees.
//!
//! This bridges tree-sitter's concrete syntax tree (CST) to difftastic's
//! `Syntax` enum, following the same patterns as difftastic's original
//! `tree_sitter_parser.rs` but using our simplified `LangConfig`.

use line_numbers::LinePositions;
use typed_arena::Arena;

use super::lang_config::LangConfig;
use super::syntax::{init_all_info, AtomKind, StringKind, Syntax};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Convert a tree-sitter tree to difftastic Syntax nodes.
pub fn ts_tree_to_syntax<'a>(
    arena: &'a Arena<Syntax<'a>>,
    tree: &tree_sitter::Tree,
    src: &str,
    config: &LangConfig,
) -> Vec<&'a Syntax<'a>> {
    // Don't return anything on empty input -- most parsers emit a
    // zero-width root node which is not useful for diffing.
    if src.trim().is_empty() {
        return vec![];
    }

    let nl_pos = LinePositions::from(src);
    let mut cursor = tree.walk();

    // The tree always has a single root; we want the top-level children.
    if !cursor.goto_first_child() {
        return vec![];
    }

    all_syntaxes_from_cursor(arena, src, &nl_pos, &mut cursor, config)
}

/// Top-level entry: convert both sides to Syntax and initialise info.
pub fn prepare_syntax_trees<'a>(
    arena: &'a Arena<Syntax<'a>>,
    lhs_src: &str,
    rhs_src: &str,
    lhs_tree: &tree_sitter::Tree,
    rhs_tree: &tree_sitter::Tree,
    config: &LangConfig,
) -> (Vec<&'a Syntax<'a>>, Vec<&'a Syntax<'a>>) {
    let lhs_nodes = ts_tree_to_syntax(arena, lhs_tree, lhs_src, config);
    let rhs_nodes = ts_tree_to_syntax(arena, rhs_tree, rhs_src, config);
    init_all_info(&lhs_nodes, &rhs_nodes);
    (lhs_nodes, rhs_nodes)
}

// ---------------------------------------------------------------------------
// Cursor walking
// ---------------------------------------------------------------------------

/// Convert all tree-sitter nodes at the current level to difftastic Syntax.
fn all_syntaxes_from_cursor<'a>(
    arena: &'a Arena<Syntax<'a>>,
    src: &str,
    nl_pos: &LinePositions,
    cursor: &mut tree_sitter::TreeCursor,
    config: &LangConfig,
) -> Vec<&'a Syntax<'a>> {
    let mut nodes: Vec<&Syntax> = vec![];

    loop {
        if let Some(node) = syntax_from_cursor(arena, src, nl_pos, cursor, config) {
            nodes.push(node);
        }

        if !cursor.goto_next_sibling() {
            break;
        }
    }

    nodes
}

/// Convert the tree-sitter node at `cursor` to a difftastic Syntax node.
fn syntax_from_cursor<'a>(
    arena: &'a Arena<Syntax<'a>>,
    src: &str,
    nl_pos: &LinePositions,
    cursor: &mut tree_sitter::TreeCursor,
    config: &LangConfig,
) -> Option<&'a Syntax<'a>> {
    let node = cursor.node();

    if config.atom_nodes.contains(node.kind()) {
        // Treat atom_nodes as flat atoms regardless of children.
        return atom_from_cursor(arena, src, nl_pos, cursor);
    }

    if node.child_count() > 0 {
        Some(list_from_cursor(arena, src, nl_pos, cursor, config))
    } else {
        // Leaf node -> atom.
        atom_from_cursor(arena, src, nl_pos, cursor)
    }
}

// ---------------------------------------------------------------------------
// Atom construction
// ---------------------------------------------------------------------------

/// Classify a tree-sitter node into an `AtomKind`.
fn classify_atom(node: &tree_sitter::Node) -> AtomKind {
    if node.is_extra() || node.kind() == "comment" || node.kind().contains("comment") {
        return AtomKind::Comment;
    }

    let kind = node.kind();
    if kind.contains("string")
        || kind.contains("char_literal")
        || kind == "regex"
        || kind.contains("heredoc")
        || kind.contains("sigil")
        || kind.contains("template_string")
    {
        return AtomKind::String(StringKind::StringLiteral);
    }

    AtomKind::Normal
}

/// Build a `Syntax::Atom` from the node at `cursor`.
fn atom_from_cursor<'a>(
    arena: &'a Arena<Syntax<'a>>,
    src: &str,
    nl_pos: &LinePositions,
    cursor: &mut tree_sitter::TreeCursor,
) -> Option<&'a Syntax<'a>> {
    let node = cursor.node();

    // Skip C/C++ preprocessor newline nodes (not useful for diffing).
    if node.kind() == "\n" {
        return None;
    }

    let start = node.start_byte();
    let end = node.end_byte();
    if start >= end && start >= src.len() {
        return None;
    }

    // Clamp end to src length to avoid panics on malformed trees.
    let end = end.min(src.len());
    let start = start.min(end);

    let content = &src[start..end];
    let position = nl_pos.from_region(start, end);
    let kind = classify_atom(&node);

    let content = truncate_content(content, 1000);

    Some(Syntax::new_atom(arena, position, content, kind))
}

// ---------------------------------------------------------------------------
// List construction
// ---------------------------------------------------------------------------

/// Collect the text of each direct child token at the current cursor level.
/// Children with their own children (i.e. non-leaf or extra) are `None`.
fn child_tokens<'a>(src: &'a str, cursor: &mut tree_sitter::TreeCursor) -> Vec<Option<&'a str>> {
    let mut tokens = vec![];

    if !cursor.goto_first_child() {
        return tokens;
    }

    loop {
        let node = cursor.node();
        if node.child_count() > 1 || node.is_extra() {
            tokens.push(None);
        } else {
            let start = node.start_byte();
            let end = node.end_byte().min(src.len());
            if start <= end && end <= src.len() {
                tokens.push(Some(&src[start..end]));
            } else {
                tokens.push(None);
            }
        }

        if !cursor.goto_next_sibling() {
            break;
        }
    }
    cursor.goto_parent();

    tokens
}

/// Find delimiter positions among the direct children of the current node.
/// Returns `(open_idx, close_idx)` if a matching pair is found.
fn find_delim_positions(
    src: &str,
    cursor: &mut tree_sitter::TreeCursor,
    lang_delims: &[(&str, &str)],
) -> Option<(usize, usize)> {
    let tokens = child_tokens(src, cursor);

    for (i, token) in tokens.iter().enumerate() {
        for (open_delim, close_delim) in lang_delims {
            if *token == Some(open_delim) {
                // Search forward for matching close.
                for (j, token) in tokens.iter().skip(i + 1).enumerate() {
                    if *token == Some(close_delim) {
                        return Some((i, i + 1 + j));
                    }
                }
            }
        }
    }

    None
}

/// Build a `Syntax::List` from the node at `cursor`.
fn list_from_cursor<'a>(
    arena: &'a Arena<Syntax<'a>>,
    src: &str,
    nl_pos: &LinePositions,
    cursor: &mut tree_sitter::TreeCursor,
    config: &LangConfig,
) -> &'a Syntax<'a> {
    let root_node = cursor.node();

    // Default: empty delimiters at the node boundaries.
    let outer_open_content = "";
    let outer_open_position = nl_pos.from_region(root_node.start_byte(), root_node.start_byte());
    let outer_close_content = "";
    let outer_close_position = nl_pos.from_region(root_node.end_byte(), root_node.end_byte());

    let (i, j) = match find_delim_positions(src, cursor, &config.delimiter_tokens) {
        Some((i, j)) => (i as isize, j as isize),
        None => (-1, root_node.child_count() as isize),
    };

    let mut inner_open_content: &str = outer_open_content;
    let mut inner_open_position = outer_open_position.clone();
    let mut inner_close_content: &str = outer_close_content;
    let mut inner_close_position = outer_close_position.clone();

    let mut before_delim: Vec<&'a Syntax<'a>> = vec![];
    let mut between_delim: Vec<&'a Syntax<'a>> = vec![];
    let mut after_delim: Vec<&'a Syntax<'a>> = vec![];

    if !cursor.goto_first_child() {
        // No children -- return empty list.
        return Syntax::new_list(
            arena,
            outer_open_content,
            outer_open_position,
            vec![],
            outer_close_content,
            outer_close_position,
        );
    }

    let mut node_i: isize = 0;
    loop {
        let node = cursor.node();

        if node_i < i {
            if let Some(syn) = syntax_from_cursor(arena, src, nl_pos, cursor, config) {
                before_delim.push(syn);
            }
        } else if node_i == i {
            let start = node.start_byte();
            let end = node.end_byte().min(src.len());
            inner_open_content = &src[start..end];
            inner_open_position = nl_pos.from_region(start, end);
        } else if node_i < j {
            if let Some(syn) = syntax_from_cursor(arena, src, nl_pos, cursor, config) {
                between_delim.push(syn);
            }
        } else if node_i == j {
            let start = node.start_byte();
            let end = node.end_byte().min(src.len());
            inner_close_content = &src[start..end];
            inner_close_position = nl_pos.from_region(start, end);
        } else if let Some(syn) = syntax_from_cursor(arena, src, nl_pos, cursor, config) {
            after_delim.push(syn);
        }

        if !cursor.goto_next_sibling() {
            break;
        }
        node_i += 1;
    }
    cursor.goto_parent();

    let inner_list = Syntax::new_list(
        arena,
        inner_open_content,
        inner_open_position,
        between_delim,
        inner_close_content,
        inner_close_position,
    );

    if before_delim.is_empty() && after_delim.is_empty() {
        inner_list
    } else {
        let mut children = before_delim;
        children.push(inner_list);
        children.append(&mut after_delim);

        Syntax::new_list(
            arena,
            outer_open_content,
            outer_open_position,
            children,
            outer_close_content,
            outer_close_position,
        )
    }
}

// ---------------------------------------------------------------------------
// RISK-15: Char-safe truncation
// ---------------------------------------------------------------------------

/// Truncate string to at most `max_chars` characters, respecting char
/// boundaries to avoid panics on multi-byte content.
fn truncate_content(content: &str, max_chars: usize) -> String {
    if content.chars().count() <= max_chars {
        content.to_owned()
    } else {
        let truncated: String = content.chars().take(max_chars - 3).collect();
        format!("{}...", truncated)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: parse Python source with tree-sitter and return the tree.
    fn parse_python(src: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        let language = tree_sitter::Language::new(tree_sitter_python::LANGUAGE);
        parser
            .set_language(&language)
            .expect("Failed to set Python language");
        parser.parse(src, None).expect("Failed to parse")
    }

    #[test]
    fn test_python_conversion() {
        let src = "def foo():\n    return 42\n";
        let tree = parse_python(src);
        let arena = Arena::new();
        let config = LangConfig::for_language("python");

        let nodes = ts_tree_to_syntax(&arena, &tree, src, &config);

        // We should get at least one top-level node (the function def).
        assert!(
            !nodes.is_empty(),
            "Expected non-empty syntax tree for Python function"
        );

        // Verify the tree contains both atoms and lists by counting.
        let (atom_count, list_count) = count_node_types(&nodes);
        assert!(
            atom_count > 0,
            "Expected at least one atom (e.g. 'def', 'foo', '42')"
        );
        assert!(
            list_count > 0,
            "Expected at least one list (e.g. function_definition, parameters)"
        );
    }

    #[test]
    fn test_atom_classification() {
        // Comments
        let mut parser = tree_sitter::Parser::new();
        let language = tree_sitter::Language::new(tree_sitter_python::LANGUAGE);
        parser.set_language(&language).unwrap();

        let src = "# a comment\nx = \"hello\"\n";
        let tree = parser.parse(src, None).unwrap();
        let arena = Arena::new();
        let config = LangConfig::for_language("python");

        let nodes = ts_tree_to_syntax(&arena, &tree, src, &config);
        assert!(!nodes.is_empty());

        // Collect all atoms
        let atoms = collect_atoms(&nodes);

        // We should find a comment atom
        let has_comment = atoms.iter().any(|a| matches!(a, AtomKind::Comment));
        assert!(
            has_comment,
            "Expected a Comment atom for '# a comment', got: {:?}",
            atoms
        );

        // We should find a string atom (the "string" node is an atom_node
        // for Python config, so it gets classified via classify_atom)
        let has_string = atoms
            .iter()
            .any(|a| matches!(a, AtomKind::String(StringKind::StringLiteral)));
        assert!(
            has_string,
            "Expected a String atom for '\"hello\"', got: {:?}",
            atoms
        );
    }

    #[test]
    fn test_truncation_unicode_safe() {
        // Basic ASCII truncation
        let long_ascii = "a".repeat(2000);
        let truncated = truncate_content(&long_ascii, 100);
        assert!(
            truncated.len() <= 103,
            "Truncated length {} exceeds expected max",
            truncated.len()
        );
        assert!(truncated.ends_with("..."));

        // Multi-byte characters: should not panic
        let emoji_str = "\u{1F600}".repeat(200); // 200 grinning face emojis
        let truncated = truncate_content(&emoji_str, 50);
        assert!(truncated.ends_with("..."));
        // Should have at most 50 chars (47 emojis + "...")
        assert!(truncated.chars().count() <= 50);

        // Short string: no truncation
        let short = "hello";
        assert_eq!(truncate_content(short, 100), "hello");

        // Mixed multi-byte
        let mixed = "cafe\u{0301} \u{00FC}ber \u{1F600} end".repeat(20);
        let truncated = truncate_content(&mixed, 30);
        assert!(truncated.ends_with("..."));
        // Must not panic -- that is the main assertion.
    }

    #[test]
    fn test_prepare_syntax_trees() {
        let lhs_src = "x = 1\n";
        let rhs_src = "x = 2\n";

        let lhs_tree = parse_python(lhs_src);
        let rhs_tree = parse_python(rhs_src);
        let arena = Arena::new();
        let config = LangConfig::for_language("python");

        let (lhs_nodes, rhs_nodes) =
            prepare_syntax_trees(&arena, lhs_src, rhs_src, &lhs_tree, &rhs_tree, &config);

        assert!(!lhs_nodes.is_empty(), "LHS should have nodes");
        assert!(!rhs_nodes.is_empty(), "RHS should have nodes");

        // After init_all_info, nodes should have valid IDs (non-MAX).
        // The id() method returns a NonZeroU32; after init it should be small.
        let lhs_id = lhs_nodes[0].id();
        assert!(
            u32::from(lhs_id) < 1000,
            "Expected small ID after init, got {}",
            u32::from(lhs_id)
        );
    }

    #[test]
    fn test_empty_source() {
        let src = "";
        let tree = parse_python(src);
        let arena = Arena::new();
        let config = LangConfig::for_language("python");

        let nodes = ts_tree_to_syntax(&arena, &tree, src, &config);
        assert!(nodes.is_empty(), "Empty source should produce no nodes");
    }

    #[test]
    fn test_whitespace_only_source() {
        let src = "   \n  \n";
        let tree = parse_python(src);
        let arena = Arena::new();
        let config = LangConfig::for_language("python");

        let nodes = ts_tree_to_syntax(&arena, &tree, src, &config);
        assert!(
            nodes.is_empty(),
            "Whitespace-only source should produce no nodes"
        );
    }

    // --- Test helpers ---

    fn count_node_types(nodes: &[&Syntax]) -> (usize, usize) {
        let mut atoms = 0;
        let mut lists = 0;
        for node in nodes {
            match node {
                Syntax::Atom { .. } => atoms += 1,
                Syntax::List { children, .. } => {
                    lists += 1;
                    let (a, l) = count_node_types(children);
                    atoms += a;
                    lists += l;
                }
            }
        }
        (atoms, lists)
    }

    fn collect_atoms(nodes: &[&Syntax]) -> Vec<AtomKind> {
        let mut result = vec![];
        for node in nodes {
            match node {
                Syntax::Atom { kind, .. } => {
                    result.push(*kind);
                }
                Syntax::List { children, .. } => {
                    result.extend(collect_atoms(children));
                }
            }
        }
        result
    }
}
