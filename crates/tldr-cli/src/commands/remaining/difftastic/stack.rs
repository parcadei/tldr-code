use bumpalo::Bump;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Node<'b, T> {
    val: T,
    next: Option<&'b Node<'b, T>>,
}

/// A persistent stack.
///
/// This is similar to `Stack` from the rpds crate, but it's faster
/// and uses less memory.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stack<'b, T> {
    head: Option<&'b Node<'b, T>>,
}

impl<'b, T> Stack<'b, T> {
    pub fn new() -> Self {
        Self { head: None }
    }

    pub fn peek(&self) -> Option<&T> {
        self.head.map(|n| &n.val)
    }

    pub fn pop(&self) -> Option<Self> {
        self.head.map(|n| Self { head: n.next })
    }

    pub fn push(&self, v: T, alloc: &'b Bump) -> Self {
        Self {
            head: Some(alloc.alloc(Node {
                val: v,
                next: self.head,
            })),
        }
    }

    // O(n)
    pub fn size(&self) -> usize {
        std::iter::successors(self.head, |&n| n.next).count()
    }

    pub fn is_empty(&self) -> bool {
        self.head.is_none()
    }
}
