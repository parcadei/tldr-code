//! Cross-dimensional composition engine for L2 findings.
//!
//! When findings from different analysis dimensions (e.g., taint analysis +
//! guard removal, impact analysis + contract regression) co-locate on the same
//! code region, this module composes them into higher-confidence findings.
//!
//! # Composition rules
//!
//! | Finding A         | Finding B            | Composed type                    | Severity | Confidence |
//! |-------------------|----------------------|----------------------------------|----------|------------|
//! | taint-flow        | guard-removed        | unguarded-injection-path         | critical | LIKELY     |
//! | impact-blast-radius | contract-regression | high-impact-contract-regression  | high     | LIKELY     |
//! | unreachable-code  | born-dead            | broken-link                      | high     | LIKELY     |
//! | complexity-increase | (any with churn)   | hotspot                          | medium   | LIKELY     |
//! | resource-leak     | guard-removed        | resource-leak-on-error           | high     | LIKELY     |
//!
//! # Behavior
//!
//! - Composed findings REPLACE their constituent findings (no double-counting).
//! - Composed findings get confidence = "LIKELY" (two dimensions agree).
//! - Composed findings inherit the higher constituent severity or the rule
//!   severity, whichever is greater.
//! - Constituent evidence is merged into the composed finding as
//!   `constituent_a` and `constituent_b` keys.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;

use crate::commands::bugbot::types::BugbotFinding;

/// Map severity string to a numeric rank for sorting (higher = more severe).
fn severity_rank(severity: &str) -> u8 {
    match severity {
        "critical" => 5,
        "high" => 4,
        "medium" => 3,
        "low" => 2,
        "info" => 1,
        _ => 0,
    }
}

/// Convert a severity rank back to a string.
fn severity_from_rank(rank: u8) -> &'static str {
    match rank {
        5 => "critical",
        4 => "high",
        3 => "medium",
        2 => "low",
        1 => "info",
        _ => "info",
    }
}

/// Compute a deterministic finding ID for composed findings.
///
/// Uses `DefaultHasher` (SipHash) over `(composed_type, file_path, function_name, line)`
/// and formats as a lowercase hex string.
fn compute_finding_id(finding_type: &str, file: &Path, function: &str, line: usize) -> String {
    let mut hasher = DefaultHasher::new();
    finding_type.hash(&mut hasher);
    file.to_string_lossy().as_ref().hash(&mut hasher);
    function.hash(&mut hasher);
    line.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

/// Determine whether two findings overlap by location.
///
/// Two findings overlap when they share the same file and function, and their
/// line numbers are within 5 of each other (composition threshold).
fn lines_overlap(a: &BugbotFinding, b: &BugbotFinding) -> bool {
    a.file == b.file
        && a.function == b.function
        && (a.line as isize - b.line as isize).unsigned_abs() <= 5
}

/// A composition rule matching two finding types to produce a composed finding.
struct CompositionRule {
    type_a: &'static str,
    type_b: &'static str,
    composed_type: &'static str,
    composed_severity: &'static str,
    /// If true, type_b is a wildcard that matches any finding with churn data in evidence.
    b_is_churn_wildcard: bool,
}

/// The set of composition rules.
const COMPOSITION_RULES: &[CompositionRule] = &[
    CompositionRule {
        type_a: "taint-flow",
        type_b: "guard-removed",
        composed_type: "unguarded-injection-path",
        composed_severity: "critical",
        b_is_churn_wildcard: false,
    },
    CompositionRule {
        type_a: "impact-blast-radius",
        type_b: "contract-regression",
        composed_type: "high-impact-contract-regression",
        composed_severity: "high",
        b_is_churn_wildcard: false,
    },
    CompositionRule {
        type_a: "unreachable-code",
        type_b: "born-dead",
        composed_type: "broken-link",
        composed_severity: "high",
        b_is_churn_wildcard: false,
    },
    CompositionRule {
        type_a: "complexity-increase",
        type_b: "",
        composed_type: "hotspot",
        composed_severity: "medium",
        b_is_churn_wildcard: true,
    },
    CompositionRule {
        type_a: "resource-leak",
        type_b: "guard-removed",
        composed_type: "resource-leak-on-error",
        composed_severity: "high",
        b_is_churn_wildcard: false,
    },
];

/// Check if a finding has churn data in its evidence.
fn has_churn_data(finding: &BugbotFinding) -> bool {
    if let Some(obj) = finding.evidence.as_object() {
        obj.contains_key("churn")
            || obj.contains_key("churn_count")
            || obj.contains_key("git_churn")
    } else {
        false
    }
}

/// Try to match a pair of findings against the composition rules.
///
/// Returns the matching rule if found, along with which finding is A and which is B.
fn match_rule(f1: &BugbotFinding, f2: &BugbotFinding) -> Option<&'static CompositionRule> {
    for rule in COMPOSITION_RULES {
        if rule.b_is_churn_wildcard {
            // type_a must match one finding, and the other must have churn data
            if f1.finding_type == rule.type_a && has_churn_data(f2) {
                return Some(rule);
            }
            if f2.finding_type == rule.type_a && has_churn_data(f1) {
                return Some(rule);
            }
        } else {
            // Both types must match (in either order)
            if f1.finding_type == rule.type_a && f2.finding_type == rule.type_b {
                return Some(rule);
            }
            if f2.finding_type == rule.type_a && f1.finding_type == rule.type_b {
                return Some(rule);
            }
        }
    }
    None
}

/// Compose a new finding from two constituents according to a composition rule.
fn compose_finding(rule: &CompositionRule, a: &BugbotFinding, b: &BugbotFinding) -> BugbotFinding {
    // Determine which is the "first" constituent (for file/function/line)
    let first = if a.finding_type == rule.type_a { a } else { b };

    // Severity: max of constituents or rule severity, whichever is higher
    let constituent_max = std::cmp::max(severity_rank(&a.severity), severity_rank(&b.severity));
    let rule_sev = severity_rank(rule.composed_severity);
    let final_severity = severity_from_rank(std::cmp::max(constituent_max, rule_sev));

    // Merge evidence
    let evidence = serde_json::json!({
        "constituent_a": {
            "finding_type": a.finding_type,
            "severity": a.severity,
            "line": a.line,
            "message": a.message,
            "evidence": a.evidence,
        },
        "constituent_b": {
            "finding_type": b.finding_type,
            "severity": b.severity,
            "line": b.line,
            "message": b.message,
            "evidence": b.evidence,
        },
    });

    let finding_id =
        compute_finding_id(rule.composed_type, &first.file, &first.function, first.line);

    BugbotFinding {
        finding_type: rule.composed_type.to_string(),
        severity: final_severity.to_string(),
        file: first.file.clone(),
        function: first.function.clone(),
        line: first.line,
        message: format!(
            "Composed: {} + {} -> {}",
            a.finding_type, b.finding_type, rule.composed_type,
        ),
        evidence,
        confidence: Some("LIKELY".to_string()),
        finding_id: Some(finding_id),
    }
}

/// Compose findings from different analysis dimensions into higher-confidence
/// findings when they co-locate on the same code region.
///
/// See module-level docs for the composition rules. Composed findings replace
/// their constituents (no double-counting).
///
/// # Arguments
/// * `findings` - The input findings (typically after dedup)
///
/// # Returns
/// A list of findings where matched pairs have been replaced by composed findings.
pub fn compose_findings(findings: Vec<BugbotFinding>) -> Vec<BugbotFinding> {
    if findings.len() < 2 {
        return findings;
    }

    let n = findings.len();
    let mut consumed = vec![false; n];
    let mut composed: Vec<BugbotFinding> = Vec::new();

    // O(N^2) scan for pairs that share location and match a rule
    for i in 0..n {
        if consumed[i] {
            continue;
        }
        for j in (i + 1)..n {
            if consumed[j] {
                continue;
            }
            if !lines_overlap(&findings[i], &findings[j]) {
                continue;
            }
            if let Some(rule) = match_rule(&findings[i], &findings[j]) {
                let new_finding = compose_finding(rule, &findings[i], &findings[j]);
                composed.push(new_finding);
                consumed[i] = true;
                consumed[j] = true;
                break; // finding i is consumed, move on
            }
        }
    }

    // Add all unconsumed findings
    for (i, finding) in findings.into_iter().enumerate() {
        if !consumed[i] {
            composed.push(finding);
        }
    }

    composed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Helper to create a BugbotFinding with minimal fields.
    fn make_finding(
        finding_type: &str,
        severity: &str,
        file: &str,
        function: &str,
        line: usize,
    ) -> BugbotFinding {
        BugbotFinding {
            finding_type: finding_type.to_string(),
            severity: severity.to_string(),
            file: PathBuf::from(file),
            function: function.to_string(),
            line,
            message: format!("{} in {}::{} at {}", finding_type, file, function, line),
            evidence: serde_json::Value::Null,
            confidence: None,
            finding_id: None,
        }
    }

    /// Helper to create a finding with churn evidence.
    fn make_finding_with_churn(
        finding_type: &str,
        severity: &str,
        file: &str,
        function: &str,
        line: usize,
    ) -> BugbotFinding {
        let mut f = make_finding(finding_type, severity, file, function, line);
        f.evidence = serde_json::json!({
            "churn_count": 15,
            "churn": true,
        });
        f
    }

    /// PM-41.1: taint-flow + guard-removed -> unguarded-injection-path (CRITICAL).
    #[test]
    fn test_compose_taint_plus_guard_removed() {
        let findings = vec![
            make_finding("taint-flow", "high", "src/api.rs", "handle", 42),
            make_finding("guard-removed", "medium", "src/api.rs", "handle", 44),
        ];

        let result = compose_findings(findings);

        assert_eq!(
            result.len(),
            1,
            "should produce exactly one composed finding"
        );
        assert_eq!(result[0].finding_type, "unguarded-injection-path");
        assert_eq!(result[0].severity, "critical");
        assert_eq!(result[0].confidence, Some("LIKELY".to_string()));
    }

    /// PM-41.2: impact-blast-radius + contract-regression -> high-impact-contract-regression.
    #[test]
    fn test_compose_impact_plus_contract() {
        let findings = vec![
            make_finding(
                "impact-blast-radius",
                "medium",
                "src/core.rs",
                "process",
                100,
            ),
            make_finding("contract-regression", "high", "src/core.rs", "process", 102),
        ];

        let result = compose_findings(findings);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].finding_type, "high-impact-contract-regression");
        assert_eq!(result[0].severity, "high");
    }

    /// PM-41.3: unreachable-code + born-dead -> broken-link.
    #[test]
    fn test_compose_unreachable_plus_born_dead() {
        let findings = vec![
            make_finding("unreachable-code", "medium", "src/lib.rs", "dead_fn", 50),
            make_finding("born-dead", "low", "src/lib.rs", "dead_fn", 50),
        ];

        let result = compose_findings(findings);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].finding_type, "broken-link");
        assert_eq!(result[0].severity, "high");
    }

    /// PM-41.4: Composed findings REPLACE constituents (no double-counting).
    #[test]
    fn test_compose_replaces_constituents() {
        let findings = vec![
            make_finding("taint-flow", "high", "src/api.rs", "handle", 42),
            make_finding("guard-removed", "medium", "src/api.rs", "handle", 44),
            make_finding("dead-store", "low", "src/other.rs", "other", 10),
        ];

        let result = compose_findings(findings);

        // Should have: 1 composed + 1 passthrough = 2
        assert_eq!(result.len(), 2);

        // No original taint-flow or guard-removed should remain
        assert!(!result.iter().any(|f| f.finding_type == "taint-flow"));
        assert!(!result.iter().any(|f| f.finding_type == "guard-removed"));

        // Composed finding and passthrough
        assert!(result
            .iter()
            .any(|f| f.finding_type == "unguarded-injection-path"));
        assert!(result.iter().any(|f| f.finding_type == "dead-store"));
    }

    /// PM-41.5: All composed findings get LIKELY confidence.
    #[test]
    fn test_compose_confidence_is_likely() {
        let findings = vec![
            make_finding("resource-leak", "high", "src/db.rs", "connect", 30),
            make_finding("guard-removed", "medium", "src/db.rs", "connect", 32),
        ];

        let result = compose_findings(findings);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].finding_type, "resource-leak-on-error");
        assert_eq!(result[0].confidence, Some("LIKELY".to_string()));
    }

    /// PM-41.6: Findings that don't match any rule pass through unchanged.
    #[test]
    fn test_compose_no_match_passthrough() {
        let findings = vec![
            make_finding("dead-store", "low", "src/lib.rs", "foo", 10),
            make_finding("null-deref", "high", "src/lib.rs", "foo", 12),
            make_finding("complexity-increase", "medium", "src/other.rs", "bar", 50),
        ];

        let result = compose_findings(findings);

        // None of these pairs match composition rules, so all pass through
        assert_eq!(result.len(), 3);
        assert!(result.iter().any(|f| f.finding_type == "dead-store"));
        assert!(result.iter().any(|f| f.finding_type == "null-deref"));
        assert!(result
            .iter()
            .any(|f| f.finding_type == "complexity-increase"));
    }

    /// PM-41.7: Constituent evidence is merged into composed finding.
    #[test]
    fn test_compose_evidence_merge() {
        let mut a = make_finding("taint-flow", "high", "src/api.rs", "handle", 42);
        a.evidence = serde_json::json!({"source": "user_input", "sink": "sql_query"});

        let mut b = make_finding("guard-removed", "medium", "src/api.rs", "handle", 44);
        b.evidence = serde_json::json!({"guard": "sanitize_input", "removed_at": "commit_abc"});

        let result = compose_findings(vec![a, b]);

        assert_eq!(result.len(), 1);
        let evidence = &result[0].evidence;

        // Should have constituent_a and constituent_b
        assert!(
            evidence.get("constituent_a").is_some(),
            "should have constituent_a"
        );
        assert!(
            evidence.get("constituent_b").is_some(),
            "should have constituent_b"
        );

        // constituent_a should contain the taint-flow evidence
        let ca = evidence.get("constituent_a").unwrap();
        assert_eq!(
            ca.get("finding_type").unwrap().as_str().unwrap(),
            "taint-flow"
        );
        assert!(ca.get("evidence").is_some());

        // constituent_b should contain the guard-removed evidence
        let cb = evidence.get("constituent_b").unwrap();
        assert_eq!(
            cb.get("finding_type").unwrap().as_str().unwrap(),
            "guard-removed"
        );
        assert!(cb.get("evidence").is_some());
    }

    /// PM-41.8: complexity-increase + any finding with churn data -> hotspot.
    #[test]
    fn test_compose_complexity_plus_churn() {
        let findings = vec![
            make_finding("complexity-increase", "low", "src/hot.rs", "process", 50),
            make_finding_with_churn("dead-store", "medium", "src/hot.rs", "process", 52),
        ];

        let result = compose_findings(findings);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].finding_type, "hotspot");
        // Severity should be max(low, medium, medium) = medium
        assert_eq!(result[0].severity, "medium");
        assert_eq!(result[0].confidence, Some("LIKELY".to_string()));
    }

    /// PM-41.9: resource-leak + guard-removed -> resource-leak-on-error.
    #[test]
    fn test_compose_resource_leak_plus_guard_removed() {
        let findings = vec![
            make_finding("resource-leak", "high", "src/db.rs", "connect", 30),
            make_finding("guard-removed", "medium", "src/db.rs", "connect", 32),
        ];

        let result = compose_findings(findings);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].finding_type, "resource-leak-on-error");
        assert_eq!(result[0].severity, "high");
        assert!(result[0].finding_id.is_some(), "should have a finding_id");
    }
}
