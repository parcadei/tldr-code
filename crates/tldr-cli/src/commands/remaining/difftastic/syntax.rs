//! Syntax tree definitions with change metadata.

#![allow(clippy::mutable_key_type)] // Hash for Syntax doesn't use mutable fields.

use std::{cell::Cell, fmt, hash::Hash, num::NonZeroU32};

use line_numbers::SingleLineSpan;
use typed_arena::Arena;

use self::Syntax::*;
use super::{changes::ChangeKind, changes::ChangeKind::*, hash::DftHashMap};

/// Inline from difftastic's lines.rs -- split on \n or \r\n
fn split_on_newlines(s: &str) -> impl Iterator<Item = &str> {
    s.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l))
}

/// A Debug implementation that does not recurse into the
/// corresponding node mentioned for Unchanged. Otherwise we will
/// infinitely loop on unchanged nodes, which both point to the other.
impl fmt::Debug for ChangeKind<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let desc = match self {
            Unchanged(node) => format!("Unchanged(ID: {})", node.id()),
            ReplacedComment(lhs_node, rhs_node) | ReplacedString(lhs_node, rhs_node) => {
                let change_kind = if let ReplacedComment(_, _) = self {
                    "ReplacedComment"
                } else {
                    "ReplacedString"
                };

                format!(
                    "{}(lhs ID: {}, rhs ID: {})",
                    change_kind,
                    lhs_node.id(),
                    rhs_node.id()
                )
            }
            Novel => "Novel".to_owned(),
        };
        f.write_str(&desc)
    }
}

pub type SyntaxId = NonZeroU32;

pub type ContentId = u32;

/// Fields that are common to both `Syntax::List` and `Syntax::Atom`.
pub struct SyntaxInfo<'a> {
    /// The previous node with the same parent as this one.
    previous_sibling: Cell<Option<&'a Syntax<'a>>>,
    /// The next node with the same parent as this one.
    next_sibling: Cell<Option<&'a Syntax<'a>>>,
    /// The syntax node that occurs before this one, in a depth-first
    /// tree traversal.
    prev: Cell<Option<&'a Syntax<'a>>>,
    /// The parent syntax node, if present.
    parent: Cell<Option<&'a Syntax<'a>>>,
    /// The number of nodes that are ancestors of this one.
    num_ancestors: Cell<u32>,
    pub num_after: Cell<usize>,
    /// A number that uniquely identifies this syntax node.
    unique_id: Cell<SyntaxId>,
    /// A number that uniquely identifies the content of this syntax
    /// node. This may be the same as nodes on the other side of the
    /// diff, or nodes at different positions.
    ///
    /// Values are sequential, not hashes. Collisions never occur.
    content_id: Cell<ContentId>,
    /// Is this the only node with this content? Ignores nodes on the
    /// other side.
    content_is_unique: Cell<bool>,
}

impl<'a> SyntaxInfo<'a> {
    pub fn new() -> Self {
        Self {
            previous_sibling: Cell::new(None),
            next_sibling: Cell::new(None),
            prev: Cell::new(None),
            parent: Cell::new(None),
            num_ancestors: Cell::new(0),
            num_after: Cell::new(0),
            unique_id: Cell::new(NonZeroU32::new(u32::MAX).unwrap()),
            content_id: Cell::new(0),
            content_is_unique: Cell::new(false),
        }
    }
}

impl Default for SyntaxInfo<'_> {
    fn default() -> Self {
        Self::new()
    }
}

pub enum Syntax<'a> {
    List {
        info: SyntaxInfo<'a>,
        open_position: Vec<SingleLineSpan>,
        open_content: String,
        children: Vec<&'a Syntax<'a>>,
        close_position: Vec<SingleLineSpan>,
        close_content: String,
        num_descendants: u32,
    },
    Atom {
        info: SyntaxInfo<'a>,
        position: Vec<SingleLineSpan>,
        content: String,
        kind: AtomKind,
    },
}

fn dbg_pos(pos: &[SingleLineSpan]) -> String {
    match pos {
        [] => "-".into(),
        [pos] => format!("{}:{}-{}", pos.line.0, pos.start_col, pos.end_col),
        [start, .., end] => format!(
            "{}:{}-{}:{}",
            start.line.0, start.start_col, end.line.0, end.end_col
        ),
    }
}

impl<'a> fmt::Debug for Syntax<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            List {
                open_content,
                open_position,
                children,
                close_content,
                close_position,
                ..
            } => {
                let ds = f.debug_struct(&format!(
                    "List id:{} content_id:{}",
                    self.id(),
                    self.content_id()
                ));
                let mut ds = ds;

                ds.field("open_content", &open_content)
                    .field("open_position", &dbg_pos(open_position))
                    .field("children", &children)
                    .field("close_content", &close_content)
                    .field("close_position", &dbg_pos(close_position));

                ds.finish()
            }
            Atom {
                content, position, ..
            } => {
                let ds = f.debug_struct(&format!(
                    "Atom id:{} content_id:{}",
                    self.id(),
                    self.content_id()
                ));
                let mut ds = ds;
                ds.field("content", &content);
                ds.field("position", &dbg_pos(position));

                ds.finish()
            }
        }
    }
}

impl<'a> Syntax<'a> {
    pub fn new_list(
        arena: &'a Arena<Self>,
        open_content: &str,
        open_position: Vec<SingleLineSpan>,
        children: Vec<&'a Self>,
        close_content: &str,
        close_position: Vec<SingleLineSpan>,
    ) -> &'a Self {
        // Skip empty atoms: they aren't displayed, so there's no
        // point making our syntax tree bigger. These occur when we're
        // parsing incomplete or malformed programs.
        let children = children
            .into_iter()
            .filter(|n| match n {
                List { .. } => true,
                Atom { content, .. } => !content.is_empty(),
            })
            .collect::<Vec<_>>();

        // Don't bother creating a list if we have no open/close and
        // there's only one child. This occurs in small files with
        // thorough tree-sitter parsers: you get parse trees like:
        //
        // (compilation-unit (top-level-def (function ...)))
        //
        // This is a small performance win as it makes the difftastic
        // syntax tree smaller. It also really helps when looking at
        // debug output for small inputs.
        if children.len() == 1 && open_content.is_empty() && close_content.is_empty() {
            return children[0];
        }

        let mut num_descendants = 0;
        for child in &children {
            num_descendants += match child {
                List {
                    num_descendants, ..
                } => *num_descendants + 1,
                Atom { .. } => 1,
            };
        }

        arena.alloc(List {
            info: SyntaxInfo::default(),
            open_position,
            open_content: open_content.into(),
            close_content: close_content.into(),
            close_position,
            children,
            num_descendants,
        })
    }

    pub fn new_atom(
        arena: &'a Arena<Self>,
        mut position: Vec<SingleLineSpan>,
        mut content: String,
        kind: AtomKind,
    ) -> &'a Self {
        // If a parser hasn't cleaned up \r on CRLF files with
        // comments, discard it.
        if content.ends_with('\r') {
            content.pop();
        }

        // If a parser adds a trailing newline to the atom, discard
        // it. It produces worse diffs: we'd rather align on real
        // content, and complicates handling of trailing newlines at
        // the end of the file.
        if content.ends_with('\n') {
            position.pop();
            content.pop();
        }

        arena.alloc(Atom {
            info: SyntaxInfo::default(),
            position,
            content,
            kind,
        })
    }

    pub fn info(&self) -> &SyntaxInfo<'a> {
        match self {
            List { info, .. } | Atom { info, .. } => info,
        }
    }

    pub fn parent(&self) -> Option<&'a Self> {
        self.info().parent.get()
    }

    pub fn next_sibling(&self) -> Option<&'a Self> {
        self.info().next_sibling.get()
    }

    /// A unique ID of this syntax node. Every node is guaranteed to
    /// have a different value.
    pub fn id(&self) -> SyntaxId {
        self.info().unique_id.get()
    }

    /// A content ID of this syntax node. Two nodes have the same
    /// content ID if they have the same content, regardless of
    /// position.
    pub fn content_id(&self) -> ContentId {
        self.info().content_id.get()
    }

    pub fn content_is_unique(&self) -> bool {
        self.info().content_is_unique.get()
    }

    pub fn num_ancestors(&self) -> u32 {
        self.info().num_ancestors.get()
    }

    pub fn dbg_content(&self) -> String {
        match self {
            List {
                open_content,
                open_position,
                close_content,
                ..
            } => {
                let line = open_position
                    .first()
                    .map(|p| p.line.display())
                    .unwrap_or_else(|| "?".to_owned());

                format!("line:{} {} ... {}", line, open_content, close_content)
            }
            Atom {
                content, position, ..
            } => {
                let line = position
                    .first()
                    .map_or_else(|| "?".to_owned(), |p| p.line.display());

                format!("line:{} {}", line, content)
            }
        }
    }
}

/// Initialise all the fields in `SyntaxInfo`.
pub fn init_all_info<'a>(lhs_roots: &[&'a Syntax<'a>], rhs_roots: &[&'a Syntax<'a>]) {
    init_info(lhs_roots, rhs_roots);
    init_next_prev(lhs_roots);
    init_next_prev(rhs_roots);
}

fn init_info<'a>(lhs_roots: &[&'a Syntax<'a>], rhs_roots: &[&'a Syntax<'a>]) {
    let mut id = NonZeroU32::new(1).unwrap();
    init_info_on_side(lhs_roots, &mut id);
    init_info_on_side(rhs_roots, &mut id);

    let mut existing = DftHashMap::default();
    set_content_id(lhs_roots, &mut existing);
    set_content_id(rhs_roots, &mut existing);

    set_content_is_unique(lhs_roots);
    set_content_is_unique(rhs_roots);
}

type ContentKey = (Option<String>, Option<String>, Vec<u32>, bool, bool);

fn set_content_id(nodes: &[&Syntax], existing: &mut DftHashMap<ContentKey, u32>) {
    for node in nodes {
        let key: ContentKey = match node {
            List {
                open_content,
                close_content,
                children,
                ..
            } => {
                // Recurse first, so children all have their content_id set.
                set_content_id(children, existing);

                let children_content_ids: Vec<_> =
                    children.iter().map(|c| c.info().content_id.get()).collect();

                (
                    Some(open_content.clone()),
                    Some(close_content.clone()),
                    children_content_ids,
                    true,
                    true,
                )
            }
            Atom {
                content,
                kind: highlight,
                ..
            } => {
                let is_comment = *highlight == AtomKind::Comment;
                let clean_content = if is_comment && split_on_newlines(content).count() > 1 {
                    split_on_newlines(content)
                        .map(|l| l.trim_start())
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    content.clone()
                };
                (Some(clean_content), None, vec![], false, is_comment)
            }
        };

        // Ensure the ID is always greater than zero, so we can
        // distinguish an uninitialised SyntaxInfo value.
        let next_id = existing.len() as u32 + 1;
        let content_id = existing.entry(key).or_insert(next_id);
        node.info().content_id.set(*content_id);
    }
}

fn set_num_after(nodes: &[&Syntax], parent_num_after: usize) {
    for (i, node) in nodes.iter().enumerate() {
        let num_after = parent_num_after + nodes.len() - 1 - i;
        node.info().num_after.set(num_after);

        if let List { children, .. } = node {
            set_num_after(children, num_after);
        }
    }
}

pub fn init_next_prev<'a>(roots: &[&'a Syntax<'a>]) {
    set_prev_sibling(roots);
    set_next_sibling(roots);
    set_prev(roots, None);
}

/// Set all the `SyntaxInfo` values for all the `roots` on a single
/// side (LHS or RHS).
fn init_info_on_side<'a>(roots: &[&'a Syntax<'a>], next_id: &mut SyntaxId) {
    set_parent(roots, None);
    set_num_ancestors(roots, 0);
    set_num_after(roots, 0);
    set_unique_id(roots, next_id);
}

fn set_unique_id(nodes: &[&Syntax], next_id: &mut SyntaxId) {
    for node in nodes {
        node.info().unique_id.set(*next_id);
        *next_id = NonZeroU32::new(u32::from(*next_id) + 1)
            .expect("Should not have more than u32::MAX nodes");
        if let List { children, .. } = node {
            set_unique_id(children, next_id);
        }
    }
}

/// Assumes that `set_content_id` has already run.
fn find_nodes_with_unique_content(nodes: &[&Syntax], counts: &mut DftHashMap<ContentId, usize>) {
    for node in nodes {
        *counts.entry(node.content_id()).or_insert(0) += 1;
        if let List { children, .. } = node {
            find_nodes_with_unique_content(children, counts);
        }
    }
}

fn set_content_is_unique_from_counts(nodes: &[&Syntax], counts: &DftHashMap<ContentId, usize>) {
    for node in nodes {
        let count = counts
            .get(&node.content_id())
            .expect("Count should be present");
        node.info().content_is_unique.set(*count == 1);

        if let List { children, .. } = node {
            set_content_is_unique_from_counts(children, counts);
        }
    }
}

fn set_content_is_unique(nodes: &[&Syntax]) {
    let mut counts = DftHashMap::default();
    find_nodes_with_unique_content(nodes, &mut counts);
    set_content_is_unique_from_counts(nodes, &counts);
}

fn set_prev_sibling<'a>(nodes: &[&'a Syntax<'a>]) {
    let mut prev = None;

    for node in nodes {
        node.info().previous_sibling.set(prev);
        prev = Some(node);

        if let List { children, .. } = node {
            set_prev_sibling(children);
        }
    }
}

fn set_next_sibling<'a>(nodes: &[&'a Syntax<'a>]) {
    for (i, node) in nodes.iter().enumerate() {
        let sibling = nodes.get(i + 1).copied();
        node.info().next_sibling.set(sibling);

        if let List { children, .. } = node {
            set_next_sibling(children);
        }
    }
}

/// For every syntax node in the tree, mark the previous node
/// according to a preorder traversal.
fn set_prev<'a>(nodes: &[&'a Syntax<'a>], parent: Option<&'a Syntax<'a>>) {
    for (i, node) in nodes.iter().enumerate() {
        let node_prev = if i == 0 { parent } else { Some(nodes[i - 1]) };

        node.info().prev.set(node_prev);
        if let List { children, .. } = node {
            set_prev(children, Some(node));
        }
    }
}

fn set_parent<'a>(nodes: &[&'a Syntax<'a>], parent: Option<&'a Syntax<'a>>) {
    for node in nodes {
        node.info().parent.set(parent);
        if let List { children, .. } = node {
            set_parent(children, Some(node));
        }
    }
}

fn set_num_ancestors(nodes: &[&Syntax], num_ancestors: u32) {
    for node in nodes {
        node.info().num_ancestors.set(num_ancestors);

        if let List { children, .. } = node {
            set_num_ancestors(children, num_ancestors + 1);
        }
    }
}

impl PartialEq for Syntax<'_> {
    fn eq(&self, other: &Self) -> bool {
        debug_assert!(self.content_id() > 0);
        debug_assert!(other.content_id() > 0);
        self.content_id() == other.content_id()
    }
}
impl<'a> Eq for Syntax<'a> {}

/// Different types of strings. We want to diff these the same way,
/// but highlight them differently.
#[derive(PartialEq, Eq, Debug, Clone, Copy, Hash)]
pub enum StringKind {
    /// A string literal, such as `"foo"`.
    StringLiteral,
    /// Plain text, such as the content of `<p>foo</p>`.
    Text,
}

#[derive(PartialEq, Eq, Debug, Clone, Copy, Hash)]
pub enum AtomKind {
    /// The kind of this atom when we don't know anything else about
    /// it. This is typically a variable, e.g. `foo`, or a literal
    /// `123`. Note that string literals have a separate kind.
    Normal,
    String(StringKind),
    Type,
    Comment,
    Keyword,
    TreeSitterError,
}
