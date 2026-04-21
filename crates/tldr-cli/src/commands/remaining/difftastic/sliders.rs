//! Prefer contiguous novel nodes on the same line.
//!
//! A slider takes the following form:
//!
//! Old:
//!
//! ```text
//! A B
//! C D
//! ```
//!
//! New:
//!
//! ```text
//! A B
//! A B
//! C D
//! ```
//!
//! It would be correct, but ugly, to show the following diff:
//!
//! ```text
//! A +B+
//! +A+ B
//! C D
//! ```
//!
//! This module fixes these cases. It identifies situations where we
//! can change which item is marked as novel (e.g. either `B` in the
//! example above) whilst still showing a valid, minimal diff.
//!
//! A similar problem exists with line-oriented diffs, see
//! [diff-slider-tools](https://github.com/mhagger/diff-slider-tools)
//! for a thorough discussion.
//!
//! Vendored from difftastic with modifications:
//! - Import paths rewritten from crate:: to super::
//! - Tests stripped
//! - prefer_outer_delimiter function removed (RISK-18)
//! - Language parameter removed: fix_all_sliders takes no language arg
//! - prefer_outer = false for all our 18 languages (C-like, not Lisp)

use line_numbers::SingleLineSpan;

use super::{
    changes::{insert_deep_novel, insert_deep_unchanged, ChangeKind::*, ChangeMap},
    syntax::Syntax::{self, *},
};

/// Fix all sliders in the given nodes.
///
/// The Language parameter has been removed (RISK-18 simplification).
/// For all 18 languages we support (C-like, not Lisp), prefer_outer = false.
pub fn fix_all_sliders<'a>(nodes: &[&'a Syntax<'a>], change_map: &mut ChangeMap<'a>) {
    // TODO: fix sliders that require more than two steps.
    fix_all_sliders_one_step(nodes, change_map);
    fix_all_sliders_one_step(nodes, change_map);

    // prefer_outer = false for all our 18 languages (C-like, not Lisp)
    fix_all_nested_sliders(false, nodes, change_map);
}

fn fix_all_sliders_one_step<'a>(nodes: &[&'a Syntax<'a>], change_map: &mut ChangeMap<'a>) {
    for node in nodes {
        if let List { children, .. } = node {
            fix_all_sliders_one_step(children, change_map);
        }
    }
    fix_sliders(nodes, change_map);
}

/// Correct sliders in middle insertions.
///
/// Consider the code:
///
/// ```text
/// // Before
/// old1(old2);
/// // After
/// old1(new1(old2));
/// ```
///
/// Tree diffing has two possible solution here. Either we've added
/// `new1(...)` or we've added `(new1...)`. Both are valid.
///
/// For C-like languages, the first case matches human intuition much
/// better. Fix the slider to make the inner delimiter novel.
fn fix_all_nested_sliders<'a>(
    prefer_outer: bool,
    nodes: &[&'a Syntax<'a>],
    change_map: &mut ChangeMap<'a>,
) {
    for node in nodes {
        if prefer_outer {
            fix_nested_slider_prefer_outer(node, change_map);
        } else {
            fix_nested_slider_prefer_inner(node, change_map);
        }
    }
}

/// When we see code of the form `(old-1 (novel (old-2)))`, prefer
/// treating the outer delimiter as novel, so `(novel ...)` in this
/// example.
fn fix_nested_slider_prefer_outer<'a>(node: &'a Syntax<'a>, change_map: &mut ChangeMap<'a>) {
    if let List { children, .. } = node {
        match change_map
            .get(node)
            .expect("Changes should be set before slider correction")
        {
            Unchanged(_) => {
                let mut candidates = vec![];
                unchanged_descendants_for_outer_slider(children, &mut candidates, change_map);

                // We can slide if there is a single unchanged
                // descendant, that descendant is a list, and that
                // list has novel delimiters.
                if let [candidate] = candidates[..] {
                    if matches!(candidate, List { .. })
                        && matches!(change_map.get(candidate), Some(Novel))
                    {
                        push_unchanged_to_descendant(node, candidate, change_map);
                    }
                }
            }
            ReplacedComment(_, _) | ReplacedString(_, _) | Novel => {}
        }

        for child in children {
            fix_nested_slider_prefer_outer(child, change_map);
        }
    }
}

/// When we see code of the form `old1(novel(old2()))`, prefer
/// treating the inner delimiter as novel, so `novel(...)` in this
/// example.
fn fix_nested_slider_prefer_inner<'a>(node: &'a Syntax<'a>, change_map: &mut ChangeMap<'a>) {
    if let List { children, .. } = node {
        match change_map
            .get(node)
            .expect("Changes should be set before slider correction")
        {
            Unchanged(_) => {}
            ReplacedComment(_, _) | ReplacedString(_, _) => {}
            Novel => {
                let mut found_unchanged = vec![];
                unchanged_descendants(children, &mut found_unchanged, change_map);

                if let [List { .. }] = found_unchanged[..] {
                    push_unchanged_to_ancestor(node, found_unchanged[0], change_map);
                }
            }
        }

        for child in children {
            fix_nested_slider_prefer_inner(child, change_map);
        }
    }
}

/// Find the unchanged descendants of `nodes`.
fn unchanged_descendants<'a>(
    nodes: &[&'a Syntax<'a>],
    found: &mut Vec<&'a Syntax<'a>>,
    change_map: &ChangeMap<'a>,
) {
    // We're only interested if there is exactly one unchanged
    // descendant, so return early if we find 2 or more.
    if found.len() > 1 {
        return;
    }

    for node in nodes {
        match change_map.get(node).unwrap() {
            Unchanged(_) => {
                found.push(node);
            }
            Novel | ReplacedComment(_, _) | ReplacedString(_, _) => {
                if let List { children, .. } = node {
                    unchanged_descendants(children, found, change_map);
                }
            }
        }
    }
}

/// Nested sliders require a single unchanged descendant whose
/// delimiters we can slide.
///
/// ```text
/// (old-1 (novel (old-2)))
/// ```
///
/// To slide, we want a single list that contains unchanged items but
/// the outer delimiters are novel.
///
/// Find all the unchanged descendants.
fn unchanged_descendants_for_outer_slider<'a>(
    nodes: &[&'a Syntax<'a>],
    found: &mut Vec<&'a Syntax<'a>>,
    change_map: &ChangeMap<'a>,
) {
    // We're only interested if there is exactly one unchanged
    // descendant, so return early if we find 2 or more.
    if found.len() > 1 {
        return;
    }

    for node in nodes {
        let is_unchanged = matches!(change_map.get(node), Some(Unchanged(_)));

        match node {
            Atom { .. } => {
                if is_unchanged {
                    // If there's an unchanged atom descendant, we
                    // can't slide. Sliding the delimiters requires a
                    // single list, or we are potentially changing the
                    // diff semantically.
                    //
                    // Add to the found items, but terminate early
                    // since we'll never slide.
                    found.push(node);
                    break;
                } else {
                    // Novel atom. This is fine, we're looking for a
                    // single unchanged node.
                }
            }
            List { children, .. } => {
                if is_unchanged {
                    // This list is unchanged, and the delimiters are
                    // unchanged. It's an unchanged descendant, but we
                    // won't be able to slide its delimiters because
                    // its delimiters are unchanged.
                    //
                    // Add to the found items, but terminate early
                    // since we'll never slide.
                    found.push(node);
                    break;
                } else {
                    // A list whose outer delimiters are novel.

                    let has_unchanged_children = children
                        .iter()
                        .any(|node| matches!(change_map.get(node), Some(Unchanged(_))));
                    if has_unchanged_children {
                        // The list has unchanged children and novel
                        // delimiters. This is a candidate for
                        // sliding.
                        found.push(node);
                    } else {
                        // All of the immediate children are novel,
                        // recurse in case they have descendants that
                        // are unchanged.
                        unchanged_descendants_for_outer_slider(children, found, change_map);
                    }
                }
            }
        }
    }
}

/// Given a nested list where the root delimiters are unchanged but
/// the inner list's delimiters are novel, mark the inner list as
/// unchanged instead.
fn push_unchanged_to_descendant<'a>(
    root: &'a Syntax<'a>,
    inner: &'a Syntax<'a>,
    change_map: &mut ChangeMap<'a>,
) {
    let root_change = change_map
        .get(root)
        .expect("Changes should be set before slider correction");

    let delimiters_match = match (root, inner) {
        (
            List {
                open_content: root_open,
                close_content: root_close,
                ..
            },
            List {
                open_content: inner_open,
                close_content: inner_close,
                ..
            },
        ) => root_open == inner_open && root_close == inner_close,
        _ => false,
    };

    if delimiters_match {
        change_map.insert(root, Novel);
        change_map.insert(inner, root_change);
    }
}

/// Given a nested list where the root delimiters are novel but
/// the inner list's delimiters are unchanged, mark the root list as
/// unchanged instead.
fn push_unchanged_to_ancestor<'a>(
    root: &'a Syntax<'a>,
    inner: &'a Syntax<'a>,
    change_map: &mut ChangeMap<'a>,
) {
    let inner_change = change_map.get(inner).expect("Node changes should be set");

    let delimiters_match = match (root, inner) {
        (
            List {
                open_content: root_open,
                close_content: root_close,
                ..
            },
            List {
                open_content: inner_open,
                close_content: inner_close,
                ..
            },
        ) => root_open == inner_open && root_close == inner_close,
        _ => false,
    };

    if delimiters_match {
        change_map.insert(root, inner_change);
        change_map.insert(inner, Novel);
    }
}

/// For every sequence of novel nodes, if it's a potential slider,
/// change which nodes are marked as novel if it produces a sequence
/// of nodes that are closer together.
fn fix_sliders<'a>(nodes: &[&'a Syntax<'a>], change_map: &mut ChangeMap<'a>) {
    for (region_start, region_end) in novel_regions_after_unchanged(nodes, change_map) {
        slide_to_prev_node(nodes, change_map, region_start, region_end);
    }
    for (region_start, region_end) in novel_regions_before_unchanged(nodes, change_map) {
        slide_to_next_node(nodes, change_map, region_start, region_end);
    }
}

/// Return the start and end indexes of sequences of novel nodes that
/// occur after unchanged nodes.
fn novel_regions_after_unchanged<'a>(
    nodes: &[&'a Syntax<'a>],
    change_map: &ChangeMap<'a>,
) -> Vec<(usize, usize)> {
    let mut regions: Vec<Vec<usize>> = vec![];
    let mut region: Option<Vec<usize>> = None;

    for (i, node) in nodes.iter().enumerate() {
        let change = change_map.get(node).expect("Node changes should be set");

        match change {
            Unchanged(_) => {
                // Could have just finished a novel region.
                if let Some(region) = region {
                    regions.push(region);
                }

                // Could be the unchanged node before a novel region.
                region = Some(vec![]);
            }
            Novel => {
                if let Some(mut r) = region {
                    r.push(i);
                    region = Some(r);
                }
            }
            ReplacedComment(_, _) | ReplacedString(_, _) => {
                // Could have just finished a novel region.
                if let Some(region) = region {
                    regions.push(region);
                }

                region = None;
            }
        }
    }

    if let Some(region) = region {
        regions.push(region);
    }

    regions
        .into_iter()
        .filter(|r| !r.is_empty())
        .map(|r| (*r.first().unwrap(), *r.last().unwrap()))
        .collect()
}

/// Return the start and end indexes of sequences of novel nodes that
/// occur before unchanged nodes.
fn novel_regions_before_unchanged<'a>(
    nodes: &[&'a Syntax<'a>],
    change_map: &ChangeMap<'a>,
) -> Vec<(usize, usize)> {
    let mut regions: Vec<Vec<usize>> = vec![];
    let mut region: Option<Vec<usize>> = None;

    for (i, node) in nodes.iter().enumerate() {
        let change = change_map.get(node).expect("Node changes should be set");

        match change {
            Unchanged(_) => {
                // Could have just finished a novel region.
                if let Some(region) = region {
                    regions.push(region);
                }

                region = None;
            }
            Novel => {
                let mut r = region.unwrap_or_default();
                r.push(i);
                region = Some(r);
            }
            ReplacedComment(_, _) | ReplacedString(_, _) => {
                region = None;
            }
        }
    }

    if let Some(region) = region {
        regions.push(region);
    }

    regions
        .into_iter()
        .filter(|r| !r.is_empty())
        .map(|r| (*r.first().unwrap(), *r.last().unwrap()))
        .collect()
}

fn is_novel_deep<'a>(node: &Syntax<'a>, change_map: &ChangeMap<'a>) -> bool {
    match node {
        List { children, .. } => {
            if !matches!(change_map.get(node), Some(Novel)) {
                return false;
            }
            for child in children {
                if !is_novel_deep(child, change_map) {
                    return false;
                }
            }

            true
        }
        Atom { .. } => matches!(change_map.get(node), Some(Novel)),
    }
}

/// If the previous node is unchanged, matches the end of the region,
/// and has a smaller text distance, mark it as novel.
///
/// ```text
/// x UNCHANGED
/// y NOVEL <- start_idx
///
/// x NOVEL <- end_idx
/// ```
///
/// After this function:
///
/// ```text
/// x NOVEL
/// y NOVEL
///
/// x UNCHANGED
/// ```
fn slide_to_prev_node<'a>(
    nodes: &[&'a Syntax<'a>],
    change_map: &mut ChangeMap<'a>,
    start_idx: usize,
    end_idx: usize,
) {
    if start_idx == 0 {
        return;
    }
    if start_idx == end_idx {
        return;
    }

    let start_node = nodes[start_idx];
    let last_node = nodes[end_idx];
    let before_start_node = nodes[start_idx - 1];
    let before_last_node = nodes[end_idx - 1];

    if before_start_node.content_id() != last_node.content_id() {
        return;
    }

    let distance_to_before_start = distance_between(before_start_node, start_node);
    let distance_to_last = distance_between(before_last_node, last_node);

    if distance_to_before_start <= distance_to_last {
        let opposite = match change_map
            .get(before_start_node)
            .expect("Node changes should be set")
        {
            Unchanged(n) => {
                if before_start_node.content_id() != n.content_id() {
                    return;
                }
                n
            }
            _ => {
                return;
            }
        };

        for node in &nodes[start_idx..=end_idx] {
            if !is_novel_deep(node, change_map) {
                return;
            }
        }

        insert_deep_novel(before_start_node, change_map);
        insert_deep_unchanged(last_node, opposite, change_map);
        insert_deep_unchanged(opposite, last_node, change_map);
    }
}

/// If the next node is unchanged, matches the beginning of the region,
/// and has a smaller text distance, mark it as novel.
///
/// ```text
/// x NOVEL <- start_idx
///
/// y NOVEL <- end_idx
/// x UNCHANGED
/// ```
///
/// After this function:
///
/// ```text
/// x UNCHANGED
///
/// y NOVEL
/// x NOVEL
/// ```
fn slide_to_next_node<'a>(
    nodes: &[&'a Syntax<'a>],
    change_map: &mut ChangeMap<'a>,
    start_idx: usize,
    end_idx: usize,
) {
    if end_idx == nodes.len() - 1 {
        return;
    }
    if start_idx == end_idx {
        return;
    }

    let start_node = nodes[start_idx];
    let last_node = nodes[end_idx];
    let after_start_node = nodes[start_idx + 1];
    let after_last_node = nodes[end_idx + 1];

    if after_last_node.content_id() != start_node.content_id() {
        return;
    }

    let distance_to_start = distance_between(start_node, after_start_node);
    let distance_to_after_last = distance_between(last_node, after_last_node);

    if distance_to_after_last < distance_to_start {
        let opposite = match change_map
            .get(after_last_node)
            .expect("Node changes should be set")
        {
            Unchanged(n) => {
                if after_last_node.content_id() != n.content_id() {
                    return;
                }
                n
            }
            _ => {
                return;
            }
        };
        for node in &nodes[start_idx..=end_idx] {
            if !is_novel_deep(node, change_map) {
                return;
            }
        }

        insert_deep_unchanged(start_node, opposite, change_map);
        insert_deep_unchanged(opposite, start_node, change_map);
        insert_deep_novel(after_last_node, change_map);
    }
}

/// Return the distance between two syntax nodes, as a tuple of number
/// of lines and number of columns.
fn distance_between(prev: &Syntax, next: &Syntax) -> (u32, u32) {
    let prev_pos = prev.last_line_span();
    let next_pos = next.first_line_span();

    if let (Some(prev_pos), Some(next_pos)) = (prev_pos, next_pos) {
        if prev_pos.line != next_pos.line {
            return (next_pos.line.0 - prev_pos.line.0, 0);
        }

        return (0, next_pos.start_col - prev_pos.end_col);
    }

    (0, 0)
}

impl Syntax<'_> {
    fn first_line_span(&self) -> Option<SingleLineSpan> {
        match self {
            List {
                open_position,
                children,
                close_position,
                ..
            } => {
                if let Some(position) = open_position.first() {
                    return Some(*position);
                }
                for child in children {
                    if let Some(position) = child.first_line_span() {
                        return Some(position);
                    }
                }

                close_position.first().copied()
            }
            Atom { position, .. } => position.first().copied(),
        }
    }

    fn last_line_span(&self) -> Option<SingleLineSpan> {
        match self {
            List {
                open_position,
                children,
                close_position,
                ..
            } => {
                if let Some(position) = close_position.last() {
                    return Some(*position);
                }
                for child in children.iter().rev() {
                    if let Some(position) = child.last_line_span() {
                        return Some(position);
                    }
                }

                open_position.last().copied()
            }
            Atom { position, .. } => position.last().copied(),
        }
    }
}
