//! L2 data types for bugbot analysis pipeline.
//!
//! Contains all types used by L2 analyzers: output containers, function
//! identifiers, structured errors, pipeline modes, and the finding store trait.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

use super::super::types::BugbotFinding;

/// Rich return type from L2 analysis engines.
///
/// Captures findings, status, timing, and function-level statistics
/// for a single analyzer run.
#[derive(Debug, Clone)]
pub struct L2AnalyzerOutput {
    /// Findings produced by this analyzer.
    pub findings: Vec<BugbotFinding>,
    /// Whether the analyzer completed fully, partially, or was skipped.
    pub status: AnalyzerStatus,
    /// Wall-clock time spent in this analyzer, in milliseconds.
    pub duration_ms: u64,
    /// Number of functions that were successfully analyzed.
    pub functions_analyzed: usize,
    /// Number of functions that were skipped (e.g. too complex, unsupported).
    pub functions_skipped: usize,
}

/// Status of an analyzer run.
///
/// Tracks whether an analyzer completed all work or encountered issues
/// that limited its coverage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnalyzerStatus {
    /// All target functions were analyzed without errors.
    Complete,
    /// Some functions were analyzed but others were skipped.
    Partial {
        /// Human-readable explanation of why analysis was partial.
        reason: String,
    },
    /// The analyzer was entirely skipped (e.g. wrong language, no functions).
    Skipped {
        /// Human-readable explanation of why the analyzer was skipped.
        reason: String,
    },
    /// The analyzer exceeded its time budget.
    TimedOut {
        /// Number of findings produced before the timeout.
        partial_findings: usize,
    },
}

/// Unique identifier for a function within the project.
///
/// Combines file path, qualified name, and definition line to
/// unambiguously identify a function even when multiple functions
/// share the same name across modules.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct FunctionId {
    /// Source file containing the function definition.
    pub file: PathBuf,
    /// Fully qualified name (e.g. `MyStruct::method`).
    pub qualified_name: String,
    /// Line number where the function definition starts (1-based).
    pub def_line: usize,
}

impl FunctionId {
    /// Create a new `FunctionId`.
    pub fn new(
        file: impl Into<PathBuf>,
        qualified_name: impl Into<String>,
        def_line: usize,
    ) -> Self {
        Self {
            file: file.into(),
            qualified_name: qualified_name.into(),
            def_line,
        }
    }
}

impl fmt::Display for FunctionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}",
            self.file.display(),
            self.def_line,
            self.qualified_name,
        )
    }
}

/// Structured error from an L2 analyzer.
///
/// Provides enough context to log actionable diagnostics without
/// aborting the entire pipeline. Includes the analyzer name, optional
/// file/function context, and a list of finding types that were
/// skipped as a result of the error.
#[derive(Debug)]
pub struct AnalyzerError {
    /// Name of the analyzer that produced this error (e.g. "dead-code", "taint").
    pub analyzer: &'static str,
    /// File being analyzed when the error occurred, if applicable.
    pub file: Option<PathBuf>,
    /// Function being analyzed when the error occurred, if applicable.
    pub function: Option<String>,
    /// The underlying error.
    pub cause: anyhow::Error,
    /// Finding types that could not be produced due to this error.
    pub skipped_finding_types: Vec<&'static str>,
    /// Whether the error is transient (e.g. timeout) vs permanent (e.g. parse failure).
    pub is_transient: bool,
}

impl fmt::Display for AnalyzerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "analyzer '{}' failed", self.analyzer)?;
        if let Some(ref file) = self.file {
            write!(f, " on {}", file.display())?;
        }
        if let Some(ref function) = self.function {
            write!(f, " in {}", function)?;
        }
        write!(f, ": {}", self.cause)
    }
}

impl std::error::Error for AnalyzerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.cause.source()
    }
}

/// Determines how the L2 pipeline is invoked.
///
/// `Check` mode analyzes only changed functions (the default `bugbot check` path).
/// `Scan` mode analyzes an entire project (stub for future `bugbot scan`).
#[derive(Debug)]
pub enum PipelineMode {
    /// Analyze only functions affected by recent changes.
    Check(CheckContext),
    /// Analyze all functions in a project (future).
    Scan(ScanContext),
}

/// Context for `Check` mode.
///
/// Marker struct -- the real per-run fields live on L2Context, which
/// is constructed by the pipeline orchestrator.
#[derive(Debug, Clone)]
pub struct CheckContext;

/// Context for `Scan` mode.
///
/// Carries the project root, detected language, and full file list
/// needed to scan an entire codebase.
#[derive(Debug, Clone)]
pub struct ScanContext {
    /// Root directory of the project being scanned.
    pub project: PathBuf,
    /// Primary language of the project.
    pub language: tldr_core::Language,
    /// All source files to analyze.
    pub all_files: Vec<PathBuf>,
}

/// Persistence layer for findings.
///
/// Allows the pipeline to record findings, check suppression status,
/// and retrieve false-positive rates for adaptive thresholding.
/// The trait is object-safe to allow `Box<dyn FindingStore>`.
pub trait FindingStore: Send + Sync {
    /// Record a batch of findings (e.g. persist to disk or database).
    fn record_findings(&self, findings: &[BugbotFinding]) -> anyhow::Result<()>;
    /// Check whether a specific finding has been suppressed by the user.
    fn was_suppressed(&self, finding_id: &str) -> bool;
    /// Return the historical false-positive rate for a finding type (0.0..1.0).
    fn false_positive_rate(&self, finding_type: &str) -> f64;
}

/// No-op implementation of [`FindingStore`] for use in tests and
/// single-run modes where persistence is not needed.
pub struct NoOpFindingStore;

impl FindingStore for NoOpFindingStore {
    fn record_findings(&self, _findings: &[BugbotFinding]) -> anyhow::Result<()> {
        Ok(())
    }

    fn was_suppressed(&self, _finding_id: &str) -> bool {
        false
    }

    fn false_positive_rate(&self, _finding_type: &str) -> f64 {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    #[test]
    fn test_analyzer_status_variants() {
        // Construct each variant and verify Debug output is non-empty
        let complete = AnalyzerStatus::Complete;
        let debug_str = format!("{:?}", complete);
        assert!(!debug_str.is_empty(), "Complete Debug should be non-empty");

        let partial = AnalyzerStatus::Partial {
            reason: "3 functions too complex".to_string(),
        };
        let debug_str = format!("{:?}", partial);
        assert!(
            debug_str.contains("Partial"),
            "Partial Debug should contain variant name"
        );

        let skipped = AnalyzerStatus::Skipped {
            reason: "wrong language".to_string(),
        };
        let debug_str = format!("{:?}", skipped);
        assert!(
            debug_str.contains("Skipped"),
            "Skipped Debug should contain variant name"
        );

        let timed_out = AnalyzerStatus::TimedOut {
            partial_findings: 7,
        };
        let debug_str = format!("{:?}", timed_out);
        assert!(
            debug_str.contains("TimedOut"),
            "TimedOut Debug should contain variant name"
        );
    }

    #[test]
    fn test_function_id_eq() {
        let id_a = FunctionId {
            file: PathBuf::from("src/main.rs"),
            qualified_name: "Foo::bar".to_string(),
            def_line: 42,
        };
        let id_b = FunctionId {
            file: PathBuf::from("src/main.rs"),
            qualified_name: "Foo::bar".to_string(),
            def_line: 42,
        };
        assert_eq!(id_a, id_b, "same fields should be equal");

        let id_c = FunctionId {
            file: PathBuf::from("src/main.rs"),
            qualified_name: "Foo::bar".to_string(),
            def_line: 99,
        };
        assert_ne!(id_a, id_c, "different def_line should be not equal");
    }

    #[test]
    fn test_function_id_hash() {
        let id_a = FunctionId {
            file: PathBuf::from("src/lib.rs"),
            qualified_name: "process".to_string(),
            def_line: 10,
        };
        let id_b = FunctionId {
            file: PathBuf::from("src/lib.rs"),
            qualified_name: "process".to_string(),
            def_line: 10,
        };

        let mut hasher_a = DefaultHasher::new();
        id_a.hash(&mut hasher_a);
        let hash_a = hasher_a.finish();

        let mut hasher_b = DefaultHasher::new();
        id_b.hash(&mut hasher_b);
        let hash_b = hasher_b.finish();

        assert_eq!(hash_a, hash_b, "equal FunctionIds must produce same hash");
    }

    #[test]
    fn test_function_id_display() {
        let id = FunctionId {
            file: PathBuf::from("src/engine.rs"),
            qualified_name: "Engine::run".to_string(),
            def_line: 55,
        };
        let display = format!("{}", id);
        assert_eq!(
            display, "src/engine.rs:55:Engine::run",
            "Display should be file:line:name"
        );
    }

    #[test]
    fn test_analyzer_error_display() {
        let err = AnalyzerError {
            analyzer: "dead-code",
            file: Some(PathBuf::from("src/lib.rs")),
            function: Some("compute".to_string()),
            cause: anyhow::anyhow!("CFG construction failed"),
            skipped_finding_types: vec!["born-dead"],
            is_transient: false,
        };
        let msg = format!("{}", err);
        assert!(
            msg.contains("dead-code"),
            "display should contain analyzer name"
        );
        assert!(
            msg.contains("CFG construction failed"),
            "display should contain cause"
        );
        assert!(
            msg.contains("src/lib.rs"),
            "display should contain file path"
        );
        assert!(
            msg.contains("compute"),
            "display should contain function name"
        );
    }

    #[test]
    fn test_analyzer_error_is_std_error() {
        let err = AnalyzerError {
            analyzer: "taint",
            file: None,
            function: None,
            cause: anyhow::anyhow!("internal"),
            skipped_finding_types: vec![],
            is_transient: true,
        };
        // Verify the type implements std::error::Error
        let _: &dyn std::error::Error = &err;
    }

    #[test]
    fn test_noop_finding_store() {
        let store = NoOpFindingStore;

        // record_findings returns Ok
        let findings = vec![BugbotFinding {
            finding_type: "born-dead".to_string(),
            severity: "high".to_string(),
            file: PathBuf::from("src/main.rs"),
            function: "main".to_string(),
            line: 10,
            message: "dead code".to_string(),
            evidence: serde_json::json!({}),
            confidence: None,
            finding_id: None,
        }];
        assert!(
            store.record_findings(&findings).is_ok(),
            "record_findings should return Ok"
        );

        // was_suppressed always returns false
        assert!(
            !store.was_suppressed("any-id"),
            "was_suppressed should return false"
        );

        // false_positive_rate always returns 0.0
        let rate = store.false_positive_rate("born-dead");
        assert!(
            (rate - 0.0).abs() < f64::EPSILON,
            "false_positive_rate should return 0.0"
        );
    }

    #[test]
    fn test_finding_store_object_safe() {
        // Store as Box<dyn FindingStore> and call all methods
        let store: Box<dyn FindingStore> = Box::new(NoOpFindingStore);

        let result = store.record_findings(&[]);
        assert!(result.is_ok(), "boxed store record_findings should work");

        assert!(
            !store.was_suppressed("test"),
            "boxed store was_suppressed should work"
        );

        let rate = store.false_positive_rate("test-type");
        assert!(
            (rate - 0.0).abs() < f64::EPSILON,
            "boxed store false_positive_rate should work"
        );
    }

    #[test]
    fn test_pipeline_mode_check() {
        let mode = PipelineMode::Check(CheckContext);
        // Verify Debug works and it's the Check variant
        let debug_str = format!("{:?}", mode);
        assert!(
            debug_str.contains("Check"),
            "PipelineMode::Check debug should contain 'Check'"
        );
    }

    #[test]
    fn test_l2_analyzer_output_default() {
        let output = L2AnalyzerOutput {
            findings: vec![],
            status: AnalyzerStatus::Complete,
            duration_ms: 0,
            functions_analyzed: 0,
            functions_skipped: 0,
        };
        assert!(output.findings.is_empty(), "findings should be empty");
        assert_eq!(output.duration_ms, 0);
        assert_eq!(output.functions_analyzed, 0);
        assert_eq!(output.functions_skipped, 0);
        // Verify status is Complete via Debug
        let debug_str = format!("{:?}", output.status);
        assert!(debug_str.contains("Complete"));
    }
}
