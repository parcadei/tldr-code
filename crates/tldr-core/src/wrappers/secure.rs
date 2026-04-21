//! Secure wrapper: Security analysis orchestrator
//!
//! Wires existing security analyzers (secrets, vulnerabilities) into a unified report.
//!
//! # Example
//! ```rust,ignore
//! use tldr_core::wrappers::secure::run_secure;
//!
//! let report = run_secure("src/", Some("python"), false)?;
//! for finding in &report.findings {
//!     println!("[{}] {}: {}", finding.severity, finding.category, finding.description);
//! }
//! ```

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::security::{scan_secrets, scan_vulnerabilities, SecretsReport, VulnReport};
use crate::types::Language;
use crate::wrappers::{progress, safe_call, SubAnalysisResult};
use crate::TldrResult;

// =============================================================================
// Types
// =============================================================================

/// Severity order for sorting (lower = more severe)
const SEVERITY_ORDER: &[(&str, u8)] = &[
    ("critical", 0),
    ("high", 1),
    ("medium", 2),
    ("low", 3),
    ("info", 4),
];

fn severity_rank(severity: &str) -> u8 {
    SEVERITY_ORDER
        .iter()
        .find(|(s, _)| *s == severity.to_lowercase())
        .map(|(_, rank)| *rank)
        .unwrap_or(99)
}

/// A single security finding (unified across secrets and vulnerabilities)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecureFinding {
    /// Category of finding: "secrets" or "vulnerability"
    pub category: String,
    /// Severity level: critical, high, medium, low, info
    pub severity: String,
    /// Description of the finding
    pub description: String,
    /// File path containing the finding
    pub file: String,
    /// Line number
    pub line: usize,
}

/// Complete security analysis report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecureReport {
    /// Wrapper identifier
    pub wrapper: String,
    /// Path analyzed
    pub path: String,
    /// All security findings (sorted by severity)
    pub findings: Vec<SecureFinding>,
    /// Sub-analysis results (secrets, vulnerabilities)
    pub sub_results: HashMap<String, SubAnalysisResult>,
    /// Summary statistics
    pub summary: HashMap<String, serde_json::Value>,
    /// Total elapsed time in milliseconds
    pub total_elapsed_ms: f64,
}

// =============================================================================
// Main API
// =============================================================================

/// Run security analyses on path.
///
/// # Arguments
/// * `path` - File or directory to analyze
/// * `lang` - Optional language filter (auto-detect if None)
/// * `quick` - If true, skip expensive analyses
///
/// # Returns
/// * `Ok(SecureReport)` - Complete security report
/// * `Err(TldrError)` - On critical failure
///
/// # Example
/// ```rust,ignore
/// let report = run_secure("src/", Some("python"), false)?;
/// println!("Found {} security issues", report.findings.len());
/// ```
pub fn run_secure(path: &str, lang: Option<&str>, quick: bool) -> TldrResult<SecureReport> {
    let start = Instant::now();
    let mut report = SecureReport {
        wrapper: "secure".to_string(),
        path: path.to_string(),
        findings: Vec::new(),
        sub_results: HashMap::new(),
        summary: HashMap::new(),
        total_elapsed_ms: 0.0,
    };

    let target_path = Path::new(path);
    // Quick mode currently runs the same two checks as full mode.
    let _ = quick;
    let total_steps = 2; // Currently just secrets + vulns

    // --- Step 1: Secrets scanning ---
    progress(1, total_steps, "secrets");
    let secrets_result = safe_call("secrets", || {
        scan_secrets(target_path, 4.5, false, None).map_err(|e| anyhow::anyhow!("{}", e))
    });
    report
        .sub_results
        .insert("secrets".to_string(), secrets_result);

    // --- Step 2: Vulnerability scanning ---
    progress(2, total_steps, "vulnerabilities");
    let language = lang.and_then(|l| Language::from_extension(&format!(".{}", l)));
    let vuln_result = safe_call("vulnerabilities", || {
        scan_vulnerabilities(target_path, language, None).map_err(|e| anyhow::anyhow!("{}", e))
    });
    report
        .sub_results
        .insert("vulnerabilities".to_string(), vuln_result);

    // Extract findings from sub-results
    report.findings = extract_findings(&report);

    // Sort findings by severity (critical first)
    report
        .findings
        .sort_by(|a, b| severity_rank(&a.severity).cmp(&severity_rank(&b.severity)));

    // Build summary
    report.summary = build_summary(&report);

    report.total_elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    Ok(report)
}

// =============================================================================
// Internal Implementation
// =============================================================================

/// Extract SecureFinding objects from sub-analysis results
fn extract_findings(report: &SecureReport) -> Vec<SecureFinding> {
    let mut findings = Vec::new();

    // Extract from secrets results
    if let Some(secrets_result) = report.sub_results.get("secrets") {
        if secrets_result.success {
            if let Some(data) = &secrets_result.data {
                if let Ok(secrets_report) = serde_json::from_value::<SecretsReport>(data.clone()) {
                    for secret in secrets_report.findings {
                        findings.push(SecureFinding {
                            category: "secrets".to_string(),
                            severity: format!("{:?}", secret.severity).to_lowercase(),
                            description: secret.description,
                            file: secret.file.to_string_lossy().to_string(),
                            line: secret.line as usize,
                        });
                    }
                }
            }
        }
    }

    // Extract from vulnerability results
    if let Some(vuln_result) = report.sub_results.get("vulnerabilities") {
        if vuln_result.success {
            if let Some(data) = &vuln_result.data {
                if let Ok(vuln_report) = serde_json::from_value::<VulnReport>(data.clone()) {
                    for vuln in vuln_report.findings {
                        findings.push(SecureFinding {
                            category: "vulnerability".to_string(),
                            severity: vuln.severity.to_lowercase(),
                            description: format!("{}: {}", vuln.vuln_type, vuln.remediation),
                            file: vuln.file.to_string_lossy().to_string(),
                            line: vuln.sink.line as usize,
                        });
                    }
                }
            }
        }
    }

    findings
}

/// Build summary statistics
fn build_summary(report: &SecureReport) -> HashMap<String, serde_json::Value> {
    let mut summary = HashMap::new();

    let total = report.findings.len();
    let secrets_count = report
        .findings
        .iter()
        .filter(|f| f.category == "secrets")
        .count();
    let vuln_count = report
        .findings
        .iter()
        .filter(|f| f.category == "vulnerability")
        .count();
    let critical_count = report
        .findings
        .iter()
        .filter(|f| f.severity == "critical")
        .count();
    let high_count = report
        .findings
        .iter()
        .filter(|f| f.severity == "high")
        .count();

    summary.insert("total_findings".to_string(), serde_json::json!(total));
    summary.insert(
        "secrets_count".to_string(),
        serde_json::json!(secrets_count),
    );
    summary.insert(
        "vulnerabilities_count".to_string(),
        serde_json::json!(vuln_count),
    );
    summary.insert(
        "critical_count".to_string(),
        serde_json::json!(critical_count),
    );
    summary.insert("high_count".to_string(), serde_json::json!(high_count));

    summary
}
