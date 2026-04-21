//! Scan Secrets API Tests
//!
//! Tests for the scan_secrets API:
//! - Happy path: Find hardcoded password in Python code
//! - Find AWS key pattern
//! - Edge case: Clean code with no secrets

use std::io::Write;
use tempfile::TempDir;

use tldr_core::security::secrets::{scan_secrets, Severity};

/// Helper function to create a Python file with given content in a temp directory
fn create_python_file(temp_dir: &TempDir, filename: &str, content: &str) -> std::path::PathBuf {
    let file_path = temp_dir.path().join(filename);
    let mut file = std::fs::File::create(&file_path).expect("Failed to create file");
    file.write_all(content.as_bytes())
        .expect("Failed to write content");
    file_path
}

/// Test: Happy path - Find hardcoded password in Python code
///
/// Verifies that scan_secrets detects password assignments like:
/// `password = "super_secret_password_123"`
#[test]
fn test_scan_secrets_finds_hardcoded_password() {
    let temp_dir = TempDir::new().unwrap();

    // Create Python file with hardcoded password
    create_python_file(
        &temp_dir,
        "config.py",
        r#"
# Database configuration
DB_HOST = "localhost"
DB_PORT = 5432
DB_USER = "admin"
password = "super_secret_password_123"
DB_NAME = "production"

# Application settings
DEBUG = False
ALLOWED_HOSTS = ["*"]
"#,
    );

    // Scan the temp directory
    let report =
        scan_secrets(temp_dir.path(), 4.5, false, None).expect("scan_secrets should succeed");

    // Should detect at least the password
    let password_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.pattern == "Password")
        .collect();

    assert!(
        !password_findings.is_empty(),
        "Should detect hardcoded password. Found patterns: {:?}",
        report
            .findings
            .iter()
            .map(|f| &f.pattern)
            .collect::<Vec<_>>()
    );

    // Verify the finding details
    let password_finding = &password_findings[0];
    assert_eq!(password_finding.severity, Severity::High);
    assert!(password_finding
        .line_content
        .as_ref()
        .unwrap()
        .contains("password"));
    assert!(password_finding.masked_value.contains("***"));
}

/// Test: Find AWS key pattern
///
/// Verifies that scan_secrets detects AWS Access Key IDs:
/// Pattern: AKIA[0-9A-Z]{16}
#[test]
fn test_scan_secrets_finds_aws_key_pattern() {
    let temp_dir = TempDir::new().unwrap();

    // Create Python file with AWS credentials
    create_python_file(
        &temp_dir,
        "aws_config.py",
        r#"
# AWS Configuration
AWS_REGION = "us-east-1"
API_KEY = "AKIAIOSFODNN7EXAMPLE"
AWS_SECRET_ACCESS_KEY = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"

# S3 bucket settings
S3_BUCKET_NAME = "my-production-bucket"
S3_PREFIX = "uploads/"
"#,
    );

    // Scan the temp directory
    let report =
        scan_secrets(temp_dir.path(), 4.5, false, None).expect("scan_secrets should succeed");

    // Should detect the AWS Access Key
    let aws_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.pattern == "AWS Access Key")
        .collect();

    assert!(
        !aws_findings.is_empty(),
        "Should detect AWS Access Key pattern. Found patterns: {:?}",
        report
            .findings
            .iter()
            .map(|f| &f.pattern)
            .collect::<Vec<_>>()
    );

    // Verify the finding details
    let aws_finding = &aws_findings[0];
    assert_eq!(aws_finding.severity, Severity::Critical);
    assert!(
        aws_finding.line_content.as_ref().unwrap().contains("AKIA"),
        "Line content should contain the AWS key"
    );
}

/// Test: Edge case - Clean code with no secrets
///
/// Verifies that scan_secrets returns empty findings for code without secrets:
/// - Safe variable assignments
/// - Normal strings
/// - No high-entropy patterns
#[test]
fn test_scan_secrets_clean_code_no_secrets() {
    let temp_dir = TempDir::new().unwrap();

    // Create Python file with NO secrets
    create_python_file(
        &temp_dir,
        "clean_app.py",
        r#"
"""
A clean application with no hardcoded secrets.
"""

# Configuration - all safe values
APP_NAME = "MyApplication"
VERSION = "1.2.3"
DEBUG = True
MAX_RETRIES = 3
TIMEOUT = 30

# Messages
WELCOME_MESSAGE = "Welcome to our application!"
ERROR_MESSAGE = "An error occurred. Please try again."

# Settings
LOG_LEVEL = "INFO"
LOG_FORMAT = "%(asctime)s - %(name)s - %(levelname)s - %(message)s"

# Database configuration - uses environment variables
DB_HOST = os.environ.get("DB_HOST", "localhost")
DB_PORT = int(os.environ.get("DB_PORT", "5432"))
DB_USER = os.environ.get("DB_USER", "")
DB_PASS = os.environ.get("DB_PASS", "")  # Loaded from env, not hardcoded!

# Feature flags
ENABLE_CACHE = True
CACHE_TTL = 300

# List of allowed hosts
ALLOWED_HOSTS = ["localhost", "127.0.0.1"]

# Safe variable - should NOT be detected
safe_variable = "just_a_normal_string"
another_safe = "hello_world_123"

# Just a comment about passwords
# Remember to never hardcode passwords in your code!
# Use environment variables or a secrets manager instead.

if __name__ == "__main__":
    print(f"Starting {APP_NAME} v{VERSION}")
    print(WELCOME_MESSAGE)
"#,
    );

    // Scan the temp directory
    let report =
        scan_secrets(temp_dir.path(), 4.5, false, None).expect("scan_secrets should succeed");

    // Should have no findings for clean code
    assert!(
        report.findings.is_empty(),
        "Should find no secrets in clean code. Unexpected findings: {:?}",
        report.findings
    );

    // Verify basic report structure
    assert_eq!(report.files_scanned, 1);
    assert!(report.patterns_checked > 0);
    assert_eq!(report.summary.total_findings, 0);
}

/// Test: Verify summary statistics are correctly calculated
#[test]
fn test_scan_secrets_summary_statistics() {
    let temp_dir = TempDir::new().unwrap();

    // Create file with multiple secret types
    create_python_file(
        &temp_dir,
        "mixed_secrets.py",
        r#"
# Multiple secrets in one file
AWS_ACCESS_KEY_ID = "AKIAIOSFODNN7EXAMPLE"
password = "my_super_secret_password"
API_SECRET = "sk-1234567890123456789012345"
"#,
    );

    let report =
        scan_secrets(temp_dir.path(), 4.5, false, None).expect("scan_secrets should succeed");

    // Verify summary counts
    assert!(report.summary.total_findings > 0);
    assert_eq!(
        report.summary.total_findings,
        report.findings.len(),
        "Summary total should match findings count"
    );

    // Verify by_pattern counts
    let pattern_count_sum: usize = report.summary.by_pattern.values().sum();
    assert_eq!(
        pattern_count_sum,
        report.findings.len(),
        "Sum of pattern counts should match total findings"
    );

    // Verify by_severity counts
    let severity_count_sum: usize = report.summary.by_severity.values().sum();
    assert_eq!(
        severity_count_sum,
        report.findings.len(),
        "Sum of severity counts should match total findings"
    );
}
