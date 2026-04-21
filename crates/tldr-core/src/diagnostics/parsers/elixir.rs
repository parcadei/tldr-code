//! Elixir diagnostic tool output parsers.
//!
//! Supports:
//! - `mix compile`: Elixir compiler with text output
//!   Error format: `** (CompileError) file.ex:line: message`
//!   Warning format: `warning: message\n  file.ex:line`
//! - `credo`: Static analysis tool with JSON output (`--format json`)
//!   JSON has `issues` array with filename, line_no, column, message, category, priority

use crate::diagnostics::{Diagnostic, Severity};
use crate::error::TldrError;
use regex::Regex;
use serde::Deserialize;
use std::path::PathBuf;

// =============================================================================
// mix compile parser (text-based)
// =============================================================================

/// Parse mix compile text output into unified Diagnostic structs.
///
/// mix compile outputs errors and warnings in different formats:
/// - Errors: `** (CompileError) file.ex:line: message`
/// - Warnings: `warning: message\n  file.ex:line`
///
/// We also handle the simpler format:
/// - `file.ex:line: warning: message`
///
/// # Arguments
/// * `output` - The raw text output from `mix compile`
///
/// # Returns
/// A vector of Diagnostic structs. Malformed lines are skipped.
pub fn parse_mix_compile_output(output: &str) -> Result<Vec<Diagnostic>, TldrError> {
    if output.trim().is_empty() {
        return Ok(Vec::new());
    }

    let mut diagnostics = Vec::new();

    // Pattern for CompileError: ** (CompileError) file.ex:line: message
    let error_regex = Regex::new(r"^\*\*\s*\(CompileError\)\s*(.+\.exs?):(\d+):\s*(.+)$")
        .expect("Invalid mix compile error regex");

    // Pattern for inline warnings: file.ex:line: warning: message
    let warning_inline_regex = Regex::new(r"^(.+\.exs?):(\d+):\s*warning:\s*(.+)$")
        .expect("Invalid mix compile warning regex");

    // Pattern for multi-line warnings: "warning: message" then "  file.ex:line"
    let warning_prefix_regex =
        Regex::new(r"^warning:\s*(.+)$").expect("Invalid mix compile warning prefix regex");

    let location_regex =
        Regex::new(r"^\s+(.+\.exs?):(\d+)").expect("Invalid mix compile location regex");

    let lines: Vec<&str> = output.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim();

        // Try CompileError pattern
        if let Some(captures) = error_regex.captures(line) {
            let file = captures.get(1).map(|m| m.as_str()).unwrap_or("");
            let line_num: u32 = captures
                .get(2)
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(1);
            let message = captures
                .get(3)
                .map(|m| m.as_str())
                .unwrap_or("")
                .to_string();

            diagnostics.push(Diagnostic {
                file: PathBuf::from(file),
                line: line_num,
                column: 1,
                end_line: None,
                end_column: None,
                severity: Severity::Error,
                message,
                code: None,
                source: "mix compile".to_string(),
                url: None,
            });
            i += 1;
            continue;
        }

        // Try inline warning pattern
        if let Some(captures) = warning_inline_regex.captures(line) {
            let file = captures.get(1).map(|m| m.as_str()).unwrap_or("");
            let line_num: u32 = captures
                .get(2)
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(1);
            let message = captures
                .get(3)
                .map(|m| m.as_str())
                .unwrap_or("")
                .to_string();

            diagnostics.push(Diagnostic {
                file: PathBuf::from(file),
                line: line_num,
                column: 1,
                end_line: None,
                end_column: None,
                severity: Severity::Warning,
                message,
                code: None,
                source: "mix compile".to_string(),
                url: None,
            });
            i += 1;
            continue;
        }

        // Try multi-line warning pattern: "warning: message" followed by "  file.ex:line"
        if let Some(captures) = warning_prefix_regex.captures(line) {
            let message = captures
                .get(1)
                .map(|m| m.as_str())
                .unwrap_or("")
                .to_string();

            // Look ahead for the location line
            if i + 1 < lines.len() {
                if let Some(loc_captures) = location_regex.captures(lines[i + 1]) {
                    let file = loc_captures.get(1).map(|m| m.as_str()).unwrap_or("");
                    let line_num: u32 = loc_captures
                        .get(2)
                        .and_then(|m| m.as_str().parse().ok())
                        .unwrap_or(1);

                    diagnostics.push(Diagnostic {
                        file: PathBuf::from(file),
                        line: line_num,
                        column: 1,
                        end_line: None,
                        end_column: None,
                        severity: Severity::Warning,
                        message,
                        code: None,
                        source: "mix compile".to_string(),
                        url: None,
                    });
                    i += 2; // Skip both lines
                    continue;
                }
            }
        }

        i += 1;
    }

    Ok(diagnostics)
}

// =============================================================================
// credo parser (JSON-based)
// =============================================================================

/// Credo JSON output root structure
#[derive(Debug, Deserialize)]
struct CredoOutput {
    issues: Vec<CredoIssue>,
}

/// A single Credo issue
#[derive(Debug, Deserialize)]
struct CredoIssue {
    filename: String,
    line_no: u32,
    #[serde(default)]
    column: Option<u32>,
    message: String,
    category: String,
    priority: i32,
}

/// Parse credo JSON output into unified Diagnostic structs.
///
/// Credo outputs JSON via `--format json`:
/// ```json
/// {
///   "issues": [
///     {
///       "filename": "lib/my_app.ex",
///       "line_no": 42,
///       "column": 5,
///       "message": "Modules should have a @moduledoc tag.",
///       "category": "readability",
///       "priority": 10
///     }
///   ]
/// }
/// ```
///
/// Priority mapping:
/// - priority >= 20: Error
/// - priority >= 10: Warning
/// - priority >= 1: Information
/// - otherwise: Hint
///
/// # Arguments
/// * `output` - The raw JSON output from `mix credo --format json`
///
/// # Returns
/// A vector of Diagnostic structs, or an error if JSON parsing fails.
pub fn parse_credo_output(output: &str) -> Result<Vec<Diagnostic>, TldrError> {
    if output.trim().is_empty() {
        return Ok(Vec::new());
    }

    let parsed: CredoOutput = serde_json::from_str(output).map_err(|e| TldrError::ParseError {
        file: PathBuf::from("<credo-output>"),
        line: None,
        message: format!("Failed to parse credo JSON: {}", e),
    })?;

    let diagnostics = parsed
        .issues
        .into_iter()
        .map(|issue| {
            let severity = if issue.priority >= 20 {
                Severity::Error
            } else if issue.priority >= 10 {
                Severity::Warning
            } else if issue.priority >= 1 {
                Severity::Information
            } else {
                Severity::Hint
            };

            Diagnostic {
                file: PathBuf::from(&issue.filename),
                line: issue.line_no,
                column: issue.column.unwrap_or(1),
                end_line: None,
                end_column: None,
                severity,
                message: issue.message,
                code: Some(issue.category),
                source: "credo".to_string(),
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
    // mix compile tests
    // =========================================================================

    #[test]
    fn test_parse_compile_error() {
        let output = "** (CompileError) lib/my_app.ex:10: undefined function foo/0";

        let result = parse_mix_compile_output(output).unwrap();
        assert_eq!(result.len(), 1);

        let d = &result[0];
        assert_eq!(d.file, PathBuf::from("lib/my_app.ex"));
        assert_eq!(d.line, 10);
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(d.message, "undefined function foo/0");
        assert_eq!(d.source, "mix compile");
    }

    #[test]
    fn test_parse_inline_warning() {
        let output = "lib/my_app.ex:25: warning: variable 'x' is unused";

        let result = parse_mix_compile_output(output).unwrap();
        assert_eq!(result.len(), 1);

        let d = &result[0];
        assert_eq!(d.severity, Severity::Warning);
        assert_eq!(d.message, "variable 'x' is unused");
    }

    #[test]
    fn test_parse_multiline_warning() {
        let output = "warning: variable \"x\" is unused\n  lib/my_app.ex:25";

        let result = parse_mix_compile_output(output).unwrap();
        assert_eq!(result.len(), 1);

        let d = &result[0];
        assert_eq!(d.file, PathBuf::from("lib/my_app.ex"));
        assert_eq!(d.line, 25);
        assert_eq!(d.severity, Severity::Warning);
        assert_eq!(d.message, "variable \"x\" is unused");
    }

    #[test]
    fn test_parse_exs_file() {
        let output = "** (CompileError) test/my_app_test.exs:5: undefined function describe/2";

        let result = parse_mix_compile_output(output).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].file, PathBuf::from("test/my_app_test.exs"));
    }

    #[test]
    fn test_parse_multiple_mixed() {
        let output = r#"** (CompileError) lib/app.ex:10: undefined function foo/0
lib/utils.ex:25: warning: unused variable
warning: redefining module MyApp
  lib/app.ex:1"#;

        let result = parse_mix_compile_output(output).unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].severity, Severity::Error);
        assert_eq!(result[1].severity, Severity::Warning);
        assert_eq!(result[2].severity, Severity::Warning);
    }

    #[test]
    fn test_parse_compile_empty() {
        let result = parse_mix_compile_output("").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_compile_malformed() {
        let output = "Compiling 3 files (.ex)\nGenerated my_app app";
        let result = parse_mix_compile_output(output).unwrap();
        assert!(result.is_empty());
    }

    // =========================================================================
    // credo tests
    // =========================================================================

    #[test]
    fn test_parse_credo_simple() {
        let json = r#"{
            "issues": [
                {
                    "filename": "lib/my_app.ex",
                    "line_no": 42,
                    "column": 5,
                    "message": "Modules should have a @moduledoc tag.",
                    "category": "readability",
                    "priority": 10
                }
            ]
        }"#;

        let result = parse_credo_output(json).unwrap();
        assert_eq!(result.len(), 1);

        let d = &result[0];
        assert_eq!(d.file, PathBuf::from("lib/my_app.ex"));
        assert_eq!(d.line, 42);
        assert_eq!(d.column, 5);
        assert_eq!(d.severity, Severity::Warning);
        assert_eq!(d.code, Some("readability".to_string()));
        assert_eq!(d.message, "Modules should have a @moduledoc tag.");
        assert_eq!(d.source, "credo");
    }

    #[test]
    fn test_parse_credo_high_priority() {
        let json = r#"{
            "issues": [
                {
                    "filename": "lib/app.ex",
                    "line_no": 10,
                    "message": "Critical issue",
                    "category": "consistency",
                    "priority": 25
                }
            ]
        }"#;

        let result = parse_credo_output(json).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].severity, Severity::Error);
    }

    #[test]
    fn test_parse_credo_low_priority() {
        let json = r#"{
            "issues": [
                {
                    "filename": "lib/app.ex",
                    "line_no": 10,
                    "message": "Minor issue",
                    "category": "refactor",
                    "priority": 5
                }
            ]
        }"#;

        let result = parse_credo_output(json).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].severity, Severity::Information);
    }

    #[test]
    fn test_parse_credo_multiple() {
        let json = r#"{
            "issues": [
                {
                    "filename": "lib/a.ex",
                    "line_no": 1,
                    "message": "Issue 1",
                    "category": "readability",
                    "priority": 25
                },
                {
                    "filename": "lib/b.ex",
                    "line_no": 2,
                    "message": "Issue 2",
                    "category": "design",
                    "priority": 10
                }
            ]
        }"#;

        let result = parse_credo_output(json).unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_parse_credo_empty() {
        let result = parse_credo_output("").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_credo_empty_issues() {
        let json = r#"{"issues": []}"#;
        let result = parse_credo_output(json).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_credo_invalid_json() {
        let result = parse_credo_output("not json");
        assert!(result.is_err());
    }
}
