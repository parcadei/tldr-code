//! Ruby diagnostic tool output parser.
//!
//! Supports:
//! - `rubocop`: Ruby linter/formatter with JSON output
//!   Uses `--format json` which outputs structured JSON with files and offenses.

use crate::diagnostics::{Diagnostic, Severity};
use crate::error::TldrError;
use serde::Deserialize;
use std::path::PathBuf;

/// Top-level RuboCop JSON output structure
#[derive(Debug, Deserialize)]
struct RubocopOutput {
    files: Vec<RubocopFile>,
}

/// A file entry in RuboCop JSON output
#[derive(Debug, Deserialize)]
struct RubocopFile {
    path: String,
    offenses: Vec<RubocopOffense>,
}

/// An individual offense/violation in RuboCop output
#[derive(Debug, Deserialize)]
struct RubocopOffense {
    severity: String,
    message: String,
    cop_name: String,
    location: RubocopLocation,
}

/// Location information for a RuboCop offense
#[derive(Debug, Deserialize)]
struct RubocopLocation {
    start_line: u32,
    start_column: u32,
    last_line: Option<u32>,
    last_column: Option<u32>,
}

/// Parse rubocop JSON output into unified Diagnostic structs.
///
/// RuboCop JSON format (via `--format json`):
/// ```json
/// {
///   "files": [
///     {
///       "path": "src/app.rb",
///       "offenses": [
///         {
///           "severity": "convention",
///           "message": "Line is too long.",
///           "cop_name": "Layout/LineLength",
///           "location": {
///             "start_line": 10,
///             "start_column": 1,
///             "last_line": 10,
///             "last_column": 120
///           }
///         }
///       ]
///     }
///   ]
/// }
/// ```
///
/// # Severity Mapping
/// - `fatal`, `error` -> Error
/// - `warning` -> Warning
/// - `convention`, `refactor` -> Information
///
/// # Arguments
/// * `output` - The raw JSON output from `rubocop --format json`
///
/// # Returns
/// A vector of Diagnostic structs, or an error if JSON parsing fails.
pub fn parse_rubocop_output(output: &str) -> Result<Vec<Diagnostic>, TldrError> {
    if output.trim().is_empty() {
        return Ok(Vec::new());
    }

    let parsed: RubocopOutput =
        serde_json::from_str(output).map_err(|e| TldrError::ParseError {
            file: PathBuf::from("<rubocop-output>"),
            line: None,
            message: format!("Failed to parse rubocop JSON: {}", e),
        })?;

    let mut diagnostics = Vec::new();

    for file in parsed.files {
        for offense in file.offenses {
            let severity = match offense.severity.as_str() {
                "fatal" | "error" => Severity::Error,
                "warning" => Severity::Warning,
                "convention" | "refactor" => Severity::Information,
                _ => Severity::Warning,
            };

            diagnostics.push(Diagnostic {
                file: PathBuf::from(&file.path),
                line: offense.location.start_line,
                column: offense.location.start_column,
                end_line: offense.location.last_line,
                end_column: offense.location.last_column,
                severity,
                message: offense.message,
                code: Some(offense.cop_name),
                source: "rubocop".to_string(),
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
    fn test_parse_rubocop_simple() {
        let json = r#"{
            "files": [
                {
                    "path": "src/app.rb",
                    "offenses": [
                        {
                            "severity": "convention",
                            "message": "Line is too long. [120/100]",
                            "cop_name": "Layout/LineLength",
                            "location": {
                                "start_line": 10,
                                "start_column": 1,
                                "last_line": 10,
                                "last_column": 120
                            }
                        }
                    ]
                }
            ]
        }"#;

        let result = parse_rubocop_output(json).unwrap();
        assert_eq!(result.len(), 1);

        let d = &result[0];
        assert_eq!(d.file, PathBuf::from("src/app.rb"));
        assert_eq!(d.line, 10);
        assert_eq!(d.column, 1);
        assert_eq!(d.end_line, Some(10));
        assert_eq!(d.end_column, Some(120));
        assert_eq!(d.severity, Severity::Information); // convention -> Information
        assert_eq!(d.message, "Line is too long. [120/100]");
        assert_eq!(d.code, Some("Layout/LineLength".to_string()));
        assert_eq!(d.source, "rubocop");
    }

    #[test]
    fn test_parse_rubocop_multiple_files() {
        let json = r#"{
            "files": [
                {
                    "path": "src/app.rb",
                    "offenses": [
                        {
                            "severity": "warning",
                            "message": "Useless assignment.",
                            "cop_name": "Lint/UselessAssignment",
                            "location": {
                                "start_line": 5,
                                "start_column": 3,
                                "last_line": null,
                                "last_column": null
                            }
                        }
                    ]
                },
                {
                    "path": "src/utils.rb",
                    "offenses": [
                        {
                            "severity": "error",
                            "message": "Syntax error, unexpected end-of-input.",
                            "cop_name": "Lint/Syntax",
                            "location": {
                                "start_line": 20,
                                "start_column": 1,
                                "last_line": null,
                                "last_column": null
                            }
                        }
                    ]
                }
            ]
        }"#;

        let result = parse_rubocop_output(json).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].file, PathBuf::from("src/app.rb"));
        assert_eq!(result[0].severity, Severity::Warning);
        assert_eq!(result[1].file, PathBuf::from("src/utils.rb"));
        assert_eq!(result[1].severity, Severity::Error);
    }

    #[test]
    fn test_parse_rubocop_severity_mapping() {
        let json = r#"{
            "files": [
                {
                    "path": "test.rb",
                    "offenses": [
                        {
                            "severity": "fatal",
                            "message": "Fatal error",
                            "cop_name": "Fatal",
                            "location": { "start_line": 1, "start_column": 1, "last_line": null, "last_column": null }
                        },
                        {
                            "severity": "error",
                            "message": "Error",
                            "cop_name": "Error",
                            "location": { "start_line": 2, "start_column": 1, "last_line": null, "last_column": null }
                        },
                        {
                            "severity": "warning",
                            "message": "Warning",
                            "cop_name": "Warning",
                            "location": { "start_line": 3, "start_column": 1, "last_line": null, "last_column": null }
                        },
                        {
                            "severity": "convention",
                            "message": "Convention",
                            "cop_name": "Convention",
                            "location": { "start_line": 4, "start_column": 1, "last_line": null, "last_column": null }
                        },
                        {
                            "severity": "refactor",
                            "message": "Refactor",
                            "cop_name": "Refactor",
                            "location": { "start_line": 5, "start_column": 1, "last_line": null, "last_column": null }
                        }
                    ]
                }
            ]
        }"#;

        let result = parse_rubocop_output(json).unwrap();
        assert_eq!(result.len(), 5);
        assert_eq!(result[0].severity, Severity::Error); // fatal
        assert_eq!(result[1].severity, Severity::Error); // error
        assert_eq!(result[2].severity, Severity::Warning); // warning
        assert_eq!(result[3].severity, Severity::Information); // convention
        assert_eq!(result[4].severity, Severity::Information); // refactor
    }

    #[test]
    fn test_parse_rubocop_empty_output() {
        let result = parse_rubocop_output("").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_rubocop_no_offenses() {
        let json = r#"{"files": [{"path": "src/clean.rb", "offenses": []}]}"#;

        let result = parse_rubocop_output(json).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_rubocop_invalid_json() {
        let result = parse_rubocop_output("not json at all");
        assert!(result.is_err());
    }
}
