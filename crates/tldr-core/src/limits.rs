//! Memory and timeout limits for analysis operations (Phase 10)
//!
//! This module provides configurable limits to prevent resource exhaustion
//! during large-scale analysis operations.
//!
//! # Mitigations
//!
//! - A32: Memory exhaustion on large graphs - enforces max node/edge limits
//! - A33: No timeout handling - provides timeout wrapper
//!
//! # Example
//!
//! ```rust,ignore
//! use tldr_core::limits::{AnalysisLimits, with_timeout, LimitExceeded};
//! use std::time::Duration;
//!
//! let limits = AnalysisLimits::default();
//!
//! // Check limits during analysis
//! if node_count > limits.max_nodes {
//!     return Err(LimitExceeded::MaxNodes(limits.max_nodes));
//! }
//!
//! // Run with timeout
//! let result = with_timeout(Duration::from_secs(30), || {
//!     expensive_analysis()
//! })?;
//! ```

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::TldrError;

// =============================================================================
// Analysis Limits Configuration (A32)
// =============================================================================

/// Configurable limits for analysis operations.
///
/// These limits prevent memory exhaustion on large codebases.
/// When a limit is exceeded, analysis returns partial results with a warning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisLimits {
    /// Maximum number of nodes (classes, files, functions) to process
    /// Default: 50,000
    pub max_nodes: usize,

    /// Maximum number of edges to process
    /// Default: 100,000
    pub max_edges: usize,

    /// Maximum diamond paths to compute in inheritance analysis
    /// Default: 1,000
    pub max_diamond_paths: usize,

    /// Maximum number of patterns to report
    /// Default: 10,000
    pub max_patterns: usize,

    /// Timeout in seconds (0 = no timeout)
    /// Default: 30
    pub timeout_secs: u64,
}

impl Default for AnalysisLimits {
    fn default() -> Self {
        Self {
            max_nodes: 50_000,
            max_edges: 100_000,
            max_diamond_paths: 1_000,
            max_patterns: 10_000,
            timeout_secs: 30,
        }
    }
}

impl AnalysisLimits {
    /// Create limits with a specific timeout
    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }

    /// Create limits with a specific max nodes
    pub fn with_max_nodes(mut self, max: usize) -> Self {
        self.max_nodes = max;
        self
    }

    /// Create limits with a specific max edges
    pub fn with_max_edges(mut self, max: usize) -> Self {
        self.max_edges = max;
        self
    }

    /// Create "unlimited" limits (for tests or small projects)
    pub fn unlimited() -> Self {
        Self {
            max_nodes: usize::MAX,
            max_edges: usize::MAX,
            max_diamond_paths: usize::MAX,
            max_patterns: usize::MAX,
            timeout_secs: 0,
        }
    }

    /// Check if node limit is exceeded
    pub fn check_nodes(&self, count: usize) -> Result<(), LimitExceeded> {
        if count > self.max_nodes {
            Err(LimitExceeded::MaxNodes {
                limit: self.max_nodes,
                actual: count,
            })
        } else {
            Ok(())
        }
    }

    /// Check if edge limit is exceeded
    pub fn check_edges(&self, count: usize) -> Result<(), LimitExceeded> {
        if count > self.max_edges {
            Err(LimitExceeded::MaxEdges {
                limit: self.max_edges,
                actual: count,
            })
        } else {
            Ok(())
        }
    }
}

// =============================================================================
// Limit Exceeded Error
// =============================================================================

/// Error when analysis limits are exceeded.
///
/// This is separate from TldrError to allow partial results
/// with a warning rather than hard failure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LimitExceeded {
    /// Maximum nodes exceeded
    MaxNodes {
        /// Configured maximum node count.
        limit: usize,
        /// Observed node count.
        actual: usize,
    },
    /// Maximum edges exceeded
    MaxEdges {
        /// Configured maximum edge count.
        limit: usize,
        /// Observed edge count.
        actual: usize,
    },
    /// Maximum diamond paths exceeded
    MaxDiamondPaths {
        /// Configured maximum diamond-path count.
        limit: usize,
        /// Observed diamond-path count.
        actual: usize,
    },
    /// Maximum patterns exceeded
    MaxPatterns {
        /// Configured maximum pattern count.
        limit: usize,
        /// Observed pattern count.
        actual: usize,
    },
    /// Analysis timed out
    Timeout {
        /// Elapsed runtime in seconds when the timeout triggered.
        elapsed_secs: u64,
        /// Configured timeout limit in seconds.
        limit_secs: u64,
    },
}

impl std::fmt::Display for LimitExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LimitExceeded::MaxNodes { limit, actual } => {
                write!(
                    f,
                    "Node limit exceeded: {} nodes found, limit is {}. Use --max-nodes or filter with --class",
                    actual, limit
                )
            }
            LimitExceeded::MaxEdges { limit, actual } => {
                write!(
                    f,
                    "Edge limit exceeded: {} edges found, limit is {}. Use --max-files to limit scope",
                    actual, limit
                )
            }
            LimitExceeded::MaxDiamondPaths { limit, actual } => {
                write!(
                    f,
                    "Diamond path limit exceeded: {} paths found, limit is {}. Use --no-patterns to skip diamond detection",
                    actual, limit
                )
            }
            LimitExceeded::MaxPatterns { limit, actual } => {
                write!(
                    f,
                    "Pattern limit exceeded: {} patterns found, limit is {}",
                    actual, limit
                )
            }
            LimitExceeded::Timeout {
                elapsed_secs,
                limit_secs,
            } => {
                write!(
                    f,
                    "Analysis timed out after {}s (limit: {}s). Try:\n  - Use --max-files to limit scope\n  - Use --class filter for inheritance\n  - Increase timeout with --timeout",
                    elapsed_secs, limit_secs
                )
            }
        }
    }
}

impl std::error::Error for LimitExceeded {}

// =============================================================================
// Timeout Handling (A33)
// =============================================================================

/// Context for checking timeout during analysis.
///
/// This provides a lightweight way to check if analysis should be interrupted
/// due to timeout, without requiring async/await.
#[derive(Clone)]
pub struct TimeoutContext {
    start: Instant,
    timeout: Duration,
    /// Shared flag for early termination
    cancelled: Arc<AtomicBool>,
    /// Nodes processed (for partial results)
    nodes_processed: Arc<AtomicUsize>,
}

impl TimeoutContext {
    /// Create a new timeout context.
    ///
    /// If `timeout_secs` is 0, the context never times out.
    pub fn new(timeout_secs: u64) -> Self {
        Self {
            start: Instant::now(),
            timeout: if timeout_secs == 0 {
                Duration::MAX
            } else {
                Duration::from_secs(timeout_secs)
            },
            cancelled: Arc::new(AtomicBool::new(false)),
            nodes_processed: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Create a context that never times out.
    pub fn no_timeout() -> Self {
        Self::new(0)
    }

    /// Check if the timeout has been exceeded or context was cancelled.
    ///
    /// Call this periodically during long-running operations.
    pub fn check(&self) -> Result<(), LimitExceeded> {
        if self.cancelled.load(Ordering::Relaxed) {
            return Err(LimitExceeded::Timeout {
                elapsed_secs: self.start.elapsed().as_secs(),
                limit_secs: self.timeout.as_secs(),
            });
        }

        if self.start.elapsed() >= self.timeout {
            return Err(LimitExceeded::Timeout {
                elapsed_secs: self.start.elapsed().as_secs(),
                limit_secs: self.timeout.as_secs(),
            });
        }

        Ok(())
    }

    /// Check timeout, but only after every N nodes (for efficiency).
    ///
    /// Returns `true` if this check actually happened.
    pub fn check_periodic(&self, check_interval: usize) -> Result<bool, LimitExceeded> {
        let count = self.nodes_processed.fetch_add(1, Ordering::Relaxed);
        if count.is_multiple_of(check_interval) {
            self.check()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Cancel the context (for signal handling).
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Get the elapsed time.
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    /// Get the number of nodes processed.
    pub fn nodes_processed(&self) -> usize {
        self.nodes_processed.load(Ordering::Relaxed)
    }

    /// Check if cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

impl Default for TimeoutContext {
    fn default() -> Self {
        Self::new(30) // Default 30 second timeout
    }
}

/// Run a closure with a timeout.
///
/// This is a synchronous timeout implementation using channels.
/// The closure runs in a separate thread; if it doesn't complete
/// within the timeout, an error is returned.
///
/// # Arguments
///
/// * `timeout` - Maximum duration to wait
/// * `f` - The closure to run
///
/// # Returns
///
/// * `Ok(T)` if the closure completes within the timeout
/// * `Err(TldrError::Timeout)` if the timeout is exceeded
///
/// # Example
///
/// ```rust,ignore
/// use std::time::Duration;
/// use tldr_core::limits::with_timeout;
///
/// let result = with_timeout(Duration::from_secs(5), || {
///     // expensive computation
///     42
/// })?;
/// ```
pub fn with_timeout<T, F>(timeout: Duration, f: F) -> Result<T, TldrError>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();

    std::thread::spawn(move || {
        let result = f();
        let _ = tx.send(result);
    });

    match rx.recv_timeout(timeout) {
        Ok(result) => Ok(result),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(TldrError::Timeout(format!(
            "Operation timed out after {}s. Try:\n  - Use --max-files to limit scope\n  - Increase timeout with --timeout",
            timeout.as_secs()
        ))),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            Err(TldrError::Timeout("Analysis thread panicked".to_string()))
        }
    }
}

/// Run a closure with a timeout, returning Result.
///
/// Like `with_timeout` but for closures that return Result.
pub fn with_timeout_result<T, E, F>(timeout: Duration, f: F) -> Result<T, TldrError>
where
    T: Send + 'static,
    E: std::error::Error + Send + 'static,
    F: FnOnce() -> Result<T, E> + Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();

    std::thread::spawn(move || {
        let result = f();
        let _ = tx.send(result);
    });

    match rx.recv_timeout(timeout) {
        Ok(Ok(result)) => Ok(result),
        Ok(Err(e)) => Err(TldrError::Timeout(format!("Analysis failed: {}", e))),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(TldrError::Timeout(format!(
            "Operation timed out after {}s",
            timeout.as_secs()
        ))),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            Err(TldrError::Timeout("Analysis thread panicked".to_string()))
        }
    }
}

// =============================================================================
// Analysis Progress (for partial results)
// =============================================================================

/// Progress tracking for analysis operations.
///
/// Allows returning partial results when limits are exceeded.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnalysisProgress {
    /// Number of files scanned
    pub files_scanned: usize,
    /// Number of files skipped (errors, encoding, etc.)
    pub files_skipped: usize,
    /// Number of nodes processed
    pub nodes_processed: usize,
    /// Number of edges processed
    pub edges_processed: usize,
    /// Whether analysis was truncated due to limits
    pub truncated: bool,
    /// Reason for truncation (if any)
    pub truncation_reason: Option<String>,
    /// Elapsed time in milliseconds
    pub elapsed_ms: u64,
}

impl AnalysisProgress {
    /// Create a new progress tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark as truncated with reason.
    pub fn truncate(&mut self, reason: impl Into<String>) {
        self.truncated = true;
        self.truncation_reason = Some(reason.into());
    }

    /// Set elapsed time.
    pub fn set_elapsed(&mut self, elapsed: Duration) {
        self.elapsed_ms = elapsed.as_millis() as u64;
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_limits_default() {
        let limits = AnalysisLimits::default();
        assert_eq!(limits.max_nodes, 50_000);
        assert_eq!(limits.max_edges, 100_000);
        assert_eq!(limits.timeout_secs, 30);
    }

    #[test]
    fn test_limits_check_nodes() {
        let limits = AnalysisLimits::default().with_max_nodes(100);
        assert!(limits.check_nodes(50).is_ok());
        assert!(limits.check_nodes(100).is_ok());
        assert!(limits.check_nodes(101).is_err());
    }

    #[test]
    fn test_limits_unlimited() {
        let limits = AnalysisLimits::unlimited();
        assert!(limits.check_nodes(1_000_000).is_ok());
        assert!(limits.check_edges(1_000_000).is_ok());
    }

    #[test]
    fn test_timeout_context_immediate() {
        let ctx = TimeoutContext::new(30);
        assert!(ctx.check().is_ok());
    }

    #[test]
    fn test_timeout_context_cancelled() {
        let ctx = TimeoutContext::new(30);
        ctx.cancel();
        assert!(ctx.check().is_err());
        assert!(ctx.is_cancelled());
    }

    #[test]
    fn test_timeout_context_no_timeout() {
        let ctx = TimeoutContext::no_timeout();
        assert!(ctx.check().is_ok());
    }

    #[test]
    fn test_with_timeout_completes() {
        let result = with_timeout(Duration::from_secs(5), || 42);
        assert_eq!(result.unwrap(), 42);
    }

    #[test]
    fn test_with_timeout_times_out() {
        let result = with_timeout(Duration::from_millis(10), || {
            std::thread::sleep(Duration::from_secs(5));
            42
        });
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, TldrError::Timeout(_)));
    }

    #[test]
    fn test_limit_exceeded_display() {
        let err = LimitExceeded::MaxNodes {
            limit: 1000,
            actual: 2000,
        };
        let msg = err.to_string();
        assert!(msg.contains("2000"));
        assert!(msg.contains("1000"));
        assert!(msg.contains("--max-nodes"));
    }

    #[test]
    fn test_progress_truncate() {
        let mut progress = AnalysisProgress::new();
        progress.files_scanned = 100;
        progress.truncate("Max nodes exceeded");
        assert!(progress.truncated);
        assert_eq!(
            progress.truncation_reason,
            Some("Max nodes exceeded".to_string())
        );
    }
}
