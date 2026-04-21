//! Go diagnostic tool parsers (go vet, golangci-lint).
//!
//! ## go vet
//! Outputs NDJSON via `go vet -json`:
//! ```json
//! {"file":"/project/main.go","line":15,"col":5,"message":"printf: wrong type"}
//! ```
//!
//! ## golangci-lint
//! Outputs JSON via `golangci-lint run --out-format json`:
//! ```json
//! {
//!   "Issues": [
//!     {
//!       "FromLinter": "govet",
//!       "Text": "printf format error",
//!       "Severity": "warning",
//!       "Pos": {"Filename": "main.go", "Line": 15, "Column": 5}
//!     }
//!   ]
//! }
//! ```

use crate::diagnostics::{Diagnostic, Severity};
use crate::error::TldrError;
use serde::Deserialize;
use std::path::PathBuf;

// =============================================================================
// go vet parser
// =============================================================================

/// go vet JSON line (NDJSON format)
#[derive(Debug, Deserialize)]
struct GoVetLine {
    file: String,
    line: u32,
    col: Option<u32>,
    message: String,
}

/// Parse go vet NDJSON output into unified Diagnostic structs.
///
/// # Arguments
/// * `output` - The raw NDJSON output from `go vet -json`
///
/// # Returns
/// A vector of Diagnostic structs, or an error if parsing fails.
pub fn parse_go_vet_output(output: &str) -> Result<Vec<Diagnostic>, TldrError> {
    let mut diagnostics = Vec::new();

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Skip lines that look like package names (go vet can output these)
        if !line.starts_with('{') {
            continue;
        }

        let vet_line: GoVetLine = match serde_json::from_str(line) {
            Ok(l) => l,
            Err(_) => continue,
        };

        diagnostics.push(Diagnostic {
            file: PathBuf::from(&vet_line.file),
            line: vet_line.line,
            column: vet_line.col.unwrap_or(1),
            end_line: None,
            end_column: None,
            severity: Severity::Warning, // go vet issues are warnings
            message: vet_line.message,
            code: None,
            source: "go vet".to_string(),
            url: None,
        });
    }

    Ok(diagnostics)
}

// =============================================================================
// golangci-lint parser
// =============================================================================

/// golangci-lint JSON output
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GolangciLintOutput {
    issues: Option<Vec<GolangciLintIssue>>,
}

/// golangci-lint issue
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GolangciLintIssue {
    from_linter: String,
    text: String,
    severity: Option<String>,
    pos: GolangciLintPos,
}

/// golangci-lint position
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GolangciLintPos {
    filename: String,
    line: u32,
    column: Option<u32>,
}

/// Parse golangci-lint JSON output into unified Diagnostic structs.
///
/// # Arguments
/// * `output` - The raw JSON output from `golangci-lint run --out-format json`
///
/// # Returns
/// A vector of Diagnostic structs, or an error if parsing fails.
pub fn parse_golangci_lint_output(output: &str) -> Result<Vec<Diagnostic>, TldrError> {
    // Handle empty output
    if output.trim().is_empty() {
        return Ok(Vec::new());
    }

    let parsed: GolangciLintOutput =
        serde_json::from_str(output).map_err(|e| TldrError::ParseError {
            file: std::path::PathBuf::from("<golangci-lint-output>"),
            line: None,
            message: format!("Failed to parse golangci-lint JSON: {}", e),
        })?;

    let issues = parsed.issues.unwrap_or_default();

    let diagnostics = issues
        .into_iter()
        .map(|issue| {
            let severity = issue
                .severity
                .as_ref()
                .map(|s| match s.to_lowercase().as_str() {
                    "error" => Severity::Error,
                    "warning" => Severity::Warning,
                    _ => Severity::Warning,
                })
                .unwrap_or(Severity::Warning);

            Diagnostic {
                file: PathBuf::from(&issue.pos.filename),
                line: issue.pos.line,
                column: issue.pos.column.unwrap_or(1),
                end_line: None,
                end_column: None,
                severity,
                message: issue.text,
                code: None,
                source: issue.from_linter,
                url: None,
            }
        })
        .collect();

    Ok(diagnostics)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GO_VET_OUTPUT: &str = r#"{"file":"/project/main.go","line":15,"col":5,"message":"printf: Sprintf format %d has arg x of wrong type string"}
{"file":"/project/utils.go","line":30,"col":1,"message":"unreachable code"}"#;

    const GOLANGCI_LINT_OUTPUT: &str = r#"{
        "Issues": [
            {
                "FromLinter": "govet",
                "Text": "printf: Sprintf format %d has arg x of wrong type string",
                "Severity": "warning",
                "SourceLines": ["fmt.Sprintf(\"%d\", x)"],
                "Pos": {
                    "Filename": "main.go",
                    "Offset": 150,
                    "Line": 15,
                    "Column": 5
                }
            },
            {
                "FromLinter": "staticcheck",
                "Text": "this value of err is never used",
                "Severity": "error",
                "SourceLines": ["err := doSomething()"],
                "Pos": {
                    "Filename": "utils.go",
                    "Offset": 300,
                    "Line": 25,
                    "Column": 1
                }
            }
        ]
    }"#;

    #[test]
    fn test_parse_go_vet() {
        let result = parse_go_vet_output(GO_VET_OUTPUT).unwrap();
        assert_eq!(result.len(), 2);

        let d = &result[0];
        assert_eq!(d.file, PathBuf::from("/project/main.go"));
        assert_eq!(d.line, 15);
        assert_eq!(d.column, 5);
        assert_eq!(d.source, "go vet");
    }

    #[test]
    fn test_parse_golangci_lint() {
        let result = parse_golangci_lint_output(GOLANGCI_LINT_OUTPUT).unwrap();
        assert_eq!(result.len(), 2);

        let warning = &result[0];
        assert_eq!(warning.file, PathBuf::from("main.go"));
        assert_eq!(warning.severity, Severity::Warning);
        assert_eq!(warning.source, "govet");

        let error = &result[1];
        assert_eq!(error.file, PathBuf::from("utils.go"));
        assert_eq!(error.severity, Severity::Error);
        assert_eq!(error.source, "staticcheck");
    }

    #[test]
    fn test_empty_output() {
        assert!(parse_go_vet_output("").unwrap().is_empty());
        assert!(parse_golangci_lint_output("").unwrap().is_empty());
    }

    #[test]
    fn test_empty_issues() {
        let output = r#"{"Issues": null}"#;
        let result = parse_golangci_lint_output(output).unwrap();
        assert!(result.is_empty());
    }
}
