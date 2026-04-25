//! Parser for `ruff check --output-format=json` output.
//!
//! Ruff outputs a JSON array of diagnostic objects. Each has `filename`,
//! `location` (row/column), `code`, `message`, and `url`.

use std::path::PathBuf;

use super::super::tools::{L1Finding, ToolCategory};
use super::ParseError;

/// Parse ruff JSON output into L1 findings.
///
/// Ruff emits a JSON array like:
/// ```json
/// [{"filename": "main.py", "location": {"row": 13, "column": 9},
///   "code": "F841", "message": "...", "url": "..."}]
/// ```
pub fn parse_ruff_output(stdout: &str) -> Result<Vec<L1Finding>, ParseError> {
    let stdout = stdout.trim();
    if stdout.is_empty() || stdout == "[]" {
        return Ok(Vec::new());
    }

    let items: Vec<serde_json::Value> = serde_json::from_str(stdout)?;
    let mut findings = Vec::new();

    for item in &items {
        let file = item.get("filename").and_then(|v| v.as_str()).unwrap_or("");
        let row = item
            .pointer("/location/row")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let col = item
            .pointer("/location/column")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let code = item.get("code").and_then(|v| v.as_str()).unwrap_or("");
        let message = item.get("message").and_then(|v| v.as_str()).unwrap_or("");

        let severity = ruff_code_to_severity(code);

        findings.push(L1Finding {
            tool: String::new(), // Set by runner [PM-6]
            category: ToolCategory::Linter,
            file: PathBuf::from(file),
            line: row as u32,
            column: col as u32,
            native_severity: "warning".to_string(),
            severity,
            message: message.to_string(),
            code: if code.is_empty() {
                None
            } else {
                Some(code.to_string())
            },
        });
    }

    Ok(findings)
}

/// Map ruff rule codes to bugbot severity levels.
fn ruff_code_to_severity(code: &str) -> String {
    match code.chars().next() {
        // E = pycodestyle errors, F = pyflakes (includes unused vars, missing returns)
        Some('E') | Some('F') => "medium".to_string(),
        // W = pycodestyle warnings
        Some('W') => "low".to_string(),
        // S = bandit security rules
        Some('S') => "high".to_string(),
        // B = bugbear (likely bugs)
        Some('B') => "medium".to_string(),
        _ => "low".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ruff_empty() {
        assert!(parse_ruff_output("").unwrap().is_empty());
        assert!(parse_ruff_output("[]").unwrap().is_empty());
    }

    #[test]
    fn test_parse_ruff_finding() {
        let json = r#"[{
            "filename": "main.py",
            "location": {"row": 13, "column": 9},
            "end_location": {"row": 13, "column": 20},
            "code": "F841",
            "message": "Local variable `transformed` is assigned to but never used",
            "url": "https://docs.astral.sh/ruff/rules/unused-variable"
        }]"#;

        let findings = parse_ruff_output(json).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].file, PathBuf::from("main.py"));
        assert_eq!(findings[0].line, 13);
        assert_eq!(findings[0].column, 9);
        assert_eq!(findings[0].code, Some("F841".to_string()));
        assert_eq!(findings[0].severity, "medium");
    }

    #[test]
    fn test_ruff_severity_mapping() {
        assert_eq!(ruff_code_to_severity("F841"), "medium");
        assert_eq!(ruff_code_to_severity("E501"), "medium");
        assert_eq!(ruff_code_to_severity("W291"), "low");
        assert_eq!(ruff_code_to_severity("S101"), "high");
        assert_eq!(ruff_code_to_severity("B006"), "medium");
    }
}
