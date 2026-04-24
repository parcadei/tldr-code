//! Wrappers module - Base infrastructure for GVN migration
//!
//! This module provides shared types and utilities for orchestrating
//! multiple sub-analyses, including:
//!
//! - `SubAnalysisResult`: Captures result of a single analysis with timing
//! - `safe_call`: Executes a closure with timing and error handling
//! - `progress`: Prints progress messages to stderr
//! - `Severity`: Enum for finding severity levels
//! - `secure`: Security analysis orchestrator (Phase P6)
//! - `todo`: Todo orchestrator for prioritized improvement suggestions (Phase P7)

mod base;
pub mod secure;
pub mod todo;
mod types;

#[cfg(test)]
mod secure_tests;

pub use base::{progress, safe_call, SubAnalysisResult};
pub use secure::{run_secure, SecureFinding, SecureReport};
pub use todo::{run_todo, TodoItem, TodoReport, TodoSummary};
pub use types::Severity;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_sub_analysis_result_serialization() {
        // Test successful result
        let result = SubAnalysisResult {
            name: "test_analysis".to_string(),
            success: true,
            data: Some(json!({"count": 42})),
            error: None,
            elapsed_ms: 123.456789,
        };

        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["name"], "test_analysis");
        assert_eq!(json["success"], true);
        assert_eq!(json["data"]["count"], 42);
        assert!(json.get("error").is_none()); // skip_serializing_if
                                              // MIT-OUT-02a: elapsed_ms should be rounded to 1 decimal
        assert_eq!(json["elapsed_ms"], 123.5);
    }

    #[test]
    fn test_sub_analysis_result_serialization_with_error() {
        // Test failed result
        let result = SubAnalysisResult {
            name: "failing_analysis".to_string(),
            success: false,
            data: None,
            error: Some("Something went wrong".to_string()),
            elapsed_ms: 5.0,
        };

        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["name"], "failing_analysis");
        assert_eq!(json["success"], false);
        assert!(json.get("data").is_none()); // skip_serializing_if
        assert_eq!(json["error"], "Something went wrong");
        assert_eq!(json["elapsed_ms"], 5.0);
    }

    #[test]
    fn test_safe_call_success() {
        let result = safe_call("my_analysis", || -> Result<i32, anyhow::Error> { Ok(42) });

        assert_eq!(result.name, "my_analysis");
        assert!(result.success);
        assert_eq!(result.data, Some(json!(42)));
        assert!(result.error.is_none());
        assert!(result.elapsed_ms >= 0.0);
    }

    #[test]
    fn test_safe_call_failure() {
        let result = safe_call("failing_analysis", || -> Result<i32, anyhow::Error> {
            Err(anyhow::anyhow!("test error"))
        });

        assert_eq!(result.name, "failing_analysis");
        assert!(!result.success);
        assert!(result.data.is_none());
        assert_eq!(result.error, Some("test error".to_string()));
        assert!(result.elapsed_ms >= 0.0);
    }

    #[test]
    fn test_safe_call_with_complex_data() {
        #[derive(serde::Serialize)]
        struct TestData {
            items: Vec<String>,
            count: usize,
        }

        let result = safe_call("complex_analysis", || -> Result<TestData, anyhow::Error> {
            Ok(TestData {
                items: vec!["a".to_string(), "b".to_string()],
                count: 2,
            })
        });

        assert!(result.success);
        let data = result.data.unwrap();
        assert_eq!(data["items"], json!(["a", "b"]));
        assert_eq!(data["count"], 2);
    }

    #[test]
    fn test_severity_ordering() {
        // Critical > High > Medium > Low > Info
        assert!(Severity::Critical > Severity::High);
        assert!(Severity::High > Severity::Medium);
        assert!(Severity::Medium > Severity::Low);
        assert!(Severity::Low > Severity::Info);

        // Test all comparisons
        let mut severities = [
            Severity::Low,
            Severity::Critical,
            Severity::Info,
            Severity::High,
            Severity::Medium,
        ];
        severities.sort();

        // After sorting (ascending), Info should be first, Critical last
        assert_eq!(severities[0], Severity::Info);
        assert_eq!(severities[4], Severity::Critical);
    }

    #[test]
    fn test_severity_serialization() {
        let severity = Severity::High;
        let json = serde_json::to_value(severity).unwrap();
        assert_eq!(json, "High");

        // Test deserialization
        let parsed: Severity = serde_json::from_str("\"Medium\"").unwrap();
        assert_eq!(parsed, Severity::Medium);
    }

    #[test]
    fn test_progress_format() {
        // MIT-OUT-03a: Match Python's exact progress format
        // Expected: "[1/5] Analyzing test_analysis..."

        // We test by capturing stderr - but for simplicity, test the format function
        // The actual progress function prints to stderr, so we verify it doesn't panic
        // and has correct signature. More thorough test would capture stderr.
        progress(1, 5, "test_analysis");
        progress(5, 5, "final_step");

        // Basic sanity - no panic means success for this simple test
        // A full integration test would capture and verify stderr output
    }

    #[test]
    fn test_elapsed_ms_rounding() {
        // MIT-OUT-02a: elapsed_ms should round to 1 decimal place
        let result = SubAnalysisResult {
            name: "test".to_string(),
            success: true,
            data: None,
            error: None,
            elapsed_ms: 99.999,
        };

        let json = serde_json::to_value(&result).unwrap();
        // 99.999 rounded to 1 decimal is 100.0
        assert_eq!(json["elapsed_ms"], 100.0);
    }
}
