//! Lifetime prototype: proves Arena<Syntax> -> diff -> ChangeMap pipeline compiles.
//! This file will be removed after Phase 1 validation.

use bumpalo::Bump;
use std::cell::Cell;
use std::collections::HashMap;
use typed_arena::Arena;

/// Simplified stand-in for difftastic's Syntax<'a>
#[derive(Debug)]
pub enum ProtoSyntax<'a> {
    Atom {
        content: String,
        id: Cell<u32>,
        next_sibling: Cell<Option<&'a ProtoSyntax<'a>>>,
    },
    List {
        open: String,
        close: String,
        children: Vec<&'a ProtoSyntax<'a>>,
        id: Cell<u32>,
        next_sibling: Cell<Option<&'a ProtoSyntax<'a>>>,
    },
}

impl<'a> ProtoSyntax<'a> {
    fn id(&self) -> u32 {
        match self {
            ProtoSyntax::Atom { id, .. } => id.get(),
            ProtoSyntax::List { id, .. } => id.get(),
        }
    }

    fn next_sibling(&self) -> &Cell<Option<&'a ProtoSyntax<'a>>> {
        match self {
            ProtoSyntax::Atom { next_sibling, .. } => next_sibling,
            ProtoSyntax::List { next_sibling, .. } => next_sibling,
        }
    }
}

/// Simplified stand-in for ChangeKind
#[derive(Debug, Clone, PartialEq)]
pub enum ProtoChangeKind {
    Novel,
    Unchanged,
}

/// Simplified ChangeMap
type ProtoChangeMap = HashMap<u32, ProtoChangeKind>;

/// Simplified stand-in for Vertex<'s, 'v> in the graph
#[derive(Debug)]
struct ProtoVertex<'s, 'v> {
    _lhs: Option<&'s ProtoSyntax<'s>>,
    _rhs: Option<&'s ProtoSyntax<'s>>,
    _arena: &'v (),
}

/// The critical lifetime test: proves the full pipeline compiles
pub fn run_lifetime_pipeline() -> Vec<(u32, ProtoChangeKind)> {
    // 1. Syntax arena outlives everything
    let syntax_arena: Arena<ProtoSyntax> = Arena::new();

    // 2. Allocate nodes
    let lhs = syntax_arena.alloc(ProtoSyntax::Atom {
        content: "hello".to_string(),
        id: Cell::new(1),
        next_sibling: Cell::new(None),
    });
    let rhs = syntax_arena.alloc(ProtoSyntax::Atom {
        content: "world".to_string(),
        id: Cell::new(2),
        next_sibling: Cell::new(None),
    });

    // 3. ChangeMap borrows from arena via IDs
    let mut change_map = ProtoChangeMap::new();

    // 4. Bump arena for graph vertices (nested scope)
    {
        let vertex_arena = Bump::new();
        let _v = vertex_arena.alloc(ProtoVertex {
            _lhs: Some(lhs),
            _rhs: Some(rhs),
            _arena: &(),
        });
        // Simulate Dijkstra: vertex references syntax nodes
        // vertex_arena is dropped here
    }

    // 5. Write to change_map after vertex_arena is dropped
    change_map.insert(lhs.id(), ProtoChangeKind::Novel);
    change_map.insert(rhs.id(), ProtoChangeKind::Novel);

    // 6. Read results while syntax_arena is still alive
    let results: Vec<_> = change_map.into_iter().collect();

    // syntax_arena dropped here (end of function)
    results
}

/// Also test with List nodes and children references
pub fn run_lifetime_pipeline_with_lists() -> Vec<(u32, ProtoChangeKind)> {
    let syntax_arena: Arena<ProtoSyntax> = Arena::new();

    // Create children first
    let child1 = syntax_arena.alloc(ProtoSyntax::Atom {
        content: "x".to_string(),
        id: Cell::new(10),
        next_sibling: Cell::new(None),
    });
    let child2 = syntax_arena.alloc(ProtoSyntax::Atom {
        content: "y".to_string(),
        id: Cell::new(11),
        next_sibling: Cell::new(None),
    });

    // Set sibling link (Cell mutation)
    child1.next_sibling().set(Some(child2));

    // Create parent list containing children
    let list = syntax_arena.alloc(ProtoSyntax::List {
        open: "(".to_string(),
        close: ")".to_string(),
        children: vec![child1, child2],
        id: Cell::new(20),
        next_sibling: Cell::new(None),
    });

    let mut change_map = ProtoChangeMap::new();

    // Graph vertices reference both list and children
    {
        let vertex_arena = Bump::new();
        let _v1 = vertex_arena.alloc(ProtoVertex {
            _lhs: Some(list),
            _rhs: Some(child1),
            _arena: &(),
        });
        let _v2 = vertex_arena.alloc(ProtoVertex {
            _lhs: Some(child1),
            _rhs: Some(child2),
            _arena: &(),
        });
    }

    change_map.insert(list.id(), ProtoChangeKind::Unchanged);
    change_map.insert(child1.id(), ProtoChangeKind::Novel);
    change_map.insert(child2.id(), ProtoChangeKind::Novel);

    change_map.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifetime_prototype_atoms() {
        let results = run_lifetime_pipeline();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|(_, ck)| *ck == ProtoChangeKind::Novel));
    }

    #[test]
    fn lifetime_prototype_lists() {
        let results = run_lifetime_pipeline_with_lists();
        assert_eq!(results.len(), 3);
        let novels: Vec<_> = results
            .iter()
            .filter(|(_, ck)| *ck == ProtoChangeKind::Novel)
            .collect();
        assert_eq!(novels.len(), 2, "Two children should be Novel");
    }

    #[test]
    fn lifetime_prototype_sibling_links() {
        let arena: Arena<ProtoSyntax> = Arena::new();
        let a = arena.alloc(ProtoSyntax::Atom {
            content: "a".to_string(),
            id: Cell::new(1),
            next_sibling: Cell::new(None),
        });
        let b = arena.alloc(ProtoSyntax::Atom {
            content: "b".to_string(),
            id: Cell::new(2),
            next_sibling: Cell::new(None),
        });

        // Set sibling link via Cell (interior mutability)
        a.next_sibling().set(Some(b));

        // Walk the chain
        let mut current: Option<&ProtoSyntax> = Some(a);
        let mut count = 0;
        while let Some(node) = current {
            count += 1;
            current = node.next_sibling().get();
        }
        assert_eq!(count, 2);
    }
}
