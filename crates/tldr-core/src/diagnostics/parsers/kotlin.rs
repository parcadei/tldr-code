//! Kotlin diagnostic tool output parsers.
//!
//! Supports:
//! - `kotlinc`: Kotlin compiler with GCC-like text output
//!   Format: `file.kt:line:col: error: message`
//! - `detekt`: Static analysis tool with text output
//!   Format: `file.kt:line:col - [RuleName] message`

use crate::diagnostics::{Diagnostic, Severity};
use crate::error::TldrError;
use regex::Regex;
use std::path::PathBuf;

/// Parse kotlinc text output into unified Diagnostic structs.
///
/// kotlinc outputs errors in GCC-like format:
/// `file.kt:line:col: severity: message`
///
/// # Arguments
/// * `output` - The raw text output from `kotlinc`
///
/// # Returns
/// A vector of Diagnostic structs. Malformed lines are skipped.
pub fn parse_kotlinc_output(output: &str) -> Result<Vec<Diagnostic>, TldrError> {
    if output.trim().is_empty() {
        return Ok(Vec::new());
    }

    // Pattern: file.kt:line:col: severity: message
    let regex = Regex::new(r"^(.+\.kts?):(\d+):(\d+):\s*(error|warning|info):\s*(.+)$")
        .expect("Invalid kotlinc regex pattern");

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
                "info" => Severity::Information,
                _ => Severity::Warning,
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
                source: "kotlinc".to_string(),
                url: None,
            });
        }
    }

    Ok(diagnostics)
}

/// Parse detekt text output into unified Diagnostic structs.
///
/// detekt outputs issues in the format:
/// `file.kt:line:col - [RuleName] message`
///
/// # Arguments
/// * `output` - The raw text output from `detekt-cli --report txt:stdout`
///
/// # Returns
/// A vector of Diagnostic structs. Malformed lines are skipped.
pub fn parse_detekt_output(output: &str) -> Result<Vec<Diagnostic>, TldrError> {
    if output.trim().is_empty() {
        return Ok(Vec::new());
    }

    // Pattern: file.kt:line:col - [RuleName] message
    let regex = Regex::new(r"^(.+\.kts?):(\d+):(\d+)\s*-\s*\[([^\]]+)\]\s*(.+)$")
        .expect("Invalid detekt regex pattern");

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
            let rule = captures.get(4).map(|m| m.as_str().to_string());
            let message = captures
                .get(5)
                .map(|m| m.as_str())
                .unwrap_or("")
                .to_string();

            diagnostics.push(Diagnostic {
                file: PathBuf::from(file),
                line: line_num,
                column,
                end_line: None,
                end_column: None,
                severity: Severity::Warning, // detekt issues are warnings
                message,
                code: rule,
                source: "detekt".to_string(),
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
    // kotlinc tests
    // =========================================================================

    #[test]
    fn test_parse_kotlinc_error() {
        let output = "src/Main.kt:10:5: error: unresolved reference: foo";

        let result = parse_kotlinc_output(output).unwrap();
        assert_eq!(result.len(), 1);

        let d = &result[0];
        assert_eq!(d.file, PathBuf::from("src/Main.kt"));
        assert_eq!(d.line, 10);
        assert_eq!(d.column, 5);
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(d.message, "unresolved reference: foo");
        assert_eq!(d.source, "kotlinc");
        assert!(d.code.is_none());
    }

    #[test]
    fn test_parse_kotlinc_warning() {
        let output = "src/Utils.kt:25:1: warning: parameter 'x' is never used";

        let result = parse_kotlinc_output(output).unwrap();
        assert_eq!(result.len(), 1);

        let d = &result[0];
        assert_eq!(d.severity, Severity::Warning);
        assert_eq!(d.message, "parameter 'x' is never used");
    }

    #[test]
    fn test_parse_kotlinc_multiple() {
        let output = r#"src/Main.kt:10:5: error: unresolved reference: foo
src/Main.kt:15:10: warning: unused variable 'bar'
src/Utils.kt:3:1: error: expecting member declaration"#;

        let result = parse_kotlinc_output(output).unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].severity, Severity::Error);
        assert_eq!(result[1].severity, Severity::Warning);
        assert_eq!(result[2].severity, Severity::Error);
    }

    #[test]
    fn test_parse_kotlinc_kts_file() {
        let output = "build.gradle.kts:42:8: error: unresolved reference: implementation";

        let result = parse_kotlinc_output(output).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(d_file_str(&result[0]), "build.gradle.kts");
    }

    #[test]
    fn test_parse_kotlinc_empty() {
        let result = parse_kotlinc_output("").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_kotlinc_malformed() {
        let output = "some random text that is not a diagnostic";
        let result = parse_kotlinc_output(output).unwrap();
        assert!(result.is_empty());
    }

    // =========================================================================
    // detekt tests
    // =========================================================================

    #[test]
    fn test_parse_detekt_simple() {
        let output = "src/Main.kt:10:5 - [MagicNumber] This expression contains a magic number.";

        let result = parse_detekt_output(output).unwrap();
        assert_eq!(result.len(), 1);

        let d = &result[0];
        assert_eq!(d.file, PathBuf::from("src/Main.kt"));
        assert_eq!(d.line, 10);
        assert_eq!(d.column, 5);
        assert_eq!(d.severity, Severity::Warning);
        assert_eq!(d.code, Some("MagicNumber".to_string()));
        assert_eq!(d.message, "This expression contains a magic number.");
        assert_eq!(d.source, "detekt");
    }

    #[test]
    fn test_parse_detekt_multiple() {
        let output = r#"src/Main.kt:10:5 - [MagicNumber] Magic number found.
src/Utils.kt:20:1 - [LongMethod] This method is too long.
src/Config.kt:5:3 - [TooManyFunctions] Too many functions in file."#;

        let result = parse_detekt_output(output).unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].code, Some("MagicNumber".to_string()));
        assert_eq!(result[1].code, Some("LongMethod".to_string()));
        assert_eq!(result[2].code, Some("TooManyFunctions".to_string()));
    }

    #[test]
    fn test_parse_detekt_empty() {
        let result = parse_detekt_output("").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_detekt_malformed() {
        let output = "some random text without detekt format";
        let result = parse_detekt_output(output).unwrap();
        assert!(result.is_empty());
    }

    // Helper to get file as string
    fn d_file_str(d: &Diagnostic) -> String {
        d.file.display().to_string()
    }
}
