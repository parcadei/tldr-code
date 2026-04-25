//! Parser for cppcheck output using `--template` format.
//!
//! We use `--template='{file}\t{line}\t{column}\t{severity}\t{id}\t{message}'`
//! to get tab-separated output that's easy to parse without XML dependencies.

use std::path::PathBuf;

use super::super::tools::{L1Finding, ToolCategory};
use super::ParseError;

pub fn parse_cppcheck_output(stdout: &str) -> Result<Vec<L1Finding>, ParseError> {
    let mut findings = Vec::new();

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.splitn(6, '\t').collect();
        if parts.len() < 6 {
            continue; // Skip malformed lines (e.g., "Checking ..." progress output)
        }

        let file = parts[0];
        let line_num: u32 = parts[1].parse().unwrap_or(0);
        let column: u32 = parts[2].parse().unwrap_or(0);
        let native_sev = parts[3];
        let id = parts[4];
        let message = parts[5];

        // Skip "information" severity (file-level notes, not bugs)
        if native_sev == "information" {
            continue;
        }

        let severity = match native_sev {
            "error" => "high",
            "warning" => "medium",
            "style" | "performance" | "portability" => "low",
            _ => "info",
        };

        findings.push(L1Finding {
            tool: String::new(),
            category: ToolCategory::Linter,
            file: PathBuf::from(file),
            line: line_num,
            column,
            native_severity: native_sev.to_string(),
            severity: severity.to_string(),
            message: message.to_string(),
            code: if id.is_empty() {
                None
            } else {
                Some(id.to_string())
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
        assert!(parse_cppcheck_output("").unwrap().is_empty());
    }

    #[test]
    fn test_parse_finding() {
        let output = "main.c\t10\t5\twarning\tunreadVariable\tVariable 'x' is assigned a value that is never used.";
        let findings = parse_cppcheck_output(output).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].file, PathBuf::from("main.c"));
        assert_eq!(findings[0].line, 10);
        assert_eq!(findings[0].column, 5);
        assert_eq!(findings[0].severity, "medium");
        assert_eq!(findings[0].code, Some("unreadVariable".to_string()));
    }

    #[test]
    fn test_parse_skips_information() {
        let output = "main.c\t0\t0\tinformation\tmissingInclude\tInclude file not found";
        assert!(parse_cppcheck_output(output).unwrap().is_empty());
    }

    #[test]
    fn test_parse_skips_malformed() {
        let output =
            "Checking main.c ...\nmain.c\t5\t1\terror\tnullPointer\tNull pointer dereference";
        let findings = parse_cppcheck_output(output).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "high");
    }
}
