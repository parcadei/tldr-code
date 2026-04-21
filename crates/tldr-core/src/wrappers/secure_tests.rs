//! Tests for the secure wrapper module
//!
//! TDD Phase 1: These tests define the expected behavior before implementation.

use super::secure::*;
use std::collections::HashMap;
use std::fs;
use tempfile::TempDir;

// =============================================================================
// SecureFinding Tests
// =============================================================================

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
fn test_secure_finding_deserialization() {
    let json = serde_json::json!({
        "category": "vulnerability",
        "severity": "critical",
        "description": "SQL injection",
        "file": "db.py",
        "line": 100
    });

    let finding: SecureFinding = serde_json::from_value(json).unwrap();
    assert_eq!(finding.category, "vulnerability");
    assert_eq!(finding.severity, "critical");
    assert_eq!(finding.line, 100);
}

// =============================================================================
// SecureReport Tests
// =============================================================================

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
}

#[test]
fn test_secure_report_serialization() {
    let mut sub_results = HashMap::new();
    sub_results.insert(
        "secrets".to_string(),
        crate::wrappers::SubAnalysisResult {
            name: "secrets".to_string(),
            success: true,
            data: Some(serde_json::json!({"count": 5})),
            error: None,
            elapsed_ms: 50.0,
        },
    );

    let findings = vec![SecureFinding {
        category: "secrets".to_string(),
        severity: "high".to_string(),
        description: "API key found".to_string(),
        file: "test.py".to_string(),
        line: 10,
    }];

    let report = SecureReport {
        wrapper: "secure".to_string(),
        path: "/test".to_string(),
        findings,
        sub_results,
        summary: HashMap::new(),
        total_elapsed_ms: 100.0,
    };

    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["wrapper"], "secure");
    assert_eq!(json["path"], "/test");
    assert!(json["findings"].is_array());
    assert!(json["sub_results"].is_object());
    // MIT-OUT-02a: elapsed_ms should be rounded to 1 decimal
    assert_eq!(json["total_elapsed_ms"], 100.0);
}

// =============================================================================
// run_secure() Integration Tests
// =============================================================================

#[test]
fn test_run_secure_empty_directory() {
    let temp_dir = TempDir::new().unwrap();
    let result = run_secure(temp_dir.path().to_str().unwrap(), None, false);

    assert!(result.is_ok());
    let report = result.unwrap();
    assert_eq!(report.wrapper, "secure");
    assert!(report.findings.is_empty());
}

#[test]
fn test_run_secure_with_secrets() {
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("config.py");

    // File with a hardcoded secret
    fs::write(
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

    // Should have detected the AWS key and password
    assert!(
        !report.findings.is_empty(),
        "Expected secret findings but found none"
    );

    // Check that secrets sub-result exists
    assert!(report.sub_results.contains_key("secrets"));
}

#[test]
fn test_run_secure_with_vulnerabilities() {
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("db.py");

    // File with SQL injection vulnerability
    fs::write(
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

    // Check that vulnerabilities sub-result exists
    assert!(report.sub_results.contains_key("vulnerabilities"));
}

#[test]
fn test_run_secure_quick_mode() {
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("test.py");
    fs::write(&test_file, "# safe code\nprint('hello')").unwrap();

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
    fs::write(&test_file, "secret = 'password123'").unwrap();

    let result = run_secure(test_file.to_str().unwrap(), None, false);

    assert!(result.is_ok());
    let report = result.unwrap();
    assert!(report.path.contains("single.py"));
}

#[test]
fn test_run_secure_findings_sorted_by_severity() {
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("mixed.py");

    // File with multiple severity levels
    fs::write(&test_file, r#"
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

    if report.findings.len() >= 2 {
        // Check that findings are sorted by severity (critical first)
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

// =============================================================================
// Summary Tests
// =============================================================================

#[test]
fn test_secure_summary_counts() {
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("secrets.py");
    fs::write(
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
    fs::write(&test_file, "print('hello')").unwrap();

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
