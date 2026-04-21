//! Pyright JSON output parser.
//!
//! Pyright outputs JSON via `--outputjson` flag with the following structure:
//! ```json
//! {
//!   "version": "1.1.350",
//!   "generalDiagnostics": [
//!     {
//!       "file": "src/auth.py",
//!       "severity": "error",
//!       "message": "...",
//!       "rule": "reportArgumentType",
//!       "range": {
//!         "start": {"line": 41, "character": 4},
//!         "end": {"line": 41, "character": 14}
//!       }
//!     }
//!   ]
//! }
//! ```
//!
//! **Important**: Pyright uses 0-indexed lines, we convert to 1-indexed.

use crate::diagnostics::{Diagnostic, Severity};
use crate::error::TldrError;
use serde::Deserialize;
use std::path::PathBuf;

/// Pyright JSON output structure
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PyrightOutput {
    #[allow(dead_code)]
    version: Option<String>,
    general_diagnostics: Vec<PyrightDiagnostic>,
}

#[derive(Debug, Deserialize)]
struct PyrightDiagnostic {
    file: String,
    severity: String,
    message: String,
    rule: Option<String>,
    range: PyrightRange,
}

#[derive(Debug, Deserialize)]
struct PyrightRange {
    start: PyrightPosition,
    end: PyrightPosition,
}

#[derive(Debug, Deserialize)]
struct PyrightPosition {
    line: u32,
    character: u32,
}

/// Parse pyright JSON output into unified Diagnostic structs.
///
/// # Arguments
/// * `output` - The raw JSON output from `pyright --outputjson`
///
/// # Returns
/// A vector of Diagnostic structs, or an error if parsing fails.
///
/// # Line Number Conversion
/// Pyright uses 0-indexed lines. This parser converts to 1-indexed
/// to match editor conventions and other tools.
pub fn parse_pyright_output(output: &str) -> Result<Vec<Diagnostic>, TldrError> {
    // Handle empty output
    if output.trim().is_empty() {
        return Ok(Vec::new());
    }

    let parsed: PyrightOutput =
        serde_json::from_str(output).map_err(|e| TldrError::ParseError {
            file: std::path::PathBuf::from("<pyright-output>"),
            line: None,
            message: format!("Failed to parse pyright JSON: {}", e),
        })?;

    let diagnostics = parsed
        .general_diagnostics
        .into_iter()
        .map(|d| Diagnostic {
            file: PathBuf::from(&d.file),
            // Convert 0-indexed to 1-indexed
            line: d.range.start.line + 1,
            column: d.range.start.character + 1,
            end_line: Some(d.range.end.line + 1),
            end_column: Some(d.range.end.character + 1),
            severity: map_pyright_severity(&d.severity),
            message: d.message,
            code: d.rule,
            source: "pyright".to_string(),
            url: None,
        })
        .collect();

    Ok(diagnostics)
}

/// Map pyright severity string to our Severity enum.
fn map_pyright_severity(s: &str) -> Severity {
    match s.to_lowercase().as_str() {
        "error" => Severity::Error,
        "warning" => Severity::Warning,
        "information" => Severity::Information,
        "hint" => Severity::Hint,
        _ => Severity::Warning,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple() {
        let json = r#"{
            "version": "1.1.350",
            "generalDiagnostics": [
                {
                    "file": "test.py",
                    "severity": "error",
                    "message": "Type error",
                    "rule": "reportArgumentType",
                    "range": {
                        "start": {"line": 10, "character": 5},
                        "end": {"line": 10, "character": 15}
                    }
                }
            ]
        }"#;

        let result = parse_pyright_output(json).unwrap();
        assert_eq!(result.len(), 1);

        let d = &result[0];
        assert_eq!(d.file, PathBuf::from("test.py"));
        assert_eq!(d.line, 11); // 0-indexed to 1-indexed
        assert_eq!(d.column, 6);
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(d.code, Some("reportArgumentType".to_string()));
        assert_eq!(d.source, "pyright");
    }

    #[test]
    fn test_empty_output() {
        let result = parse_pyright_output("").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_invalid_json() {
        let result = parse_pyright_output("not json");
        assert!(result.is_err());
    }
}
