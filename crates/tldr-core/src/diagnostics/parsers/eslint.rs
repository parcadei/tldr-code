//! ESLint JSON output parser.
//!
//! ESLint outputs JSON via `-f json` with the following structure:
//! ```json
//! [
//!   {
//!     "filePath": "/abs/path/src/auth.ts",
//!     "messages": [
//!       {
//!         "ruleId": "no-unused-vars",
//!         "severity": 2,
//!         "message": "'x' is defined but never used",
//!         "line": 10,
//!         "column": 5,
//!         "endLine": 10,
//!         "endColumn": 6
//!       }
//!     ]
//!   }
//! ]
//! ```
//!
//! ESLint severity: 2=error, 1=warning

use crate::diagnostics::{Diagnostic, Severity};
use crate::error::TldrError;
use serde::Deserialize;
use std::path::PathBuf;

/// ESLint file result
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EslintFileResult {
    file_path: String,
    messages: Vec<EslintMessage>,
    #[allow(dead_code)]
    error_count: Option<u32>,
    #[allow(dead_code)]
    warning_count: Option<u32>,
}

/// ESLint message
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EslintMessage {
    rule_id: Option<String>,
    severity: u8,
    message: String,
    line: u32,
    column: u32,
    end_line: Option<u32>,
    end_column: Option<u32>,
}

/// Parse ESLint JSON output into unified Diagnostic structs.
///
/// # Arguments
/// * `output` - The raw JSON output from `eslint -f json`
///
/// # Returns
/// A vector of Diagnostic structs, or an error if parsing fails.
///
/// # Severity Mapping
/// - severity 2 = Error
/// - severity 1 = Warning
pub fn parse_eslint_output(output: &str) -> Result<Vec<Diagnostic>, TldrError> {
    // Handle empty output
    if output.trim().is_empty() {
        return Ok(Vec::new());
    }

    // Handle empty array
    if output.trim() == "[]" {
        return Ok(Vec::new());
    }

    let parsed: Vec<EslintFileResult> =
        serde_json::from_str(output).map_err(|e| TldrError::ParseError {
            file: std::path::PathBuf::from("<eslint-output>"),
            line: None,
            message: format!("Failed to parse eslint JSON: {}", e),
        })?;

    let mut diagnostics = Vec::new();

    for file_result in parsed {
        for msg in file_result.messages {
            let severity = match msg.severity {
                2 => Severity::Error,
                1 => Severity::Warning,
                _ => Severity::Warning,
            };

            diagnostics.push(Diagnostic {
                file: PathBuf::from(&file_result.file_path),
                line: msg.line,
                column: msg.column,
                end_line: msg.end_line,
                end_column: msg.end_column,
                severity,
                message: msg.message,
                code: msg.rule_id,
                source: "eslint".to_string(),
                url: None,
            });
        }
    }

    Ok(diagnostics)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple() {
        let json = r#"[
            {
                "filePath": "/project/src/auth.ts",
                "messages": [
                    {
                        "ruleId": "no-unused-vars",
                        "severity": 2,
                        "message": "'x' is defined but never used.",
                        "line": 10,
                        "column": 5,
                        "endLine": 10,
                        "endColumn": 6
                    }
                ],
                "errorCount": 1,
                "warningCount": 0
            }
        ]"#;

        let result = parse_eslint_output(json).unwrap();
        assert_eq!(result.len(), 1);

        let d = &result[0];
        assert_eq!(d.file, PathBuf::from("/project/src/auth.ts"));
        assert_eq!(d.line, 10);
        assert_eq!(d.column, 5);
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(d.code, Some("no-unused-vars".to_string()));
        assert_eq!(d.source, "eslint");
    }

    #[test]
    fn test_severity_mapping() {
        let json = r#"[
            {
                "filePath": "test.ts",
                "messages": [
                    {"ruleId": "a", "severity": 2, "message": "error", "line": 1, "column": 1},
                    {"ruleId": "b", "severity": 1, "message": "warning", "line": 2, "column": 1}
                ]
            }
        ]"#;

        let result = parse_eslint_output(json).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].severity, Severity::Error);
        assert_eq!(result[1].severity, Severity::Warning);
    }

    #[test]
    fn test_empty_messages() {
        let json = r#"[
            {
                "filePath": "/project/src/clean.ts",
                "messages": [],
                "errorCount": 0,
                "warningCount": 0
            }
        ]"#;

        let result = parse_eslint_output(json).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_empty_output() {
        let result = parse_eslint_output("").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_empty_array() {
        let result = parse_eslint_output("[]").unwrap();
        assert!(result.is_empty());
    }
}
