//! Parser for `cargo audit --json` output
//!
//! Parses the single JSON object output from `cargo audit --json`, which
//! reports known vulnerabilities in dependency crates listed in `Cargo.lock`.
//!
//! Unlike the cargo/clippy parser (NDJSON, one JSON object per line), this
//! parser handles a single top-level JSON object containing all results.
//!
//! The `tool` field on produced findings is set to an empty string. The runner
//! fills it in after parsing. [PM-6]

use serde::Deserialize;
use std::path::PathBuf;

use super::super::tools::{L1Finding, ToolCategory};
use super::ParseError;

/// Top-level cargo-audit JSON report
#[derive(Deserialize)]
struct AuditReport {
    vulnerabilities: AuditVulnerabilities,
}

/// Vulnerabilities section of the audit report
#[derive(Deserialize)]
struct AuditVulnerabilities {
    #[serde(default)]
    list: Vec<AuditVulnerability>,
}

/// A single vulnerability entry
#[derive(Deserialize)]
struct AuditVulnerability {
    advisory: AuditAdvisory,
}

/// Advisory metadata for a vulnerability
#[derive(Deserialize)]
struct AuditAdvisory {
    /// RUSTSEC advisory identifier (e.g., "RUSTSEC-2020-0071")
    id: String,
    /// Affected crate name
    package: String,
    /// Human-readable vulnerability title
    title: String,
}

/// Parse `cargo audit --json` output into L1 findings.
///
/// # Contract
/// - Empty output -> `ParseError::Format` (not empty Vec -- audit should always produce JSON)
/// - Valid JSON with 0 vulnerabilities -> empty Vec
/// - Each vulnerability -> one `L1Finding` with severity `"high"`
/// - File is always `"Cargo.lock"` (vulnerabilities are dependency issues)
/// - Line is always 0 (no specific line for dependency issues)
/// - `tool` field is empty string (runner fills it in later) [PM-6]
pub fn parse_cargo_audit_output(stdout: &str) -> Result<Vec<L1Finding>, ParseError> {
    let stdout = stdout.trim();
    if stdout.is_empty() {
        return Err(ParseError::Format("Empty cargo-audit output".into()));
    }

    let report: AuditReport = serde_json::from_str(stdout)?;

    let findings = report
        .vulnerabilities
        .list
        .into_iter()
        .map(|vuln| L1Finding {
            tool: String::new(), // Runner fills this in [PM-6]
            category: ToolCategory::SecurityScanner,
            file: PathBuf::from("Cargo.lock"),
            line: 0,
            column: 0,
            native_severity: "vulnerability".to_string(),
            severity: "high".to_string(),
            message: format!("{}: {}", vuln.advisory.package, vuln.advisory.title),
            code: Some(vuln.advisory.id),
        })
        .collect();

    Ok(findings)
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Test constants: realistic cargo-audit JSON samples --

    const SAMPLE_NO_VULNS: &str = r#"{
        "database": {"advisory-count": 582},
        "lockfile": {"dependency-count": 42},
        "vulnerabilities": {"found": false, "count": 0, "list": []},
        "warnings": {}
    }"#;

    const SAMPLE_ONE_VULN: &str = r#"{
        "database": {"advisory-count": 582},
        "lockfile": {"dependency-count": 42},
        "vulnerabilities": {
            "found": true,
            "count": 1,
            "list": [{
                "advisory": {
                    "id": "RUSTSEC-2020-0071",
                    "package": "time",
                    "title": "Potential segfault in the time crate",
                    "description": "Unix-like operating systems may segfault",
                    "date": "2020-11-18",
                    "aliases": ["CVE-2020-26235"],
                    "categories": ["code-execution"],
                    "keywords": ["segfault"],
                    "url": "https://rustsec.org/advisories/RUSTSEC-2020-0071.html"
                },
                "versions": {
                    "patched": [">=0.2.23"],
                    "unaffected": []
                },
                "package": {
                    "name": "time",
                    "version": "0.1.43",
                    "source": "registry+https://github.com/rust-lang/crates.io-index"
                }
            }]
        },
        "warnings": {}
    }"#;

    const SAMPLE_TWO_VULNS: &str = r#"{
        "database": {"advisory-count": 582},
        "lockfile": {"dependency-count": 42},
        "vulnerabilities": {
            "found": true,
            "count": 2,
            "list": [{
                "advisory": {
                    "id": "RUSTSEC-2020-0071",
                    "package": "time",
                    "title": "Potential segfault in the time crate",
                    "description": "Unix-like operating systems may segfault",
                    "date": "2020-11-18",
                    "aliases": ["CVE-2020-26235"],
                    "categories": ["code-execution"],
                    "keywords": ["segfault"],
                    "url": "https://rustsec.org/advisories/RUSTSEC-2020-0071.html"
                },
                "versions": {
                    "patched": [">=0.2.23"],
                    "unaffected": []
                },
                "package": {
                    "name": "time",
                    "version": "0.1.43",
                    "source": "registry+https://github.com/rust-lang/crates.io-index"
                }
            }, {
                "advisory": {
                    "id": "RUSTSEC-2021-0145",
                    "package": "atty",
                    "title": "Potential unaligned read",
                    "description": "On windows, atty dereferences a potentially unaligned pointer",
                    "date": "2021-07-04",
                    "aliases": [],
                    "categories": ["memory-corruption"],
                    "keywords": ["unaligned"],
                    "url": "https://rustsec.org/advisories/RUSTSEC-2021-0145.html"
                },
                "versions": {
                    "patched": [],
                    "unaffected": []
                },
                "package": {
                    "name": "atty",
                    "version": "0.2.14",
                    "source": "registry+https://github.com/rust-lang/crates.io-index"
                }
            }]
        },
        "warnings": {}
    }"#;

    #[test]
    fn test_parse_empty_output() {
        let result = parse_cargo_audit_output("");
        assert!(result.is_err(), "empty string should produce ParseError");
        let err = result.unwrap_err();
        assert!(
            matches!(err, ParseError::Format(_)),
            "expected Format error, got: {:?}",
            err
        );
    }

    #[test]
    fn test_parse_no_vulnerabilities() {
        let findings = parse_cargo_audit_output(SAMPLE_NO_VULNS).unwrap();
        assert!(
            findings.is_empty(),
            "zero vulnerabilities should produce empty Vec"
        );
    }

    #[test]
    fn test_parse_single_vulnerability() {
        let findings = parse_cargo_audit_output(SAMPLE_ONE_VULN).unwrap();
        assert_eq!(findings.len(), 1, "should produce exactly 1 finding");

        let f = &findings[0];
        assert_eq!(f.severity, "high");
        assert_eq!(f.native_severity, "vulnerability");
        assert_eq!(f.category, ToolCategory::SecurityScanner);
        assert_eq!(f.message, "time: Potential segfault in the time crate");
    }

    #[test]
    fn test_parse_multiple_vulnerabilities() {
        let findings = parse_cargo_audit_output(SAMPLE_TWO_VULNS).unwrap();
        assert_eq!(findings.len(), 2, "should produce exactly 2 findings");

        assert_eq!(
            findings[0].message,
            "time: Potential segfault in the time crate"
        );
        assert_eq!(findings[1].message, "atty: Potential unaligned read");
    }

    #[test]
    fn test_finding_file_is_cargo_lock() {
        let findings = parse_cargo_audit_output(SAMPLE_ONE_VULN).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].file,
            PathBuf::from("Cargo.lock"),
            "file should always be Cargo.lock"
        );
    }

    #[test]
    fn test_finding_code_is_advisory_id() {
        let findings = parse_cargo_audit_output(SAMPLE_ONE_VULN).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].code,
            Some("RUSTSEC-2020-0071".to_string()),
            "code should be the RUSTSEC advisory ID"
        );
    }

    #[test]
    fn test_finding_message_format() {
        let findings = parse_cargo_audit_output(SAMPLE_ONE_VULN).unwrap();
        assert_eq!(findings.len(), 1);

        let msg = &findings[0].message;
        assert!(
            msg.starts_with("time: "),
            "message should start with 'package: '"
        );
        assert!(
            msg.ends_with("Potential segfault in the time crate"),
            "message should end with advisory title"
        );
        assert_eq!(msg, "time: Potential segfault in the time crate");
    }

    #[test]
    fn test_malformed_json() {
        let result = parse_cargo_audit_output("{ not valid json at all");
        assert!(result.is_err(), "malformed JSON should produce ParseError");
        let err = result.unwrap_err();
        assert!(
            matches!(err, ParseError::Json(_)),
            "expected Json error, got: {:?}",
            err
        );
    }
}
