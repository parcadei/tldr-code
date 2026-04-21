//! PHP diagnostic tool output parsers.
//!
//! Supports:
//! - `php -l`: PHP syntax checker with text output
//!   Format: `PHP Parse error: ... in file.php on line N`
//!   Format: `PHP Fatal error: ... in file.php on line N`
//!   Success: `No syntax errors detected in file.php`
//! - `phpstan`: Static analysis tool with JSON output
//!   Uses `--error-format=json` for structured output.

use crate::diagnostics::{Diagnostic, Severity};
use crate::error::TldrError;
use regex::Regex;
use serde::Deserialize;
use std::path::PathBuf;

/// Parse `php -l` text output into unified Diagnostic structs.
///
/// php -l outputs errors in the format:
/// `PHP Parse error: syntax error, unexpected ... in file.php on line N`
/// `PHP Fatal error: ... in file.php on line N`
///
/// On success, it outputs:
/// `No syntax errors detected in file.php`
///
/// # Arguments
/// * `output` - The raw text output from `php -l`
///
/// # Returns
/// A vector of Diagnostic structs. Non-error lines are skipped.
pub fn parse_php_lint_output(output: &str) -> Result<Vec<Diagnostic>, TldrError> {
    if output.trim().is_empty() {
        return Ok(Vec::new());
    }

    // Pattern: PHP (Parse|Fatal) error: message in file.php on line N
    let regex = Regex::new(
        r"^PHP\s+(Parse error|Fatal error|Warning|Notice|Deprecated):\s*(.+?)\s+in\s+(.+?)\s+on\s+line\s+(\d+)"
    ).expect("Invalid php -l regex pattern");

    let mut diagnostics = Vec::new();

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("No syntax errors") {
            continue;
        }

        if let Some(captures) = regex.captures(line) {
            let error_type = captures.get(1).map(|m| m.as_str()).unwrap_or("Parse error");
            let message = captures
                .get(2)
                .map(|m| m.as_str())
                .unwrap_or("")
                .to_string();
            let file = captures.get(3).map(|m| m.as_str()).unwrap_or("");
            let line_num: u32 = captures
                .get(4)
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(1);

            let severity = match error_type {
                "Parse error" | "Fatal error" => Severity::Error,
                "Warning" => Severity::Warning,
                "Notice" | "Deprecated" => Severity::Information,
                _ => Severity::Error,
            };

            diagnostics.push(Diagnostic {
                file: PathBuf::from(file),
                line: line_num,
                column: 1, // php -l doesn't provide column numbers
                end_line: None,
                end_column: None,
                severity,
                message,
                code: None,
                source: "php".to_string(),
                url: None,
            });
        }
    }

    Ok(diagnostics)
}

/// PHPStan JSON output structure
#[derive(Debug, Deserialize)]
struct PhpstanOutput {
    #[serde(rename = "totals")]
    _totals: Option<PhpstanTotals>,
    files: std::collections::HashMap<String, PhpstanFile>,
}

/// PHPStan totals section
#[derive(Debug, Deserialize)]
struct PhpstanTotals {
    #[allow(dead_code)]
    errors: u32,
    #[allow(dead_code)]
    file_errors: u32,
}

/// PHPStan file entry with messages
#[derive(Debug, Deserialize)]
struct PhpstanFile {
    #[serde(rename = "errors")]
    _errors: u32,
    messages: Vec<PhpstanMessage>,
}

/// Individual PHPStan error message
#[derive(Debug, Deserialize)]
struct PhpstanMessage {
    message: String,
    line: Option<u32>,
    #[serde(default, rename = "ignorable")]
    _ignorable: bool,
}

/// Parse phpstan JSON output into unified Diagnostic structs.
///
/// PHPStan JSON format (via `--error-format=json`):
/// ```json
/// {
///   "totals": {"errors": 0, "file_errors": 2},
///   "files": {
///     "src/Controller.php": {
///       "errors": 2,
///       "messages": [
///         {"message": "Parameter $id has no type.", "line": 15, "ignorable": true},
///         {"message": "Method foo() has no return type.", "line": 20, "ignorable": true}
///       ]
///     }
///   }
/// }
/// ```
///
/// # Arguments
/// * `output` - The raw JSON output from `phpstan analyse --error-format=json --no-progress`
///
/// # Returns
/// A vector of Diagnostic structs, or an error if JSON parsing fails.
pub fn parse_phpstan_output(output: &str) -> Result<Vec<Diagnostic>, TldrError> {
    if output.trim().is_empty() {
        return Ok(Vec::new());
    }

    let parsed: PhpstanOutput =
        serde_json::from_str(output).map_err(|e| TldrError::ParseError {
            file: PathBuf::from("<phpstan-output>"),
            line: None,
            message: format!("Failed to parse phpstan JSON: {}", e),
        })?;

    let mut diagnostics = Vec::new();

    for (file_path, file_data) in &parsed.files {
        for msg in &file_data.messages {
            diagnostics.push(Diagnostic {
                file: PathBuf::from(file_path),
                line: msg.line.unwrap_or(1),
                column: 1, // phpstan doesn't provide column numbers
                end_line: None,
                end_column: None,
                severity: Severity::Error, // phpstan reports are errors by default
                message: msg.message.clone(),
                code: None,
                source: "phpstan".to_string(),
                url: None,
            });
        }
    }

    Ok(diagnostics)
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // php -l tests
    // =========================================================================

    #[test]
    fn test_parse_php_lint_parse_error() {
        let output =
            "PHP Parse error: syntax error, unexpected '}' in src/Controller.php on line 42";

        let result = parse_php_lint_output(output).unwrap();
        assert_eq!(result.len(), 1);

        let d = &result[0];
        assert_eq!(d.file, PathBuf::from("src/Controller.php"));
        assert_eq!(d.line, 42);
        assert_eq!(d.column, 1); // php -l doesn't provide columns
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(d.message, "syntax error, unexpected '}'");
        assert_eq!(d.source, "php");
    }

    #[test]
    fn test_parse_php_lint_fatal_error() {
        let output = "PHP Fatal error: Cannot redeclare function foo() in src/utils.php on line 10";

        let result = parse_php_lint_output(output).unwrap();
        assert_eq!(result.len(), 1);

        let d = &result[0];
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(d.message, "Cannot redeclare function foo()");
    }

    #[test]
    fn test_parse_php_lint_warning() {
        let output = "PHP Warning: Use of undefined constant FOO in src/config.php on line 5";

        let result = parse_php_lint_output(output).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].severity, Severity::Warning);
    }

    #[test]
    fn test_parse_php_lint_no_errors() {
        let output = "No syntax errors detected in src/Controller.php";

        let result = parse_php_lint_output(output).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_php_lint_empty() {
        let result = parse_php_lint_output("").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_php_lint_malformed() {
        let output = r#"Some random output
that doesn't match the pattern
at all"#;

        let result = parse_php_lint_output(output).unwrap();
        assert!(result.is_empty());
    }

    // =========================================================================
    // phpstan tests
    // =========================================================================

    #[test]
    fn test_parse_phpstan_simple() {
        let json = r#"{
            "totals": {"errors": 0, "file_errors": 1},
            "files": {
                "src/Controller.php": {
                    "errors": 1,
                    "messages": [
                        {
                            "message": "Parameter $id of method Controller::show() has no type specified.",
                            "line": 15,
                            "ignorable": true
                        }
                    ]
                }
            }
        }"#;

        let result = parse_phpstan_output(json).unwrap();
        assert_eq!(result.len(), 1);

        let d = &result[0];
        assert_eq!(d.file, PathBuf::from("src/Controller.php"));
        assert_eq!(d.line, 15);
        assert_eq!(d.severity, Severity::Error);
        assert!(d.message.contains("Parameter $id"));
        assert_eq!(d.source, "phpstan");
    }

    #[test]
    fn test_parse_phpstan_multiple_files() {
        let json = r#"{
            "totals": {"errors": 0, "file_errors": 3},
            "files": {
                "src/Controller.php": {
                    "errors": 2,
                    "messages": [
                        {"message": "Error 1", "line": 10, "ignorable": true},
                        {"message": "Error 2", "line": 20, "ignorable": true}
                    ]
                },
                "src/Model.php": {
                    "errors": 1,
                    "messages": [
                        {"message": "Error 3", "line": 5, "ignorable": false}
                    ]
                }
            }
        }"#;

        let result = parse_phpstan_output(json).unwrap();
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_parse_phpstan_no_line() {
        let json = r#"{
            "totals": {"errors": 0, "file_errors": 1},
            "files": {
                "src/app.php": {
                    "errors": 1,
                    "messages": [
                        {"message": "General error without line", "line": null, "ignorable": false}
                    ]
                }
            }
        }"#;

        let result = parse_phpstan_output(json).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].line, 1); // defaults to 1
    }

    #[test]
    fn test_parse_phpstan_empty_output() {
        let result = parse_phpstan_output("").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_phpstan_no_errors() {
        let json = r#"{"totals": {"errors": 0, "file_errors": 0}, "files": {}}"#;

        let result = parse_phpstan_output(json).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_phpstan_invalid_json() {
        let result = parse_phpstan_output("not json");
        assert!(result.is_err());
    }
}
