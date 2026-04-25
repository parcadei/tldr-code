//! Types for bugbot analysis reports
//!
//! All types derive Serialize and Deserialize for JSON output compatibility
//! with the OutputWriter system.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;

use super::tools::ToolResult;

/// Exit status from bugbot check, used to propagate exit codes without
/// calling `process::exit` (which skips Drop destructors and is untestable).
#[derive(Debug)]
pub enum BugbotExitError {
    /// Findings were detected and `--no-fail` was not set.
    FindingsDetected {
        /// Number of findings in the report.
        count: usize,
    },
    /// Critical findings detected — highest priority, exit code 3.
    CriticalFindings {
        /// Number of critical findings in the report.
        count: usize,
    },
    /// Analysis pipeline encountered errors but produced no findings.
    /// A broken pipeline should not report "clean."
    AnalysisErrors {
        /// Number of non-fatal errors encountered.
        count: usize,
    },
}

impl BugbotExitError {
    /// Return the process exit code for this error.
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::FindingsDetected { .. } => 1,
            Self::AnalysisErrors { .. } => 2,
            Self::CriticalFindings { .. } => 3,
        }
    }
}

impl fmt::Display for BugbotExitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FindingsDetected { count } => {
                write!(f, "bugbot: {} finding(s) detected", count)
            }
            Self::CriticalFindings { count } => {
                write!(f, "bugbot: {} CRITICAL finding(s) detected", count)
            }
            Self::AnalysisErrors { count } => {
                write!(
                    f,
                    "bugbot: analysis had {} error(s) with no findings",
                    count
                )
            }
        }
    }
}

impl std::error::Error for BugbotExitError {}

/// Top-level report output from bugbot check
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BugbotCheckReport {
    /// Always "bugbot"
    pub tool: String,
    /// Always "check"
    pub mode: String,
    /// Language detected or specified
    pub language: String,
    /// Git base reference (e.g. "HEAD", "main")
    pub base_ref: String,
    /// How changes were detected (e.g. "git:uncommitted", "git:staged")
    pub detection_method: String,
    /// ISO 8601 timestamp
    pub timestamp: String,
    /// Files that had changes
    pub changed_files: Vec<PathBuf>,
    /// The actual findings
    pub findings: Vec<BugbotFinding>,
    /// Summary statistics
    pub summary: BugbotSummary,
    /// Pipeline timing in milliseconds
    pub elapsed_ms: u64,
    /// Non-fatal errors encountered
    pub errors: Vec<String>,
    /// Informational notes (e.g. "stub_implementation", "no_changes_detected", "truncated_to_50")
    pub notes: Vec<String>,
    /// Tool execution results (L1)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_results: Vec<ToolResult>,
    /// Tools that were available to run
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools_available: Vec<String>,
    /// Tools that were not found
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools_missing: Vec<String>,
    /// L2 engine execution results
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub l2_engine_results: Vec<L2AnalyzerResult>,
}

/// A single finding from bugbot analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BugbotFinding {
    /// e.g. "signature-regression", "born-dead"
    pub finding_type: String,
    /// "high", "medium", "low"
    pub severity: String,
    /// File path (relative to project root)
    pub file: PathBuf,
    /// Function/method name
    pub function: String,
    /// Line number in current file
    pub line: usize,
    /// Human-readable description
    pub message: String,
    /// Type-specific evidence
    pub evidence: serde_json::Value,
    /// Confidence level (L2/L3 only). L1 findings leave this as None.
    /// Values: "CONFIRMED", "LIKELY", "POSSIBLE", "FALSE_POSITIVE"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<String>,
    /// Deterministic finding ID for cross-run tracking.
    /// Hash of (finding_type, file, function, line).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finding_id: Option<String>,
}

/// Per-engine execution result for the report.
/// Mirrors ToolResult for L1 tools, providing identical observability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct L2AnalyzerResult {
    /// Engine name (e.g. "DeltaEngine", "FlowEngine")
    pub name: String,
    /// Whether the engine completed fully
    pub success: bool,
    /// Execution time in milliseconds
    pub duration_ms: u64,
    /// Number of findings produced
    pub finding_count: usize,
    /// Number of functions analyzed
    pub functions_analyzed: usize,
    /// Number of functions skipped
    pub functions_skipped: usize,
    /// Engine completion status description
    pub status: String,
    /// Errors encountered (empty if success)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

/// Summary statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BugbotSummary {
    /// Total number of findings
    pub total_findings: usize,
    /// Findings grouped by severity
    pub by_severity: HashMap<String, usize>,
    /// Findings grouped by finding type
    pub by_type: HashMap<String, usize>,
    /// Number of files analyzed
    pub files_analyzed: usize,
    /// Number of functions analyzed
    pub functions_analyzed: usize,
    /// L1 tool-based findings count
    #[serde(default)]
    pub l1_findings: usize,
    /// L2 AST-based findings count
    #[serde(default)]
    pub l2_findings: usize,
    /// Number of tools that ran
    #[serde(default)]
    pub tools_run: usize,
    /// Number of tools that failed
    #[serde(default)]
    pub tools_failed: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bugbot_exit_error_findings_detected_exit_code() {
        let err = BugbotExitError::FindingsDetected { count: 3 };
        assert_eq!(err.exit_code(), 1);
    }

    #[test]
    fn test_bugbot_exit_error_analysis_errors_exit_code() {
        let err = BugbotExitError::AnalysisErrors { count: 2 };
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn test_bugbot_exit_error_critical_exit_code() {
        let err = BugbotExitError::CriticalFindings { count: 1 };
        assert_eq!(err.exit_code(), 3);
    }

    #[test]
    fn test_bugbot_exit_error_critical_display() {
        let err = BugbotExitError::CriticalFindings { count: 2 };
        let msg = format!("{}", err);
        assert!(msg.contains("CRITICAL"));
        assert!(msg.contains("2"));
    }

    #[test]
    fn test_bugbot_exit_error_findings_detected_display() {
        let err = BugbotExitError::FindingsDetected { count: 5 };
        let msg = format!("{}", err);
        assert!(msg.contains("5"), "should contain count");
        assert!(msg.contains("finding"), "should describe findings");
    }

    #[test]
    fn test_bugbot_exit_error_analysis_errors_display() {
        let err = BugbotExitError::AnalysisErrors { count: 1 };
        let msg = format!("{}", err);
        assert!(msg.contains("1"), "should contain count");
        assert!(msg.contains("error"), "should describe errors");
    }

    #[test]
    fn test_bugbot_exit_error_is_std_error() {
        // Verify the type implements std::error::Error (for anyhow compatibility)
        let err = BugbotExitError::FindingsDetected { count: 1 };
        let _: &dyn std::error::Error = &err;
    }

    #[test]
    fn test_bugbot_exit_error_into_anyhow() {
        // Verify it can be converted into anyhow::Error and downcast back
        let err: anyhow::Error = BugbotExitError::FindingsDetected { count: 7 }.into();
        let downcast = err.downcast_ref::<BugbotExitError>().unwrap();
        assert_eq!(downcast.exit_code(), 1);
    }

    #[test]
    fn test_report_backward_compat_no_tool_fields() {
        // Old JSON without tool_results, tools_available, tools_missing
        // should deserialize with those fields defaulting to empty vecs
        let json = r#"{
            "tool": "bugbot",
            "mode": "check",
            "language": "rust",
            "base_ref": "HEAD",
            "detection_method": "git:uncommitted",
            "timestamp": "2026-02-27T00:00:00Z",
            "changed_files": [],
            "findings": [],
            "summary": {
                "total_findings": 0,
                "by_severity": {},
                "by_type": {},
                "files_analyzed": 0,
                "functions_analyzed": 0
            },
            "elapsed_ms": 100,
            "errors": [],
            "notes": []
        }"#;

        let report: BugbotCheckReport = serde_json::from_str(json).unwrap();
        assert!(report.tool_results.is_empty());
        assert!(report.tools_available.is_empty());
        assert!(report.tools_missing.is_empty());
    }

    #[test]
    fn test_summary_backward_compat() {
        // Old summary JSON without l1_findings, l2_findings, tools_run, tools_failed
        // should deserialize with those fields defaulting to 0
        let json = r#"{
            "total_findings": 5,
            "by_severity": {"high": 2, "low": 3},
            "by_type": {"signature-regression": 2, "born-dead": 3},
            "files_analyzed": 10,
            "functions_analyzed": 42
        }"#;

        let summary: BugbotSummary = serde_json::from_str(json).unwrap();
        assert_eq!(summary.total_findings, 5);
        assert_eq!(summary.files_analyzed, 10);
        assert_eq!(summary.functions_analyzed, 42);
        assert_eq!(summary.l1_findings, 0);
        assert_eq!(summary.l2_findings, 0);
        assert_eq!(summary.tools_run, 0);
        assert_eq!(summary.tools_failed, 0);
    }

    #[test]
    fn test_bugbot_finding_confidence_serde() {
        // Test that confidence serializes when present
        let finding = BugbotFinding {
            finding_type: "test".to_string(),
            severity: "high".to_string(),
            file: PathBuf::from("test.rs"),
            function: "foo".to_string(),
            line: 1,
            message: "test".to_string(),
            evidence: serde_json::json!({}),
            confidence: Some("LIKELY".to_string()),
            finding_id: Some("abc123".to_string()),
        };
        let json = serde_json::to_string(&finding).unwrap();
        assert!(json.contains("confidence"));
        assert!(json.contains("LIKELY"));
        assert!(json.contains("finding_id"));
        assert!(json.contains("abc123"));

        // Test that None confidence is omitted
        let finding_no_conf = BugbotFinding {
            finding_type: "test".to_string(),
            severity: "high".to_string(),
            file: PathBuf::from("test.rs"),
            function: "foo".to_string(),
            line: 1,
            message: "test".to_string(),
            evidence: serde_json::json!({}),
            confidence: None,
            finding_id: None,
        };
        let json2 = serde_json::to_string(&finding_no_conf).unwrap();
        assert!(!json2.contains("confidence"));
        assert!(!json2.contains("finding_id"));
    }

    #[test]
    fn test_bugbot_finding_backward_compat_deserialize() {
        // Old JSON without confidence/finding_id should deserialize with None
        let json = r#"{
            "finding_type": "test",
            "severity": "high",
            "file": "test.rs",
            "function": "foo",
            "line": 1,
            "message": "test",
            "evidence": {}
        }"#;
        let finding: BugbotFinding = serde_json::from_str(json).unwrap();
        assert!(finding.confidence.is_none());
        assert!(finding.finding_id.is_none());
    }

    #[test]
    fn test_l2_analyzer_result_serde() {
        let result = L2AnalyzerResult {
            name: "FlowEngine".to_string(),
            success: true,
            duration_ms: 42,
            finding_count: 3,
            functions_analyzed: 10,
            functions_skipped: 2,
            status: "Complete".to_string(),
            errors: vec![],
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("FlowEngine"));
        assert!(!json.contains("errors")); // empty vec skipped

        let result_with_errors = L2AnalyzerResult {
            name: "DeltaEngine".to_string(),
            success: false,
            duration_ms: 100,
            finding_count: 0,
            functions_analyzed: 5,
            functions_skipped: 5,
            status: "Partial".to_string(),
            errors: vec!["timeout on foo()".to_string()],
        };
        let json2 = serde_json::to_string(&result_with_errors).unwrap();
        assert!(json2.contains("errors"));
        assert!(json2.contains("timeout on foo()"));
    }

    #[test]
    fn test_report_backward_compat_no_l2_engine_results() {
        // Old JSON without l2_engine_results should deserialize with empty vec
        let json = r#"{
            "tool": "bugbot",
            "mode": "check",
            "language": "rust",
            "base_ref": "HEAD",
            "detection_method": "git:uncommitted",
            "timestamp": "2026-02-27T00:00:00Z",
            "changed_files": [],
            "findings": [],
            "summary": {
                "total_findings": 0,
                "by_severity": {},
                "by_type": {},
                "files_analyzed": 0,
                "functions_analyzed": 0
            },
            "elapsed_ms": 100,
            "errors": [],
            "notes": []
        }"#;
        let report: BugbotCheckReport = serde_json::from_str(json).unwrap();
        assert!(report.l2_engine_results.is_empty());
    }

    // -------------------------------------------------------------------
    // JSON output integration tests (Phase 8.7)
    // -------------------------------------------------------------------

    /// Helper: build a minimal valid BugbotCheckReport for testing.
    fn make_test_report(
        findings: Vec<BugbotFinding>,
        l2_engine_results: Vec<L2AnalyzerResult>,
    ) -> BugbotCheckReport {
        BugbotCheckReport {
            tool: "bugbot".to_string(),
            mode: "check".to_string(),
            language: "rust".to_string(),
            base_ref: "HEAD".to_string(),
            detection_method: "git:uncommitted".to_string(),
            timestamp: "2026-03-02T00:00:00Z".to_string(),
            changed_files: vec![PathBuf::from("src/api.rs")],
            findings,
            summary: BugbotSummary {
                total_findings: 0,
                by_severity: HashMap::new(),
                by_type: HashMap::new(),
                files_analyzed: 1,
                functions_analyzed: 5,
                l1_findings: 0,
                l2_findings: 0,
                tools_run: 0,
                tools_failed: 0,
            },
            elapsed_ms: 500,
            errors: Vec::new(),
            notes: Vec::new(),
            tool_results: Vec::new(),
            tools_available: Vec::new(),
            tools_missing: Vec::new(),
            l2_engine_results,
        }
    }

    #[test]
    fn test_json_output_l2_engine_results_present_in_full_report() {
        // Verify that l2_engine_results appears as a top-level array in
        // the serialized JSON when populated.
        let report = make_test_report(
            vec![],
            vec![
                L2AnalyzerResult {
                    name: "FlowEngine".to_string(),
                    success: true,
                    duration_ms: 1203,
                    finding_count: 4,
                    functions_analyzed: 48,
                    functions_skipped: 2,
                    status: "complete".to_string(),
                    errors: vec![],
                },
                L2AnalyzerResult {
                    name: "DeltaEngine".to_string(),
                    success: true,
                    duration_ms: 350,
                    finding_count: 1,
                    functions_analyzed: 20,
                    functions_skipped: 0,
                    status: "complete".to_string(),
                    errors: vec![],
                },
            ],
        );

        let json_val = serde_json::to_value(&report).unwrap();

        // l2_engine_results must be a top-level key
        assert!(
            json_val.get("l2_engine_results").is_some(),
            "l2_engine_results must be present in report JSON"
        );

        let l2_arr = json_val["l2_engine_results"].as_array().unwrap();
        assert_eq!(l2_arr.len(), 2, "should have 2 engine results");
    }

    #[test]
    fn test_json_output_l2_engine_result_fields_match_spec() {
        // Verify each L2AnalyzerResult entry has exactly the fields from the spec:
        //   name, success, duration_ms, finding_count, functions_analyzed, functions_skipped
        let report = make_test_report(
            vec![],
            vec![L2AnalyzerResult {
                name: "FlowEngine".to_string(),
                success: true,
                duration_ms: 1203,
                finding_count: 4,
                functions_analyzed: 48,
                functions_skipped: 2,
                status: "complete".to_string(),
                errors: vec![],
            }],
        );

        let json_val = serde_json::to_value(&report).unwrap();
        let entry = &json_val["l2_engine_results"][0];

        // All spec-required fields present with correct values
        assert_eq!(entry["name"], "FlowEngine");
        assert_eq!(entry["success"], serde_json::Value::Bool(true));
        assert_eq!(entry["duration_ms"], 1203);
        assert_eq!(entry["finding_count"], 4);
        assert_eq!(entry["functions_analyzed"], 48);
        assert_eq!(entry["functions_skipped"], 2);

        // status is also serialized (implementation detail beyond spec minimum)
        assert_eq!(entry["status"], "complete");

        // errors should be absent when empty (skip_serializing_if)
        assert!(
            entry.get("errors").is_none(),
            "empty errors vec should be omitted from JSON"
        );
    }

    #[test]
    fn test_json_output_l2_engine_result_with_errors() {
        // Verify that errors array appears when non-empty
        let report = make_test_report(
            vec![],
            vec![L2AnalyzerResult {
                name: "DeltaEngine".to_string(),
                success: false,
                duration_ms: 100,
                finding_count: 0,
                functions_analyzed: 5,
                functions_skipped: 5,
                status: "partial (timeout on complex_fn)".to_string(),
                errors: vec!["timeout on complex_fn()".to_string()],
            }],
        );

        let json_val = serde_json::to_value(&report).unwrap();
        let entry = &json_val["l2_engine_results"][0];

        assert_eq!(entry["success"], serde_json::Value::Bool(false));
        let errors = entry["errors"].as_array().unwrap();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0], "timeout on complex_fn()");
    }

    #[test]
    fn test_json_output_l2_engine_results_omitted_when_empty() {
        // Verify that l2_engine_results is omitted from JSON when the vec is empty
        // (skip_serializing_if = "Vec::is_empty")
        let report = make_test_report(vec![], vec![]);

        let json_val = serde_json::to_value(&report).unwrap();
        assert!(
            json_val.get("l2_engine_results").is_none(),
            "l2_engine_results should be omitted when empty"
        );
    }

    #[test]
    fn test_json_output_finding_with_confidence_and_finding_id() {
        // Verify that confidence and finding_id appear in the finding JSON
        // when set (top-level on finding, not inside evidence)
        let finding = BugbotFinding {
            finding_type: "taint-flow".to_string(),
            severity: "high".to_string(),
            file: PathBuf::from("src/api.rs"),
            function: "handle_request".to_string(),
            line: 42,
            message: "Taint flow: user_input reaches sql_query without sanitization".to_string(),
            evidence: serde_json::json!({
                "source": "user_input",
                "sink": "sql_query"
            }),
            confidence: Some("POSSIBLE".to_string()),
            finding_id: Some("a3f8c2d1".to_string()),
        };

        let report = make_test_report(vec![finding], vec![]);

        let json_val = serde_json::to_value(&report).unwrap();
        let finding_json = &json_val["findings"][0];

        // confidence is top-level on finding (not inside evidence)
        assert_eq!(finding_json["confidence"], "POSSIBLE");
        assert_eq!(finding_json["finding_id"], "a3f8c2d1");

        // Verify it's NOT nested inside evidence
        assert!(
            finding_json["evidence"].get("confidence").is_none(),
            "confidence must be top-level on finding, not inside evidence"
        );
        assert!(
            finding_json["evidence"].get("finding_id").is_none(),
            "finding_id must be top-level on finding, not inside evidence"
        );
    }

    #[test]
    fn test_json_output_finding_omits_none_confidence_and_finding_id() {
        // Verify that None confidence/finding_id are omitted from JSON
        // (skip_serializing_if = "Option::is_none")
        let finding = BugbotFinding {
            finding_type: "signature-regression".to_string(),
            severity: "medium".to_string(),
            file: PathBuf::from("src/lib.rs"),
            function: "process".to_string(),
            line: 10,
            message: "Signature changed".to_string(),
            evidence: serde_json::json!({}),
            confidence: None,
            finding_id: None,
        };

        let report = make_test_report(vec![finding], vec![]);
        let json_val = serde_json::to_value(&report).unwrap();
        let finding_json = &json_val["findings"][0];

        assert!(
            finding_json.get("confidence").is_none(),
            "None confidence must be omitted from JSON"
        );
        assert!(
            finding_json.get("finding_id").is_none(),
            "None finding_id must be omitted from JSON"
        );
    }

    #[test]
    fn test_json_output_full_report_matches_spec_shape() {
        // End-to-end: build a report matching the spec example and verify
        // the JSON shape matches the spec at spec.md lines 2109-2143.
        let findings = vec![BugbotFinding {
            finding_type: "taint-flow".to_string(),
            severity: "high".to_string(),
            file: PathBuf::from("src/api.rs"),
            function: "handle_request".to_string(),
            line: 42,
            message: "Taint flow: user_input reaches sql_query without sanitization".to_string(),
            evidence: serde_json::json!({
                "source": "user_input",
                "sink": "sql_query",
                "hops": 3
            }),
            confidence: Some("POSSIBLE".to_string()),
            finding_id: Some("a3f8c2d1".to_string()),
        }];

        let l2_results = vec![L2AnalyzerResult {
            name: "FlowEngine".to_string(),
            success: true,
            duration_ms: 1203,
            finding_count: 4,
            functions_analyzed: 48,
            functions_skipped: 2,
            status: "complete".to_string(),
            errors: vec![],
        }];

        let report = make_test_report(findings, l2_results);
        let json_val = serde_json::to_value(&report).unwrap();

        // Top-level report structure
        assert!(json_val.is_object());
        assert!(json_val.get("findings").unwrap().is_array());
        assert!(json_val.get("l2_engine_results").unwrap().is_array());
        assert!(json_val.get("summary").unwrap().is_object());

        // Finding shape matches spec
        let f = &json_val["findings"][0];
        assert_eq!(f["finding_type"], "taint-flow");
        assert_eq!(f["severity"], "high");
        assert_eq!(f["file"], "src/api.rs");
        assert_eq!(f["function"], "handle_request");
        assert_eq!(f["line"], 42);
        assert!(
            f["message"]
                .as_str()
                .unwrap()
                .to_lowercase()
                .contains("taint"),
            "message should mention taint flow"
        );
        assert_eq!(f["confidence"], "POSSIBLE");
        assert_eq!(f["finding_id"], "a3f8c2d1");
        assert!(f["evidence"].is_object());

        // L2 engine result shape matches spec
        let e = &json_val["l2_engine_results"][0];
        assert_eq!(e["name"], "FlowEngine");
        assert_eq!(e["success"], serde_json::Value::Bool(true));
        assert_eq!(e["duration_ms"], 1203);
        assert_eq!(e["finding_count"], 4);
        assert_eq!(e["functions_analyzed"], 48);
        assert_eq!(e["functions_skipped"], 2);
    }

    #[test]
    fn test_json_output_roundtrip_with_l2_engine_results() {
        // Verify serialize -> deserialize roundtrip preserves all L2 fields
        let report = make_test_report(
            vec![BugbotFinding {
                finding_type: "born-dead".to_string(),
                severity: "low".to_string(),
                file: PathBuf::from("src/utils.rs"),
                function: "unused_helper".to_string(),
                line: 99,
                message: "Function is never called".to_string(),
                evidence: serde_json::json!({}),
                confidence: Some("CONFIRMED".to_string()),
                finding_id: Some("deadbeef".to_string()),
            }],
            vec![
                L2AnalyzerResult {
                    name: "FlowEngine".to_string(),
                    success: true,
                    duration_ms: 500,
                    finding_count: 0,
                    functions_analyzed: 30,
                    functions_skipped: 1,
                    status: "complete".to_string(),
                    errors: vec![],
                },
                L2AnalyzerResult {
                    name: "ContractEngine".to_string(),
                    success: false,
                    duration_ms: 200,
                    finding_count: 0,
                    functions_analyzed: 10,
                    functions_skipped: 20,
                    status: "partial (unsupported patterns)".to_string(),
                    errors: vec!["unsupported patterns".to_string()],
                },
            ],
        );

        let json_str = serde_json::to_string(&report).unwrap();
        let deserialized: BugbotCheckReport = serde_json::from_str(&json_str).unwrap();

        // L2 engine results roundtrip
        assert_eq!(deserialized.l2_engine_results.len(), 2);
        assert_eq!(deserialized.l2_engine_results[0].name, "FlowEngine");
        assert!(deserialized.l2_engine_results[0].success);
        assert_eq!(deserialized.l2_engine_results[0].duration_ms, 500);
        assert_eq!(deserialized.l2_engine_results[0].finding_count, 0);
        assert_eq!(deserialized.l2_engine_results[0].functions_analyzed, 30);
        assert_eq!(deserialized.l2_engine_results[0].functions_skipped, 1);

        assert_eq!(deserialized.l2_engine_results[1].name, "ContractEngine");
        assert!(!deserialized.l2_engine_results[1].success);
        assert_eq!(deserialized.l2_engine_results[1].errors.len(), 1);

        // Finding roundtrip with confidence and finding_id
        assert_eq!(deserialized.findings.len(), 1);
        assert_eq!(
            deserialized.findings[0].confidence,
            Some("CONFIRMED".to_string())
        );
        assert_eq!(
            deserialized.findings[0].finding_id,
            Some("deadbeef".to_string())
        );
    }

    #[test]
    fn test_json_output_multiple_confidence_levels() {
        // Verify all defined confidence levels serialize correctly
        let levels = vec!["CONFIRMED", "LIKELY", "POSSIBLE", "FALSE_POSITIVE"];

        for level in &levels {
            let finding = BugbotFinding {
                finding_type: "test".to_string(),
                severity: "medium".to_string(),
                file: PathBuf::from("test.rs"),
                function: "test_fn".to_string(),
                line: 1,
                message: "test".to_string(),
                evidence: serde_json::json!({}),
                confidence: Some(level.to_string()),
                finding_id: Some("id123".to_string()),
            };
            let json_val = serde_json::to_value(&finding).unwrap();
            assert_eq!(
                json_val["confidence"].as_str().unwrap(),
                *level,
                "confidence level '{}' should serialize correctly",
                level
            );
        }
    }

    #[test]
    fn test_json_output_finding_id_is_string_not_number() {
        // finding_id should serialize as a JSON string (hex hash), never as a number
        let finding = BugbotFinding {
            finding_type: "test".to_string(),
            severity: "low".to_string(),
            file: PathBuf::from("test.rs"),
            function: "f".to_string(),
            line: 1,
            message: "test".to_string(),
            evidence: serde_json::json!({}),
            confidence: None,
            finding_id: Some("a3f8c2d1".to_string()),
        };

        let json_val = serde_json::to_value(&finding).unwrap();
        assert!(
            json_val["finding_id"].is_string(),
            "finding_id must serialize as a JSON string"
        );
        assert_eq!(json_val["finding_id"].as_str().unwrap(), "a3f8c2d1");
    }
}
