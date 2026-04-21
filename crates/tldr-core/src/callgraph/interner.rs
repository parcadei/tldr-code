//! String interning for memory-efficient call graph storage.
//!
//! Without interning, a call graph with 5M edges uses ~1.9GB for path strings alone.
//! With interning, the same graph uses ~80MB (24x reduction).
//!
//! # Overview
//!
//! This module provides three interners:
//!
//! 1. [`StringInterner`] - Single-threaded string interner for general use
//! 2. [`PathInterner`] - Specialized for file paths with normalization (backslash -> forward slash)
//! 3. [`ConcurrentInterner`] - Thread-safe interner for parallel processing
//!
//! # Example
//!
//! ```rust
//! use tldr_core::callgraph::interner::{StringInterner, PathInterner, InternedId};
//! use std::path::Path;
//!
//! // Basic string interning
//! let mut interner = StringInterner::new();
//! let id1 = interner.intern("hello");
//! let id2 = interner.intern("hello"); // Returns same ID
//! assert_eq!(id1, id2);
//!
//! // Path interning with normalization
//! let mut path_interner = PathInterner::new();
//! let id_unix = path_interner.intern_path(Path::new("src/main.rs"));
//! let id_win = path_interner.intern_path(Path::new("src\\main.rs")); // Backslash normalized
//! assert_eq!(id_unix, id_win);
//! ```

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::RwLock;

/// Opaque ID for an interned string.
///
/// This is a lightweight handle (4 bytes) that can be used instead of
/// storing full strings. It implements `Copy`, `Clone`, `Hash`, `Eq`,
/// making it suitable for use as HashMap keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct InternedId(u32);

impl InternedId {
    /// Returns the raw ID value.
    ///
    /// This is primarily useful for debugging or serialization.
    #[inline]
    pub fn as_u32(self) -> u32 {
        self.0
    }

    /// Creates an InternedId from a raw u32 value.
    ///
    /// # Safety (logical)
    ///
    /// This should only be used when you know the ID is valid
    /// (e.g., deserializing a previously-serialized ID).
    #[inline]
    pub fn from_raw(id: u32) -> Self {
        InternedId(id)
    }
}

/// Statistics about interning operations.
#[derive(Debug, Clone, Default)]
pub struct InternerStats {
    /// Number of unique strings stored
    pub unique_count: usize,
    /// Total number of intern calls (including duplicates)
    pub total_intern_calls: usize,
    /// Estimated memory usage in bytes
    pub estimated_memory_bytes: usize,
}

impl InternerStats {
    /// Returns the deduplication ratio (0.0 to 1.0).
    ///
    /// A ratio of 0.6 means 60% of intern calls found duplicates.
    pub fn dedup_ratio(&self) -> f64 {
        if self.total_intern_calls == 0 {
            return 0.0;
        }
        let duplicates = self.total_intern_calls.saturating_sub(self.unique_count);
        duplicates as f64 / self.total_intern_calls as f64
    }
}

/// Single-threaded string interner for deduplicating strings.
///
/// Use this when processing files sequentially or when the interner
/// is not shared across threads.
#[derive(Debug, Default)]
pub struct StringInterner {
    /// Storage for interned strings (id -> string)
    strings: Vec<String>,
    /// Reverse lookup for deduplication (string -> id)
    lookup: HashMap<String, u32>,
    /// Counter for statistics
    total_intern_calls: usize,
}

impl StringInterner {
    /// Creates a new empty interner.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an interner with pre-allocated capacity.
    ///
    /// Use this when you know approximately how many unique strings
    /// will be interned.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            strings: Vec::with_capacity(capacity),
            lookup: HashMap::with_capacity(capacity),
            total_intern_calls: 0,
        }
    }

    /// Interns a string, returning its ID.
    ///
    /// If the string was previously interned, returns the existing ID.
    /// Otherwise, stores the string and returns a new ID.
    ///
    /// # Example
    ///
    /// ```rust
    /// use tldr_core::callgraph::interner::StringInterner;
    ///
    /// let mut interner = StringInterner::new();
    /// let id1 = interner.intern("hello");
    /// let id2 = interner.intern("hello");
    /// assert_eq!(id1, id2); // Same string, same ID
    /// ```
    pub fn intern(&mut self, s: &str) -> InternedId {
        self.total_intern_calls += 1;

        // Check if already interned
        if let Some(&id) = self.lookup.get(s) {
            return InternedId(id);
        }

        // Intern new string
        let id = self.strings.len() as u32;
        self.strings.push(s.to_string());
        self.lookup.insert(s.to_string(), id);
        InternedId(id)
    }

    /// Gets the string for an ID, if valid.
    ///
    /// Returns `None` if the ID is out of bounds.
    #[inline]
    pub fn get(&self, id: InternedId) -> Option<&str> {
        self.strings.get(id.0 as usize).map(|s| s.as_str())
    }

    /// Interns a string if not present, or returns existing ID.
    ///
    /// This is equivalent to `intern()` but is named to be more explicit
    /// about the "get or create" semantics.
    #[inline]
    pub fn get_or_intern(&mut self, s: &str) -> InternedId {
        self.intern(s)
    }

    /// Returns the number of unique strings interned.
    #[inline]
    pub fn len(&self) -> usize {
        self.strings.len()
    }

    /// Returns true if no strings have been interned.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.strings.is_empty()
    }

    /// Returns statistics about interning operations.
    pub fn stats(&self) -> InternerStats {
        let estimated_memory_bytes = self
            .strings
            .iter()
            .map(|s| s.len() + std::mem::size_of::<String>())
            .sum::<usize>()
            + self.lookup.capacity() * (std::mem::size_of::<String>() + std::mem::size_of::<u32>());

        InternerStats {
            unique_count: self.strings.len(),
            total_intern_calls: self.total_intern_calls,
            estimated_memory_bytes,
        }
    }
}

/// Path interner with normalization.
///
/// This is specialized for file paths and provides:
/// - Backslash to forward slash normalization (Windows compatibility)
/// - Consistent path representation across platforms
///
/// # Example
///
/// ```rust
/// use tldr_core::callgraph::interner::PathInterner;
/// use std::path::Path;
///
/// let mut interner = PathInterner::new();
///
/// // Windows-style and Unix-style paths normalize to the same ID
/// let id1 = interner.intern_path(Path::new("src\\main.rs"));
/// let id2 = interner.intern_path(Path::new("src/main.rs"));
/// assert_eq!(id1, id2);
/// ```
#[derive(Debug, Default)]
pub struct PathInterner {
    inner: StringInterner,
}

impl PathInterner {
    /// Creates a new empty path interner.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a path interner with pre-allocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: StringInterner::with_capacity(capacity),
        }
    }

    /// Interns a path, returning its ID.
    ///
    /// The path is normalized:
    /// - Backslashes are converted to forward slashes
    pub fn intern_path(&mut self, path: &Path) -> InternedId {
        let normalized = normalize_path(path);
        self.inner.intern(&normalized)
    }

    /// Gets the path string for an ID, if valid.
    #[inline]
    pub fn get_path(&self, id: InternedId) -> Option<&str> {
        self.inner.get(id)
    }

    /// Returns the number of unique paths interned.
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns true if no paths have been interned.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns statistics about interning operations.
    pub fn stats(&self) -> InternerStats {
        self.inner.stats()
    }
}

/// Normalizes a path for consistent storage.
///
/// - Converts backslashes to forward slashes
/// - Preserves absolute/relative path structure
fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Thread-safe concurrent string interner.
///
/// Use this when interning strings from multiple threads simultaneously,
/// such as during parallel file processing.
///
/// # Thread Safety
///
/// This interner uses `RwLock` for interior mutability:
/// - Multiple threads can read (get) simultaneously
/// - Only one thread can write (intern) at a time
///
/// # Example
///
/// ```rust
/// use tldr_core::callgraph::interner::ConcurrentInterner;
/// use std::sync::Arc;
/// use std::thread;
///
/// let interner = Arc::new(ConcurrentInterner::new());
///
/// let handles: Vec<_> = (0..4).map(|i| {
///     let interner = Arc::clone(&interner);
///     thread::spawn(move || {
///         interner.intern(&format!("string_{}", i))
///     })
/// }).collect();
///
/// for handle in handles {
///     let _id = handle.join().unwrap();
/// }
/// ```
#[derive(Debug)]
pub struct ConcurrentInterner {
    /// Storage for interned strings (id -> string)
    strings: RwLock<Vec<String>>,
    /// Reverse lookup for deduplication (string -> id)
    lookup: RwLock<HashMap<String, u32>>,
    /// Counter for statistics
    total_intern_calls: AtomicUsize,
}

impl ConcurrentInterner {
    /// Creates a new empty concurrent interner.
    pub fn new() -> Self {
        Self {
            strings: RwLock::new(Vec::new()),
            lookup: RwLock::new(HashMap::new()),
            total_intern_calls: AtomicUsize::new(0),
        }
    }

    /// Creates a concurrent interner with pre-allocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            strings: RwLock::new(Vec::with_capacity(capacity)),
            lookup: RwLock::new(HashMap::with_capacity(capacity)),
            total_intern_calls: AtomicUsize::new(0),
        }
    }

    /// Interns a string, returning its ID.
    ///
    /// This method is thread-safe. If multiple threads try to intern
    /// the same string simultaneously, only one will create the entry
    /// and all will receive the same ID.
    pub fn intern(&self, s: &str) -> InternedId {
        self.total_intern_calls.fetch_add(1, Ordering::Relaxed);

        // Fast path: check if already interned (read lock only)
        {
            let lookup = self.lookup.read().unwrap();
            if let Some(&id) = lookup.get(s) {
                return InternedId(id);
            }
        }

        // Slow path: need to intern (write lock)
        let mut lookup = self.lookup.write().unwrap();

        // Double-check after acquiring write lock
        if let Some(&id) = lookup.get(s) {
            return InternedId(id);
        }

        let mut strings = self.strings.write().unwrap();
        let id = strings.len() as u32;
        strings.push(s.to_string());
        lookup.insert(s.to_string(), id);
        InternedId(id)
    }

    /// Gets the string for an ID, if valid.
    ///
    /// Returns owned String because we can't return a reference
    /// through the RwLock.
    pub fn get(&self, id: InternedId) -> Option<String> {
        let strings = self.strings.read().unwrap();
        strings.get(id.0 as usize).cloned()
    }

    /// Returns the number of unique strings interned.
    pub fn len(&self) -> usize {
        self.strings.read().unwrap().len()
    }

    /// Returns true if no strings have been interned.
    pub fn is_empty(&self) -> bool {
        self.strings.read().unwrap().is_empty()
    }

    /// Returns statistics about interning operations.
    pub fn stats(&self) -> InternerStats {
        let strings = self.strings.read().unwrap();
        let lookup = self.lookup.read().unwrap();

        let estimated_memory_bytes = strings
            .iter()
            .map(|s| s.len() + std::mem::size_of::<String>())
            .sum::<usize>()
            + lookup.capacity() * (std::mem::size_of::<String>() + std::mem::size_of::<u32>());

        InternerStats {
            unique_count: strings.len(),
            total_intern_calls: self.total_intern_calls.load(Ordering::Relaxed),
            estimated_memory_bytes,
        }
    }
}

impl Default for ConcurrentInterner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_path_backslash() {
        let path = Path::new("src\\main\\lib.rs");
        assert_eq!(normalize_path(path), "src/main/lib.rs");
    }

    #[test]
    fn test_normalize_path_already_normalized() {
        let path = Path::new("src/main/lib.rs");
        assert_eq!(normalize_path(path), "src/main/lib.rs");
    }

    #[test]
    fn test_normalize_path_mixed() {
        let path = Path::new("src\\main/lib.rs");
        assert_eq!(normalize_path(path), "src/main/lib.rs");
    }

    #[test]
    fn test_normalize_path_absolute_windows() {
        let path = Path::new("C:\\Users\\project\\main.rs");
        assert_eq!(normalize_path(path), "C:/Users/project/main.rs");
    }
}
