//! Text output formatter for bugbot check reports
//!
//! Produces human-readable text output for terminal display, as an alternative
//! to the default JSON output. Used when `--format text` is specified.

use std::fmt::Write;

use super::types::{BugbotCheckReport, L2AnalyzerResult};

/// Format a `BugbotCheckReport` as human-readable text.
///
/// Output structure:
/// - Summary line with finding counts by severity
/// - Stats line with files/functions analyzed and elapsed time
/// - One block per finding with severity tag, location, message, and evidence
/// - Optional errors section
/// - Optional truncation note
pub fn format_bugbot_text(report: &BugbotCheckReport) -> String {
    let mut out = String::new();

    // Summary line
    if report.findings.is_empty() {
        writeln!(out, "bugbot check -- no issues found").unwrap();
    } else {
        let severity_breakdown = format_severity_breakdown(&report.summary.by_severity);
        writeln!(
            out,
            "bugbot check -- {} findings ({})",
            report.summary.total_findings, severity_breakdown
        )
        .unwrap();
    }

    // Stats line
    writeln!(
        out,
        "  {} files analyzed, {} functions, {}ms",
        report.summary.files_analyzed, report.summary.functions_analyzed, report.elapsed_ms
    )
    .unwrap();

    // Individual findings
    for finding in &report.findings {
        writeln!(out).unwrap(); // blank line separator

        // PM-42: Critical findings use [!!!CRITICAL] marker for visibility
        let tag = if finding.severity == "critical" {
            "!!!CRITICAL".to_string()
        } else {
            finding.severity.to_uppercase()
        };
        writeln!(
            out,
            "[{}] {} in {}",
            tag,
            finding.finding_type,
            finding.file.display()
        )
        .unwrap();
        // PM-4: L1 findings have empty function field. Show "line N" directly
        // instead of "functionName (line N)" when function is empty.
        if finding.function.is_empty() {
            writeln!(out, "  line {}", finding.line).unwrap();
        } else {
            writeln!(out, "  {} (line {})", finding.function, finding.line).unwrap();
        }
        writeln!(out, "  {}", finding.message).unwrap();

        // Confidence line for L2 findings (when confidence is Some)
        if let Some(ref confidence) = finding.confidence {
            writeln!(out, "  Confidence: {}", confidence).unwrap();
        }

        // Evidence lines -- type-specific rendering
        format_finding_evidence(&mut out, finding);
    }

    // Critical summary line -- appears before tools/engines sections
    let critical_count = report
        .findings
        .iter()
        .filter(|f| f.severity == "critical")
        .count();
    if critical_count > 0 {
        writeln!(out).unwrap();
        writeln!(
            out,
            "CRITICAL: {} finding(s) require immediate attention",
            critical_count
        )
        .unwrap();
    }

    // Tool results section -- shows which L1 tools ran and their status
    if !report.tool_results.is_empty() || !report.tools_missing.is_empty() {
        writeln!(out).unwrap();
        writeln!(out, "tools:").unwrap();
        for result in &report.tool_results {
            let status = if result.success {
                format!(
                    "ok ({} findings, {}ms)",
                    result.finding_count, result.duration_ms
                )
            } else {
                let err_detail = result.error.as_deref().unwrap_or("unknown error");
                format!("failed ({})", err_detail)
            };
            writeln!(out, "  {} - {}", result.name, status).unwrap();
        }
        for name in &report.tools_missing {
            writeln!(out, "  {} - skipped (not installed)", name).unwrap();
        }
        if !report.tools_missing.is_empty() {
            writeln!(
                out,
                "  hint: run `tldr doctor --install {}` to set up missing tools",
                report.language
            )
            .unwrap();
        }
    }

    // L2 engine results section
    if !report.l2_engine_results.is_empty() {
        writeln!(out).unwrap();
        writeln!(out, "L2 engines:").unwrap();
        for result in &report.l2_engine_results {
            let status_label = format_engine_status(result);
            writeln!(
                out,
                "  {} - {} ({} findings, {}ms)",
                result.name, status_label, result.finding_count, result.duration_ms
            )
            .unwrap();
            // Append partial/error detail inline
            if !result.errors.is_empty() {
                for err_detail in &result.errors {
                    writeln!(out, "    [{}]", err_detail).unwrap();
                }
            }
        }
    }

    // ANALYSIS GAPS section -- shown when any engine has errors
    let engines_with_errors: Vec<&L2AnalyzerResult> = report
        .l2_engine_results
        .iter()
        .filter(|r| !r.errors.is_empty())
        .collect();
    if !engines_with_errors.is_empty() {
        writeln!(out).unwrap();
        writeln!(out, "ANALYSIS GAPS ({}):", engines_with_errors.len()).unwrap();
        for result in engines_with_errors {
            for error in &result.errors {
                writeln!(out, "  {}: {}", result.name, error).unwrap();
            }
        }
    }

    // Errors section
    if !report.errors.is_empty() {
        writeln!(out).unwrap();
        writeln!(out, "errors:").unwrap();
        for error in &report.errors {
            writeln!(out, "  - {}", error).unwrap();
        }
    }

    // Truncation note
    for note in &report.notes {
        if let Some(rest) = note.strip_prefix("truncated_to_") {
            writeln!(out).unwrap();
            writeln!(out, "(output truncated to {} findings)", rest).unwrap();
        }
    }

    // Remove trailing newline to let write_text add its own
    let trimmed = out.trim_end_matches('\n');
    trimmed.to_string()
}

/// Format severity counts as "N high, M medium, L low", omitting zeroes.
///
/// Severities are always printed in high, medium, low order regardless of
/// HashMap iteration order.
fn format_severity_breakdown(by_severity: &std::collections::HashMap<String, usize>) -> String {
    let mut parts = Vec::new();
    // Known severities in descending order (PM-8: includes "info", PM-42: includes "critical")
    for level in &["critical", "high", "medium", "low", "info"] {
        if let Some(&count) = by_severity.get(*level) {
            if count > 0 {
                parts.push(format!("{} {}", count, level));
            }
        }
    }
    // Include any unknown severity levels
    let mut keys: Vec<&String> = by_severity
        .keys()
        .filter(|k| !["critical", "high", "medium", "low", "info"].contains(&k.as_str()))
        .collect();
    keys.sort();
    for key in keys {
        if let Some(&count) = by_severity.get(key) {
            if count > 0 {
                parts.push(format!("{} {}", count, key));
            }
        }
    }
    parts.join(", ")
}

/// Format type-specific evidence lines for a finding.
///
/// Renders evidence differently depending on finding_type:
/// - `signature-regression`: Before/After signature comparison
/// - `secret-exposed`: Masked value display
/// - `taint-flow`: Source -> Sink flow with types
/// - `born-dead`: Reference count if available
/// - `complexity-increase` / `maintainability-drop`: Before/after values
/// - `resource-leak`: Sub-type and resource name
/// - `new-clone`: Clone type and similarity percentage
/// - `impact-blast-radius`: Caller counts
/// - `temporal-violation`: Expected vs actual call order
/// - `guard-removed`: Removed variable and constraint
/// - `contract-regression`: Category, variable, and constraint
/// - Other types: Show all evidence values (strings, numbers, booleans, arrays)
fn format_finding_evidence(out: &mut String, finding: &super::types::BugbotFinding) {
    match finding.finding_type.as_str() {
        "signature-regression" => {
            if let Some(before) = finding
                .evidence
                .get("before_signature")
                .and_then(|v| v.as_str())
            {
                writeln!(out, "  Before: {}", before).unwrap();
            }
            if let Some(after) = finding
                .evidence
                .get("after_signature")
                .and_then(|v| v.as_str())
            {
                writeln!(out, "  After:  {}", after).unwrap();
            }
        }
        "secret-exposed" => {
            if let Some(val) = finding
                .evidence
                .get("masked_value")
                .and_then(|v| v.as_str())
            {
                writeln!(out, "  Value: {}", val).unwrap();
            }
        }
        "taint-flow" => {
            // Production evidence uses source_var/sink_var/source_type/sink_type keys.
            // Legacy test evidence uses source/sink keys.
            let source_var = finding
                .evidence
                .get("source_var")
                .and_then(|v| v.as_str())
                .or_else(|| finding.evidence.get("source").and_then(|v| v.as_str()));
            let sink_var = finding
                .evidence
                .get("sink_var")
                .and_then(|v| v.as_str())
                .or_else(|| finding.evidence.get("sink").and_then(|v| v.as_str()));
            let source_type = finding.evidence.get("source_type").and_then(|v| v.as_str());
            let sink_type = finding.evidence.get("sink_type").and_then(|v| v.as_str());

            match (source_var, sink_var) {
                (Some(src), Some(snk)) => {
                    let src_label = match source_type {
                        Some(st) => format!("{} ({})", src, st),
                        None => src.to_string(),
                    };
                    let snk_label = match sink_type {
                        Some(st) => format!("{} ({})", snk, st),
                        None => snk.to_string(),
                    };
                    writeln!(out, "  Flow: {} -> {}", src_label, snk_label).unwrap();
                }
                _ => {
                    if let Some(src) = source_var {
                        writeln!(out, "  Source: {}", src).unwrap();
                    }
                    if let Some(snk) = sink_var {
                        writeln!(out, "  Sink: {}", snk).unwrap();
                    }
                }
            }
        }
        "born-dead" => {
            if let Some(count) = finding.evidence.get("ref_count").and_then(|v| v.as_u64()) {
                writeln!(out, "  References: {}", count).unwrap();
            }
        }
        "complexity-increase" | "maintainability-drop" => {
            let before = finding.evidence.get("before").and_then(|v| v.as_u64());
            let after = finding.evidence.get("after").and_then(|v| v.as_u64());
            if let (Some(b), Some(a)) = (before, after) {
                let label = if finding.finding_type == "complexity-increase" {
                    "Complexity"
                } else {
                    "Maintainability"
                };
                writeln!(out, "  {}: {} -> {}", label, b, a).unwrap();
            }
        }
        "resource-leak" => {
            let sub_type = finding
                .evidence
                .get("sub_type")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let resource = finding
                .evidence
                .get("resource")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            writeln!(out, "  Resource: {} ({})", resource, sub_type).unwrap();
        }
        "new-clone" => {
            if let Some(clone_type) = finding.evidence.get("clone_type").and_then(|v| v.as_str()) {
                writeln!(out, "  Clone type: {}", clone_type).unwrap();
            }
            if let Some(similarity) = finding.evidence.get("similarity").and_then(|v| v.as_f64()) {
                writeln!(out, "  Similarity: {:.0}%", similarity * 100.0).unwrap();
            }
        }
        "impact-blast-radius" => {
            let total = finding
                .evidence
                .get("total_callers")
                .and_then(|v| v.as_u64());
            let direct = finding
                .evidence
                .get("direct_callers")
                .and_then(|v| v.as_u64());
            if let Some(t) = total {
                writeln!(out, "  Total callers: {}", t).unwrap();
            }
            if let Some(d) = direct {
                writeln!(out, "  Direct callers: {}", d).unwrap();
            }
        }
        "temporal-violation" => {
            let expected = finding
                .evidence
                .get("expected_order")
                .and_then(|v| v.as_array());
            let actual = finding
                .evidence
                .get("actual_order")
                .and_then(|v| v.as_array());
            if let Some(exp) = expected {
                let items: Vec<&str> = exp.iter().filter_map(|v| v.as_str()).collect();
                if !items.is_empty() {
                    writeln!(out, "  Expected order: {}", items.join(" -> ")).unwrap();
                }
            }
            if let Some(act) = actual {
                let items: Vec<&str> = act.iter().filter_map(|v| v.as_str()).collect();
                if !items.is_empty() {
                    writeln!(out, "  Actual order: {}", items.join(" -> ")).unwrap();
                }
            }
        }
        "guard-removed" => {
            let variable = finding
                .evidence
                .get("removed_variable")
                .and_then(|v| v.as_str());
            let constraint = finding
                .evidence
                .get("removed_constraint")
                .and_then(|v| v.as_str());
            if let (Some(var), Some(con)) = (variable, constraint) {
                writeln!(out, "  Removed guard: {} {}", var, con).unwrap();
            } else {
                format_generic_evidence(out, &finding.evidence);
            }
        }
        "contract-regression" => {
            let category = finding.evidence.get("category").and_then(|v| v.as_str());
            let variable = finding
                .evidence
                .get("removed_variable")
                .and_then(|v| v.as_str());
            let constraint = finding
                .evidence
                .get("removed_constraint")
                .and_then(|v| v.as_str());
            if let (Some(cat), Some(var), Some(con)) = (category, variable, constraint) {
                writeln!(out, "  Removed {}: {} {}", cat, var, con).unwrap();
            } else {
                format_generic_evidence(out, &finding.evidence);
            }
        }
        _ => {
            format_generic_evidence(out, &finding.evidence);
        }
    }
}

/// Format evidence generically by showing all non-null values from a JSON object.
///
/// Handles strings, numbers (integer and float), booleans, and arrays of strings.
/// Nested objects are shown as compact JSON. Null values are skipped.
fn format_generic_evidence(out: &mut String, evidence: &serde_json::Value) {
    if let Some(obj) = evidence.as_object() {
        for (key, value) in obj {
            if value.is_null() {
                continue;
            }
            if let Some(s) = value.as_str() {
                writeln!(out, "  {}: {}", key, s).unwrap();
            } else if let Some(n) = value.as_u64() {
                writeln!(out, "  {}: {}", key, n).unwrap();
            } else if let Some(n) = value.as_i64() {
                writeln!(out, "  {}: {}", key, n).unwrap();
            } else if let Some(n) = value.as_f64() {
                // Avoid trailing zeros for clean display
                if n.fract() == 0.0 {
                    writeln!(out, "  {}: {}", key, n as i64).unwrap();
                } else {
                    writeln!(out, "  {}: {}", key, n).unwrap();
                }
            } else if let Some(b) = value.as_bool() {
                writeln!(out, "  {}: {}", key, b).unwrap();
            } else if let Some(arr) = value.as_array() {
                let items: Vec<String> = arr
                    .iter()
                    .map(|v| {
                        if let Some(s) = v.as_str() {
                            s.to_string()
                        } else {
                            v.to_string()
                        }
                    })
                    .collect();
                writeln!(out, "  {}: {}", key, items.join(", ")).unwrap();
            } else if value.is_object() {
                // Nested objects: show as compact JSON
                writeln!(out, "  {}: {}", key, value).unwrap();
            }
        }
    }
}

/// Format engine status label for display.
///
/// Returns a short lowercase status string: "complete", "partial", "skipped",
/// or "timed out" based on the engine result's success flag and status string.
fn format_engine_status(result: &L2AnalyzerResult) -> String {
    if result.success {
        "complete".to_string()
    } else if result.status.starts_with("partial") || result.status.starts_with("Partial") {
        "partial".to_string()
    } else if result.status.starts_with("skipped") || result.status.starts_with("Skipped") {
        "skipped".to_string()
    } else if result.status.contains("timed out") || result.status.contains("TimedOut") {
        "timed out".to_string()
    } else {
        "failed".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::bugbot::types::{BugbotFinding, BugbotSummary};
    use std::collections::HashMap;
    use std::path::PathBuf;

    /// Build a minimal report with no findings for testing.
    fn empty_report() -> BugbotCheckReport {
        BugbotCheckReport {
            tool: "bugbot".to_string(),
            mode: "check".to_string(),
            language: "rust".to_string(),
            base_ref: "HEAD".to_string(),
            detection_method: "git:uncommitted".to_string(),
            timestamp: "2026-02-25T00:00:00Z".to_string(),
            changed_files: Vec::new(),
            findings: Vec::new(),
            summary: BugbotSummary {
                total_findings: 0,
                by_severity: HashMap::new(),
                by_type: HashMap::new(),
                files_analyzed: 3,
                functions_analyzed: 12,
                l1_findings: 0,
                l2_findings: 0,
                tools_run: 0,
                tools_failed: 0,
            },
            elapsed_ms: 42,
            errors: Vec::new(),
            notes: Vec::new(),
            tool_results: vec![],
            tools_available: vec![],
            tools_missing: vec![],
            l2_engine_results: vec![],
        }
    }

    /// Build a signature-regression finding with before/after evidence.
    fn signature_finding() -> BugbotFinding {
        BugbotFinding {
            finding_type: "signature-regression".to_string(),
            severity: "high".to_string(),
            file: PathBuf::from("src/lib.rs"),
            function: "compute".to_string(),
            line: 10,
            message: "parameter removed from public function".to_string(),
            evidence: serde_json::json!({
                "before_signature": "fn compute(x: i32, y: i32) -> i32",
                "after_signature": "fn compute(x: i32) -> i32",
                "changes": [{"change_type": "param_removed", "detail": "y: i32"}]
            }),
            confidence: None,
            finding_id: None,
        }
    }

    /// Build a born-dead finding with no evidence.
    fn born_dead_finding() -> BugbotFinding {
        BugbotFinding {
            finding_type: "born-dead".to_string(),
            severity: "low".to_string(),
            file: PathBuf::from("src/utils.rs"),
            function: "unused_helper".to_string(),
            line: 25,
            message: "function has no callers in the project".to_string(),
            evidence: serde_json::Value::Null,
            confidence: None,
            finding_id: None,
        }
    }

    #[test]
    fn test_text_format_no_findings() {
        let report = empty_report();
        let output = format_bugbot_text(&report);

        assert!(
            output.contains("no issues found"),
            "Expected 'no issues found' in output, got: {}",
            output
        );
        assert!(
            output.contains("3 files analyzed"),
            "Expected '3 files analyzed' in output, got: {}",
            output
        );
        assert!(
            output.contains("12 functions"),
            "Expected '12 functions' in output, got: {}",
            output
        );
        assert!(
            output.contains("42ms"),
            "Expected '42ms' in output, got: {}",
            output
        );
    }

    #[test]
    fn test_text_format_summary_line() {
        let mut report = empty_report();
        report.findings = vec![
            signature_finding(),
            signature_finding(),
            born_dead_finding(),
        ];
        report.summary = BugbotSummary {
            total_findings: 3,
            by_severity: {
                let mut m = HashMap::new();
                m.insert("high".to_string(), 2);
                m.insert("low".to_string(), 1);
                m
            },
            by_type: HashMap::new(),
            files_analyzed: 5,
            functions_analyzed: 20,
            l1_findings: 0,
            l2_findings: 0,
            tools_run: 0,
            tools_failed: 0,
        };

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("3 findings"),
            "Expected '3 findings' in output, got: {}",
            output
        );
        assert!(
            output.contains("2 high"),
            "Expected '2 high' in output, got: {}",
            output
        );
        assert!(
            output.contains("1 low"),
            "Expected '1 low' in output, got: {}",
            output
        );
        assert!(
            output.contains("5 files analyzed"),
            "Expected '5 files analyzed' in output, got: {}",
            output
        );
    }

    #[test]
    fn test_text_format_signature_finding() {
        let mut report = empty_report();
        report.findings = vec![signature_finding()];
        report.summary = BugbotSummary {
            total_findings: 1,
            by_severity: {
                let mut m = HashMap::new();
                m.insert("high".to_string(), 1);
                m
            },
            by_type: HashMap::new(),
            files_analyzed: 1,
            functions_analyzed: 1,
            l1_findings: 0,
            l2_findings: 0,
            tools_run: 0,
            tools_failed: 0,
        };

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("Before: fn compute(x: i32, y: i32) -> i32"),
            "Expected 'Before:' line with old signature, got: {}",
            output
        );
        assert!(
            output.contains("After:  fn compute(x: i32) -> i32"),
            "Expected 'After:' line with new signature, got: {}",
            output
        );
    }

    #[test]
    fn test_text_format_born_dead_finding() {
        let mut report = empty_report();
        report.findings = vec![born_dead_finding()];
        report.summary = BugbotSummary {
            total_findings: 1,
            by_severity: {
                let mut m = HashMap::new();
                m.insert("low".to_string(), 1);
                m
            },
            by_type: HashMap::new(),
            files_analyzed: 1,
            functions_analyzed: 1,
            l1_findings: 0,
            l2_findings: 0,
            tools_run: 0,
            tools_failed: 0,
        };

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("[LOW] born-dead in src/utils.rs"),
            "Expected '[LOW] born-dead in src/utils.rs', got: {}",
            output
        );
        assert!(
            output.contains("unused_helper (line 25)"),
            "Expected 'unused_helper (line 25)', got: {}",
            output
        );
        assert!(
            output.contains("function has no callers in the project"),
            "Expected message text, got: {}",
            output
        );
        // born-dead should NOT have Before:/After: lines
        assert!(
            !output.contains("Before:"),
            "born-dead should not have 'Before:' line, got: {}",
            output
        );
        assert!(
            !output.contains("After:"),
            "born-dead should not have 'After:' line, got: {}",
            output
        );
    }

    #[test]
    fn test_text_format_severity_tags() {
        let mut report = empty_report();
        let mut medium_finding = born_dead_finding();
        medium_finding.severity = "medium".to_string();
        medium_finding.file = PathBuf::from("src/mid.rs");

        report.findings = vec![
            signature_finding(), // high
            medium_finding,      // medium
            born_dead_finding(), // low
        ];
        report.summary = BugbotSummary {
            total_findings: 3,
            by_severity: {
                let mut m = HashMap::new();
                m.insert("high".to_string(), 1);
                m.insert("medium".to_string(), 1);
                m.insert("low".to_string(), 1);
                m
            },
            by_type: HashMap::new(),
            files_analyzed: 3,
            functions_analyzed: 3,
            l1_findings: 0,
            l2_findings: 0,
            tools_run: 0,
            tools_failed: 0,
        };

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("[HIGH]"),
            "Expected [HIGH] tag, got: {}",
            output
        );
        assert!(
            output.contains("[MEDIUM]"),
            "Expected [MEDIUM] tag, got: {}",
            output
        );
        assert!(
            output.contains("[LOW]"),
            "Expected [LOW] tag, got: {}",
            output
        );
    }

    #[test]
    fn test_text_format_errors_section() {
        let mut report = empty_report();
        report.errors = vec![
            "diff failed for src/a.rs: parse error".to_string(),
            "baseline error for src/b.rs: git show failed".to_string(),
        ];

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("errors:"),
            "Expected 'errors:' section header, got: {}",
            output
        );
        assert!(
            output.contains("  - diff failed for src/a.rs: parse error"),
            "Expected first error line, got: {}",
            output
        );
        assert!(
            output.contains("  - baseline error for src/b.rs: git show failed"),
            "Expected second error line, got: {}",
            output
        );
    }

    #[test]
    fn test_text_format_truncation_note() {
        let mut report = empty_report();
        report.notes = vec!["truncated_to_10".to_string()];

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("(output truncated to 10 findings)"),
            "Expected truncation message, got: {}",
            output
        );
    }

    // ===================================================================
    // Phase 6: L1 integration tests
    // ===================================================================

    #[test]
    fn test_text_format_empty_function_renders_file_line_only() {
        // PM-4: L1 findings have empty function field.
        // Should render "file:line" instead of " (line N)"
        let mut report = empty_report();
        report.findings = vec![BugbotFinding {
            finding_type: "tool:clippy".to_string(),
            severity: "medium".to_string(),
            file: PathBuf::from("src/main.rs"),
            function: String::new(), // empty function from L1
            line: 42,
            message: "unused variable `x`".to_string(),
            evidence: serde_json::json!({
                "tool": "clippy",
                "category": "Linter",
                "code": "clippy::unused_variables",
            }),
            confidence: None,
            finding_id: None,
        }];
        report.summary = BugbotSummary {
            total_findings: 1,
            by_severity: {
                let mut m = HashMap::new();
                m.insert("medium".to_string(), 1);
                m
            },
            by_type: HashMap::new(),
            files_analyzed: 1,
            functions_analyzed: 0,
            l1_findings: 1,
            l2_findings: 0,
            tools_run: 1,
            tools_failed: 0,
        };

        let output = format_bugbot_text(&report);

        // Should show "line 42" but NOT " (line 42)" preceded by empty function
        assert!(
            output.contains("line 42"),
            "Expected 'line 42' in output, got: {}",
            output
        );
        // Should NOT have leading space before "(line" when function is empty
        assert!(
            !output.contains("  (line 42)"),
            "PM-4: empty function should not render as '  (line 42)', got: {}",
            output
        );
        // Should contain the file path
        assert!(
            output.contains("src/main.rs"),
            "Expected file path in output, got: {}",
            output
        );
    }

    #[test]
    fn test_text_format_nonempty_function_unchanged() {
        // Non-empty function should still render as "function (line N)"
        let mut report = empty_report();
        report.findings = vec![BugbotFinding {
            finding_type: "born-dead".to_string(),
            severity: "low".to_string(),
            file: PathBuf::from("src/lib.rs"),
            function: "my_function".to_string(),
            line: 10,
            message: "no callers".to_string(),
            evidence: serde_json::Value::Null,
            confidence: None,
            finding_id: None,
        }];
        report.summary = BugbotSummary {
            total_findings: 1,
            by_severity: {
                let mut m = HashMap::new();
                m.insert("low".to_string(), 1);
                m
            },
            by_type: HashMap::new(),
            files_analyzed: 1,
            functions_analyzed: 1,
            l1_findings: 0,
            l2_findings: 1,
            tools_run: 0,
            tools_failed: 0,
        };

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("my_function (line 10)"),
            "Non-empty function should render as 'my_function (line 10)', got: {}",
            output
        );
    }

    #[test]
    fn test_text_format_tool_results_section() {
        // Tool results section should appear when tools were run
        let mut report = empty_report();
        report.tool_results = vec![
            crate::commands::bugbot::tools::ToolResult {
                name: "clippy".to_string(),
                category: crate::commands::bugbot::tools::ToolCategory::Linter,
                success: true,
                duration_ms: 1500,
                finding_count: 3,
                error: None,
                exit_code: Some(0),
            },
            crate::commands::bugbot::tools::ToolResult {
                name: "cargo-audit".to_string(),
                category: crate::commands::bugbot::tools::ToolCategory::SecurityScanner,
                success: false,
                duration_ms: 200,
                finding_count: 0,
                error: Some("Parse error: invalid JSON".to_string()),
                exit_code: Some(1),
            },
        ];
        report.tools_missing = vec!["pyright".to_string()];

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("tools:"),
            "Expected 'tools:' section header, got: {}",
            output
        );
        assert!(
            output.contains("clippy"),
            "Expected clippy in tool results, got: {}",
            output
        );
        assert!(
            output.contains("cargo-audit"),
            "Expected cargo-audit in tool results, got: {}",
            output
        );
        // Failed tool should show status
        assert!(
            output.contains("failed"),
            "Expected 'failed' status for cargo-audit, got: {}",
            output
        );
        // Missing tools should be listed
        assert!(
            output.contains("pyright"),
            "Expected missing tool 'pyright' in output, got: {}",
            output
        );
    }

    #[test]
    fn test_text_format_no_tool_results_no_section() {
        // When no tools were run, the tool results section should not appear
        let report = empty_report();
        let output = format_bugbot_text(&report);

        assert!(
            !output.contains("tools:"),
            "Should not have 'tools:' section when no tools ran, got: {}",
            output
        );
    }

    // ===================================================================
    // Phase 8: Integration & Polish tests
    // ===================================================================

    #[test]
    fn test_text_format_critical_finding_marker() {
        // Critical findings should use [!!!CRITICAL] marker instead of [CRITICAL]
        let mut report = empty_report();
        report.findings = vec![BugbotFinding {
            finding_type: "secret-exposed".to_string(),
            severity: "critical".to_string(),
            file: PathBuf::from("src/config.rs"),
            function: "load_config".to_string(),
            line: 42,
            message: "API key exposed in source code".to_string(),
            evidence: serde_json::json!({
                "masked_value": "sk-****REDACTED****",
            }),
            confidence: Some("CONFIRMED".to_string()),
            finding_id: None,
        }];
        report.summary = BugbotSummary {
            total_findings: 1,
            by_severity: {
                let mut m = HashMap::new();
                m.insert("critical".to_string(), 1);
                m
            },
            by_type: HashMap::new(),
            files_analyzed: 1,
            functions_analyzed: 1,
            l1_findings: 0,
            l2_findings: 1,
            tools_run: 0,
            tools_failed: 0,
        };

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("[!!!CRITICAL]"),
            "Critical findings should use [!!!CRITICAL] marker, got: {}",
            output
        );
        assert!(
            !output.contains("[CRITICAL]") || output.contains("[!!!CRITICAL]"),
            "Should not have bare [CRITICAL] without !!! prefix, got: {}",
            output
        );
    }

    #[test]
    fn test_text_format_confidence_display() {
        // L2 findings with confidence should show "Confidence: VALUE" line
        let mut report = empty_report();
        report.findings = vec![BugbotFinding {
            finding_type: "taint-flow".to_string(),
            severity: "high".to_string(),
            file: PathBuf::from("src/api.rs"),
            function: "handle_request".to_string(),
            line: 15,
            message: "Unsanitized input reaches SQL query".to_string(),
            evidence: serde_json::json!({
                "source": "request.query",
                "sink": "db.execute()",
            }),
            confidence: Some("POSSIBLE".to_string()),
            finding_id: None,
        }];
        report.summary = BugbotSummary {
            total_findings: 1,
            by_severity: {
                let mut m = HashMap::new();
                m.insert("high".to_string(), 1);
                m
            },
            by_type: HashMap::new(),
            files_analyzed: 1,
            functions_analyzed: 1,
            l1_findings: 0,
            l2_findings: 1,
            tools_run: 0,
            tools_failed: 0,
        };

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("Confidence: POSSIBLE"),
            "L2 findings with confidence should show 'Confidence: POSSIBLE', got: {}",
            output
        );
    }

    #[test]
    fn test_text_format_no_confidence_for_l1() {
        // L1 findings (confidence=None) should NOT show confidence line
        let mut report = empty_report();
        report.findings = vec![BugbotFinding {
            finding_type: "tool:clippy".to_string(),
            severity: "medium".to_string(),
            file: PathBuf::from("src/main.rs"),
            function: String::new(),
            line: 10,
            message: "unused variable".to_string(),
            evidence: serde_json::Value::Null,
            confidence: None,
            finding_id: None,
        }];
        report.summary = BugbotSummary {
            total_findings: 1,
            by_severity: {
                let mut m = HashMap::new();
                m.insert("medium".to_string(), 1);
                m
            },
            by_type: HashMap::new(),
            files_analyzed: 1,
            functions_analyzed: 0,
            l1_findings: 1,
            l2_findings: 0,
            tools_run: 1,
            tools_failed: 0,
        };

        let output = format_bugbot_text(&report);

        assert!(
            !output.contains("Confidence:"),
            "L1 findings should not show Confidence line, got: {}",
            output
        );
    }

    #[test]
    fn test_text_format_l2_engine_results_section() {
        // L2 engine results should appear in output when present
        use crate::commands::bugbot::types::L2AnalyzerResult;

        let mut report = empty_report();
        report.l2_engine_results = vec![L2AnalyzerResult {
            name: "TldrDifferentialEngine".to_string(),
            success: true,
            duration_ms: 23,
            finding_count: 5,
            functions_analyzed: 10,
            functions_skipped: 0,
            status: "Complete".to_string(),
            errors: vec![],
        }];

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("L2 engines:"),
            "Expected 'L2 engines:' section header, got: {}",
            output
        );
        assert!(
            output.contains("TldrDifferentialEngine"),
            "Expected TldrDifferentialEngine in L2 results, got: {}",
            output
        );
    }

    #[test]
    fn test_text_format_analysis_gaps_section() {
        // When an engine has errors, ANALYSIS GAPS section should appear
        use crate::commands::bugbot::types::L2AnalyzerResult;

        let mut report = empty_report();
        report.l2_engine_results = vec![L2AnalyzerResult {
            name: "DeltaEngine".to_string(),
            success: false,
            duration_ms: 500,
            finding_count: 2,
            functions_analyzed: 5,
            functions_skipped: 3,
            status: "Partial: analysis incomplete".to_string(),
            errors: vec!["Failed to read baseline for src/macro.rs".to_string()],
        }];

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("ANALYSIS GAPS"),
            "Expected 'ANALYSIS GAPS' section when engine has errors, got: {}",
            output
        );
        assert!(
            output.contains("DeltaEngine"),
            "Expected DeltaEngine in analysis gaps, got: {}",
            output
        );
        assert!(
            output.contains("Failed to read baseline"),
            "Expected error detail in analysis gaps, got: {}",
            output
        );
    }

    #[test]
    fn test_text_format_no_analysis_gaps_when_all_ok() {
        // When no engines have errors, ANALYSIS GAPS should not appear
        use crate::commands::bugbot::types::L2AnalyzerResult;

        let mut report = empty_report();
        report.l2_engine_results = vec![L2AnalyzerResult {
            name: "DeltaEngine".to_string(),
            success: true,
            duration_ms: 23,
            finding_count: 5,
            functions_analyzed: 10,
            functions_skipped: 0,
            status: "Complete".to_string(),
            errors: vec![],
        }];

        let output = format_bugbot_text(&report);

        assert!(
            !output.contains("ANALYSIS GAPS"),
            "Should not have 'ANALYSIS GAPS' when all engines succeeded, got: {}",
            output
        );
    }

    #[test]
    fn test_text_format_critical_summary_line() {
        // When critical findings exist, a summary line should appear
        let mut report = empty_report();
        report.findings = vec![
            BugbotFinding {
                finding_type: "secret-exposed".to_string(),
                severity: "critical".to_string(),
                file: PathBuf::from("src/config.rs"),
                function: "load".to_string(),
                line: 5,
                message: "exposed secret".to_string(),
                evidence: serde_json::Value::Null,
                confidence: Some("CONFIRMED".to_string()),
                finding_id: None,
            },
            BugbotFinding {
                finding_type: "taint-flow".to_string(),
                severity: "critical".to_string(),
                file: PathBuf::from("src/api.rs"),
                function: "handle".to_string(),
                line: 20,
                message: "SQL injection".to_string(),
                evidence: serde_json::Value::Null,
                confidence: Some("LIKELY".to_string()),
                finding_id: None,
            },
        ];
        report.summary = BugbotSummary {
            total_findings: 2,
            by_severity: {
                let mut m = HashMap::new();
                m.insert("critical".to_string(), 2);
                m
            },
            by_type: HashMap::new(),
            files_analyzed: 2,
            functions_analyzed: 2,
            l1_findings: 0,
            l2_findings: 2,
            tools_run: 0,
            tools_failed: 0,
        };

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("CRITICAL: 2 finding(s) require immediate attention"),
            "Expected critical summary line, got: {}",
            output
        );
    }

    #[test]
    fn test_text_format_no_critical_summary_without_critical() {
        // When no critical findings exist, no critical summary line
        let mut report = empty_report();
        report.findings = vec![signature_finding()]; // high, not critical
        report.summary = BugbotSummary {
            total_findings: 1,
            by_severity: {
                let mut m = HashMap::new();
                m.insert("high".to_string(), 1);
                m
            },
            by_type: HashMap::new(),
            files_analyzed: 1,
            functions_analyzed: 1,
            l1_findings: 0,
            l2_findings: 1,
            tools_run: 0,
            tools_failed: 0,
        };

        let output = format_bugbot_text(&report);

        assert!(
            !output.contains("CRITICAL:"),
            "Should not have CRITICAL summary line without critical findings, got: {}",
            output
        );
    }

    #[test]
    fn test_text_format_critical_in_severity_breakdown() {
        // "critical" should appear in severity breakdown before "high"
        let mut report = empty_report();
        report.findings = vec![
            BugbotFinding {
                finding_type: "secret-exposed".to_string(),
                severity: "critical".to_string(),
                file: PathBuf::from("src/a.rs"),
                function: "a".to_string(),
                line: 1,
                message: "secret".to_string(),
                evidence: serde_json::Value::Null,
                confidence: None,
                finding_id: None,
            },
            signature_finding(), // high severity
        ];
        report.summary = BugbotSummary {
            total_findings: 2,
            by_severity: {
                let mut m = HashMap::new();
                m.insert("critical".to_string(), 1);
                m.insert("high".to_string(), 1);
                m
            },
            by_type: HashMap::new(),
            files_analyzed: 2,
            functions_analyzed: 2,
            l1_findings: 0,
            l2_findings: 2,
            tools_run: 0,
            tools_failed: 0,
        };

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("1 critical"),
            "Expected '1 critical' in severity breakdown, got: {}",
            output
        );
        assert!(
            output.contains("1 high"),
            "Expected '1 high' in severity breakdown, got: {}",
            output
        );
        // critical should come before high in breakdown
        let crit_pos = output.find("1 critical").unwrap();
        let high_pos = output.find("1 high").unwrap();
        assert!(
            crit_pos < high_pos,
            "critical ({}) should appear before high ({}) in breakdown, got: {}",
            crit_pos,
            high_pos,
            output
        );
    }

    #[test]
    fn test_text_format_secret_exposed_evidence() {
        // secret-exposed findings should show masked_value from evidence
        let mut report = empty_report();
        report.findings = vec![BugbotFinding {
            finding_type: "secret-exposed".to_string(),
            severity: "high".to_string(),
            file: PathBuf::from("src/config.rs"),
            function: String::new(),
            line: 10,
            message: "Exposed secret: AWS_KEY".to_string(),
            evidence: serde_json::json!({
                "pattern": "AWS_KEY",
                "masked_value": "AKIA****REDACTED",
            }),
            confidence: Some("POSSIBLE".to_string()),
            finding_id: None,
        }];
        report.summary = BugbotSummary {
            total_findings: 1,
            by_severity: {
                let mut m = HashMap::new();
                m.insert("high".to_string(), 1);
                m
            },
            by_type: HashMap::new(),
            files_analyzed: 1,
            functions_analyzed: 0,
            l1_findings: 0,
            l2_findings: 1,
            tools_run: 0,
            tools_failed: 0,
        };

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("Value: AKIA****REDACTED"),
            "secret-exposed should show 'Value: <masked_value>', got: {}",
            output
        );
    }

    #[test]
    fn test_text_format_taint_flow_evidence() {
        // taint-flow findings should show Source -> Sink path
        let mut report = empty_report();
        report.findings = vec![BugbotFinding {
            finding_type: "taint-flow".to_string(),
            severity: "high".to_string(),
            file: PathBuf::from("src/api.rs"),
            function: "handle_request".to_string(),
            line: 15,
            message: "Unsanitized input reaches SQL query".to_string(),
            evidence: serde_json::json!({
                "source": "request.query",
                "sink": "db.execute()",
            }),
            confidence: Some("POSSIBLE".to_string()),
            finding_id: None,
        }];
        report.summary = BugbotSummary {
            total_findings: 1,
            by_severity: {
                let mut m = HashMap::new();
                m.insert("high".to_string(), 1);
                m
            },
            by_type: HashMap::new(),
            files_analyzed: 1,
            functions_analyzed: 1,
            l1_findings: 0,
            l2_findings: 1,
            tools_run: 0,
            tools_failed: 0,
        };

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("request.query"),
            "taint-flow should show source, got: {}",
            output
        );
        assert!(
            output.contains("db.execute()"),
            "taint-flow should show sink, got: {}",
            output
        );
    }

    #[test]
    fn test_text_format_info_severity_in_breakdown() {
        // "info" severity should appear in the severity breakdown
        let mut report = empty_report();
        report.findings = vec![BugbotFinding {
            finding_type: "tool:clippy".to_string(),
            severity: "info".to_string(),
            file: PathBuf::from("src/main.rs"),
            function: String::new(),
            line: 1,
            message: "informational note".to_string(),
            evidence: serde_json::Value::Null,
            confidence: None,
            finding_id: None,
        }];
        report.summary = BugbotSummary {
            total_findings: 1,
            by_severity: {
                let mut m = HashMap::new();
                m.insert("info".to_string(), 1);
                m
            },
            by_type: HashMap::new(),
            files_analyzed: 1,
            functions_analyzed: 0,
            l1_findings: 1,
            l2_findings: 0,
            tools_run: 1,
            tools_failed: 0,
        };

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("1 info"),
            "Expected '1 info' in severity breakdown, got: {}",
            output
        );
        assert!(
            output.contains("[INFO]"),
            "Expected '[INFO]' tag on finding, got: {}",
            output
        );
    }

    // ===================================================================
    // Evidence rendering coverage for all 24 L2 finding types
    // ===================================================================

    /// Helper: build a single-finding report for testing evidence rendering.
    fn single_finding_report(finding: BugbotFinding) -> BugbotCheckReport {
        let mut report = empty_report();
        let severity = finding.severity.clone();
        report.findings = vec![finding];
        report.summary = BugbotSummary {
            total_findings: 1,
            by_severity: {
                let mut m = HashMap::new();
                m.insert(severity, 1);
                m
            },
            by_type: HashMap::new(),
            files_analyzed: 1,
            functions_analyzed: 1,
            l1_findings: 0,
            l2_findings: 1,
            tools_run: 0,
            tools_failed: 0,
        };
        report
    }

    // --- #17 taint-flow: evidence with actual production keys ---

    #[test]
    fn test_text_format_taint_flow_production_evidence() {
        // The taint extractor produces keys: source_var, source_type,
        // sink_var, sink_type, source_line, sink_line, path_length.
        // The text formatter should render these meaningfully.
        let report = single_finding_report(BugbotFinding {
            finding_type: "taint-flow".to_string(),
            severity: "high".to_string(),
            file: PathBuf::from("src/api.rs"),
            function: "handle_request".to_string(),
            line: 15,
            message: "Taint flow detected".to_string(),
            evidence: serde_json::json!({
                "source_var": "user_input",
                "source_line": 5,
                "source_type": "UserInput",
                "sink_var": "query",
                "sink_line": 15,
                "sink_type": "SqlQuery",
                "path_length": 3,
            }),
            confidence: Some("POSSIBLE".to_string()),
            finding_id: None,
        });

        let output = format_bugbot_text(&report);

        // Should show source and sink variables
        assert!(
            output.contains("user_input"),
            "taint-flow should show source variable, got: {}",
            output
        );
        assert!(
            output.contains("query"),
            "taint-flow should show sink variable, got: {}",
            output
        );
        // Should show source and sink types
        assert!(
            output.contains("UserInput"),
            "taint-flow should show source type, got: {}",
            output
        );
        assert!(
            output.contains("SqlQuery"),
            "taint-flow should show sink type, got: {}",
            output
        );
    }

    // --- #22 resource-leak: sub_type and resource fields ---

    #[test]
    fn test_text_format_resource_leak_evidence() {
        let report = single_finding_report(BugbotFinding {
            finding_type: "resource-leak".to_string(),
            severity: "medium".to_string(),
            file: PathBuf::from("src/io.rs"),
            function: "process_file".to_string(),
            line: 10,
            message: "Resource not closed".to_string(),
            evidence: serde_json::json!({
                "sub_type": "leak",
                "resource": "file_handle",
                "open_line": 10,
                "paths": 2,
            }),
            confidence: Some("POSSIBLE".to_string()),
            finding_id: None,
        });

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("leak"),
            "resource-leak should show sub_type, got: {}",
            output
        );
        assert!(
            output.contains("file_handle"),
            "resource-leak should show resource name, got: {}",
            output
        );
    }

    #[test]
    fn test_text_format_resource_leak_double_close() {
        let report = single_finding_report(BugbotFinding {
            finding_type: "resource-leak".to_string(),
            severity: "high".to_string(),
            file: PathBuf::from("src/io.rs"),
            function: "cleanup".to_string(),
            line: 25,
            message: "Resource closed twice".to_string(),
            evidence: serde_json::json!({
                "sub_type": "double-close",
                "resource": "db_conn",
                "first_close_line": 20,
                "second_close_line": 25,
            }),
            confidence: Some("LIKELY".to_string()),
            finding_id: None,
        });

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("double-close"),
            "resource-leak should show sub_type 'double-close', got: {}",
            output
        );
        assert!(
            output.contains("db_conn"),
            "resource-leak should show resource name, got: {}",
            output
        );
    }

    #[test]
    fn test_text_format_resource_leak_use_after_close() {
        let report = single_finding_report(BugbotFinding {
            finding_type: "resource-leak".to_string(),
            severity: "high".to_string(),
            file: PathBuf::from("src/io.rs"),
            function: "read_after_close".to_string(),
            line: 30,
            message: "Resource used after close".to_string(),
            evidence: serde_json::json!({
                "sub_type": "use-after-close",
                "resource": "socket",
                "close_line": 25,
                "use_line": 30,
            }),
            confidence: Some("LIKELY".to_string()),
            finding_id: None,
        });

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("use-after-close"),
            "resource-leak should show sub_type 'use-after-close', got: {}",
            output
        );
        assert!(
            output.contains("socket"),
            "resource-leak should show resource name, got: {}",
            output
        );
    }

    // --- #9 impact-blast-radius: numeric caller counts ---

    #[test]
    fn test_text_format_impact_blast_radius_evidence() {
        let report = single_finding_report(BugbotFinding {
            finding_type: "impact-blast-radius".to_string(),
            severity: "info".to_string(),
            file: PathBuf::from("src/core.rs"),
            function: "compute".to_string(),
            line: 10,
            message: "Function has wide impact".to_string(),
            evidence: serde_json::json!({
                "total_callers": 15,
                "direct_callers": 5,
            }),
            confidence: Some("POSSIBLE".to_string()),
            finding_id: None,
        });

        let output = format_bugbot_text(&report);

        // Should display caller counts (these are u64, generic handler drops them)
        assert!(
            output.contains("15"),
            "impact-blast-radius should show total_callers count, got: {}",
            output
        );
        assert!(
            output.contains("5"),
            "impact-blast-radius should show direct_callers count, got: {}",
            output
        );
    }

    // --- #23 temporal-violation: expected/actual order ---

    #[test]
    fn test_text_format_temporal_violation_evidence() {
        let report = single_finding_report(BugbotFinding {
            finding_type: "temporal-violation".to_string(),
            severity: "medium".to_string(),
            file: PathBuf::from("src/db.rs"),
            function: "process".to_string(),
            line: 20,
            message: "'open' should be called before 'query'".to_string(),
            evidence: serde_json::json!({
                "expected_order": ["open", "query"],
                "actual_order": ["query", "open"],
                "confidence": 0.85,
                "support": 12,
            }),
            confidence: Some("POSSIBLE".to_string()),
            finding_id: None,
        });

        let output = format_bugbot_text(&report);

        // Should show expected order
        assert!(
            output.contains("open"),
            "temporal-violation should show expected order, got: {}",
            output
        );
        assert!(
            output.contains("query"),
            "temporal-violation should show expected order, got: {}",
            output
        );
    }

    // --- #3 new-clone: similarity percentage ---

    #[test]
    fn test_text_format_new_clone_evidence() {
        let report = single_finding_report(BugbotFinding {
            finding_type: "new-clone".to_string(),
            severity: "medium".to_string(),
            file: PathBuf::from("src/utils.rs"),
            function: "helper".to_string(),
            line: 10,
            message: "New code clone detected".to_string(),
            evidence: serde_json::json!({
                "clone_type": "Type2",
                "similarity": 0.92,
                "fragment1": {
                    "file": "src/utils.rs",
                    "start_line": 10,
                    "end_line": 25,
                },
                "fragment2": {
                    "file": "src/other.rs",
                    "start_line": 30,
                    "end_line": 45,
                },
            }),
            confidence: Some("POSSIBLE".to_string()),
            finding_id: None,
        });

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("92%") || output.contains("0.92"),
            "new-clone should show similarity percentage, got: {}",
            output
        );
        assert!(
            output.contains("Type2"),
            "new-clone should show clone type, got: {}",
            output
        );
    }

    // --- #20 guard-removed: removed guard details ---

    #[test]
    fn test_text_format_guard_removed_evidence() {
        let report = single_finding_report(BugbotFinding {
            finding_type: "guard-removed".to_string(),
            severity: "high".to_string(),
            file: PathBuf::from("src/validate.rs"),
            function: "check_input".to_string(),
            line: 5,
            message: "Guard removed".to_string(),
            evidence: serde_json::json!({
                "removed_variable": "input",
                "removed_constraint": "!= null",
                "confidence": "HIGH",
                "baseline_source_line": 5,
            }),
            confidence: Some("LIKELY".to_string()),
            finding_id: None,
        });

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("input"),
            "guard-removed should show removed variable, got: {}",
            output
        );
        assert!(
            output.contains("!= null"),
            "guard-removed should show removed constraint, got: {}",
            output
        );
    }

    // --- #21 contract-regression: contract details ---

    #[test]
    fn test_text_format_contract_regression_evidence() {
        let report = single_finding_report(BugbotFinding {
            finding_type: "contract-regression".to_string(),
            severity: "medium".to_string(),
            file: PathBuf::from("src/math.rs"),
            function: "divide".to_string(),
            line: 10,
            message: "Contract weakened".to_string(),
            evidence: serde_json::json!({
                "category": "postcondition",
                "removed_variable": "result",
                "removed_constraint": "> 0",
                "confidence": "HIGH",
                "baseline_source_line": 10,
            }),
            confidence: Some("LIKELY".to_string()),
            finding_id: None,
        });

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("postcondition"),
            "contract-regression should show category, got: {}",
            output
        );
        assert!(
            output.contains("result"),
            "contract-regression should show removed variable, got: {}",
            output
        );
        assert!(
            output.contains("> 0"),
            "contract-regression should show removed constraint, got: {}",
            output
        );
    }

    // --- #8 architecture-violation: directory pair ---

    #[test]
    fn test_text_format_architecture_violation_evidence() {
        let report = single_finding_report(BugbotFinding {
            finding_type: "architecture-violation".to_string(),
            severity: "medium".to_string(),
            file: PathBuf::from("src/api"),
            function: String::new(),
            line: 0,
            message: "Circular dependency".to_string(),
            evidence: serde_json::json!({
                "dir_a": "src/api",
                "dir_b": "src/db",
            }),
            confidence: Some("POSSIBLE".to_string()),
            finding_id: None,
        });

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("src/api"),
            "architecture-violation should show dir_a, got: {}",
            output
        );
        assert!(
            output.contains("src/db"),
            "architecture-violation should show dir_b, got: {}",
            output
        );
    }

    // --- #24 api-misuse: rule details and fix suggestion ---

    #[test]
    fn test_text_format_api_misuse_evidence() {
        let report = single_finding_report(BugbotFinding {
            finding_type: "api-misuse".to_string(),
            severity: "medium".to_string(),
            file: PathBuf::from("src/http.py"),
            function: String::new(),
            line: 5,
            message: "API misuse: missing timeout".to_string(),
            evidence: serde_json::json!({
                "rule_id": "PY-HTTP-001",
                "rule_name": "missing-timeout",
                "category": "Reliability",
                "api_call": "requests.get",
                "fix_suggestion": "Add timeout=30 parameter",
                "correct_usage": "requests.get(url, timeout=30)",
            }),
            confidence: Some("POSSIBLE".to_string()),
            finding_id: None,
        });

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("requests.get"),
            "api-misuse should show api_call, got: {}",
            output
        );
        assert!(
            output.contains("Add timeout=30 parameter"),
            "api-misuse should show fix_suggestion, got: {}",
            output
        );
    }

    // --- #5 complexity-increase: before/after with delta ---

    #[test]
    fn test_text_format_complexity_increase_evidence() {
        let report = single_finding_report(BugbotFinding {
            finding_type: "complexity-increase".to_string(),
            severity: "medium".to_string(),
            file: PathBuf::from("src/parser.rs"),
            function: "parse_expr".to_string(),
            line: 50,
            message: "Complexity increased".to_string(),
            evidence: serde_json::json!({
                "before": 8,
                "after": 15,
            }),
            confidence: Some("POSSIBLE".to_string()),
            finding_id: None,
        });

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("8"),
            "complexity-increase should show before value, got: {}",
            output
        );
        assert!(
            output.contains("15"),
            "complexity-increase should show after value, got: {}",
            output
        );
        assert!(
            output.contains("Complexity:"),
            "complexity-increase should show 'Complexity:' label, got: {}",
            output
        );
    }

    // --- #15 div-by-zero: variable name ---

    #[test]
    fn test_text_format_div_by_zero_evidence() {
        let report = single_finding_report(BugbotFinding {
            finding_type: "div-by-zero".to_string(),
            severity: "high".to_string(),
            file: PathBuf::from("src/math.rs"),
            function: "average".to_string(),
            line: 8,
            message: "Potential division by zero".to_string(),
            evidence: serde_json::json!({
                "variable": "count",
                "line": 8,
            }),
            confidence: Some("POSSIBLE".to_string()),
            finding_id: None,
        });

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("count"),
            "div-by-zero should show variable name, got: {}",
            output
        );
    }

    // --- #16 null-deref: variable name ---

    #[test]
    fn test_text_format_null_deref_evidence() {
        let report = single_finding_report(BugbotFinding {
            finding_type: "null-deref".to_string(),
            severity: "high".to_string(),
            file: PathBuf::from("src/api.py"),
            function: "get_user".to_string(),
            line: 12,
            message: "Potential null dereference".to_string(),
            evidence: serde_json::json!({
                "variable": "user",
                "line": 12,
            }),
            confidence: Some("POSSIBLE".to_string()),
            finding_id: None,
        });

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("user"),
            "null-deref should show variable name, got: {}",
            output
        );
    }

    // --- #12 dead-store: variable name ---

    #[test]
    fn test_text_format_dead_store_evidence() {
        let report = single_finding_report(BugbotFinding {
            finding_type: "dead-store".to_string(),
            severity: "low".to_string(),
            file: PathBuf::from("src/calc.rs"),
            function: "compute".to_string(),
            line: 7,
            message: "Dead store: variable never read".to_string(),
            evidence: serde_json::json!({
                "variable": "temp",
                "def_line": 7,
            }),
            confidence: Some("POSSIBLE".to_string()),
            finding_id: None,
        });

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("temp"),
            "dead-store should show variable name, got: {}",
            output
        );
    }

    // --- #14 redundant-computation: original and redundant text ---

    #[test]
    fn test_text_format_redundant_computation_evidence() {
        let report = single_finding_report(BugbotFinding {
            finding_type: "redundant-computation".to_string(),
            severity: "low".to_string(),
            file: PathBuf::from("src/calc.rs"),
            function: "process".to_string(),
            line: 20,
            message: "Redundant computation".to_string(),
            evidence: serde_json::json!({
                "original_line": 10,
                "original_text": "a + b",
                "redundant_line": 20,
                "redundant_text": "a + b",
                "reason": "same_expression",
            }),
            confidence: Some("POSSIBLE".to_string()),
            finding_id: None,
        });

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("a + b"),
            "redundant-computation should show expression text, got: {}",
            output
        );
        assert!(
            output.contains("same_expression"),
            "redundant-computation should show reason, got: {}",
            output
        );
    }

    // --- #4 new-smell: smell type and reason ---

    #[test]
    fn test_text_format_new_smell_evidence() {
        let report = single_finding_report(BugbotFinding {
            finding_type: "new-smell".to_string(),
            severity: "low".to_string(),
            file: PathBuf::from("src/service.rs"),
            function: "handle_all".to_string(),
            line: 1,
            message: "New code smell detected".to_string(),
            evidence: serde_json::json!({
                "smell_type": "LongMethod",
                "reason": "Method has 150 lines, exceeds threshold of 50",
                "severity_level": 3,
            }),
            confidence: Some("POSSIBLE".to_string()),
            finding_id: None,
        });

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("LongMethod"),
            "new-smell should show smell_type, got: {}",
            output
        );
        assert!(
            output.contains("150 lines"),
            "new-smell should show reason, got: {}",
            output
        );
    }

    // --- #13 uninitialized-use: variable name ---

    #[test]
    fn test_text_format_uninitialized_use_evidence() {
        let report = single_finding_report(BugbotFinding {
            finding_type: "uninitialized-use".to_string(),
            severity: "high".to_string(),
            file: PathBuf::from("src/parser.rs"),
            function: "parse".to_string(),
            line: 15,
            message: "Variable may be used before initialization".to_string(),
            evidence: serde_json::json!({
                "variable": "result",
                "def_line": 15,
            }),
            confidence: Some("POSSIBLE".to_string()),
            finding_id: None,
        });

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("result"),
            "uninitialized-use should show variable name, got: {}",
            output
        );
    }

    // --- #10 unreachable-code: evidence ---

    #[test]
    fn test_text_format_unreachable_code_evidence() {
        let report = single_finding_report(BugbotFinding {
            finding_type: "unreachable-code".to_string(),
            severity: "low".to_string(),
            file: PathBuf::from("src/utils.rs"),
            function: "helper".to_string(),
            line: 30,
            message: "Code after return is unreachable".to_string(),
            evidence: serde_json::json!({
                "reason": "code_after_return",
                "block_id": 3,
            }),
            confidence: Some("POSSIBLE".to_string()),
            finding_id: None,
        });

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("code_after_return"),
            "unreachable-code should show reason, got: {}",
            output
        );
    }

    // --- #11 sccp-dead-code: evidence ---

    #[test]
    fn test_text_format_sccp_dead_code_evidence() {
        let report = single_finding_report(BugbotFinding {
            finding_type: "sccp-dead-code".to_string(),
            severity: "low".to_string(),
            file: PathBuf::from("src/cond.rs"),
            function: "check".to_string(),
            line: 20,
            message: "Branch is dead: condition is always false".to_string(),
            evidence: serde_json::json!({
                "condition": "x > 100",
                "resolved_value": "false",
            }),
            confidence: Some("POSSIBLE".to_string()),
            finding_id: None,
        });

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("x > 100"),
            "sccp-dead-code should show condition, got: {}",
            output
        );
        assert!(
            output.contains("false"),
            "sccp-dead-code should show resolved value, got: {}",
            output
        );
    }

    // --- #18 vulnerability: vuln type and description ---

    #[test]
    fn test_text_format_vulnerability_evidence() {
        let report = single_finding_report(BugbotFinding {
            finding_type: "vulnerability".to_string(),
            severity: "high".to_string(),
            file: PathBuf::from("src/auth.rs"),
            function: "login".to_string(),
            line: 25,
            message: "SQL injection vulnerability".to_string(),
            evidence: serde_json::json!({
                "vuln_type": "sql_injection",
                "cwe": "CWE-89",
                "description": "User input concatenated into SQL query",
            }),
            confidence: Some("POSSIBLE".to_string()),
            finding_id: None,
        });

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("sql_injection"),
            "vulnerability should show vuln_type, got: {}",
            output
        );
        assert!(
            output.contains("CWE-89"),
            "vulnerability should show CWE, got: {}",
            output
        );
    }

    // --- #19 secret-exposed: pattern and secret type ---

    #[test]
    fn test_text_format_secret_exposed_with_pattern() {
        let report = single_finding_report(BugbotFinding {
            finding_type: "secret-exposed".to_string(),
            severity: "critical".to_string(),
            file: PathBuf::from("src/config.rs"),
            function: String::new(),
            line: 3,
            message: "AWS access key exposed".to_string(),
            evidence: serde_json::json!({
                "pattern": "AWS_ACCESS_KEY",
                "masked_value": "AKIA****XXXX",
                "secret_type": "aws_key",
            }),
            confidence: Some("CONFIRMED".to_string()),
            finding_id: None,
        });

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("AKIA****XXXX"),
            "secret-exposed should show masked_value, got: {}",
            output
        );
    }

    // --- #6 maintainability-drop: before/after scores ---

    #[test]
    fn test_text_format_maintainability_drop_evidence() {
        let report = single_finding_report(BugbotFinding {
            finding_type: "maintainability-drop".to_string(),
            severity: "medium".to_string(),
            file: PathBuf::from("src/engine.rs"),
            function: "run".to_string(),
            line: 1,
            message: "Maintainability index dropped".to_string(),
            evidence: serde_json::json!({
                "before": 75,
                "after": 45,
                "threshold": 10,
            }),
            confidence: Some("POSSIBLE".to_string()),
            finding_id: None,
        });

        let output = format_bugbot_text(&report);

        // Numeric values should be displayed
        assert!(
            output.contains("75"),
            "maintainability-drop should show before score, got: {}",
            output
        );
        assert!(
            output.contains("45"),
            "maintainability-drop should show after score, got: {}",
            output
        );
    }

    // --- #2 param-renamed: old and new parameter names ---

    #[test]
    fn test_text_format_param_renamed_evidence() {
        let report = single_finding_report(BugbotFinding {
            finding_type: "param-renamed".to_string(),
            severity: "medium".to_string(),
            file: PathBuf::from("src/api.rs"),
            function: "create_user".to_string(),
            line: 10,
            message: "Parameter renamed".to_string(),
            evidence: serde_json::json!({
                "old_name": "user_name",
                "new_name": "username",
                "position": 0,
            }),
            confidence: Some("POSSIBLE".to_string()),
            finding_id: None,
        });

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("user_name"),
            "param-renamed should show old parameter name, got: {}",
            output
        );
        assert!(
            output.contains("username"),
            "param-renamed should show new parameter name, got: {}",
            output
        );
    }

    // --- Generic handler: should show numeric values too ---

    #[test]
    fn test_text_format_generic_evidence_shows_numbers() {
        // The generic fallback handler should display numeric values,
        // not just string values.
        let report = single_finding_report(BugbotFinding {
            finding_type: "some-unknown-type".to_string(),
            severity: "low".to_string(),
            file: PathBuf::from("src/test.rs"),
            function: "test_fn".to_string(),
            line: 1,
            message: "Test finding".to_string(),
            evidence: serde_json::json!({
                "string_field": "hello",
                "number_field": 42,
                "float_field": 2.5,
                "bool_field": true,
            }),
            confidence: None,
            finding_id: None,
        });

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("hello"),
            "generic should show string values, got: {}",
            output
        );
        assert!(
            output.contains("42"),
            "generic should show integer values, got: {}",
            output
        );
        assert!(
            output.contains("3.14"),
            "generic should show float values, got: {}",
            output
        );
        assert!(
            output.contains("true"),
            "generic should show boolean values, got: {}",
            output
        );
    }

    // --- Generic handler: should show array values ---

    #[test]
    fn test_text_format_generic_evidence_shows_arrays() {
        let report = single_finding_report(BugbotFinding {
            finding_type: "some-array-type".to_string(),
            severity: "low".to_string(),
            file: PathBuf::from("src/test.rs"),
            function: "test_fn".to_string(),
            line: 1,
            message: "Test finding".to_string(),
            evidence: serde_json::json!({
                "items": ["alpha", "beta", "gamma"],
            }),
            confidence: None,
            finding_id: None,
        });

        let output = format_bugbot_text(&report);

        assert!(
            output.contains("alpha"),
            "generic should show array string elements, got: {}",
            output
        );
        assert!(
            output.contains("beta"),
            "generic should show array string elements, got: {}",
            output
        );
    }
}
