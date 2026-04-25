//! Wrappers Module Integration Tests
//!
//! Tests for wrapper modules:
//! - base: SubAnalysisResult, safe_call, progress
//! - secure: Security analysis orchestrator
//! - todo: Improvement suggestions orchestrator
//! - types: Severity enum

use std::collections::HashMap;
use tempfile::TempDir;

use tldr_core::wrappers::{
    progress, run_secure, run_todo, safe_call, SecureFinding, SecureReport, Severity,
    SubAnalysisResult, TodoItem, TodoReport, TodoSummary,
};

// =============================================================================
// Base Module Tests
// =============================================================================

#[test]
fn test_sub_analysis_result_success() {
    let result = safe_call("test_analysis", || -> Result<i32, anyhow::Error> { Ok(42) });

    assert_eq!(result.name, "test_analysis");
    assert!(result.success);
    assert_eq!(result.data, Some(serde_json::json!(42)));
    assert!(result.error.is_none());
    assert!(result.elapsed_ms >= 0.0);
}

#[test]
fn test_sub_analysis_result_failure() {
    let result = safe_call("failing_analysis", || -> Result<i32, anyhow::Error> {
        Err(anyhow::anyhow!("test error"))
    });

    assert_eq!(result.name, "failing_analysis");
    assert!(!result.success);
    assert!(result.data.is_none());
    assert_eq!(result.error, Some("test error".to_string()));
}

#[test]
fn test_sub_analysis_result_serialization() {
    let result = SubAnalysisResult {
        name: "test".to_string(),
        success: true,
        data: Some(serde_json::json!({"count": 42})),
        error: None,
        elapsed_ms: 123.456,
    };

    let json = serde_json::to_value(&result).unwrap();
    assert_eq!(json["name"], "test");
    assert_eq!(json["success"], true);
    assert_eq!(json["data"]["count"], 42);
    // MIT-OUT-02a: elapsed_ms should be rounded to 1 decimal
    assert_eq!(json["elapsed_ms"], 123.5);
}

#[test]
fn test_sub_analysis_result_with_error() {
    let result = SubAnalysisResult {
        name: "failing".to_string(),
        success: false,
        data: None,
        error: Some("Something went wrong".to_string()),
        elapsed_ms: 5.0,
    };

    let json = serde_json::to_value(&result).unwrap();
    assert_eq!(json["name"], "failing");
    assert_eq!(json["success"], false);
    assert!(json.get("data").is_none()); // skip_serializing_if
    assert_eq!(json["error"], "Something went wrong");
}

#[test]
fn test_progress_does_not_panic() {
    // Progress prints to stderr, just verify it doesn't panic
    progress(1, 5, "test_analysis");
    progress(5, 5, "final_step");
}

// =============================================================================
// Severity Tests
// =============================================================================

#[test]
fn test_severity_ordering() {
    // Critical > High > Medium > Low > Info
    assert!(Severity::Critical > Severity::High);
    assert!(Severity::High > Severity::Medium);
    assert!(Severity::Medium > Severity::Low);
    assert!(Severity::Low > Severity::Info);
}

#[test]
fn test_severity_sorting() {
    let mut severities = [
        Severity::Low,
        Severity::Critical,
        Severity::Info,
        Severity::High,
        Severity::Medium,
    ];
    severities.sort();

    assert_eq!(severities[0], Severity::Info);
    assert_eq!(severities[1], Severity::Low);
    assert_eq!(severities[2], Severity::Medium);
    assert_eq!(severities[3], Severity::High);
    assert_eq!(severities[4], Severity::Critical);
}

#[test]
fn test_severity_serialization() {
    let severity = Severity::High;
    let json = serde_json::to_string(&severity).unwrap();
    assert!(json.contains("High"));

    // Deserialize
    let parsed: Severity = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, Severity::High);
}

#[test]
fn test_severity_debug() {
    assert_eq!(format!("{:?}", Severity::Critical), "Critical");
}

#[test]
fn test_severity_clone() {
    let s = Severity::High;
    let cloned = s;
    assert_eq!(s, cloned);
}

#[test]
fn test_severity_copy() {
    let s = Severity::Medium;
    let copied = s; // Copy
    assert_eq!(s, copied); // Original still usable
}

// =============================================================================
// Secure Wrapper Tests
// =============================================================================

#[test]
fn test_secure_finding_new() {
    let finding = SecureFinding {
        category: "secrets".to_string(),
        severity: "high".to_string(),
        description: "AWS key detected".to_string(),
        file: "config.py".to_string(),
        line: 42,
    };

    assert_eq!(finding.category, "secrets");
    assert_eq!(finding.severity, "high");
    assert_eq!(finding.description, "AWS key detected");
    assert_eq!(finding.file, "config.py");
    assert_eq!(finding.line, 42);
}

#[test]
fn test_secure_finding_serialization() {
    let finding = SecureFinding {
        category: "secrets".to_string(),
        severity: "high".to_string(),
        description: "AWS key detected".to_string(),
        file: "config.py".to_string(),
        line: 42,
    };

    let json = serde_json::to_value(&finding).unwrap();
    assert_eq!(json["category"], "secrets");
    assert_eq!(json["severity"], "high");
    assert_eq!(json["description"], "AWS key detected");
    assert_eq!(json["file"], "config.py");
    assert_eq!(json["line"], 42);
}

#[test]
fn test_secure_report_structure() {
    let report = SecureReport {
        wrapper: "secure".to_string(),
        path: "/test/path".to_string(),
        findings: vec![],
        sub_results: HashMap::new(),
        summary: HashMap::new(),
        total_elapsed_ms: 100.5,
    };

    assert_eq!(report.wrapper, "secure");
    assert_eq!(report.path, "/test/path");
    assert!(report.findings.is_empty());
    assert!(report.sub_results.is_empty());
    assert!(report.summary.is_empty());
}

#[test]
fn test_run_secure_empty_directory() {
    let temp_dir = TempDir::new().unwrap();
    let result = run_secure(temp_dir.path().to_str().unwrap(), None, false);

    assert!(result.is_ok());
    let report = result.unwrap();
    assert_eq!(report.wrapper, "secure");
    assert!(report.findings.is_empty());
    assert!(report.sub_results.contains_key("secrets"));
    assert!(report.sub_results.contains_key("vulnerabilities"));
}

#[test]
fn test_run_secure_with_secrets() {
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("config.py");

    std::fs::write(
        &test_file,
        r#"
API_KEY = "AKIAIOSFODNN7EXAMPLE"
password = "supersecret123"
"#,
    )
    .unwrap();

    let result = run_secure(temp_dir.path().to_str().unwrap(), Some("python"), false);

    assert!(result.is_ok());
    let report = result.unwrap();
    assert_eq!(report.wrapper, "secure");

    // Should have sub-results
    assert!(report.sub_results.contains_key("secrets"));
    assert!(report.sub_results.contains_key("vulnerabilities"));
}

#[test]
fn test_run_secure_with_vulnerabilities() {
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("db.py");

    std::fs::write(
        &test_file,
        r#"
from flask import request

def get_user():
    user_id = request.args.get('id')
    cursor.execute("SELECT * FROM users WHERE id = " + user_id)
"#,
    )
    .unwrap();

    let result = run_secure(temp_dir.path().to_str().unwrap(), Some("python"), false);

    assert!(result.is_ok());
    let report = result.unwrap();

    assert!(report.sub_results.contains_key("vulnerabilities"));
}

#[test]
fn test_run_secure_quick_mode() {
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("test.py");
    std::fs::write(&test_file, "# safe code\nprint('hello')").unwrap();

    let result = run_secure(temp_dir.path().to_str().unwrap(), None, true);

    assert!(result.is_ok());
    let report = result.unwrap();

    // Quick mode should still run essential analyses
    assert!(report.sub_results.contains_key("secrets"));
    assert!(report.sub_results.contains_key("vulnerabilities"));
}

#[test]
fn test_run_secure_single_file() {
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("single.py");
    std::fs::write(&test_file, "secret = 'password123'").unwrap();

    let result = run_secure(test_file.to_str().unwrap(), None, false);

    assert!(result.is_ok());
    let report = result.unwrap();
    assert!(report.path.contains("single.py"));
}

#[test]
fn test_run_secure_findings_sorted_by_severity() {
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("mixed.py");

    std::fs::write(&test_file, r#"
# Critical: AWS key
key = "AKIAIOSFODNN7EXAMPLE"
# High: password
pwd = "secret123"
# Medium: JWT
token = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U"
"#).unwrap();

    let result = run_secure(temp_dir.path().to_str().unwrap(), Some("python"), false);

    assert!(result.is_ok());
    let report = result.unwrap();

    // Verify findings are sorted by severity (critical first)
    if report.findings.len() >= 2 {
        let severities: Vec<&str> = report
            .findings
            .iter()
            .map(|f| f.severity.as_str())
            .collect();

        // Verify critical/high come before medium/low
        let critical_idx = severities.iter().position(|&s| s == "critical");
        let high_idx = severities.iter().position(|&s| s == "high");
        let medium_idx = severities.iter().position(|&s| s == "medium");

        if let (Some(c), Some(m)) = (critical_idx, medium_idx) {
            assert!(c < m, "Critical findings should come before medium");
        }
        if let (Some(h), Some(m)) = (high_idx, medium_idx) {
            assert!(h < m, "High findings should come before medium");
        }
    }
}

#[test]
fn test_run_secure_summary_counts() {
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("secrets.py");
    std::fs::write(
        &test_file,
        r#"
key1 = "AKIAIOSFODNN7EXAMPLE"
key2 = "AKIAI44QH8DHBEXAMPLE"
"#,
    )
    .unwrap();

    let result = run_secure(temp_dir.path().to_str().unwrap(), Some("python"), false);

    assert!(result.is_ok());
    let report = result.unwrap();

    // Summary should contain counts
    assert!(report.summary.contains_key("total_findings"));
    assert!(report.summary.contains_key("secrets_count"));
    assert!(report.summary.contains_key("vulnerabilities_count"));
}

#[test]
fn test_run_secure_timing() {
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("test.py");
    std::fs::write(&test_file, "print('hello')").unwrap();

    let result = run_secure(temp_dir.path().to_str().unwrap(), None, true);

    assert!(result.is_ok());
    let report = result.unwrap();

    // Should have measured elapsed time
    assert!(report.total_elapsed_ms >= 0.0);

    // Each sub-result should have elapsed time
    for sub_result in report.sub_results.values() {
        assert!(sub_result.elapsed_ms >= 0.0);
    }
}

// =============================================================================
// Todo Wrapper Tests
// =============================================================================

#[test]
fn test_todo_item_new() {
    let item = TodoItem::new(
        "dead",
        1,
        "Remove foo() - never called",
        "src/main.py",
        42,
        "low",
        0.0,
    );

    assert_eq!(item.category, "dead");
    assert_eq!(item.priority, 1);
    assert_eq!(item.description, "Remove foo() - never called");
    assert_eq!(item.file, "src/main.py");
    assert_eq!(item.line, 42);
    assert_eq!(item.severity, "low");
    assert_eq!(item.score, 0.0);
}

#[test]
fn test_todo_item_serialization() {
    let item = TodoItem::new(
        "complexity",
        2,
        "Simplify foo()",
        "src/main.py",
        10,
        "high",
        25.0,
    );

    let json = serde_json::to_value(&item).unwrap();
    assert_eq!(json["category"], "complexity");
    assert_eq!(json["priority"], 2);
    assert_eq!(json["description"], "Simplify foo()");
    assert_eq!(json["file"], "src/main.py");
    assert_eq!(json["line"], 10);
    assert_eq!(json["severity"], "high");
    assert_eq!(json["score"], 25.0);
}

#[test]
fn test_todo_report_new() {
    let report = TodoReport::new("src/");

    assert_eq!(report.wrapper, "todo");
    assert_eq!(report.path, "src/");
    assert!(report.items.is_empty());
    assert!(report.sub_results.is_empty());
    assert_eq!(report.summary.total_items, 0);
}

#[test]
fn test_todo_report_to_text() {
    let mut report = TodoReport::new("src/");
    report.items.push(TodoItem::new(
        "dead",
        1,
        "Remove unused_func() - never called",
        "src/main.py",
        10,
        "low",
        0.0,
    ));
    report.items.push(TodoItem::new(
        "complexity",
        2,
        "Simplify complex_func() CC=25",
        "src/utils.py",
        20,
        "high",
        25.0,
    ));
    report.summary = TodoSummary {
        dead_count: 1,
        hotspot_count: 1,
        low_cohesion_count: 0,
        similar_pairs: 0,
        equivalence_groups: 0,
        total_items: 2,
    };
    report.total_elapsed_ms = 123.456;

    let text = report.to_text();

    assert!(text.contains("TODO: Improvement Opportunities"));
    assert!(text.contains("Dead Code:    1 functions to remove"));
    assert!(text.contains("[DEAD] Remove unused_func()"));
    assert!(text.contains("[COMPLEXITY] Simplify complex_func() CC=25"));
    assert!(text.contains("src/main.py:10"));
    assert!(text.contains("Elapsed: 123ms"));
}

#[test]
fn test_run_todo_empty_directory() {
    let temp_dir = TempDir::new().unwrap();
    let result = run_todo(temp_dir.path().to_str().unwrap(), None, true);

    assert!(result.is_ok());
    let report = result.unwrap();
    assert_eq!(report.wrapper, "todo");
    // May have items or may not, depending on analysis
}

#[test]
fn test_run_todo_quick_mode() {
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("test.py");
    std::fs::write(
        &test_file,
        r#"
def simple():
    pass

def complex(x, y, z):
    if x > 0:
        if y > 0:
            if z > 0:
                return x + y + z
    return 0
"#,
    )
    .unwrap();

    let result = run_todo(temp_dir.path().to_str().unwrap(), Some("python"), true);

    assert!(result.is_ok());
    let report = result.unwrap();

    // Quick mode should skip similarity analysis
    assert!(!report.sub_results.contains_key("similar"));
}

#[test]
fn test_run_todo_full_mode() {
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("test.py");
    std::fs::write(
        &test_file,
        r#"
def foo():
    return 1 + 2

def bar():
    return 1 + 2
"#,
    )
    .unwrap();

    let result = run_todo(temp_dir.path().to_str().unwrap(), Some("python"), false);

    assert!(result.is_ok());
    let report = result.unwrap();

    // Full mode should include similarity analysis
    assert!(report.sub_results.contains_key("similar"));
}

#[test]
fn test_run_todo_items_sorted_by_priority() {
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("test.py");

    // Create code that should generate multiple priorities
    std::fs::write(
        &test_file,
        r#"
def unused_func():
    pass

def complex_func(x, y, z, a, b):
    if x > 0:
        if y > 0:
            if z > 0:
                if a > 0:
                    if b > 0:
                        return x + y + z + a + b
    return 0
"#,
    )
    .unwrap();

    let result = run_todo(temp_dir.path().to_str().unwrap(), Some("python"), true);

    assert!(result.is_ok());
    let report = result.unwrap();

    // Verify items are sorted by priority
    for i in 1..report.items.len() {
        assert!(
            report.items[i].priority >= report.items[i - 1].priority,
            "Items should be sorted by priority"
        );
    }
}

#[test]
fn test_run_todo_timing() {
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("test.py");
    std::fs::write(&test_file, "print('hello')").unwrap();

    let result = run_todo(temp_dir.path().to_str().unwrap(), None, true);

    assert!(result.is_ok());
    let report = result.unwrap();

    // Should have measured elapsed time
    assert!(report.total_elapsed_ms >= 0.0);

    // Each sub-result should have elapsed time
    for sub_result in report.sub_results.values() {
        assert!(sub_result.elapsed_ms >= 0.0);
    }
}

// =============================================================================
// Todo Summary Tests
// =============================================================================

#[test]
fn test_todo_summary_default() {
    let summary = TodoSummary::default();

    assert_eq!(summary.dead_count, 0);
    assert_eq!(summary.hotspot_count, 0);
    assert_eq!(summary.low_cohesion_count, 0);
    assert_eq!(summary.similar_pairs, 0);
    assert_eq!(summary.equivalence_groups, 0);
    assert_eq!(summary.total_items, 0);
}

#[test]
fn test_todo_summary_serialization() {
    let summary = TodoSummary {
        dead_count: 3,
        hotspot_count: 5,
        low_cohesion_count: 2,
        similar_pairs: 1,
        equivalence_groups: 0,
        total_items: 11,
    };

    let json = serde_json::to_value(&summary).unwrap();
    assert_eq!(json["dead_count"], 3);
    assert_eq!(json["hotspot_count"], 5);
    assert_eq!(json["low_cohesion_count"], 2);
    assert_eq!(json["similar_pairs"], 1);
    assert_eq!(json["equivalence_groups"], 0);
    assert_eq!(json["total_items"], 11);
}

// =============================================================================
// Integration Tests
// =============================================================================

#[test]
fn test_secure_and_todo_integration() {
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("app.py");

    std::fs::write(
        &test_file,
        r#"
API_KEY = "AKIAIOSFODNN7EXAMPLE"

def unused_func():
    pass

def complex_func(x, y, z, a, b, c):
    if x > 0:
        if y > 0:
            if z > 0:
                if a > 0:
                    if b > 0:
                        if c > 0:
                            return x + y + z + a + b + c
    return 0
"#,
    )
    .unwrap();

    // Test secure wrapper
    let secure_result = run_secure(temp_dir.path().to_str().unwrap(), Some("python"), true);
    assert!(secure_result.is_ok());

    // Test todo wrapper
    let todo_result = run_todo(temp_dir.path().to_str().unwrap(), Some("python"), true);
    assert!(todo_result.is_ok());
}

// =============================================================================
// Bug Documentation Tests
// =============================================================================

/// Test to document behavior: run_secure with non-existent path
/// Should handle gracefully
#[test]
fn test_run_secure_nonexistent_path() {
    let result = run_secure("/nonexistent/path/12345", None, false);
    // Should either succeed with no findings or return an error
    // Documents current behavior
    let _ = result;
}

/// Test to document behavior: run_todo with binary file
/// Should handle non-text files gracefully
#[test]
fn test_run_todo_binary_file() {
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("binary.bin");

    // Write binary data
    std::fs::write(&test_file, vec![0u8, 1, 2, 255, 254]).unwrap();

    let result = run_todo(temp_dir.path().to_str().unwrap(), None, true);
    // Should not panic
    assert!(result.is_ok() || result.is_err());
}

/// Test to document behavior: SubAnalysisResult with complex data
/// Verify serialization handles complex nested structures
#[test]
fn test_sub_analysis_result_complex_data() {
    #[derive(serde::Serialize)]
    struct ComplexData {
        nested: NestedData,
        items: Vec<Item>,
    }

    #[derive(serde::Serialize)]
    struct NestedData {
        value: i32,
        name: String,
    }

    #[derive(serde::Serialize)]
    struct Item {
        id: u64,
        label: String,
    }

    let result = safe_call("complex", || -> Result<ComplexData, anyhow::Error> {
        Ok(ComplexData {
            nested: NestedData {
                value: 42,
                name: "test".to_string(),
            },
            items: vec![
                Item {
                    id: 1,
                    label: "a".to_string(),
                },
                Item {
                    id: 2,
                    label: "b".to_string(),
                },
            ],
        })
    });

    assert!(result.success);
    let data = result.data.unwrap();
    assert_eq!(data["nested"]["value"], 42);
    assert_eq!(data["items"][0]["id"], 1);
}

/// Test to document behavior: SecureFinding with special characters
/// Verify serialization handles special characters in strings
#[test]
fn test_secure_finding_special_characters() {
    let finding = SecureFinding {
        category: "secrets".to_string(),
        severity: "high".to_string(),
        description: "Key with \"quotes\" and \nnewlines\tand\ttabs".to_string(),
        file: "/path/with spaces/file.py".to_string(),
        line: 42,
    };

    let json = serde_json::to_string(&finding).unwrap();
    let deserialized: SecureFinding = serde_json::from_str(&json).unwrap();

    assert_eq!(finding.description, deserialized.description);
    assert_eq!(finding.file, deserialized.file);
}

/// Test to document behavior: TodoItem with zero priority
/// Edge case: priority should handle 0 correctly
#[test]
fn test_todo_item_zero_priority() {
    let item = TodoItem::new("test", 0, "Test item", "test.py", 1, "low", 0.0);

    assert_eq!(item.priority, 0);
}

/// Test to document behavior: elapsed_ms rounding
/// MIT-OUT-02a: elapsed_ms should round to 1 decimal place
#[test]
fn test_elapsed_ms_rounding() {
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
