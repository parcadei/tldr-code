//! Security tools: secrets, vuln, api_check, secure
//!
//! These tools provide security analysis for codebases.

use crate::protocol::ToolsCallResult;
use serde_json::Value;

use super::{get_optional_bool, get_optional_string, get_required_string, to_path};

/// Handle tldr_secrets tool call
pub fn handle_secrets(args: Value) -> ToolsCallResult {
    let path = match get_required_string(&args, "path") {
        Ok(p) => p,
        Err(e) => return ToolsCallResult::error(e),
    };

    let entropy_threshold = args
        .get("entropy_threshold")
        .and_then(|v| v.as_f64())
        .unwrap_or(4.5);
    let include_test = get_optional_bool(&args, "include_test").unwrap_or(false);
    let severity_filter = get_optional_string(&args, "severity_filter");

    let path = to_path(&path);
    if !path.exists() {
        return ToolsCallResult::error(format!("Path not found: {}", path.display()));
    }

    let severity = severity_filter.and_then(|s| match s.to_lowercase().as_str() {
        "low" => Some(tldr_core::Severity::Low),
        "medium" => Some(tldr_core::Severity::Medium),
        "high" => Some(tldr_core::Severity::High),
        "critical" => Some(tldr_core::Severity::Critical),
        _ => None,
    });

    match tldr_core::scan_secrets(&path, entropy_threshold, include_test, severity) {
        Ok(report) => match serde_json::to_string_pretty(&report) {
            Ok(json) => ToolsCallResult::text(json),
            Err(e) => ToolsCallResult::error(format!("Serialization error: {}", e)),
        },
        Err(e) => ToolsCallResult::error(format!("Error: {}", e)),
    }
}

/// Handle tldr_vuln tool call
pub fn handle_vuln(args: Value) -> ToolsCallResult {
    let path = match get_required_string(&args, "path") {
        Ok(p) => p,
        Err(e) => return ToolsCallResult::error(e),
    };

    let language = get_optional_string(&args, "language");
    let vuln_type_str = get_optional_string(&args, "vuln_type");

    let path = to_path(&path);
    if !path.exists() {
        return ToolsCallResult::error(format!("Path not found: {}", path.display()));
    }

    let lang = language.and_then(|l| l.parse::<tldr_core::Language>().ok());

    let vuln_type = vuln_type_str.and_then(|s| match s.as_str() {
        "SqlInjection" => Some(tldr_core::VulnType::SqlInjection),
        "Xss" => Some(tldr_core::VulnType::Xss),
        "CommandInjection" => Some(tldr_core::VulnType::CommandInjection),
        "PathTraversal" => Some(tldr_core::VulnType::PathTraversal),
        "Ssrf" => Some(tldr_core::VulnType::Ssrf),
        "Deserialization" => Some(tldr_core::VulnType::Deserialization),
        _ => None,
    });

    match tldr_core::scan_vulnerabilities(&path, lang, vuln_type) {
        Ok(report) => match serde_json::to_string_pretty(&report) {
            Ok(json) => ToolsCallResult::text(json),
            Err(e) => ToolsCallResult::error(format!("Serialization error: {}", e)),
        },
        Err(e) => ToolsCallResult::error(format!("Error: {}", e)),
    }
}

/// Handle tldr_api_check tool call
pub fn handle_api_check(args: Value) -> ToolsCallResult {
    let path = match get_required_string(&args, "path") {
        Ok(p) => p,
        Err(e) => return ToolsCallResult::error(e),
    };

    let _language = get_optional_string(&args, "language");

    let path = to_path(&path);
    if !path.exists() {
        return ToolsCallResult::error(format!("Path not found: {}", path.display()));
    }

    // API check looks for insecure API usage patterns
    // This is a simplified implementation that leverages the vuln scanner
    match tldr_core::scan_vulnerabilities(&path, None, None) {
        Ok(report) => {
            // Filter for API-related issues
            let api_findings: Vec<_> = report
                .findings
                .iter()
                .filter(|f| {
                    matches!(
                        f.vuln_type,
                        tldr_core::VulnType::SqlInjection
                            | tldr_core::VulnType::CommandInjection
                            | tldr_core::VulnType::Deserialization
                    )
                })
                .collect();

            match serde_json::to_string_pretty(&serde_json::json!({
                "files_scanned": report.files_scanned,
                "api_issues_found": api_findings.len(),
                "findings": api_findings
            })) {
                Ok(json) => ToolsCallResult::text(json),
                Err(e) => ToolsCallResult::error(format!("Serialization error: {}", e)),
            }
        }
        Err(e) => ToolsCallResult::error(format!("Error: {}", e)),
    }
}

/// Handle tldr_secure tool call (composite security summary)
pub fn handle_secure(args: Value) -> ToolsCallResult {
    let path = match get_required_string(&args, "path") {
        Ok(p) => p,
        Err(e) => return ToolsCallResult::error(e),
    };

    let _language = get_optional_string(&args, "language");

    let path = to_path(&path);
    if !path.exists() {
        return ToolsCallResult::error(format!("Path not found: {}", path.display()));
    }

    // Combine secrets and vulnerability scanning
    let secrets_result = tldr_core::scan_secrets(&path, 4.5, false, None);
    let vuln_result = tldr_core::scan_vulnerabilities(&path, None, None);

    let secrets_summary = match &secrets_result {
        Ok(report) => serde_json::json!({
            "total_findings": report.findings.len(),
            "files_scanned": report.files_scanned,
            "critical": report.findings.iter().filter(|f| f.severity == tldr_core::Severity::Critical).count(),
            "high": report.findings.iter().filter(|f| f.severity == tldr_core::Severity::High).count(),
            "medium": report.findings.iter().filter(|f| f.severity == tldr_core::Severity::Medium).count(),
            "low": report.findings.iter().filter(|f| f.severity == tldr_core::Severity::Low).count(),
        }),
        Err(e) => serde_json::json!({"error": e.to_string()}),
    };

    let vuln_summary = match &vuln_result {
        Ok(report) => serde_json::json!({
            "total_findings": report.findings.len(),
            "files_scanned": report.files_scanned,
            "by_type": report.summary.by_type,
            "affected_files": report.summary.affected_files,
        }),
        Err(e) => serde_json::json!({"error": e.to_string()}),
    };

    // Calculate overall security score
    let secrets_issues = secrets_result
        .as_ref()
        .map(|r| r.findings.len())
        .unwrap_or(0);
    let vuln_issues = vuln_result.as_ref().map(|r| r.findings.len()).unwrap_or(0);
    let total_issues = secrets_issues + vuln_issues;

    let security_score = if total_issues == 0 {
        "excellent"
    } else if total_issues < 3 {
        "good"
    } else if total_issues < 10 {
        "fair"
    } else {
        "needs_attention"
    };

    ToolsCallResult::text(
        serde_json::json!({
            "security_score": security_score,
            "total_issues": total_issues,
            "secrets": secrets_summary,
            "vulnerabilities": vuln_summary
        })
        .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_handle_secrets_missing_path() {
        let result = handle_secrets(json!({}));
        assert!(result.is_error == Some(true));
    }

    #[test]
    fn test_handle_secrets_path_not_found() {
        let result = handle_secrets(json!({"path": "/nonexistent/path"}));
        assert!(result.is_error == Some(true));
        assert!(result.content[0].text.contains("Path not found"));
    }

    #[test]
    fn test_handle_vuln_missing_path() {
        let result = handle_vuln(json!({}));
        assert!(result.is_error == Some(true));
    }

    #[test]
    fn test_handle_api_check_missing_path() {
        let result = handle_api_check(json!({}));
        assert!(result.is_error == Some(true));
    }
}
