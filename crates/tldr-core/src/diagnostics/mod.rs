//! Diagnostics module - Unified type checking and linting across languages
//!
//! This module provides a unified interface for running diagnostic tools
//! (type checkers and linters) and parsing their output into a common format.
//!
//! # Supported Tools by Language
//!
//! - **Python**: pyright (type checking), ruff (linting)
//! - **TypeScript/JavaScript**: tsc (type checking), eslint (linting)
//! - **Go**: go vet, golangci-lint
//! - **Rust**: cargo check, clippy
//! - **Java**: javac (type checking), checkstyle (linting)
//! - **C/C++**: clang (syntax checking), clang-tidy (static analysis)
//! - **Ruby**: rubocop (linting)
//! - **PHP**: php -l (syntax checking), phpstan (static analysis)
//! - **Kotlin**: kotlinc, detekt
//! - **Swift**: swiftc, swiftlint
//! - **C#**: dotnet build
//! - **Scala**: scalac
//! - **Elixir**: mix compile, credo
//! - **Lua**: luacheck
//!
//! # Example
//!
//! ```rust,ignore
//! use tldr_core::diagnostics::{run_diagnostics, Severity};
//!
//! let report = run_diagnostics(
//!     Path::new("src/"),
//!     Language::Python,
//!     DiagnosticsOptions::default(),
//! )?;
//!
//! for diag in &report.diagnostics {
//!     if diag.severity == Severity::Error {
//!         println!("{}:{}:{}: {}", diag.file, diag.line, diag.column, diag.message);
//!     }
//! }
//! ```

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Diagnostic severity (LSP-compatible)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Error severity - must be fixed
    Error = 1,
    /// Warning severity - should be addressed
    Warning = 2,
    /// Information severity - informational message
    Information = 3,
    /// Hint severity - suggestion for improvement
    #[default]
    Hint = 4,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Error => write!(f, "error"),
            Severity::Warning => write!(f, "warning"),
            Severity::Information => write!(f, "info"),
            Severity::Hint => write!(f, "hint"),
        }
    }
}

/// A single diagnostic message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    /// Source file path
    pub file: PathBuf,

    /// Start line (1-indexed)
    pub line: u32,

    /// Start column (1-indexed)
    pub column: u32,

    /// End line (optional)
    pub end_line: Option<u32>,

    /// End column (optional)
    pub end_column: Option<u32>,

    /// Diagnostic severity
    pub severity: Severity,

    /// Human-readable message
    pub message: String,

    /// Error/rule code (e.g., "E501", "TS2339", "reportUnusedVariable")
    pub code: Option<String>,

    /// Tool that generated this diagnostic
    pub source: String,

    /// URL for more information
    pub url: Option<String>,
}

impl Diagnostic {
    /// Generate a deduplication key for this diagnostic.
    /// Two diagnostics with the same key are considered duplicates.
    /// Key is based on file, line, column, and a hash of the message.
    pub fn dedupe_key(&self) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        self.message.hash(&mut hasher);
        let msg_hash = hasher.finish();

        format!(
            "{}:{}:{}:{:x}",
            self.file.display(),
            self.line,
            self.column,
            msg_hash
        )
    }
}

/// Complete diagnostics report
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DiagnosticsReport {
    /// All diagnostics found
    pub diagnostics: Vec<Diagnostic>,

    /// Summary counts by severity
    pub summary: DiagnosticsSummary,

    /// Tools that were run
    pub tools_run: Vec<ToolResult>,

    /// Files analyzed
    pub files_analyzed: usize,
}

/// Summary of diagnostic counts by severity
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DiagnosticsSummary {
    /// Number of errors
    pub errors: usize,
    /// Number of warnings
    pub warnings: usize,
    /// Number of informational messages
    pub info: usize,
    /// Number of hints
    pub hints: usize,
    /// Total number of diagnostics
    pub total: usize,
}

/// Result from running a diagnostic tool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// Tool name
    pub name: String,
    /// Tool version (if available)
    pub version: Option<String>,
    /// Whether the tool ran successfully
    pub success: bool,
    /// Duration in milliseconds
    pub duration_ms: u64,
    /// Number of diagnostics produced
    pub diagnostic_count: usize,
    /// Error message if tool failed
    pub error: Option<String>,
}

/// Tool configuration for running diagnostics
#[derive(Debug, Clone)]
pub struct ToolConfig {
    /// Tool name for display
    pub name: &'static str,
    /// Binary name to execute
    pub binary: &'static str,
    /// Command line arguments
    pub args: Vec<String>,
    /// Whether this is a type checker
    pub is_type_checker: bool,
    /// Whether this is a linter
    pub is_linter: bool,
}

/// Options for running diagnostics
#[derive(Debug, Clone, Default)]
pub struct DiagnosticsOptions {
    /// Minimum severity to report
    pub min_severity: Severity,
    /// Skip type checkers
    pub no_typecheck: bool,
    /// Skip linters
    pub no_lint: bool,
    /// Specific tools to run (empty = auto-detect)
    pub tools: Vec<String>,
    /// Timeout per tool in seconds
    pub timeout_secs: u64,
    /// Analyze entire project
    pub project_mode: bool,
    /// File patterns to filter
    pub filter_files: Vec<String>,
    /// Error codes to ignore
    pub ignore_codes: Vec<String>,
}

// =============================================================================
// Phase 6: Filtering and Summary Functions
// =============================================================================

/// Filter diagnostics by minimum severity level.
/// Only diagnostics with severity <= min_severity are returned.
/// (Error=1 < Warning=2 < Information=3 < Hint=4)
pub fn filter_diagnostics_by_severity(
    diagnostics: &[Diagnostic],
    min_severity: Severity,
) -> Vec<Diagnostic> {
    diagnostics
        .iter()
        .filter(|d| d.severity <= min_severity)
        .cloned()
        .collect()
}

/// Compute summary statistics from a list of diagnostics.
pub fn compute_summary(diagnostics: &[Diagnostic]) -> DiagnosticsSummary {
    let mut summary = DiagnosticsSummary::default();

    for diag in diagnostics {
        match diag.severity {
            Severity::Error => summary.errors += 1,
            Severity::Warning => summary.warnings += 1,
            Severity::Information => summary.info += 1,
            Severity::Hint => summary.hints += 1,
        }
    }

    summary.total = diagnostics.len();
    summary
}

/// Deduplicate diagnostics based on their dedupe_key.
/// Returns diagnostics with duplicates removed (keeps first occurrence).
pub fn dedupe_diagnostics(diagnostics: Vec<Diagnostic>) -> Vec<Diagnostic> {
    use std::collections::HashSet;

    let mut seen = HashSet::new();
    diagnostics
        .into_iter()
        .filter(|d| seen.insert(d.dedupe_key()))
        .collect()
}

/// Compute exit code based on diagnostics summary.
/// - 0 if no errors (or only warnings without strict mode)
/// - 1 if errors found (or warnings with strict mode)
pub fn compute_exit_code(summary: &DiagnosticsSummary, strict: bool) -> i32 {
    if summary.errors > 0 || (strict && summary.warnings > 0) {
        1
    } else {
        0
    }
}

// =============================================================================
// Submodules (Phases 7-9)
// =============================================================================

pub mod parsers;
pub mod runner;

// Re-exports
pub use parsers::*;
pub use runner::*;
