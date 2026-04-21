//! Java diagnostic tool output parsers.
//!
//! Supports:
//! - `javac`: Java compiler with text output
//!   Format: `file.java:line: error: message`
//! - `checkstyle`: Style checker with plain text output
//!   Format: `[WARN] file.java:line:col: message [CheckName]`

use crate::diagnostics::{Diagnostic, Severity};
use crate::error::TldrError;
use regex::Regex;
use std::path::PathBuf;

/// Parse javac text output into unified Diagnostic structs.
///
/// javac outputs errors in the format:
/// `file.java:line: error: message`
///
/// Note: javac does not include column numbers in its default output.
///
/// # Arguments
/// * `output` - The raw text output from `javac -Xlint:all`
///
/// # Returns
/// A vector of Diagnostic structs. Malformed lines are skipped.
pub fn parse_javac_output(output: &str) -> Result<Vec<Diagnostic>, TldrError> {
    if output.trim().is_empty() {
        return Ok(Vec::new());
    }

    // Pattern: file.java:line: severity: message
    // javac format: "File.java:42: error: ';' expected"
    let regex = Regex::new(r"^(.+\.java):(\d+):\s*(error|warning):\s*(.+)$")
        .expect("Invalid javac regex pattern");

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
            let severity_str = captures.get(3).map(|m| m.as_str()).unwrap_or("error");
            let message = captures
                .get(4)
                .map(|m| m.as_str())
                .unwrap_or("")
                .to_string();

            let severity = match severity_str {
                "error" => Severity::Error,
                "warning" => Severity::Warning,
                _ => Severity::Error,
            };

            diagnostics.push(Diagnostic {
                file: PathBuf::from(file),
                line: line_num,
                column: 1, // javac doesn't provide column numbers
                end_line: None,
                end_column: None,
                severity,
                message,
                code: None, // javac doesn't provide error codes
                source: "javac".to_string(),
                url: None,
            });
        }
    }

    Ok(diagnostics)
}

/// Parse checkstyle plain text output into unified Diagnostic structs.
///
/// checkstyle plain format:
/// `[WARN] file.java:line:col: message [CheckName]`
/// or without column:
/// `[WARN] file.java:line: message [CheckName]`
///
/// # Arguments
/// * `output` - The raw text output from `checkstyle -f plain`
///
/// # Returns
/// A vector of Diagnostic structs. Malformed lines are skipped.
pub fn parse_checkstyle_output(output: &str) -> Result<Vec<Diagnostic>, TldrError> {
    if output.trim().is_empty() {
        return Ok(Vec::new());
    }

    // Pattern: [SEVERITY] file.java:line:col: message [CheckName]
    // The col part is optional, and the CheckName at the end is optional
    let regex = Regex::new(
        r"^\[(WARN|ERROR|INFO)\]\s+(.+\.java):(\d+)(?::(\d+))?:\s*(.+?)(?:\s+\[([^\]]+)\])?\s*$",
    )
    .expect("Invalid checkstyle regex pattern");

    let mut diagnostics = Vec::new();

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if let Some(captures) = regex.captures(line) {
            let severity_str = captures.get(1).map(|m| m.as_str()).unwrap_or("WARN");
            let file = captures.get(2).map(|m| m.as_str()).unwrap_or("");
            let line_num: u32 = captures
                .get(3)
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(1);
            let column: u32 = captures
                .get(4)
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(1);
            let message = captures
                .get(5)
                .map(|m| m.as_str())
                .unwrap_or("")
                .to_string();
            let check_name = captures.get(6).map(|m| m.as_str().to_string());

            let severity = match severity_str {
                "ERROR" => Severity::Error,
                "WARN" => Severity::Warning,
                "INFO" => Severity::Information,
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
                code: check_name,
                source: "checkstyle".to_string(),
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
    // javac tests
    // =========================================================================

    #[test]
    fn test_parse_javac_error() {
        let output = "src/Main.java:42: error: ';' expected";

        let result = parse_javac_output(output).unwrap();
        assert_eq!(result.len(), 1);

        let d = &result[0];
        assert_eq!(d.file, PathBuf::from("src/Main.java"));
        assert_eq!(d.line, 42);
        assert_eq!(d.column, 1); // javac doesn't provide column
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(d.message, "';' expected");
        assert_eq!(d.source, "javac");
        assert!(d.code.is_none());
    }

    #[test]
    fn test_parse_javac_warning() {
        let output = "src/Utils.java:15: warning: [unchecked] unchecked call to add(E)";

        let result = parse_javac_output(output).unwrap();
        assert_eq!(result.len(), 1);

        let d = &result[0];
        assert_eq!(d.severity, Severity::Warning);
        assert_eq!(d.message, "[unchecked] unchecked call to add(E)");
    }

    #[test]
    fn test_parse_javac_multiple() {
        let output = r#"src/Main.java:10: error: cannot find symbol
src/Main.java:15: warning: [deprecation] foo() in Bar has been deprecated
src/Utils.java:3: error: incompatible types"#;

        let result = parse_javac_output(output).unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].severity, Severity::Error);
        assert_eq!(result[1].severity, Severity::Warning);
        assert_eq!(result[2].severity, Severity::Error);
    }

    #[test]
    fn test_parse_javac_empty() {
        let result = parse_javac_output("").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_javac_malformed() {
        // Lines that don't match should be skipped, not cause errors
        let output = r#"Note: Some input files use unchecked or unsafe operations.
Note: Recompile with -Xlint:unchecked for details.
1 error"#;

        let result = parse_javac_output(output).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_javac_with_caret_lines() {
        // javac output often includes caret lines showing error position
        let output = r#"src/Main.java:42: error: ';' expected
        System.out.println("hello")
                                   ^
1 error"#;

        let result = parse_javac_output(output).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].line, 42);
    }

    // =========================================================================
    // checkstyle tests
    // =========================================================================

    #[test]
    fn test_parse_checkstyle_warning() {
        let output =
            "[WARN] src/Main.java:42:10: Missing a Javadoc comment. [MissingJavadocMethod]";

        let result = parse_checkstyle_output(output).unwrap();
        assert_eq!(result.len(), 1);

        let d = &result[0];
        assert_eq!(d.file, PathBuf::from("src/Main.java"));
        assert_eq!(d.line, 42);
        assert_eq!(d.column, 10);
        assert_eq!(d.severity, Severity::Warning);
        assert_eq!(d.message, "Missing a Javadoc comment.");
        assert_eq!(d.code, Some("MissingJavadocMethod".to_string()));
        assert_eq!(d.source, "checkstyle");
    }

    #[test]
    fn test_parse_checkstyle_error() {
        let output = "[ERROR] src/Main.java:10:5: '{' at column 5 should be on the previous line. [LeftCurly]";

        let result = parse_checkstyle_output(output).unwrap();
        assert_eq!(result.len(), 1);

        let d = &result[0];
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(d.code, Some("LeftCurly".to_string()));
    }

    #[test]
    fn test_parse_checkstyle_no_column() {
        let output = "[WARN] src/Main.java:42: Line is longer than 100 characters. [LineLength]";

        let result = parse_checkstyle_output(output).unwrap();
        assert_eq!(result.len(), 1);

        let d = &result[0];
        assert_eq!(d.line, 42);
        assert_eq!(d.column, 1); // default when no column
    }

    #[test]
    fn test_parse_checkstyle_multiple() {
        let output = r#"[WARN] src/Main.java:10:5: Missing Javadoc. [MissingJavadocMethod]
[ERROR] src/Main.java:20:1: Unused import. [UnusedImports]
[WARN] src/Utils.java:5:3: Magic number. [MagicNumber]"#;

        let result = parse_checkstyle_output(output).unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].severity, Severity::Warning);
        assert_eq!(result[1].severity, Severity::Error);
        assert_eq!(result[2].file, PathBuf::from("src/Utils.java"));
    }

    #[test]
    fn test_parse_checkstyle_empty() {
        let result = parse_checkstyle_output("").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_checkstyle_malformed() {
        let output = r#"Starting audit...
Audit done.
Checkstyle ends with 0 errors."#;

        let result = parse_checkstyle_output(output).unwrap();
        assert!(result.is_empty());
    }
}
