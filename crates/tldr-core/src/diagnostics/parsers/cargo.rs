//! Cargo/Clippy JSON output parser.
//!
//! Cargo outputs NDJSON (one JSON object per line) via `--message-format=json`:
//! ```json
//! {"reason":"compiler-message","message":{"level":"warning","message":"unused variable",...}}
//! ```
//!
//! Only messages with "reason": "compiler-message" are diagnostics.

use crate::diagnostics::{Diagnostic, Severity};
use crate::error::TldrError;
use serde::Deserialize;
use std::path::PathBuf;

/// Cargo JSON line (NDJSON)
#[derive(Debug, Deserialize)]
struct CargoLine {
    reason: String,
    message: Option<CargoMessage>,
}

/// Cargo compiler message
#[derive(Debug, Deserialize)]
struct CargoMessage {
    code: Option<CargoCode>,
    level: String,
    message: String,
    spans: Vec<CargoSpan>,
    #[allow(dead_code)]
    rendered: Option<String>,
}

/// Cargo error code
#[derive(Debug, Deserialize)]
struct CargoCode {
    code: String,
    #[allow(dead_code)]
    explanation: Option<String>,
}

/// Cargo source span
#[derive(Debug, Deserialize)]
struct CargoSpan {
    file_name: String,
    line_start: u32,
    line_end: u32,
    column_start: u32,
    column_end: u32,
    is_primary: bool,
    #[allow(dead_code)]
    label: Option<String>,
}

/// Parse cargo NDJSON output into unified Diagnostic structs.
///
/// # Arguments
/// * `output` - The raw NDJSON output from `cargo check --message-format=json`
///
/// # Returns
/// A vector of Diagnostic structs, or an error if parsing fails.
///
/// # Format
/// Each line is a separate JSON object. Only lines with
/// "reason": "compiler-message" are processed.
pub fn parse_cargo_output(output: &str) -> Result<Vec<Diagnostic>, TldrError> {
    let mut diagnostics = Vec::new();

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Parse each line as JSON
        let cargo_line: CargoLine = match serde_json::from_str(line) {
            Ok(l) => l,
            Err(_) => continue, // Skip lines that don't parse (e.g., summary lines)
        };

        // Only process compiler messages
        if cargo_line.reason != "compiler-message" {
            continue;
        }

        let message = match cargo_line.message {
            Some(m) => m,
            None => continue,
        };

        // Find the primary span
        let primary_span = message.spans.iter().find(|s| s.is_primary);
        let span = match primary_span.or(message.spans.first()) {
            Some(s) => s,
            None => continue, // No span information
        };

        let severity = match message.level.as_str() {
            "error" => Severity::Error,
            "warning" => Severity::Warning,
            "note" => Severity::Information,
            "help" => Severity::Hint,
            _ => Severity::Warning,
        };

        let code = message.code.map(|c| c.code);

        diagnostics.push(Diagnostic {
            file: PathBuf::from(&span.file_name),
            line: span.line_start,
            column: span.column_start,
            end_line: Some(span.line_end),
            end_column: Some(span.column_end),
            severity,
            message: message.message,
            code,
            source: "cargo".to_string(),
            url: None,
        });
    }

    Ok(diagnostics)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CARGO_OUTPUT: &str = r#"{"reason":"compiler-message","package_id":"myproject 0.1.0","manifest_path":"/project/Cargo.toml","target":{"kind":["lib"],"crate_types":["lib"],"name":"myproject","src_path":"/project/src/lib.rs"},"message":{"code":{"code":"unused_variables","explanation":null},"level":"warning","message":"unused variable: `x`","spans":[{"file_name":"src/main.rs","byte_start":100,"byte_end":101,"line_start":10,"line_end":10,"column_start":5,"column_end":6,"is_primary":true,"text":[{"text":"    let x = 5;","highlight_start":5,"highlight_end":6}],"label":"help: if this is intentional, prefix it with an underscore: `_x`"}],"children":[],"rendered":"warning: unused variable"}}
{"reason":"compiler-message","package_id":"myproject 0.1.0","manifest_path":"/project/Cargo.toml","target":{"kind":["lib"],"crate_types":["lib"],"name":"myproject","src_path":"/project/src/lib.rs"},"message":{"code":{"code":"E0308","explanation":"Expected type did not match the received type."},"level":"error","message":"mismatched types","spans":[{"file_name":"src/lib.rs","byte_start":200,"byte_end":210,"line_start":20,"line_end":20,"column_start":10,"column_end":20,"is_primary":true,"text":[{"text":"    return \"hello\";","highlight_start":10,"highlight_end":17}],"label":"expected `i32`, found `&str`"}],"children":[],"rendered":"error[E0308]: mismatched types"}}"#;

    #[test]
    fn test_parse_cargo() {
        let result = parse_cargo_output(CARGO_OUTPUT).unwrap();
        assert_eq!(result.len(), 2);

        let warning = &result[0];
        assert_eq!(warning.file, PathBuf::from("src/main.rs"));
        assert_eq!(warning.line, 10);
        assert_eq!(warning.column, 5);
        assert_eq!(warning.severity, Severity::Warning);
        assert_eq!(warning.code, Some("unused_variables".to_string()));
        assert_eq!(warning.source, "cargo");

        let error = &result[1];
        assert_eq!(error.file, PathBuf::from("src/lib.rs"));
        assert_eq!(error.line, 20);
        assert_eq!(error.severity, Severity::Error);
        assert_eq!(error.code, Some("E0308".to_string()));
    }

    #[test]
    fn test_level_mapping() {
        let result = parse_cargo_output(CARGO_OUTPUT).unwrap();
        assert_eq!(result[0].severity, Severity::Warning);
        assert_eq!(result[1].severity, Severity::Error);
    }

    #[test]
    fn test_empty_output() {
        let result = parse_cargo_output("").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_ndjson_parsing() {
        // Two lines = two diagnostics
        let result = parse_cargo_output(CARGO_OUTPUT).unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_skip_non_compiler_messages() {
        let output = r#"{"reason":"build-script-executed","package_id":"test 0.1.0"}
{"reason":"compiler-artifact","package_id":"test 0.1.0"}"#;
        let result = parse_cargo_output(output).unwrap();
        assert!(result.is_empty());
    }
}
