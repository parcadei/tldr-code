//! Implements Dijkstra's algorithm for shortest path, to find an
//! optimal and readable diff between two ASTs.
//!
//! Vendored from difftastic with modifications:
//! - Import paths rewritten from crate:: to super::
//! - All logging removed (no log/humansize dependency)
//! - Tests stripped
//! - DEFAULT_GRAPH_LIMIT inlined

use std::cmp::Reverse;

use bumpalo::Bump;
use radix_heap::RadixHeapMap;

use super::{
    changes::ChangeMap,
    graph::{populate_change_map, set_neighbours, Edge, Vertex},
    hash::DftHashMap,
    syntax::Syntax,
};

/// The default limit on the number of graph nodes before bailing out.
/// Inlined from crate::options::DEFAULT_GRAPH_LIMIT.
pub const DEFAULT_GRAPH_LIMIT: usize = 3_000_000;

#[derive(Debug)]
pub struct ExceededGraphLimit {}

/// Return the shortest route from `start` to the end vertex.
fn shortest_vertex_path<'s, 'v>(
    start: &'v Vertex<'s, 'v>,
    vertex_arena: &'v Bump,
    size_hint: usize,
    graph_limit: usize,
) -> Result<Vec<&'v Vertex<'s, 'v>>, ExceededGraphLimit> {
    // We want to visit nodes with the shortest distance first, but
    // RadixHeapMap is a max-heap. Ensure nodes are wrapped with
    // Reverse to flip comparisons.
    let mut heap: RadixHeapMap<Reverse<_>, &'v Vertex<'s, 'v>> = RadixHeapMap::new();

    heap.push(Reverse(0), start);

    let mut seen = DftHashMap::default();
    seen.reserve(size_hint);

    let end: &'v Vertex<'s, 'v> = loop {
        match heap.pop() {
            Some((Reverse(distance), current)) => {
                if current.is_end() {
                    break current;
                }

                set_neighbours(current, vertex_arena, &mut seen);
                for neighbour in *current.neighbours.borrow().as_ref().unwrap() {
                    let (edge, next) = neighbour;
                    let distance_to_next = distance + edge.cost();

                    let found_shorter_route = match next.predecessor.get() {
                        Some((prev_shortest, _)) => distance_to_next < prev_shortest,
                        None => true,
                    };

                    if found_shorter_route {
                        next.predecessor.replace(Some((distance_to_next, current)));
                        heap.push(Reverse(distance_to_next), next);
                    }
                }

                if seen.len() > graph_limit {
                    return Err(ExceededGraphLimit {});
                }
            }
            None => panic!("Ran out of graph nodes before reaching end"),
        }
    };

    let mut current = Some((0, end));
    let mut vertex_route: Vec<&'v Vertex<'s, 'v>> = vec![];
    while let Some((_, node)) = current {
        vertex_route.push(node);
        current = node.predecessor.get();
    }

    vertex_route.reverse();
    Ok(vertex_route)
}

fn shortest_path_with_edges<'s, 'v>(
    route: &[&'v Vertex<'s, 'v>],
) -> Vec<(Edge, &'v Vertex<'s, 'v>)> {
    let mut prev = route.first().expect("Expected non-empty route");

    let mut res = vec![];

    for vertex in route.iter().skip(1) {
        let edge = edge_between(prev, vertex);
        res.push((edge, *prev));

        prev = vertex;
    }

    res
}

/// Return the shortest route from the `start` to the end vertex.
///
/// The vec returned does not return the very last vertex. This is
/// necessary because a route of N vertices only has N-1 edges.
fn shortest_path<'s, 'v>(
    start: Vertex<'s, 'v>,
    vertex_arena: &'v Bump,
    size_hint: usize,
    graph_limit: usize,
) -> Result<Vec<(Edge, &'v Vertex<'s, 'v>)>, ExceededGraphLimit> {
    let start: &'v Vertex<'s, 'v> = vertex_arena.alloc(start);
    let vertex_path = shortest_vertex_path(start, vertex_arena, size_hint, graph_limit)?;
    Ok(shortest_path_with_edges(&vertex_path))
}

fn edge_between<'s, 'v>(before: &Vertex<'s, 'v>, after: &Vertex<'s, 'v>) -> Edge {
    assert_ne!(before, after);

    let mut shortest_edge: Option<Edge> = None;
    if let Some(neighbours) = &*before.neighbours.borrow() {
        for neighbour in *neighbours {
            let (edge, next) = *neighbour;
            // If there are multiple edges that can take us to `next`,
            // prefer the shortest.
            if *next == *after {
                let is_shorter = match shortest_edge {
                    Some(prev_edge) => edge.cost() < prev_edge.cost(),
                    None => true,
                };

                if is_shorter {
                    shortest_edge = Some(edge);
                }
            }
        }
    }

    if let Some(edge) = shortest_edge {
        return edge;
    }

    panic!(
        "Expected a route between the two vertices {:#?} and {:#?}",
        before, after
    );
}

/// What is the total number of AST nodes?
fn node_count(root: Option<&Syntax>) -> u32 {
    let iter = std::iter::successors(root, |node| node.next_sibling());

    iter.map(|node| match node {
        Syntax::List {
            num_descendants, ..
        } => *num_descendants,
        Syntax::Atom { .. } => 1,
    })
    .sum::<u32>()
}

pub fn mark_syntax<'a>(
    lhs_syntax: Option<&'a Syntax<'a>>,
    rhs_syntax: Option<&'a Syntax<'a>>,
    change_map: &mut ChangeMap<'a>,
    graph_limit: usize,
) -> Result<(), ExceededGraphLimit> {
    let lhs_node_count = node_count(lhs_syntax) as usize;
    let rhs_node_count = node_count(rhs_syntax) as usize;

    // When there are a large number of changes, we end up building a
    // graph whose size is roughly quadratic. Use this as a size hint,
    // so we don't spend too much time re-hashing and expanding the
    // predecessors hashmap.
    //
    // Cap this number to the graph limit, so we don't try to allocate
    // an absurdly large (i.e. greater than physical memory) hashmap
    // when there is a large number of nodes. We'll never visit more
    // than graph_limit nodes.
    let size_hint = std::cmp::min(lhs_node_count * rhs_node_count, graph_limit);

    let start = Vertex::new(lhs_syntax, rhs_syntax);
    let vertex_arena = Bump::new();

    let route = shortest_path(start, &vertex_arena, size_hint, graph_limit)?;

    populate_change_map(&route, change_map);
    Ok(())
}
