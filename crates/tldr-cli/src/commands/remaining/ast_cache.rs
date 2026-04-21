//! AST Cache for efficient multi-analysis
//!
//! Provides a caching layer for parsed ASTs to prevent redundant parsing
//! when running multiple sub-analyses on the same file (TIGER-03 mitigation).
//!
//! # Usage
//!
//! ```ignore
//! let mut cache = AstCache::new(100);
//! let tree = cache.get_or_parse(&path, &source)?;
//! // Tree is cached for subsequent calls
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use tree_sitter::{Parser, Tree};

use super::error::{RemainingError, RemainingResult};

/// Maximum cache size (number of ASTs to cache)
pub const MAX_CACHE_SIZE: usize = 100;

/// Cache key combining path and modification time
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct AstCacheKey {
    pub path: PathBuf,
    pub mtime: Option<SystemTime>,
}

impl AstCacheKey {
    /// Create a new cache key
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        Self { path, mtime }
    }
}

/// Cache statistics
#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
}

/// AST cache with LRU eviction
pub struct AstCache {
    /// Cached trees indexed by path/mtime key
    cache: HashMap<PathBuf, (Option<SystemTime>, Tree)>,
    /// Maximum number of entries
    capacity: usize,
    /// Access order for LRU (most recent last)
    access_order: Vec<PathBuf>,
    /// Statistics
    stats: CacheStats,
}

impl AstCache {
    /// Create a new cache with specified capacity
    pub fn new(capacity: usize) -> Self {
        Self {
            cache: HashMap::new(),
            capacity,
            access_order: Vec::new(),
            stats: CacheStats::default(),
        }
    }

    /// Get or parse a file, caching the result
    pub fn get_or_parse(&mut self, path: &Path, source: &str) -> RemainingResult<&Tree> {
        let key = AstCacheKey::new(path);

        // Check if we have a valid cached entry
        if let Some((cached_mtime, _)) = self.cache.get(path) {
            if *cached_mtime == key.mtime {
                self.stats.hits += 1;
                self.update_access_order(path);
                return Ok(&self.cache.get(path).unwrap().1);
            }
        }

        self.stats.misses += 1;

        // Parse the file using extension-aware language selection.
        let tree = self.parse_source(source, path)?;

        // Evict if at capacity
        while self.cache.len() >= self.capacity {
            self.evict_lru();
        }

        // Insert into cache
        self.cache.insert(path.to_path_buf(), (key.mtime, tree));
        self.access_order.push(path.to_path_buf());

        Ok(&self.cache.get(path).unwrap().1)
    }

    /// Parse source code using a language selected from file extension.
    fn parse_source(&self, source: &str, path: &Path) -> RemainingResult<Tree> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default();
        match ext {
            "rs" => self.parse_rust(source, path),
            _ => self.parse_python(source, path),
        }
    }

    /// Parse Python source code.
    fn parse_python(&self, source: &str, path: &Path) -> RemainingResult<Tree> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .map_err(|e| RemainingError::parse_error(path, e.to_string()))?;

        parser
            .parse(source, None)
            .ok_or_else(|| RemainingError::parse_error(path, "Failed to parse"))
    }

    /// Parse Rust source code.
    fn parse_rust(&self, source: &str, path: &Path) -> RemainingResult<Tree> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .map_err(|e| RemainingError::parse_error(path, e.to_string()))?;

        parser
            .parse(source, None)
            .ok_or_else(|| RemainingError::parse_error(path, "Failed to parse"))
    }

    /// Update access order for LRU
    fn update_access_order(&mut self, path: &Path) {
        if let Some(pos) = self.access_order.iter().position(|p| p == path) {
            self.access_order.remove(pos);
        }
        self.access_order.push(path.to_path_buf());
    }

    /// Evict least recently used entry
    fn evict_lru(&mut self) {
        if let Some(path) = self.access_order.first().cloned() {
            self.cache.remove(&path);
            self.access_order.remove(0);
            self.stats.evictions += 1;
        }
    }

    /// Invalidate a specific path
    pub fn invalidate(&mut self, path: &Path) {
        self.cache.remove(path);
        self.access_order.retain(|p| p != path);
    }

    /// Clear the entire cache
    pub fn clear(&mut self) {
        self.cache.clear();
        self.access_order.clear();
    }

    /// Get cache statistics
    pub fn stats(&self) -> &CacheStats {
        &self.stats
    }

    /// Get current cache size
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// Check if cache is empty
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}

impl Default for AstCache {
    fn default() -> Self {
        Self::new(MAX_CACHE_SIZE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_file(dir: &TempDir, name: &str, content: &str) -> PathBuf {
        let path = dir.path().join(name);
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn test_cache_hit() {
        let temp = TempDir::new().unwrap();
        let path = create_test_file(&temp, "test.py", "def foo(): pass");
        let source = fs::read_to_string(&path).unwrap();

        let mut cache = AstCache::new(10);

        // First access - miss
        let _ = cache.get_or_parse(&path, &source).unwrap();
        assert_eq!(cache.stats().misses, 1);
        assert_eq!(cache.stats().hits, 0);

        // Second access - hit
        let _ = cache.get_or_parse(&path, &source).unwrap();
        assert_eq!(cache.stats().misses, 1);
        assert_eq!(cache.stats().hits, 1);
    }

    #[test]
    fn test_cache_invalidation() {
        let temp = TempDir::new().unwrap();
        let path = create_test_file(&temp, "test.py", "def foo(): pass");
        let source = fs::read_to_string(&path).unwrap();

        let mut cache = AstCache::new(10);

        let _ = cache.get_or_parse(&path, &source).unwrap();
        assert_eq!(cache.len(), 1);

        cache.invalidate(&path);
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn test_cache_eviction() {
        let temp = TempDir::new().unwrap();

        let mut cache = AstCache::new(2);

        for i in 0..3 {
            let path = create_test_file(&temp, &format!("test{}.py", i), "def foo(): pass");
            let source = fs::read_to_string(&path).unwrap();
            let _ = cache.get_or_parse(&path, &source).unwrap();
        }

        // Should have evicted one entry
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.stats().evictions, 1);
    }

    #[test]
    fn test_cache_parses_rust_source() {
        let temp = TempDir::new().unwrap();
        let path = create_test_file(&temp, "lib.rs", "fn main() { println!(\"ok\"); }");
        let source = fs::read_to_string(&path).unwrap();

        let mut cache = AstCache::new(10);
        let tree = cache.get_or_parse(&path, &source).unwrap();
        assert_eq!(tree.root_node().kind(), "source_file");
    }
}
