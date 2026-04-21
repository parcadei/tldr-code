//! Graph utilities for cycle detection and traversal
//!
//! This module provides shared utilities for call graph traversal,
//! implementing TIGER-02 mitigation for circular import protection.
//!
//! # TIGER-02 Mitigation
//!
//! Risk: Cycle detection in diff-impact call graph traversal
//! Severity: Critical
//! Mitigation: Implement visited set for call graph traversal.
//!             Use HashSet<(PathBuf, String)> to track (file, function) pairs.
//!             Abort with partial results if cycle detected.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

// =============================================================================
// Constants
// =============================================================================

/// Maximum depth for graph traversal to prevent stack overflow
pub const MAX_GRAPH_DEPTH: usize = 100;

/// Maximum depth for import resolution
pub const MAX_IMPORT_DEPTH: usize = 10;

// =============================================================================
// Cycle Detection
// =============================================================================

/// Type alias for the visited set
pub type VisitedSet = HashSet<(PathBuf, String)>;

/// Cycle detector for graph traversal operations
///
/// Tracks visited (file, function) pairs to detect cycles during
/// call graph traversal, import resolution, or other graph operations.
///
/// # Example
///
/// ```rust,ignore
/// use tldr_cli::commands::remaining::graph_utils::CycleDetector;
/// use std::path::Path;
///
/// let mut detector = CycleDetector::new();
///
/// // First visit returns false (no cycle)
/// assert!(!detector.visit(Path::new("a.py"), "func"));
///
/// // Visiting same location returns true (cycle detected)
/// assert!(detector.visit(Path::new("a.py"), "func"));
/// ```
#[derive(Debug, Clone)]
pub struct CycleDetector {
    /// Set of visited (file, function) pairs
    visited: VisitedSet,
    /// Whether a cycle has been detected
    cycle_detected: bool,
    /// Current depth in traversal
    depth: usize,
}

impl CycleDetector {
    /// Create a new cycle detector
    pub fn new() -> Self {
        Self {
            visited: HashSet::new(),
            cycle_detected: false,
            depth: 0,
        }
    }

    /// Create a cycle detector with a specific capacity hint
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            visited: HashSet::with_capacity(capacity),
            cycle_detected: false,
            depth: 0,
        }
    }

    /// Visit a (file, function) pair.
    ///
    /// Returns `true` if this location was already visited (cycle detected).
    /// Returns `false` if this is a new location.
    pub fn visit(&mut self, file: &Path, func: &str) -> bool {
        let key = (file.to_path_buf(), func.to_string());
        if !self.visited.insert(key) {
            self.cycle_detected = true;
            true
        } else {
            false
        }
    }

    /// Visit a file path only (for import resolution)
    pub fn visit_file(&mut self, file: &Path) -> bool {
        self.visit(file, "")
    }

    /// Check if a cycle has been detected during traversal
    pub fn is_cycle_detected(&self) -> bool {
        self.cycle_detected
    }

    /// Get the number of visited locations
    pub fn visited_count(&self) -> usize {
        self.visited.len()
    }

    /// Check if a specific location has been visited
    pub fn was_visited(&self, file: &Path, func: &str) -> bool {
        let key = (file.to_path_buf(), func.to_string());
        self.visited.contains(&key)
    }

    /// Get all visited files (unique)
    pub fn visited_files(&self) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = self.visited.iter().map(|(path, _)| path.clone()).collect();
        files.sort();
        files.dedup();
        files
    }

    /// Reset the detector for reuse
    pub fn reset(&mut self) {
        self.visited.clear();
        self.cycle_detected = false;
        self.depth = 0;
    }

    /// Enter a new depth level. Returns false if max depth exceeded.
    pub fn enter_depth(&mut self) -> bool {
        if self.depth >= MAX_GRAPH_DEPTH {
            return false;
        }
        self.depth += 1;
        true
    }

    /// Exit a depth level
    pub fn exit_depth(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// Get current depth
    pub fn current_depth(&self) -> usize {
        self.depth
    }
}

impl Default for CycleDetector {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Graph Traversal Helpers
// =============================================================================

/// Result of a graph traversal operation
#[derive(Debug, Clone)]
pub struct TraversalResult<T> {
    /// The collected results
    pub results: Vec<T>,
    /// Whether the traversal was complete (no cycles or depth limit hit)
    pub complete: bool,
    /// Number of nodes visited
    pub nodes_visited: usize,
    /// Cycle detected during traversal
    pub cycle_detected: bool,
    /// Maximum depth reached
    pub max_depth_reached: usize,
}

impl<T> TraversalResult<T> {
    /// Create a new empty traversal result
    pub fn new() -> Self {
        Self {
            results: Vec::new(),
            complete: true,
            nodes_visited: 0,
            cycle_detected: false,
            max_depth_reached: 0,
        }
    }

    /// Mark traversal as incomplete due to cycle
    pub fn mark_cycle(&mut self) {
        self.cycle_detected = true;
        self.complete = false;
    }

    /// Mark traversal as incomplete due to depth limit
    pub fn mark_depth_limit(&mut self, depth: usize) {
        self.complete = false;
        if depth > self.max_depth_reached {
            self.max_depth_reached = depth;
        }
    }

    /// Add a result
    pub fn add(&mut self, item: T) {
        self.results.push(item);
        self.nodes_visited += 1;
    }
}

impl<T> Default for TraversalResult<T> {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cycle_detector_new() {
        let detector = CycleDetector::new();
        assert!(!detector.is_cycle_detected());
        assert_eq!(detector.visited_count(), 0);
    }

    #[test]
    fn test_cycle_detector_visit() {
        let mut detector = CycleDetector::new();

        // First visit should return false (not a cycle)
        assert!(!detector.visit(Path::new("file.py"), "func"));
        assert_eq!(detector.visited_count(), 1);

        // Second visit to same location should return true (cycle)
        assert!(detector.visit(Path::new("file.py"), "func"));
        assert!(detector.is_cycle_detected());

        // Different location should return false
        assert!(!detector.visit(Path::new("other.py"), "func"));
        assert_eq!(detector.visited_count(), 2);

        // Same file, different function
        assert!(!detector.visit(Path::new("file.py"), "other_func"));
        assert_eq!(detector.visited_count(), 3);
    }

    #[test]
    fn test_cycle_detector_was_visited() {
        let mut detector = CycleDetector::new();

        assert!(!detector.was_visited(Path::new("file.py"), "func"));

        detector.visit(Path::new("file.py"), "func");

        assert!(detector.was_visited(Path::new("file.py"), "func"));
        assert!(!detector.was_visited(Path::new("file.py"), "other"));
    }

    #[test]
    fn test_cycle_detector_reset() {
        let mut detector = CycleDetector::new();

        detector.visit(Path::new("file.py"), "func");
        detector.visit(Path::new("file.py"), "func"); // causes cycle

        assert!(detector.is_cycle_detected());
        assert_eq!(detector.visited_count(), 1);

        detector.reset();

        assert!(!detector.is_cycle_detected());
        assert_eq!(detector.visited_count(), 0);
    }

    #[test]
    fn test_cycle_detector_depth() {
        let mut detector = CycleDetector::new();

        assert_eq!(detector.current_depth(), 0);

        assert!(detector.enter_depth());
        assert_eq!(detector.current_depth(), 1);

        detector.exit_depth();
        assert_eq!(detector.current_depth(), 0);

        // Test depth limit
        for _ in 0..MAX_GRAPH_DEPTH {
            assert!(detector.enter_depth());
        }
        assert!(!detector.enter_depth()); // Should fail at max depth
    }

    #[test]
    fn test_cycle_detector_visited_files() {
        let mut detector = CycleDetector::new();

        detector.visit(Path::new("a.py"), "func1");
        detector.visit(Path::new("a.py"), "func2");
        detector.visit(Path::new("b.py"), "func1");

        let files = detector.visited_files();
        assert_eq!(files.len(), 2);
        assert!(files.contains(&PathBuf::from("a.py")));
        assert!(files.contains(&PathBuf::from("b.py")));
    }

    #[test]
    fn test_traversal_result() {
        let mut result: TraversalResult<String> = TraversalResult::new();

        assert!(result.complete);
        assert_eq!(result.nodes_visited, 0);

        result.add("node1".to_string());
        assert_eq!(result.nodes_visited, 1);
        assert_eq!(result.results.len(), 1);

        result.mark_cycle();
        assert!(!result.complete);
        assert!(result.cycle_detected);
    }
}
