//! Types for the `tldr fix` diagnostic and auto-fix system.
//!
//! These types model the full lifecycle of error diagnosis:
//! - `ParsedError`: Structured representation of an error from compiler/runtime output
//! - `Diagnosis`: Result of analyzing an error, with optional fix
//! - `Fix`: A set of text edits that resolve the error
//! - `TextEdit`: A single edit operation on source text

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A parsed error extracted from compiler/runtime output.
///
/// This is the normalized representation of an error regardless of the
/// source format (Python traceback, rustc JSON, tsc line format, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedError {
    /// The error type/class (e.g., "UnboundLocalError", "E0599", "TS2304")
    pub error_type: String,
    /// The error message text
    pub message: String,
    /// Source file where the error occurred
    pub file: Option<PathBuf>,
    /// Line number (1-indexed)
    pub line: Option<usize>,
    /// Column number (0-indexed)
    pub column: Option<usize>,
    /// Detected or specified language
    pub language: String,
    /// The raw error text before parsing
    pub raw_text: String,
    /// The function name where the error occurred (extracted from traceback)
    pub function_name: Option<String>,
    /// The offending source line from the traceback
    pub offending_line: Option<String>,
}

/// Result of analyzing an error.
///
/// Contains the diagnosis explanation, confidence level, and an optional
/// fix that can be applied to resolve the error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnosis {
    /// Language the error was in
    pub language: String,
    /// Error code (e.g., "UnboundLocalError", "E0599", "TS2304")
    pub error_code: String,
    /// Human-readable explanation of what went wrong
    pub message: String,
    /// Source file and line where the error occurred
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<FixLocation>,
    /// Confidence that the fix will resolve the error
    pub confidence: FixConfidence,
    /// The fix to apply (None means cannot fix, escalate to a model)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<Fix>,
}

/// A source location for fix diagnostics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixLocation {
    /// Source file path
    pub file: PathBuf,
    /// Line number (1-indexed)
    pub line: usize,
    /// Column number (0-indexed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<usize>,
}

/// A fix consisting of one or more text edits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fix {
    /// What the fix does (human-readable)
    pub description: String,
    /// The edits to apply
    pub edits: Vec<TextEdit>,
}

/// A single text edit operation on source code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextEdit {
    /// Line number (1-indexed)
    pub line: usize,
    /// Column (0-indexed, for range operations)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<usize>,
    /// Kind of edit
    pub kind: EditKind,
    /// New text to insert or replace with
    pub new_text: String,
}

/// The kind of text edit operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum EditKind {
    /// Insert text as a new line before the specified line
    InsertBefore,
    /// Insert text as a new line after the specified line
    InsertAfter,
    /// Replace the entire line
    ReplaceLine,
    /// Delete the line entirely (removes it from output, unlike ReplaceLine
    /// with empty string which leaves a blank line)
    DeleteLine,
    /// Replace a specific column range on the line
    ReplaceRange {
        /// Start column (0-indexed, inclusive)
        start_col: usize,
        /// End column (0-indexed, exclusive)
        end_col: usize,
    },
}

/// Confidence level for a fix.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum FixConfidence {
    /// Fix is deterministic and proven correct for this error pattern
    High,
    /// Fix is likely correct but has edge cases
    Medium,
    /// Fix is a guess -- escalate to model if possible
    Low,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parsed_error_creation() {
        let err = ParsedError {
            error_type: "UnboundLocalError".to_string(),
            message: "cannot access local variable 'counter'".to_string(),
            file: Some(PathBuf::from("app.py")),
            line: Some(3),
            column: None,
            language: "python".to_string(),
            raw_text: "UnboundLocalError: cannot access local variable 'counter'".to_string(),
            function_name: Some("inc".to_string()),
            offending_line: Some("    counter += 1".to_string()),
        };
        assert_eq!(err.error_type, "UnboundLocalError");
        assert_eq!(err.line, Some(3));
    }

    #[test]
    fn test_diagnosis_serialization() {
        let diag = Diagnosis {
            language: "python".to_string(),
            error_code: "UnboundLocalError".to_string(),
            message: "Variable 'counter' needs global declaration".to_string(),
            location: Some(FixLocation {
                file: PathBuf::from("app.py"),
                line: 3,
                column: None,
            }),
            confidence: FixConfidence::High,
            fix: Some(Fix {
                description: "Inject `global counter` at function top".to_string(),
                edits: vec![TextEdit {
                    line: 2,
                    column: None,
                    kind: EditKind::InsertAfter,
                    new_text: "    global counter".to_string(),
                }],
            }),
        };
        let json = serde_json::to_string(&diag).unwrap();
        assert!(json.contains("UnboundLocalError"));
        assert!(json.contains("global counter"));
    }

    #[test]
    fn test_edit_kind_variants() {
        let insert_before = EditKind::InsertBefore;
        let insert_after = EditKind::InsertAfter;
        let replace_line = EditKind::ReplaceLine;
        let replace_range = EditKind::ReplaceRange {
            start_col: 4,
            end_col: 10,
        };
        assert_eq!(insert_before, EditKind::InsertBefore);
        assert_eq!(insert_after, EditKind::InsertAfter);
        assert_eq!(replace_line, EditKind::ReplaceLine);
        assert!(matches!(replace_range, EditKind::ReplaceRange { .. }));
    }

    #[test]
    fn test_fix_confidence_ordering() {
        assert_eq!(FixConfidence::High, FixConfidence::High);
        assert_ne!(FixConfidence::High, FixConfidence::Low);
    }

    #[test]
    fn test_diagnosis_without_fix() {
        let diag = Diagnosis {
            language: "python".to_string(),
            error_code: "RecursionError".to_string(),
            message: "Maximum recursion depth exceeded".to_string(),
            location: None,
            confidence: FixConfidence::Low,
            fix: None,
        };
        let json = serde_json::to_string(&diag).unwrap();
        // fix should be omitted due to skip_serializing_if
        assert!(!json.contains("\"fix\""));
    }
}
