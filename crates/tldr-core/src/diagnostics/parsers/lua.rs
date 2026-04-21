//! Luacheck diagnostic output parser.
//!
//! Parses luacheck plain text output format:
//! ```text
//! file.lua:line:col: (W611) line is too long
//! ```
//!
//! Error codes follow the pattern:
//! - W### for warnings
//! - E### for errors
//!
//! Use `luacheck --formatter plain --no-color` for consistent output.

use crate::diagnostics::{Diagnostic, Severity};
use crate::error::TldrError;
use regex::Regex;
use std::path::PathBuf;

/// Parse luacheck plain text output into unified Diagnostic structs.
///
/// luacheck outputs issues in the format:
/// `file.lua:line:col: (CODE) message`
///
/// Where CODE is:
/// - `W###` for warnings
/// - `E###` for errors
///
/// # Arguments
/// * `output` - The raw text output from `luacheck --formatter plain --no-color`
///
/// # Returns
/// A vector of Diagnostic structs. Malformed lines are skipped.
pub fn parse_luacheck_output(output: &str) -> Result<Vec<Diagnostic>, TldrError> {
    if output.trim().is_empty() {
        return Ok(Vec::new());
    }

    // Pattern: file.lua:line:col: (CODE) message
    let regex = Regex::new(r"^(.+\.lua):(\d+):(\d+):\s*\(([EW]\d+)\)\s*(.+)$")
        .expect("Invalid luacheck regex pattern");

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
            let code = captures.get(4).map(|m| m.as_str().to_string());
            let message = captures
                .get(5)
                .map(|m| m.as_str())
                .unwrap_or("")
                .to_string();

            // Determine severity from code prefix
            let severity = match code.as_ref().map(|c| c.chars().next()) {
                Some(Some('E')) => Severity::Error,
                Some(Some('W')) => Severity::Warning,
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
                code,
                source: "luacheck".to_string(),
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
    fn test_parse_warning() {
        let output = "src/main.lua:10:5: (W611) line contains trailing whitespace";

        let result = parse_luacheck_output(output).unwrap();
        assert_eq!(result.len(), 1);

        let d = &result[0];
        assert_eq!(d.file, PathBuf::from("src/main.lua"));
        assert_eq!(d.line, 10);
        assert_eq!(d.column, 5);
        assert_eq!(d.severity, Severity::Warning);
        assert_eq!(d.code, Some("W611".to_string()));
        assert_eq!(d.message, "line contains trailing whitespace");
        assert_eq!(d.source, "luacheck");
    }

    #[test]
    fn test_parse_error() {
        let output = "src/main.lua:15:1: (E011) expected expression near 'end'";

        let result = parse_luacheck_output(output).unwrap();
        assert_eq!(result.len(), 1);

        let d = &result[0];
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(d.code, Some("E011".to_string()));
    }

    #[test]
    fn test_parse_unused_variable() {
        let output = "src/utils.lua:3:7: (W211) unused variable 'helper'";

        let result = parse_luacheck_output(output).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].code, Some("W211".to_string()));
        assert_eq!(result[0].message, "unused variable 'helper'");
    }

    #[test]
    fn test_parse_multiple() {
        let output = r#"src/main.lua:10:5: (W611) trailing whitespace
src/main.lua:15:1: (E011) expected expression near 'end'
src/utils.lua:3:7: (W211) unused variable 'helper'"#;

        let result = parse_luacheck_output(output).unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].severity, Severity::Warning);
        assert_eq!(result[1].severity, Severity::Error);
        assert_eq!(result[2].severity, Severity::Warning);
    }

    #[test]
    fn test_parse_empty() {
        let result = parse_luacheck_output("").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_malformed() {
        let output = "Total: 0 warnings / 0 errors in 3 files";
        let result = parse_luacheck_output(output).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_summary_line_ignored() {
        // luacheck often ends with a summary line - make sure it's ignored
        let output = r#"src/main.lua:10:5: (W611) trailing whitespace
Checking src/main.lua                            1 warning

Total: 1 warning / 0 errors in 1 file"#;

        let result = parse_luacheck_output(output).unwrap();
        assert_eq!(result.len(), 1);
    }
}
