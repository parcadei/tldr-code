//! Signal handling for graceful interruption (Phase 10)
//!
//! This module provides signal handling for Ctrl+C (SIGINT) interruption
//! to allow graceful shutdown and partial result reporting.
//!
//! # Mitigations
//!
//! - A36: No signal handling for graceful interruption
//!   - Catches SIGINT (Ctrl+C)
//!   - Allows current file to complete
//!   - Returns partial results with interrupt metadata
//!
//! # Example
//!
//! ```rust,ignore
//! use tldr_cli::signals::{setup_signal_handler, is_interrupted, InterruptState};
//!
//! // Set up handler at start of main
//! let state = setup_signal_handler()?;
//!
//! // Check periodically in analysis loops
//! for file in files {
//!     if is_interrupted() {
//!         eprintln!("Interrupted. Returning partial results...");
//!         break;
//!     }
//!     process_file(file)?;
//! }
//!
//! // Report partial results
//! if state.was_interrupted() {
//!     eprintln!("Analyzed {}/{} files before interrupt", state.files_completed(), total);
//! }
//! ```

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

/// Global interrupted flag.
///
/// This is set to true when SIGINT is received.
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

/// Check if the process has been interrupted.
///
/// Call this periodically in long-running loops to allow graceful shutdown.
pub fn is_interrupted() -> bool {
    INTERRUPTED.load(Ordering::Relaxed)
}

/// Reset the interrupted flag.
///
/// Useful for tests or if you want to handle multiple interrupts.
pub fn reset_interrupted() {
    INTERRUPTED.store(false, Ordering::SeqCst);
}

/// State tracking for interruptible operations.
#[derive(Clone)]
pub struct InterruptState {
    /// Whether an interrupt was received
    interrupted: Arc<AtomicBool>,
    /// Number of items completed before interrupt
    completed: Arc<AtomicUsize>,
    /// Total items expected
    total: Arc<AtomicUsize>,
}

impl InterruptState {
    /// Create a new interrupt state.
    pub fn new() -> Self {
        Self {
            interrupted: Arc::new(AtomicBool::new(false)),
            completed: Arc::new(AtomicUsize::new(0)),
            total: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Set the total number of items.
    pub fn set_total(&self, total: usize) {
        self.total.store(total, Ordering::SeqCst);
    }

    /// Increment completed count.
    pub fn increment_completed(&self) {
        self.completed.fetch_add(1, Ordering::SeqCst);
    }

    /// Mark as interrupted.
    pub fn mark_interrupted(&self) {
        self.interrupted.store(true, Ordering::SeqCst);
    }

    /// Check if interrupted.
    pub fn was_interrupted(&self) -> bool {
        self.interrupted.load(Ordering::Relaxed) || is_interrupted()
    }

    /// Get completed count.
    pub fn files_completed(&self) -> usize {
        self.completed.load(Ordering::Relaxed)
    }

    /// Get total count.
    pub fn total_files(&self) -> usize {
        self.total.load(Ordering::Relaxed)
    }

    /// Check global interrupt flag and update local state.
    ///
    /// Returns `true` if interrupted.
    pub fn check_interrupt(&self) -> bool {
        if is_interrupted() {
            self.mark_interrupted();
            true
        } else {
            false
        }
    }
}

impl Default for InterruptState {
    fn default() -> Self {
        Self::new()
    }
}

/// Set up the signal handler for SIGINT.
///
/// This should be called once at the start of main().
/// Returns an InterruptState for tracking progress.
///
/// # Returns
///
/// * `Ok(InterruptState)` - Handler installed successfully
/// * `Err(String)` - Failed to install handler
pub fn setup_signal_handler() -> Result<InterruptState, String> {
    let state = InterruptState::new();

    ctrlc::set_handler(move || {
        // Check if already interrupted (double Ctrl+C = force exit)
        if INTERRUPTED.load(Ordering::Relaxed) {
            eprintln!("\nForce exit.");
            std::process::exit(130); // 128 + SIGINT (2)
        }

        INTERRUPTED.store(true, Ordering::SeqCst);
        eprintln!("\nInterrupted. Completing current operation...");
    })
    .map_err(|e| format!("Failed to set signal handler: {}", e))?;

    Ok(state)
}

/// Report on interrupt status.
///
/// Call this at the end of analysis to report partial results.
pub fn report_interrupt_status(state: &InterruptState) {
    if state.was_interrupted() {
        let completed = state.files_completed();
        let total = state.total_files();
        if total > 0 {
            eprintln!(
                "Interrupted: Analyzed {}/{} files ({:.1}%)",
                completed,
                total,
                (completed as f64 / total as f64) * 100.0
            );
        } else {
            eprintln!("Interrupted: Analyzed {} files", completed);
        }
    }
}

/// Metadata about an interrupted analysis.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InterruptMetadata {
    /// Whether the analysis was interrupted
    pub interrupted: bool,
    /// Number of items completed
    pub completed: usize,
    /// Total items expected
    pub total: usize,
    /// Percentage complete
    pub percent_complete: f64,
}

impl InterruptMetadata {
    /// Create metadata from interrupt state.
    pub fn from_state(state: &InterruptState) -> Self {
        let completed = state.files_completed();
        let total = state.total_files();
        let percent = if total > 0 {
            (completed as f64 / total as f64) * 100.0
        } else {
            0.0
        };

        Self {
            interrupted: state.was_interrupted(),
            completed,
            total,
            percent_complete: percent,
        }
    }

    /// Create metadata for a non-interrupted analysis.
    pub fn complete(total: usize) -> Self {
        Self {
            interrupted: false,
            completed: total,
            total,
            percent_complete: 100.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interrupt_state_new() {
        let state = InterruptState::new();
        assert!(!state.was_interrupted());
        assert_eq!(state.files_completed(), 0);
        assert_eq!(state.total_files(), 0);
    }

    #[test]
    fn test_interrupt_state_tracking() {
        let state = InterruptState::new();
        state.set_total(100);
        state.increment_completed();
        state.increment_completed();

        assert_eq!(state.files_completed(), 2);
        assert_eq!(state.total_files(), 100);
    }

    #[test]
    fn test_interrupt_state_mark_interrupted() {
        let state = InterruptState::new();
        assert!(!state.was_interrupted());

        state.mark_interrupted();
        assert!(state.was_interrupted());
    }

    #[test]
    fn test_interrupt_metadata_from_state() {
        let state = InterruptState::new();
        state.set_total(100);
        for _ in 0..50 {
            state.increment_completed();
        }

        let metadata = InterruptMetadata::from_state(&state);
        assert_eq!(metadata.completed, 50);
        assert_eq!(metadata.total, 100);
        assert!((metadata.percent_complete - 50.0).abs() < 0.01);
    }

    #[test]
    fn test_interrupt_metadata_complete() {
        let metadata = InterruptMetadata::complete(100);
        assert!(!metadata.interrupted);
        assert_eq!(metadata.completed, 100);
        assert_eq!(metadata.total, 100);
        assert_eq!(metadata.percent_complete, 100.0);
    }

    #[test]
    fn test_reset_interrupted() {
        // Note: This test modifies global state
        reset_interrupted();
        assert!(!is_interrupted());
    }
}
