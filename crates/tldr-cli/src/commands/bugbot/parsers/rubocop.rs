//! Parser for `rubocop --format json` output.

use std::path::PathBuf;

use super::super::tools::{L1Finding, ToolCategory};
use super::ParseError;

pub fn parse_rubocop_output(stdout: &str) -> Result<Vec<L1Finding>, ParseError> {
    let stdout = stdout.trim();
    if stdout.is_empty() {
        return Ok(Vec::new());
    }

    let root: serde_json::Value = serde_json::from_str(stdout)?;
    let files = match root.get("files").and_then(|v| v.as_array()) {
        Some(f) => f,
        None => return Ok(Vec::new()),
    };

    let mut findings = Vec::new();
    for file_entry in files {
        let path = file_entry
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let offenses = match file_entry.get("offenses").and_then(|v| v.as_array()) {
            Some(o) => o,
            None => continue,
        };

        for offense in offenses {
            let cop = offense
                .get("cop_name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let native_sev = offense
                .get("severity")
                .and_then(|v| v.as_str())
                .unwrap_or("warning");
            let message = offense
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let line = offense
                .pointer("/location/start_line")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let column = offense
                .pointer("/location/start_column")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);

            let severity = match native_sev {
                "fatal" | "error" => "high",
                "warning" => "medium",
                "convention" | "refactor" => "low",
                _ => "info",
            };

            findings.push(L1Finding {
                tool: String::new(),
                category: ToolCategory::Linter,
                file: PathBuf::from(path),
                line: line as u32,
                column: column as u32,
                native_severity: native_sev.to_string(),
                severity: severity.to_string(),
                message: message.to_string(),
                code: if cop.is_empty() {
                    None
                } else {
                    Some(cop.to_string())
                },
            });
        }
    }

    Ok(findings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_empty() {
        assert!(parse_rubocop_output("").unwrap().is_empty());
    }

    #[test]
    fn test_parse_finding() {
        let json = r#"{"files": [{"path": "app.rb", "offenses": [{
            "cop_name": "Style/FrozenStringLiteralComment",
            "severity": "convention",
            "message": "Missing frozen string literal comment.",
            "location": {"start_line": 1, "start_column": 1}
        }]}]}"#;
        let findings = parse_rubocop_output(json).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "low");
        assert_eq!(
            findings[0].code,
            Some("Style/FrozenStringLiteralComment".to_string())
        );
    }

    #[test]
    fn test_parse_error_severity() {
        let json = r#"{"files": [{"path": "x.rb", "offenses": [{
            "cop_name": "Lint/UselessAssignment",
            "severity": "warning",
            "message": "Useless assignment",
            "location": {"start_line": 5, "start_column": 3}
        }]}]}"#;
        let findings = parse_rubocop_output(json).unwrap();
        assert_eq!(findings[0].severity, "medium");
    }
}
