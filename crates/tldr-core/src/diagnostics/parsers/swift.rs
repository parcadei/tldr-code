//! Swift diagnostic tool output parsers.
//!
//! Supports:
//! - `swiftc`: Swift compiler with GCC-like text output
//!   Format: `file.swift:line:col: error: message`
//! - `swiftlint`: Linter with JSON output (`--reporter json`)
//!   JSON array of objects with file, line, column, severity, reason, rule_id

use crate::diagnostics::{Diagnostic, Severity};
use crate::error::TldrError;
use regex::Regex;
use serde::Deserialize;
use std::path::PathBuf;

// =============================================================================
// swiftc parser (text-based, GCC-like format)
// =============================================================================

/// Parse swiftc text output into unified Diagnostic structs.
///
/// swiftc outputs errors in GCC-like format:
/// `file.swift:line:col: severity: message`
///
/// # Arguments
/// * `output` - The raw text output from `swiftc -typecheck`
///
/// # Returns
/// A vector of Diagnostic structs. Malformed lines are skipped.
pub fn parse_swiftc_output(output: &str) -> Result<Vec<Diagnostic>, TldrError> {
    if output.trim().is_empty() {
        return Ok(Vec::new());
    }

    // Pattern: file.swift:line:col: severity: message
    let regex = Regex::new(r"^(.+\.swift):(\d+):(\d+):\s*(error|warning|note):\s*(.+)$")
        .expect("Invalid swiftc regex pattern");

    let mut diagnostics = Vec::new();

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if let Some(captures) = regex.captures(line) {
            let file = captures.get(1).map(|m| m.as_str()).unwrap_or("");
            let line_num: u32 = captures
                .get(2)
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(1);
            let column: u32 = captures
                .get(3)
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(1);
            let severity_str = captures.get(4).map(|m| m.as_str()).unwrap_or("error");
            let message = captures
                .get(5)
                .map(|m| m.as_str())
                .unwrap_or("")
                .to_string();

            let severity = match severity_str {
                "error" => Severity::Error,
                "warning" => Severity::Warning,
                "note" => Severity::Information,
                _ => Severity::Error,
            };

            diagnostics.push(Diagnostic {
                file: PathBuf::from(file),
                line: line_num,
                column,
                end_line: None,
                end_column: None,
                severity,
                message,
                code: None,
                source: "swiftc".to_string(),
                url: None,
            });
        }
    }

    Ok(diagnostics)
}

// =============================================================================
// swiftlint parser (JSON-based)
// =============================================================================

/// SwiftLint JSON output structure
#[derive(Debug, Deserialize)]
struct SwiftLintDiagnostic {
    file: String,
    line: u32,
    #[serde(default = "default_column")]
    column: u32,
    severity: String,
    reason: String,
    rule_id: String,
}

fn default_column() -> u32 {
    1
}

/// Parse swiftlint JSON output into unified Diagnostic structs.
///
/// SwiftLint outputs JSON via `--reporter json`:
/// ```json
/// [
///   {
///     "file": "/path/to/file.swift",
///     "line": 42,
///     "column": 5,
///     "severity": "Warning",
///     "reason": "Force unwrapping should be avoided.",
///     "rule_id": "force_unwrapping"
///   }
/// ]
/// ```
///
/// # Arguments
/// * `output` - The raw JSON output from `swiftlint lint --reporter json`
///
/// # Returns
/// A vector of Diagnostic structs, or an error if JSON parsing fails.
pub fn parse_swiftlint_output(output: &str) -> Result<Vec<Diagnostic>, TldrError> {
    if output.trim().is_empty() {
        return Ok(Vec::new());
    }

    if output.trim() == "[]" {
        return Ok(Vec::new());
    }

    let parsed: Vec<SwiftLintDiagnostic> =
        serde_json::from_str(output).map_err(|e| TldrError::ParseError {
            file: PathBuf::from("<swiftlint-output>"),
            line: None,
            message: format!("Failed to parse swiftlint JSON: {}", e),
        })?;

    let diagnostics = parsed
        .into_iter()
        .map(|d| {
            let severity = match d.severity.to_lowercase().as_str() {
                "error" => Severity::Error,
                "warning" => Severity::Warning,
                _ => Severity::Warning,
            };

            Diagnostic {
                file: PathBuf::from(&d.file),
                line: d.line,
                column: d.column,
                end_line: None,
                end_column: None,
                severity,
                message: d.reason,
                code: Some(d.rule_id),
                source: "swiftlint".to_string(),
                url: None,
            }
        })
        .collect();

    Ok(diagnostics)
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // swiftc tests
    // =========================================================================

    #[test]
    fn test_parse_swiftc_error() {
        let output = "Sources/main.swift:10:5: error: use of unresolved identifier 'foo'";

        let result = parse_swiftc_output(output).unwrap();
        assert_eq!(result.len(), 1);

        let d = &result[0];
        assert_eq!(d.file, PathBuf::from("Sources/main.swift"));
        assert_eq!(d.line, 10);
        assert_eq!(d.column, 5);
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(d.message, "use of unresolved identifier 'foo'");
        assert_eq!(d.source, "swiftc");
    }

    #[test]
    fn test_parse_swiftc_warning() {
        let output = "Sources/utils.swift:25:1: warning: variable 'x' was never used";

        let result = parse_swiftc_output(output).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].severity, Severity::Warning);
    }

    #[test]
    fn test_parse_swiftc_note() {
        let output = "Sources/main.swift:15:3: note: did you mean 'bar'?";

        let result = parse_swiftc_output(output).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].severity, Severity::Information);
    }

    #[test]
    fn test_parse_swiftc_multiple() {
        let output = r#"Sources/main.swift:10:5: error: use of unresolved identifier 'foo'
Sources/main.swift:15:10: warning: unused variable 'bar'
Sources/utils.swift:3:1: error: expected declaration"#;

        let result = parse_swiftc_output(output).unwrap();
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_parse_swiftc_empty() {
        let result = parse_swiftc_output("").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_swiftc_malformed() {
        let output = "some random text";
        let result = parse_swiftc_output(output).unwrap();
        assert!(result.is_empty());
    }

    // =========================================================================
    // swiftlint tests
    // =========================================================================

    #[test]
    fn test_parse_swiftlint_simple() {
        let json = r#"[
            {
                "file": "/path/to/file.swift",
                "line": 42,
                "column": 5,
                "severity": "Warning",
                "reason": "Force unwrapping should be avoided.",
                "rule_id": "force_unwrapping"
            }
        ]"#;

        let result = parse_swiftlint_output(json).unwrap();
        assert_eq!(result.len(), 1);

        let d = &result[0];
        assert_eq!(d.file, PathBuf::from("/path/to/file.swift"));
        assert_eq!(d.line, 42);
        assert_eq!(d.column, 5);
        assert_eq!(d.severity, Severity::Warning);
        assert_eq!(d.code, Some("force_unwrapping".to_string()));
        assert_eq!(d.message, "Force unwrapping should be avoided.");
        assert_eq!(d.source, "swiftlint");
    }

    #[test]
    fn test_parse_swiftlint_error() {
        let json = r#"[
            {
                "file": "main.swift",
                "line": 10,
                "column": 1,
                "severity": "Error",
                "reason": "Line length violation.",
                "rule_id": "line_length"
            }
        ]"#;

        let result = parse_swiftlint_output(json).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].severity, Severity::Error);
    }

    #[test]
    fn test_parse_swiftlint_multiple() {
        let json = r#"[
            {
                "file": "main.swift",
                "line": 10,
                "column": 1,
                "severity": "Warning",
                "reason": "Issue 1",
                "rule_id": "rule_1"
            },
            {
                "file": "utils.swift",
                "line": 20,
                "column": 5,
                "severity": "Error",
                "reason": "Issue 2",
                "rule_id": "rule_2"
            }
        ]"#;

        let result = parse_swiftlint_output(json).unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_parse_swiftlint_empty() {
        let result = parse_swiftlint_output("").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_swiftlint_empty_array() {
        let result = parse_swiftlint_output("[]").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_swiftlint_invalid_json() {
        let result = parse_swiftlint_output("not json");
        assert!(result.is_err());
    }
}
