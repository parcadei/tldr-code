//! Clang/GCC diagnostic tool output parsers.
//!
//! Supports:
//! - `clang`/`gcc`: C/C++ compilers with GCC-style text output
//!   Format: `file.c:line:col: warning: message [-Wflag]`
//! - `clang-tidy`: Static analysis tool with same GCC-style output
//!   Format: `file.c:line:col: warning: message [check-name]`
//!
//! Both clang and clang-tidy use the same output format, so a single
//! parser handles both. The source field differentiates them.

use crate::diagnostics::{Diagnostic, Severity};
use crate::error::TldrError;
use regex::Regex;
use std::path::PathBuf;

/// Parse GCC/clang-style text output into unified Diagnostic structs.
///
/// This handles output from clang, gcc, and clang-tidy, which all share
/// the same format:
/// `file.c:line:col: severity: message [-Wflag]`
/// or
/// `file.c:line:col: severity: message [check-name]`
///
/// # Arguments
/// * `output` - The raw text output from clang/gcc/clang-tidy
/// * `source` - The tool name to use in the Diagnostic source field
///
/// # Returns
/// A vector of Diagnostic structs. Malformed lines are skipped.
pub fn parse_clang_output(output: &str, source: &str) -> Result<Vec<Diagnostic>, TldrError> {
    if output.trim().is_empty() {
        return Ok(Vec::new());
    }

    // Pattern: file:line:col: severity: message [-Wflag] or [check-name]
    // The bracket part at the end is optional
    let regex = Regex::new(
        r"^(.+):(\d+):(\d+):\s*(error|warning|note|fatal error):\s*(.+?)(?:\s+\[([^\]]+)\])?\s*$",
    )
    .expect("Invalid clang regex pattern");

    let mut diagnostics = Vec::new();
    let source = source.to_string();

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
            let flag = captures.get(6).map(|m| m.as_str().to_string());

            let severity = match severity_str {
                "error" | "fatal error" => Severity::Error,
                "warning" => Severity::Warning,
                "note" => Severity::Information,
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
                code: flag,
                source: source.clone(),
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
    // clang/gcc tests
    // =========================================================================

    #[test]
    fn test_parse_clang_error() {
        let output = "src/main.c:42:10: error: use of undeclared identifier 'foo'";

        let result = parse_clang_output(output, "clang").unwrap();
        assert_eq!(result.len(), 1);

        let d = &result[0];
        assert_eq!(d.file, PathBuf::from("src/main.c"));
        assert_eq!(d.line, 42);
        assert_eq!(d.column, 10);
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(d.message, "use of undeclared identifier 'foo'");
        assert_eq!(d.source, "clang");
        assert!(d.code.is_none());
    }

    #[test]
    fn test_parse_clang_warning_with_flag() {
        let output = "src/main.c:15:5: warning: unused variable 'x' [-Wunused-variable]";

        let result = parse_clang_output(output, "clang").unwrap();
        assert_eq!(result.len(), 1);

        let d = &result[0];
        assert_eq!(d.severity, Severity::Warning);
        assert_eq!(d.message, "unused variable 'x'");
        assert_eq!(d.code, Some("-Wunused-variable".to_string()));
    }

    #[test]
    fn test_parse_clang_fatal_error() {
        let output = "src/main.c:1:10: fatal error: 'missing.h' file not found";

        let result = parse_clang_output(output, "clang").unwrap();
        assert_eq!(result.len(), 1);

        let d = &result[0];
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(d.message, "'missing.h' file not found");
    }

    #[test]
    fn test_parse_clang_note() {
        let output = "src/main.c:10:5: note: previous definition is here";

        let result = parse_clang_output(output, "clang").unwrap();
        assert_eq!(result.len(), 1);

        let d = &result[0];
        assert_eq!(d.severity, Severity::Information);
    }

    #[test]
    fn test_parse_clang_multiple() {
        let output = r#"src/main.c:10:5: error: use of undeclared identifier 'foo'
src/main.c:15:10: warning: comparison of integers of different signs [-Wsign-compare]
src/utils.c:3:1: error: expected ';' after top level declarator"#;

        let result = parse_clang_output(output, "clang").unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].severity, Severity::Error);
        assert_eq!(result[1].severity, Severity::Warning);
        assert_eq!(result[1].code, Some("-Wsign-compare".to_string()));
        assert_eq!(result[2].severity, Severity::Error);
    }

    #[test]
    fn test_parse_clang_cpp_file() {
        let output = "src/main.cpp:42:10: error: no matching function for call to 'foo'";

        let result = parse_clang_output(output, "clang").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].file, PathBuf::from("src/main.cpp"));
    }

    #[test]
    fn test_parse_clang_empty() {
        let result = parse_clang_output("", "clang").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_clang_malformed() {
        // Lines that don't match should be skipped
        let output = r#"In file included from src/main.c:1:
/usr/include/stdio.h:33:11: note: previous declaration is here
1 error generated."#;

        let result = parse_clang_output(output, "clang").unwrap();
        // Only the note line matches the pattern
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].severity, Severity::Information);
    }

    // =========================================================================
    // clang-tidy tests
    // =========================================================================

    #[test]
    fn test_parse_clang_tidy() {
        let output = "src/main.c:42:5: warning: use of 'malloc' [-cppcoreguidelines-no-malloc]";

        let result = parse_clang_output(output, "clang-tidy").unwrap();
        assert_eq!(result.len(), 1);

        let d = &result[0];
        assert_eq!(d.source, "clang-tidy");
        assert_eq!(d.severity, Severity::Warning);
        assert_eq!(d.code, Some("-cppcoreguidelines-no-malloc".to_string()));
    }

    #[test]
    fn test_parse_clang_tidy_multiple_checks() {
        let output = r#"src/main.c:10:5: warning: do not use 'else' after 'return' [readability-else-after-return]
src/main.c:20:10: warning: function 'foo' is too complex [readability-function-cognitive-complexity]
src/main.c:30:1: error: no matching function for call to 'bar'"#;

        let result = parse_clang_output(output, "clang-tidy").unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(
            result[0].code,
            Some("readability-else-after-return".to_string())
        );
        assert_eq!(
            result[1].code,
            Some("readability-function-cognitive-complexity".to_string())
        );
        assert_eq!(result[2].severity, Severity::Error);
        assert!(result[2].code.is_none());
    }

    #[test]
    fn test_parse_clang_tidy_empty() {
        let result = parse_clang_output("", "clang-tidy").unwrap();
        assert!(result.is_empty());
    }
}
