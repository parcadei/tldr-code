//! Adapter: convert difftastic ChangeMap to our DiffReport format.
//!
//! This module provides:
//! - `changemap_to_l1_report`: walks the LHS and RHS syntax trees and emits
//!   flat, token-level `ASTChange` entries (L1 granularity).
//! - `changemap_to_l2_report`: walks the LHS and RHS syntax trees and emits
//!   expression-grouped `ASTChange` entries (L2 granularity), where token
//!   changes are wrapped under their nearest `Syntax::List` parent.

use super::changes::{ChangeKind, ChangeMap};
use super::syntax::Syntax;
use crate::commands::remaining::types::{
    ASTChange, ChangeType, DiffGranularity, DiffReport, DiffSummary, Location, NodeKind,
};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Convert a ChangeMap to a flat L1 (token-level) DiffReport.
///
/// Walks LHS nodes to emit Delete and Update changes, then RHS nodes to emit
/// Insert changes. Updates (ReplacedComment / ReplacedString) are emitted only
/// from the LHS walk to avoid duplicates.
pub fn changemap_to_l1_report<'a>(
    lhs_nodes: &[&'a Syntax<'a>],
    rhs_nodes: &[&'a Syntax<'a>],
    change_map: &ChangeMap<'a>,
    file_a: &str,
    file_b: &str,
) -> DiffReport {
    let mut changes: Vec<ASTChange> = Vec::new();

    // Pass 1: Walk LHS for Delete and Update changes
    walk_lhs_nodes(lhs_nodes, change_map, file_a, file_b, &mut changes);

    // Pass 2: Walk RHS for Insert changes
    walk_rhs_nodes(rhs_nodes, change_map, file_b, &mut changes);

    // Build summary
    let mut summary = DiffSummary::default();
    for change in &changes {
        summary.total_changes += 1;
        summary.semantic_changes += 1;
        match change.change_type {
            ChangeType::Insert => summary.inserts += 1,
            ChangeType::Delete => summary.deletes += 1,
            ChangeType::Update => summary.updates += 1,
            _ => {}
        }
    }

    DiffReport {
        file_a: file_a.to_string(),
        file_b: file_b.to_string(),
        identical: changes.is_empty(),
        changes,
        summary: Some(summary),
        granularity: DiffGranularity::Token,
        file_changes: None,
        module_changes: None,
        import_graph_summary: None,
        arch_changes: None,
        arch_summary: None,
    }
}

/// Convert a ChangeMap to an L2 (expression-level) DiffReport.
///
/// Token changes are grouped into expression-level parent changes.
/// A `Syntax::List` whose delimiters are `Unchanged` but whose children
/// contain changes is emitted as an expression-level `Update` with
/// `children = Some(token_changes)`. Entirely novel `Syntax::List` nodes
/// are emitted as expression-level `Insert`/`Delete` with `children = None`.
pub fn changemap_to_l2_report<'a>(
    lhs_nodes: &[&'a Syntax<'a>],
    rhs_nodes: &[&'a Syntax<'a>],
    change_map: &ChangeMap<'a>,
    file_a: &str,
    file_b: &str,
) -> DiffReport {
    let mut changes: Vec<ASTChange> = Vec::new();

    // Pass 1: Walk LHS for expression-level Delete and Update changes
    l2_walk_lhs_nodes(lhs_nodes, change_map, file_a, file_b, &mut changes);

    // Pass 2: Walk RHS for expression-level Insert changes
    l2_walk_rhs_nodes(rhs_nodes, change_map, file_b, &mut changes);

    // Build summary from top-level expression changes
    let mut summary = DiffSummary::default();
    for change in &changes {
        summary.total_changes += 1;
        summary.semantic_changes += 1;
        match change.change_type {
            ChangeType::Insert => summary.inserts += 1,
            ChangeType::Delete => summary.deletes += 1,
            ChangeType::Update => summary.updates += 1,
            _ => {}
        }
    }

    DiffReport {
        file_a: file_a.to_string(),
        file_b: file_b.to_string(),
        identical: changes.is_empty(),
        changes,
        summary: Some(summary),
        granularity: DiffGranularity::Expression,
        file_changes: None,
        module_changes: None,
        import_graph_summary: None,
        arch_changes: None,
        arch_summary: None,
    }
}

// ---------------------------------------------------------------------------
// L2 LHS walk (expression-level Delete + Update)
// ---------------------------------------------------------------------------

fn l2_walk_lhs_nodes<'a>(
    nodes: &[&'a Syntax<'a>],
    change_map: &ChangeMap<'a>,
    file_a: &str,
    file_b: &str,
    changes: &mut Vec<ASTChange>,
) {
    for node in nodes {
        l2_walk_lhs_node(node, change_map, file_a, file_b, changes);
    }
}

fn l2_walk_lhs_node<'a>(
    node: &'a Syntax<'a>,
    change_map: &ChangeMap<'a>,
    file_a: &str,
    file_b: &str,
    changes: &mut Vec<ASTChange>,
) {
    let change_kind = change_map.get(node);

    match node {
        Syntax::Atom {
            content, position, ..
        } => match change_kind {
            Some(ChangeKind::Novel) => {
                // Standalone atom delete (not grouped under a List)
                changes.push(ASTChange {
                    change_type: ChangeType::Delete,
                    node_kind: NodeKind::Expression,
                    name: Some(truncate_name(content)),
                    old_location: Some(span_to_location(position, file_a)),
                    new_location: None,
                    old_text: None,
                    new_text: None,
                    similarity: None,
                    children: None,
                    base_changes: None,
                });
            }
            Some(ChangeKind::ReplacedComment(_, rhs_node))
            | Some(ChangeKind::ReplacedString(_, rhs_node)) => {
                let (rhs_content, rhs_position) = atom_content_and_position(rhs_node);
                let sim = strsim::normalized_levenshtein(content, rhs_content);
                changes.push(ASTChange {
                    change_type: ChangeType::Update,
                    node_kind: NodeKind::Expression,
                    name: Some(truncate_name(content)),
                    old_location: Some(span_to_location(position, file_a)),
                    new_location: Some(span_to_location(rhs_position, file_b)),
                    old_text: Some(content.clone()),
                    new_text: Some(rhs_content.to_string()),
                    similarity: Some(sim),
                    children: None,
                    base_changes: None,
                });
            }
            Some(ChangeKind::Unchanged(_)) | None => {
                // Skip unchanged atoms
            }
        },
        Syntax::List {
            open_content,
            open_position,
            children,
            ..
        } => match change_kind {
            Some(ChangeKind::Novel) => {
                // Entire expression deleted (all novel)
                let name = if !open_content.is_empty() {
                    truncate_name(open_content)
                } else if let Some(first_child) = children.first() {
                    first_atom_name(first_child)
                } else {
                    "<list>".to_string()
                };
                changes.push(ASTChange {
                    change_type: ChangeType::Delete,
                    node_kind: NodeKind::Expression,
                    name: Some(name),
                    old_location: Some(span_to_location(open_position, file_a)),
                    new_location: None,
                    old_text: None,
                    new_text: None,
                    similarity: None,
                    children: None, // entire expression is novel; no children
                    base_changes: None,
                });
                // Do NOT recurse -- children are all Novel too
            }
            Some(ChangeKind::Unchanged(opposite)) => {
                // Delimiters matched. Check if any direct children on
                // EITHER side changed. A pure-insert scenario (LHS
                // children all Unchanged, RHS has Novel nodes) must
                // still be captured at this expression level.
                let lhs_has_changes = has_any_changed_child(children, change_map);
                let rhs_has_changes = if let Syntax::List {
                    children: opp_children,
                    ..
                } = opposite
                {
                    has_any_changed_child(opp_children, change_map)
                } else {
                    false
                };

                if lhs_has_changes || rhs_has_changes {
                    // This List has at least one direct child on one
                    // side that is Novel or replaced. Emit an
                    // expression-level Update with token-level children.
                    let mut child_changes: Vec<ASTChange> = Vec::new();

                    // Collect LHS (delete/update) token changes from children
                    walk_lhs_nodes(children, change_map, file_a, file_b, &mut child_changes);

                    // Get opposite (RHS) list's children and collect insert changes
                    if let Syntax::List {
                        children: opp_children,
                        ..
                    } = opposite
                    {
                        walk_rhs_nodes(opp_children, change_map, file_b, &mut child_changes);
                    }

                    if !child_changes.is_empty() {
                        let name = if !open_content.is_empty() {
                            truncate_name(open_content)
                        } else if let Some(first_child) = children.first() {
                            first_atom_name(first_child)
                        } else {
                            "<list>".to_string()
                        };

                        // Compute old_location from this list, new_location from opposite
                        let new_loc = match opposite {
                            Syntax::List {
                                open_position: opp_pos,
                                ..
                            } => Some(span_to_location(opp_pos, file_b)),
                            _ => None,
                        };

                        changes.push(ASTChange {
                            change_type: ChangeType::Update,
                            node_kind: NodeKind::Expression,
                            name: Some(name),
                            old_location: Some(span_to_location(open_position, file_a)),
                            new_location: new_loc,
                            old_text: None,
                            new_text: None,
                            similarity: None,
                            children: Some(child_changes),
                            base_changes: None,
                        });
                    }
                } else {
                    // Direct children on both sides are all Unchanged,
                    // but deeper descendants may have changes. Recurse
                    // to find the deepest List that directly contains
                    // changed children.
                    l2_walk_lhs_nodes(children, change_map, file_a, file_b, changes);
                }
            }
            None => {
                // No change info -- recurse into children to find changes deeper
                l2_walk_lhs_nodes(children, change_map, file_a, file_b, changes);
            }
            Some(ChangeKind::ReplacedComment(_, _)) | Some(ChangeKind::ReplacedString(_, _)) => {
                // Unusual for Lists but handle gracefully: recurse
                l2_walk_lhs_nodes(children, change_map, file_a, file_b, changes);
            }
        },
    }
}

// ---------------------------------------------------------------------------
// L2 RHS walk (expression-level Insert only)
// ---------------------------------------------------------------------------

fn l2_walk_rhs_nodes<'a>(
    nodes: &[&'a Syntax<'a>],
    change_map: &ChangeMap<'a>,
    file_b: &str,
    changes: &mut Vec<ASTChange>,
) {
    for node in nodes {
        l2_walk_rhs_node(node, change_map, file_b, changes);
    }
}

fn l2_walk_rhs_node<'a>(
    node: &'a Syntax<'a>,
    change_map: &ChangeMap<'a>,
    file_b: &str,
    changes: &mut Vec<ASTChange>,
) {
    let change_kind = change_map.get(node);

    match node {
        Syntax::Atom {
            content, position, ..
        } => match change_kind {
            Some(ChangeKind::Novel) => {
                // Standalone atom insert (not grouped under a List)
                changes.push(ASTChange {
                    change_type: ChangeType::Insert,
                    node_kind: NodeKind::Expression,
                    name: Some(truncate_name(content)),
                    old_location: None,
                    new_location: Some(span_to_location(position, file_b)),
                    old_text: None,
                    new_text: None,
                    similarity: None,
                    children: None,
                    base_changes: None,
                });
            }
            Some(ChangeKind::ReplacedComment(_, _)) | Some(ChangeKind::ReplacedString(_, _)) => {
                // Already emitted from LHS walk -- skip
            }
            Some(ChangeKind::Unchanged(_)) | None => {
                // Skip unchanged atoms
            }
        },
        Syntax::List {
            open_content,
            open_position,
            children,
            ..
        } => match change_kind {
            Some(ChangeKind::Novel) => {
                // Entire expression inserted (all novel)
                let name = if !open_content.is_empty() {
                    truncate_name(open_content)
                } else if let Some(first_child) = children.first() {
                    first_atom_name(first_child)
                } else {
                    "<list>".to_string()
                };
                changes.push(ASTChange {
                    change_type: ChangeType::Insert,
                    node_kind: NodeKind::Expression,
                    name: Some(name),
                    old_location: None,
                    new_location: Some(span_to_location(open_position, file_b)),
                    old_text: None,
                    new_text: None,
                    similarity: None,
                    children: None, // entire expression is novel; no children
                    base_changes: None,
                });
                // Do NOT recurse -- children are all Novel too
            }
            Some(ChangeKind::Unchanged(opposite)) => {
                // The LHS walk might have already emitted an Update for
                // this expression. But due to asymmetric ChangeMap
                // pairings (Dijkstra can overwrite entries), the LHS
                // walk might have seen a DIFFERENT opposite node and
                // missed changes visible here. We must handle:
                //
                // 1. Both sides unchanged -> recurse deeper
                // 2. This RHS has changed children -> check if LHS
                //    walk actually captured them (by checking the LHS
                //    node's opposite reference)
                // 3. Only paired LHS has changes -> LHS walk captured
                //    them; skip

                let this_rhs_has_changes = has_any_changed_child(children, change_map);

                if !this_rhs_has_changes {
                    // No changes on RHS side. Recurse deeper to find
                    // changes in nested Unchanged Lists.
                    l2_walk_rhs_nodes(children, change_map, file_b, changes);
                } else {
                    // RHS has changed children. Check whether the LHS
                    // walk already captured them. The LHS walk would
                    // have captured them if the LHS node's opposite
                    // reference points to THIS RHS node (symmetric
                    // pairing). If the LHS node's opposite points to
                    // a DIFFERENT node (asymmetric), we must handle
                    // the changes here.
                    let lhs_already_captured = if let Syntax::List { .. } = opposite {
                        // Check if the LHS node's ChangeKind::Unchanged
                        // points back to THIS node (symmetric pairing).
                        match change_map.get(opposite) {
                            Some(ChangeKind::Unchanged(lhs_opposite)) => {
                                // lhs_opposite is what the LHS node
                                // sees as its opposite. If it's THIS
                                // node, the pairing is symmetric and
                                // the LHS walk saw the same children.
                                std::ptr::eq(lhs_opposite, node)
                            }
                            _ => false,
                        }
                    } else {
                        false
                    };

                    if lhs_already_captured {
                        // LHS walk saw the same RHS node. It either
                        // emitted an Update (if it also detected the
                        // changes) or recursed. Either way, the RHS
                        // inserts are captured.
                        // Skip to avoid double-counting.
                    } else {
                        // Asymmetric pairing: LHS walk saw a different
                        // opposite. We must emit the RHS changes here.
                        // Collect the RHS children that are Novel.
                        l2_walk_rhs_nodes(children, change_map, file_b, changes);
                    }
                }
            }
            None => {
                // No change info -- recurse to find insertions deeper
                l2_walk_rhs_nodes(children, change_map, file_b, changes);
            }
            Some(ChangeKind::ReplacedComment(_, _)) | Some(ChangeKind::ReplacedString(_, _)) => {
                // Already emitted from LHS walk
            }
        },
    }
}

// ---------------------------------------------------------------------------
// L2 Helpers
// ---------------------------------------------------------------------------

/// Check if any direct child of a List has a non-Unchanged status.
fn has_any_changed_child<'a>(children: &[&'a Syntax<'a>], change_map: &ChangeMap<'a>) -> bool {
    children
        .iter()
        .any(|c| !matches!(change_map.get(c), Some(ChangeKind::Unchanged(_))))
}

/// Extract a display name from the first Atom descendant.
fn first_atom_name(node: &Syntax) -> String {
    match node {
        Syntax::Atom { content, .. } => truncate_name(content),
        Syntax::List {
            children,
            open_content,
            ..
        } => {
            if !open_content.is_empty() {
                truncate_name(open_content)
            } else if let Some(child) = children.first() {
                first_atom_name(child)
            } else {
                "<list>".to_string()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// L1 LHS walk (Delete + Update)
// ---------------------------------------------------------------------------

fn walk_lhs_nodes<'a>(
    nodes: &[&'a Syntax<'a>],
    change_map: &ChangeMap<'a>,
    file_a: &str,
    file_b: &str,
    changes: &mut Vec<ASTChange>,
) {
    for node in nodes {
        walk_lhs_node(node, change_map, file_a, file_b, changes);
    }
}

fn walk_lhs_node<'a>(
    node: &'a Syntax<'a>,
    change_map: &ChangeMap<'a>,
    file_a: &str,
    file_b: &str,
    changes: &mut Vec<ASTChange>,
) {
    let change_kind = change_map.get(node);

    match node {
        Syntax::Atom {
            content, position, ..
        } => match change_kind {
            Some(ChangeKind::Novel) => {
                changes.push(ASTChange {
                    change_type: ChangeType::Delete,
                    node_kind: NodeKind::Expression,
                    name: Some(truncate_name(content)),
                    old_location: Some(span_to_location(position, file_a)),
                    new_location: None,
                    old_text: None,
                    new_text: None,
                    similarity: None,
                    children: None,
                    base_changes: None,
                });
            }
            Some(ChangeKind::ReplacedComment(_, rhs_node))
            | Some(ChangeKind::ReplacedString(_, rhs_node)) => {
                let (rhs_content, rhs_position) = atom_content_and_position(rhs_node);
                let sim = strsim::normalized_levenshtein(content, rhs_content);
                changes.push(ASTChange {
                    change_type: ChangeType::Update,
                    node_kind: NodeKind::Expression,
                    name: Some(truncate_name(content)),
                    old_location: Some(span_to_location(position, file_a)),
                    new_location: Some(span_to_location(rhs_position, file_b)),
                    old_text: Some(content.clone()),
                    new_text: Some(rhs_content.to_string()),
                    similarity: Some(sim),
                    children: None,
                    base_changes: None,
                });
            }
            Some(ChangeKind::Unchanged(_)) | None => {
                // Skip unchanged atoms
            }
        },
        Syntax::List {
            open_content,
            open_position,
            children,
            close_content,
            close_position,
            ..
        } => match change_kind {
            Some(ChangeKind::Novel) => {
                // Emit deletes for the delimiters if they have content
                if !open_content.is_empty() {
                    changes.push(ASTChange {
                        change_type: ChangeType::Delete,
                        node_kind: NodeKind::Expression,
                        name: Some(truncate_name(open_content)),
                        old_location: Some(span_to_location(open_position, file_a)),
                        new_location: None,
                        old_text: None,
                        new_text: None,
                        similarity: None,
                        children: None,
                        base_changes: None,
                    });
                }
                // Recurse into children -- they will also be Novel
                walk_lhs_nodes(children, change_map, file_a, file_b, changes);
                if !close_content.is_empty() {
                    changes.push(ASTChange {
                        change_type: ChangeType::Delete,
                        node_kind: NodeKind::Expression,
                        name: Some(truncate_name(close_content)),
                        old_location: Some(span_to_location(close_position, file_a)),
                        new_location: None,
                        old_text: None,
                        new_text: None,
                        similarity: None,
                        children: None,
                        base_changes: None,
                    });
                }
            }
            Some(ChangeKind::Unchanged(_)) | None => {
                // Delimiters are unchanged but children may differ -- recurse
                walk_lhs_nodes(children, change_map, file_a, file_b, changes);
            }
            Some(ChangeKind::ReplacedComment(_, _)) | Some(ChangeKind::ReplacedString(_, _)) => {
                // Unusual for Lists but handle gracefully: recurse into children
                walk_lhs_nodes(children, change_map, file_a, file_b, changes);
            }
        },
    }
}

// ---------------------------------------------------------------------------
// RHS walk (Insert only)
// ---------------------------------------------------------------------------

fn walk_rhs_nodes<'a>(
    nodes: &[&'a Syntax<'a>],
    change_map: &ChangeMap<'a>,
    file_b: &str,
    changes: &mut Vec<ASTChange>,
) {
    for node in nodes {
        walk_rhs_node(node, change_map, file_b, changes);
    }
}

fn walk_rhs_node<'a>(
    node: &'a Syntax<'a>,
    change_map: &ChangeMap<'a>,
    file_b: &str,
    changes: &mut Vec<ASTChange>,
) {
    let change_kind = change_map.get(node);

    match node {
        Syntax::Atom {
            content, position, ..
        } => match change_kind {
            Some(ChangeKind::Novel) => {
                changes.push(ASTChange {
                    change_type: ChangeType::Insert,
                    node_kind: NodeKind::Expression,
                    name: Some(truncate_name(content)),
                    old_location: None,
                    new_location: Some(span_to_location(position, file_b)),
                    old_text: None,
                    new_text: None,
                    similarity: None,
                    children: None,
                    base_changes: None,
                });
            }
            Some(ChangeKind::ReplacedComment(_, _)) | Some(ChangeKind::ReplacedString(_, _)) => {
                // Already emitted from LHS walk -- skip
            }
            Some(ChangeKind::Unchanged(_)) | None => {
                // Skip unchanged atoms
            }
        },
        Syntax::List {
            open_content,
            open_position,
            children,
            close_content,
            close_position,
            ..
        } => match change_kind {
            Some(ChangeKind::Novel) => {
                // Emit inserts for the delimiters if they have content
                if !open_content.is_empty() {
                    changes.push(ASTChange {
                        change_type: ChangeType::Insert,
                        node_kind: NodeKind::Expression,
                        name: Some(truncate_name(open_content)),
                        old_location: None,
                        new_location: Some(span_to_location(open_position, file_b)),
                        old_text: None,
                        new_text: None,
                        similarity: None,
                        children: None,
                        base_changes: None,
                    });
                }
                // Recurse into children -- they will also be Novel
                walk_rhs_nodes(children, change_map, file_b, changes);
                if !close_content.is_empty() {
                    changes.push(ASTChange {
                        change_type: ChangeType::Insert,
                        node_kind: NodeKind::Expression,
                        name: Some(truncate_name(close_content)),
                        old_location: None,
                        new_location: Some(span_to_location(close_position, file_b)),
                        old_text: None,
                        new_text: None,
                        similarity: None,
                        children: None,
                        base_changes: None,
                    });
                }
            }
            Some(ChangeKind::Unchanged(_)) | None => {
                // Delimiters are unchanged but children may differ -- recurse
                walk_rhs_nodes(children, change_map, file_b, changes);
            }
            Some(ChangeKind::ReplacedComment(_, _)) | Some(ChangeKind::ReplacedString(_, _)) => {
                // Unusual for Lists but handle gracefully: recurse into children
                walk_rhs_nodes(children, change_map, file_b, changes);
            }
        },
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract content and position from an Atom node.
/// Panics if the node is not an Atom (should only be called on matched nodes).
fn atom_content_and_position<'a>(
    node: &'a Syntax<'a>,
) -> (&'a str, &'a [line_numbers::SingleLineSpan]) {
    match node {
        Syntax::Atom {
            content, position, ..
        } => (content.as_str(), position.as_slice()),
        Syntax::List { .. } => {
            // Fallback: should not happen for ReplacedComment/ReplacedString
            // but return empty data rather than panicking
            ("", &[])
        }
    }
}

/// Convert a slice of `SingleLineSpan` to our `Location` type.
///
/// `SingleLineSpan.line` is a `LineNumber(pub u32)` that is 0-indexed.
/// Our `Location.line` is 1-indexed.
fn span_to_location(spans: &[line_numbers::SingleLineSpan], file: &str) -> Location {
    match spans {
        [] => Location::new(file, 1),
        [first, ..] => {
            let line = first.line.0 + 1; // Convert from 0-indexed to 1-indexed
            let col = first.start_col;
            let last = spans.last().unwrap();
            let end_line = last.line.0 + 1;
            let end_col = last.end_col;
            Location {
                file: file.to_string(),
                line,
                column: col,
                end_line: Some(end_line),
                end_column: Some(end_col),
            }
        }
    }
}

/// Truncate a token name to at most 80 characters, appending "..." if truncated.
fn truncate_name(content: &str) -> String {
    // Replace newlines with spaces for display
    let cleaned: String = content
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    if cleaned.len() <= 80 {
        cleaned
    } else {
        let mut s = cleaned[..80].to_string();
        s.push_str("...");
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_name_short() {
        assert_eq!(truncate_name("hello"), "hello");
    }

    #[test]
    fn test_truncate_name_long() {
        let long = "a".repeat(100);
        let result = truncate_name(&long);
        assert_eq!(result.len(), 83); // 80 + "..."
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_truncate_name_newlines() {
        assert_eq!(truncate_name("foo\nbar"), "foo bar");
    }
}
