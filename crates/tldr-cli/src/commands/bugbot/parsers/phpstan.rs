//! Parser for `phpstan analyse --error-format=json` output.

use std::path::PathBuf;

use super::super::tools::{L1Finding, ToolCategory};
use super::ParseError;

pub fn parse_phpstan_output(stdout: &str) -> Result<Vec<L1Finding>, ParseError> {
    let stdout = stdout.trim();
    if stdout.is_empty() {
        return Ok(Vec::new());
    }

    let root: serde_json::Value = serde_json::from_str(stdout)?;
    let files = match root.get("files").and_then(|v| v.as_object()) {
        Some(f) => f,
        None => return Ok(Vec::new()),
    };

    let mut findings = Vec::new();
    for (path, file_data) in files {
        let messages = match file_data.get("messages").and_then(|v| v.as_array()) {
            Some(m) => m,
            None => continue,
        };

        for msg in messages {
            let message = msg
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let line = msg.get("line").and_then(|v| v.as_u64()).unwrap_or(0);
            let tip = msg.get("tip").and_then(|v| v.as_str());

            // PHPStan doesn't have severity levels — all findings are errors
            findings.push(L1Finding {
                tool: String::new(),
                category: ToolCategory::Linter,
                file: PathBuf::from(path),
                line: line as u32,
                column: 0,
                native_severity: "error".to_string(),
                severity: "medium".to_string(),
                message: if let Some(t) = tip {
                    format!("{} (tip: {})", message, t)
                } else {
                    message.to_string()
                },
                code: None,
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
        assert!(parse_phpstan_output("").unwrap().is_empty());
    }

    #[test]
    fn test_parse_finding() {
        let json = r#"{"totals": {"errors": 1, "file_errors": 1},
            "files": {"/var/www/app.php": {"errors": 1, "messages": [{
                "message": "Variable $x might not be defined.",
                "line": 15,
                "ignorable": true
            }]}}}"#;
        let findings = parse_phpstan_output(json).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].file, PathBuf::from("/var/www/app.php"));
        assert_eq!(findings[0].line, 15);
        assert_eq!(findings[0].severity, "medium");
    }

    #[test]
    fn test_parse_no_files() {
        let json = r#"{"totals": {"errors": 0}, "files": {}}"#;
        assert!(parse_phpstan_output(json).unwrap().is_empty());
    }
}
