//! Parser for `swiftlint lint --reporter json` output.

use std::path::PathBuf;

use super::super::tools::{L1Finding, ToolCategory};
use super::ParseError;

pub fn parse_swiftlint_output(stdout: &str) -> Result<Vec<L1Finding>, ParseError> {
    let stdout = stdout.trim();
    if stdout.is_empty() || stdout == "[]" {
        return Ok(Vec::new());
    }

    let items: Vec<serde_json::Value> = serde_json::from_str(stdout)?;
    let mut findings = Vec::new();

    for item in &items {
        let file = item.get("file").and_then(|v| v.as_str()).unwrap_or("");
        let line = item.get("line").and_then(|v| v.as_u64()).unwrap_or(0);
        let column = item.get("character").and_then(|v| v.as_u64()).unwrap_or(0);
        let native_sev = item
            .get("severity")
            .and_then(|v| v.as_str())
            .unwrap_or("Warning");
        let reason = item.get("reason").and_then(|v| v.as_str()).unwrap_or("");
        let rule_id = item.get("rule_id").and_then(|v| v.as_str()).unwrap_or("");

        let severity = match native_sev {
            "Error" => "high",
            _ => "medium",
        };

        findings.push(L1Finding {
            tool: String::new(),
            category: ToolCategory::Linter,
            file: PathBuf::from(file),
            line: line as u32,
            column: column as u32,
            native_severity: native_sev.to_lowercase(),
            severity: severity.to_string(),
            message: reason.to_string(),
            code: if rule_id.is_empty() {
                None
            } else {
                Some(rule_id.to_string())
            },
        });
    }

    Ok(findings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_empty() {
        assert!(parse_swiftlint_output("").unwrap().is_empty());
        assert!(parse_swiftlint_output("[]").unwrap().is_empty());
    }

    #[test]
    fn test_parse_finding() {
        let json = r#"[{
            "file": "/Users/dev/App.swift",
            "line": 10,
            "character": 5,
            "severity": "Warning",
            "type": "Trailing Whitespace",
            "rule_id": "trailing_whitespace",
            "reason": "Lines should not have trailing whitespace."
        }]"#;
        let findings = parse_swiftlint_output(json).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "medium");
        assert_eq!(findings[0].code, Some("trailing_whitespace".to_string()));
    }

    #[test]
    fn test_parse_error() {
        let json = r#"[{
            "file": "x.swift", "line": 1, "character": 1,
            "severity": "Error", "rule_id": "force_unwrapping",
            "reason": "Force unwrapping should be avoided."
        }]"#;
        let findings = parse_swiftlint_output(json).unwrap();
        assert_eq!(findings[0].severity, "high");
    }
}
