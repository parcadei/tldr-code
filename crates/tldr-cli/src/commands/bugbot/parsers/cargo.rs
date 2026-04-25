//! Parser for cargo clippy NDJSON output
//!
//! Parses the `--message-format=json` output from `cargo clippy`, where each
//! line is a separate JSON object. Non-JSON lines and non-compiler-message
//! lines are silently skipped (match/continue, not `?` abort) per PM-5.
//!
//! The `tool` field on produced findings is set to an empty string. The runner
//! fills it in after parsing. [PM-6]

use serde::Deserialize;
use std::path::PathBuf;

use super::super::tools::{L1Finding, ToolCategory};

/// Top-level cargo JSON message
#[derive(Deserialize)]
struct CargoMessage {
    reason: String,
    #[serde(default)]
    message: Option<CargoCompilerMessage>,
}

/// Inner compiler message with level, message text, optional code, and spans
#[derive(Deserialize)]
struct CargoCompilerMessage {
    level: String,
    message: String,
    #[serde(default)]
    code: Option<CargoCode>,
    #[serde(default)]
    spans: Vec<CargoSpan>,
}

/// Diagnostic code (e.g., "unused_variables", "clippy::needless_return")
#[derive(Deserialize)]
struct CargoCode {
    code: String,
}

/// Source location span within the diagnostic
#[derive(Deserialize)]
struct CargoSpan {
    file_name: String,
    line_start: u32,
    column_start: u32,
    is_primary: bool,
}

/// Map cargo severity level strings to normalized severity strings.
///
/// - `"error"` and `"error: internal compiler error"` map to `"high"`
/// - `"warning"` maps to `"medium"`
/// - `"note"` and `"help"` map to `"info"`
/// - All other values map to `"low"`
pub fn map_cargo_severity(level: &str) -> &'static str {
    match level {
        "error" | "error: internal compiler error" => "high",
        "warning" => "medium",
        "note" | "help" => "info",
        _ => "low",
    }
}

/// Maximum number of findings to collect from a single tool run.
///
/// This is a safety limit to prevent unbounded memory growth when parsing
/// output from a very large project. 10,000 findings is far more than any
/// developer can act on; beyond this point we stop parsing and return what
/// we have.
pub const MAX_FINDINGS: usize = 10_000;

/// Parse cargo/clippy NDJSON output into L1 findings.
///
/// # Contract
/// - Skips non-JSON lines (continue, not abort) [PM-5]
/// - Skips non "compiler-message" lines
/// - Skips messages without spans
/// - Uses primary span if available, falls back to first span
/// - `tool` field is set to empty string (runner fills it in later) [PM-6]
/// - `category` is always `ToolCategory::Linter` (clippy is a linter)
/// - Severity mapping: error -> high, warning -> medium, note/help -> info, other -> low
/// - Stops collecting after `MAX_FINDINGS` to prevent unbounded growth [F2]
pub fn parse_cargo_output(stdout: &str) -> Vec<L1Finding> {
    let mut findings = Vec::new();

    for line in stdout.lines() {
        // F2: Stop collecting once we hit the safety limit
        if findings.len() >= MAX_FINDINGS {
            eprintln!(
                "bugbot: cargo parser hit MAX_FINDINGS limit ({}), stopping parse",
                MAX_FINDINGS
            );
            break;
        }

        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Step 1: Try to parse as JSON -- skip on failure (continue, NOT ?) [PM-5]
        let cargo_msg: CargoMessage = match serde_json::from_str(line) {
            Ok(msg) => msg,
            Err(_) => continue,
        };

        // Step 2: Only process "compiler-message" reason
        if cargo_msg.reason != "compiler-message" {
            continue;
        }

        // Step 3: Extract the inner compiler message
        let compiler_msg = match cargo_msg.message {
            Some(msg) => msg,
            None => continue,
        };

        // Step 4: Find the span to use -- skip if no spans at all
        if compiler_msg.spans.is_empty() {
            continue;
        }

        // Prefer the primary span; fall back to the first span
        let span = compiler_msg
            .spans
            .iter()
            .find(|s| s.is_primary)
            .unwrap_or(&compiler_msg.spans[0]);

        // Step 5: Map severity
        let severity = map_cargo_severity(&compiler_msg.level);

        // Step 6: Extract code if present
        let code = compiler_msg.code.map(|c| c.code);

        // Step 7: Build L1Finding
        findings.push(L1Finding {
            tool: String::new(), // Runner fills this in [PM-6]
            category: ToolCategory::Linter,
            file: PathBuf::from(&span.file_name),
            line: span.line_start,
            column: span.column_start,
            native_severity: compiler_msg.level,
            severity: severity.to_string(),
            message: compiler_msg.message,
            code,
        });
    }

    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Test constants: realistic cargo NDJSON samples --

    const SAMPLE_WARNING: &str = r#"{"reason":"compiler-message","package_id":"test 0.1.0 (path+file:///test)","manifest_path":"/test/Cargo.toml","target":{"kind":["lib"],"crate_types":["lib"],"name":"test","src_path":"/test/src/lib.rs","edition":"2021","doc":false,"doctest":false,"test":false},"message":{"rendered":"warning: unused variable: `x`\n","children":[],"code":{"code":"unused_variables","explanation":null},"level":"warning","message":"unused variable: `x`","spans":[{"byte_end":100,"byte_start":99,"column_end":10,"column_start":9,"expansion":null,"file_name":"src/main.rs","is_primary":true,"label":null,"line_end":10,"line_start":10,"suggested_replacement":null,"suggestion_applicability":null,"text":[]}]}}"#;

    const SAMPLE_ERROR: &str = r#"{"reason":"compiler-message","package_id":"test 0.1.0 (path+file:///test)","manifest_path":"/test/Cargo.toml","target":{"kind":["lib"],"crate_types":["lib"],"name":"test","src_path":"/test/src/lib.rs","edition":"2021","doc":false,"doctest":false,"test":false},"message":{"rendered":"error[E0308]: mismatched types\n","children":[],"code":{"code":"E0308","explanation":null},"level":"error","message":"mismatched types","spans":[{"byte_end":200,"byte_start":190,"column_end":15,"column_start":5,"expansion":null,"file_name":"src/lib.rs","is_primary":true,"label":null,"line_end":20,"line_start":20,"suggested_replacement":null,"suggestion_applicability":null,"text":[]}]}}"#;

    const SAMPLE_BUILD_FINISHED: &str = r#"{"reason":"build-finished","success":true}"#;

    const SAMPLE_COMPILER_ARTIFACT: &str = r#"{"reason":"compiler-artifact","package_id":"test 0.1.0","manifest_path":"/test/Cargo.toml","target":{"kind":["lib"],"crate_types":["lib"],"name":"test","src_path":"/test/src/lib.rs","edition":"2021","doc":false,"doctest":false,"test":false},"profile":{"opt_level":"0","debuginfo":2,"debug_assertions":true,"overflow_checks":true,"test":false},"features":[],"filenames":["/test/target/debug/libtest.rlib"],"executable":null,"fresh":false}"#;

    const SAMPLE_NO_SPANS: &str = r#"{"reason":"compiler-message","package_id":"test 0.1.0","manifest_path":"/test/Cargo.toml","target":{"kind":["lib"],"crate_types":["lib"],"name":"test","src_path":"/test/src/lib.rs","edition":"2021","doc":false,"doctest":false,"test":false},"message":{"rendered":"note: something general\n","children":[],"code":null,"level":"note","message":"something general","spans":[]}}"#;

    const SAMPLE_TWO_SPANS_NO_PRIMARY: &str = r#"{"reason":"compiler-message","package_id":"test 0.1.0","manifest_path":"/test/Cargo.toml","target":{"kind":["lib"],"crate_types":["lib"],"name":"test","src_path":"/test/src/lib.rs","edition":"2021","doc":false,"doctest":false,"test":false},"message":{"rendered":"warning: related spans\n","children":[],"code":{"code":"related_spans","explanation":null},"level":"warning","message":"related spans","spans":[{"byte_end":50,"byte_start":40,"column_end":8,"column_start":3,"expansion":null,"file_name":"src/first.rs","is_primary":false,"label":null,"line_end":7,"line_start":7,"suggested_replacement":null,"suggestion_applicability":null,"text":[]},{"byte_end":90,"byte_start":80,"column_end":12,"column_start":5,"expansion":null,"file_name":"src/second.rs","is_primary":false,"label":null,"line_end":15,"line_start":15,"suggested_replacement":null,"suggestion_applicability":null,"text":[]}]}}"#;

    const SAMPLE_NO_CODE: &str = r#"{"reason":"compiler-message","package_id":"test 0.1.0","manifest_path":"/test/Cargo.toml","target":{"kind":["lib"],"crate_types":["lib"],"name":"test","src_path":"/test/src/lib.rs","edition":"2021","doc":false,"doctest":false,"test":false},"message":{"rendered":"warning: something\n","children":[],"code":null,"level":"warning","message":"something without code","spans":[{"byte_end":30,"byte_start":20,"column_end":6,"column_start":1,"expansion":null,"file_name":"src/lib.rs","is_primary":true,"label":null,"line_end":3,"line_start":3,"suggested_replacement":null,"suggestion_applicability":null,"text":[]}]}}"#;

    const SAMPLE_CLIPPY_CODE: &str = r#"{"reason":"compiler-message","package_id":"test 0.1.0","manifest_path":"/test/Cargo.toml","target":{"kind":["lib"],"crate_types":["lib"],"name":"test","src_path":"/test/src/lib.rs","edition":"2021","doc":false,"doctest":false,"test":false},"message":{"rendered":"warning: needless return\n","children":[],"code":{"code":"clippy::needless_return","explanation":null},"level":"warning","message":"unneeded `return` statement","spans":[{"byte_end":150,"byte_start":140,"column_end":12,"column_start":5,"expansion":null,"file_name":"src/lib.rs","is_primary":true,"label":null,"line_end":25,"line_start":25,"suggested_replacement":null,"suggestion_applicability":null,"text":[]}]}}"#;

    #[test]
    fn test_parse_empty_output() {
        let findings = parse_cargo_output("");
        assert!(
            findings.is_empty(),
            "empty input should produce no findings"
        );
    }

    #[test]
    fn test_parse_single_warning() {
        let findings = parse_cargo_output(SAMPLE_WARNING);
        assert_eq!(findings.len(), 1, "should produce exactly 1 finding");

        let f = &findings[0];
        assert_eq!(f.severity, "medium");
        assert_eq!(f.file, PathBuf::from("src/main.rs"));
        assert_eq!(f.line, 10);
        assert_eq!(f.column, 9);
        assert_eq!(f.message, "unused variable: `x`");
        assert_eq!(f.native_severity, "warning");
        assert_eq!(f.category, ToolCategory::Linter);
    }

    #[test]
    fn test_parse_error_finding() {
        let findings = parse_cargo_output(SAMPLE_ERROR);
        assert_eq!(findings.len(), 1);

        let f = &findings[0];
        assert_eq!(f.severity, "high");
        assert_eq!(f.native_severity, "error");
        assert_eq!(f.file, PathBuf::from("src/lib.rs"));
        assert_eq!(f.line, 20);
        assert_eq!(f.column, 5);
        assert_eq!(f.message, "mismatched types");
        assert_eq!(f.code, Some("E0308".to_string()));
    }

    #[test]
    fn test_parse_multiple_findings() {
        let input = format!(
            "{}\n{}\n{}\n{}",
            SAMPLE_WARNING, SAMPLE_ERROR, SAMPLE_CLIPPY_CODE, SAMPLE_BUILD_FINISHED
        );
        let findings = parse_cargo_output(&input);
        assert_eq!(
            findings.len(),
            3,
            "should produce 3 findings (build-finished skipped)"
        );
    }

    #[test]
    fn test_skip_non_compiler_message() {
        let findings = parse_cargo_output(SAMPLE_BUILD_FINISHED);
        assert!(
            findings.is_empty(),
            "build-finished should produce no findings"
        );
    }

    #[test]
    fn test_skip_compiler_artifact() {
        let findings = parse_cargo_output(SAMPLE_COMPILER_ARTIFACT);
        assert!(
            findings.is_empty(),
            "compiler-artifact should produce no findings"
        );
    }

    #[test]
    fn test_skip_non_json_lines() {
        let input = format!(
            "{}\nthis is not json at all\n{}\n   \n",
            SAMPLE_WARNING, SAMPLE_ERROR
        );
        let findings = parse_cargo_output(&input);
        assert_eq!(
            findings.len(),
            2,
            "bad JSON line should be skipped, not abort [PM-5]"
        );
    }

    #[test]
    fn test_skip_message_without_spans() {
        let findings = parse_cargo_output(SAMPLE_NO_SPANS);
        assert!(
            findings.is_empty(),
            "message without spans should be skipped"
        );
    }

    #[test]
    fn test_primary_span_preferred() {
        // SAMPLE_WARNING has a single primary span at src/main.rs:10:9
        let findings = parse_cargo_output(SAMPLE_WARNING);
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.file, PathBuf::from("src/main.rs"));
        assert_eq!(f.line, 10);
        assert_eq!(f.column, 9);
    }

    #[test]
    fn test_first_span_fallback() {
        // SAMPLE_TWO_SPANS_NO_PRIMARY: 2 spans, neither is_primary=true
        // Should use first span: src/first.rs:7:3
        let findings = parse_cargo_output(SAMPLE_TWO_SPANS_NO_PRIMARY);
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.file, PathBuf::from("src/first.rs"));
        assert_eq!(f.line, 7);
        assert_eq!(f.column, 3);
    }

    #[test]
    fn test_severity_mapping_error() {
        assert_eq!(map_cargo_severity("error"), "high");
    }

    #[test]
    fn test_severity_mapping_warning() {
        assert_eq!(map_cargo_severity("warning"), "medium");
    }

    #[test]
    fn test_severity_mapping_note() {
        assert_eq!(map_cargo_severity("note"), "info");
    }

    #[test]
    fn test_severity_mapping_help() {
        assert_eq!(map_cargo_severity("help"), "info");
    }

    #[test]
    fn test_severity_mapping_ice() {
        assert_eq!(map_cargo_severity("error: internal compiler error"), "high");
    }

    #[test]
    fn test_severity_mapping_unknown() {
        assert_eq!(map_cargo_severity("something_else"), "low");
    }

    #[test]
    fn test_code_extracted() {
        let findings = parse_cargo_output(SAMPLE_CLIPPY_CODE);
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].code,
            Some("clippy::needless_return".to_string())
        );
    }

    #[test]
    fn test_no_code_field() {
        let findings = parse_cargo_output(SAMPLE_NO_CODE);
        assert_eq!(findings.len(), 1);
        assert!(
            findings[0].code.is_none(),
            "message with code:null should produce finding.code = None"
        );
    }

    #[test]
    fn test_tool_name_is_empty_string() {
        let findings = parse_cargo_output(SAMPLE_WARNING);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].tool, "", "parser must NOT set tool name [PM-6]");
    }

    #[test]
    fn test_map_cargo_severity_all_levels() {
        assert_eq!(map_cargo_severity("error"), "high");
        assert_eq!(map_cargo_severity("error: internal compiler error"), "high");
        assert_eq!(map_cargo_severity("warning"), "medium");
        assert_eq!(map_cargo_severity("note"), "info");
        assert_eq!(map_cargo_severity("help"), "info");
        assert_eq!(map_cargo_severity("anything_else"), "low");
    }

    // =========================================================================
    // F2: Unbounded findings Vec has a safety limit
    // =========================================================================

    #[test]
    fn test_max_findings_constant_exists() {
        // F2: There should be a MAX_FINDINGS constant to prevent unbounded growth
        let max_findings = std::hint::black_box(super::MAX_FINDINGS);
        assert!(
            max_findings > 0,
            "MAX_FINDINGS should be a positive constant"
        );
        assert!(
            max_findings >= 1_000,
            "MAX_FINDINGS should be at least 1000, got {}",
            max_findings
        );
    }

    #[test]
    fn test_parse_cargo_output_respects_max_findings() {
        // F2: When input contains more findings than MAX_FINDINGS,
        // the parser should stop and return what it has.
        let line_count = super::MAX_FINDINGS + 500;

        let mut input = String::new();
        for _ in 0..line_count {
            input.push_str(SAMPLE_WARNING);
            input.push('\n');
        }

        let findings = parse_cargo_output(&input);

        assert!(
            findings.len() <= super::MAX_FINDINGS,
            "findings should be capped at MAX_FINDINGS ({}), got {}",
            super::MAX_FINDINGS,
            findings.len()
        );
        assert_eq!(
            findings.len(),
            super::MAX_FINDINGS,
            "should return exactly MAX_FINDINGS when input exceeds limit"
        );
    }
}
