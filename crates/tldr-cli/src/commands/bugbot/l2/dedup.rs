//! Post-collection deduplication and prioritization for L2 findings.
//!
//! Groups findings by location (file + function + line proximity), applies
//! suppression rules (born-dead, taint-flow-beats-vulnerability, root-cause
//! precedence), and sorts by severity for final output.
//!
//! # Grouping
//!
//! Two findings overlap when they share the same file and function, and their
//! line numbers are within 3 of each other.
//!
//! # Suppression rules
//!
//! 1. Within a group, the highest-severity finding is kept; others are stored
//!    as `related_findings` in the kept finding's evidence JSON.
//! 2. A `born-dead` finding suppresses ALL other findings for the same function
//!    (except itself).
//! 3. `taint-flow` beats `vulnerability` at the same location (more precise).
//! 4. `uninitialized-use` beats any downstream finding on the same variable
//!    (root cause takes precedence).
//!
//! # Sorting
//!
//! Final output is sorted by severity (critical > high > medium > low > info),
//! then file path, then line number.

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

/// Determine whether two findings overlap by location.
///
/// Two findings overlap when they share the same file and function, and their
/// line numbers are within `threshold` of each other.
fn lines_overlap(a: &BugbotFinding, b: &BugbotFinding, threshold: usize) -> bool {
    a.file == b.file
        && a.function == b.function
        && (a.line as isize - b.line as isize).unsigned_abs() <= threshold
}

/// Deduplicate and prioritize a list of bugbot findings.
///
/// Applies grouping, suppression rules, severity sorting, and optional
/// truncation. See module-level docs for the full algorithm.
///
/// # Arguments
/// * `findings` - The raw findings to deduplicate
/// * `max` - Maximum number of findings to return (0 = unlimited)
///
/// # Returns
/// A deduplicated, sorted, and optionally truncated list of findings.
pub fn dedup_and_prioritize(findings: Vec<BugbotFinding>, max: usize) -> Vec<BugbotFinding> {
    if findings.is_empty() {
        return findings;
    }

    // Phase 1: Identify functions with born-dead findings
    let born_dead_functions: Vec<(String, String)> = findings
        .iter()
        .filter(|f| f.finding_type == "born-dead")
        .map(|f| (f.file.to_string_lossy().to_string(), f.function.clone()))
        .collect();

    // Phase 2: Apply born-dead suppression
    // If a function has a born-dead finding, suppress ALL other findings for that function
    let mut after_born_dead: Vec<BugbotFinding> = Vec::new();
    for finding in findings {
        let key = (finding.file.to_string_lossy().to_string(), finding.function.clone());
        if finding.finding_type == "born-dead" {
            // Always keep born-dead findings
            after_born_dead.push(finding);
        } else if born_dead_functions.contains(&key) {
            // Suppress: another finding exists for a function that has born-dead
            continue;
        } else {
            after_born_dead.push(finding);
        }
    }

    // Phase 3: Apply taint-flow-beats-vulnerability suppression
    // For same location: if both taint-flow and vulnerability exist, keep taint-flow
    let mut taint_locations: Vec<(String, String, usize)> = Vec::new();
    for f in &after_born_dead {
        if f.finding_type == "taint-flow" {
            taint_locations.push((
                f.file.to_string_lossy().to_string(),
                f.function.clone(),
                f.line,
            ));
        }
    }

    let mut after_taint: Vec<BugbotFinding> = Vec::new();
    for finding in after_born_dead {
        if finding.finding_type == "vulnerability" {
            let file_str = finding.file.to_string_lossy();
            let dominated = taint_locations.iter().any(|(file, func, line)| {
                file.as_str() == file_str.as_ref()
                    && func == &finding.function
                    && (*line as isize - finding.line as isize).unsigned_abs() <= 3
            });
            if dominated {
                continue;
            }
        }
        after_taint.push(finding);
    }

    // Phase 4: Apply uninitialized-use root-cause suppression
    // For same function+variable: if uninitialized-use exists, suppress downstream findings
    let mut uninit_functions: Vec<(String, String)> = Vec::new();
    for f in &after_taint {
        if f.finding_type == "uninitialized-use" {
            uninit_functions.push((
                f.file.to_string_lossy().to_string(),
                f.function.clone(),
            ));
        }
    }

    let mut after_uninit: Vec<BugbotFinding> = Vec::new();
    for finding in after_taint {
        if finding.finding_type == "uninitialized-use" {
            after_uninit.push(finding);
        } else {
            let file_str = finding.file.to_string_lossy();
            let dominated = uninit_functions.iter().any(|(file, func)| {
                file.as_str() == file_str.as_ref()
                    && func == &finding.function
            });
            if dominated {
                // Check if this finding mentions the same variable in evidence
                // Heuristic: suppress findings in same function as uninitialized-use
                continue;
            }
            after_uninit.push(finding);
        }
    }

    // Phase 5: Group by location (file + function + line proximity within 3)
    // Use a union-find-like approach: findings in the same file+function are candidates,
    // and we merge groups when any two members are within 3 lines.
    let mut groups: Vec<Vec<BugbotFinding>> = Vec::new();

    for finding in after_uninit {
        let mut merged = false;
        for group in &mut groups {
            if group.iter().any(|g| lines_overlap(g, &finding, 3)) {
                group.push(finding.clone());
                merged = true;
                break;
            }
        }
        if !merged {
            groups.push(vec![finding]);
        }
    }

    // Phase 6: Within each group, keep highest-severity finding
    // Store suppressed findings as related_findings in evidence
    let mut result: Vec<BugbotFinding> = Vec::new();
    for mut group in groups {
        if group.len() == 1 {
            result.push(group.remove(0));
        } else {
            // Sort group by severity descending
            group.sort_by(|a, b| severity_rank(&b.severity).cmp(&severity_rank(&a.severity)));

            let mut best = group.remove(0);

            // Store remaining as related_findings in evidence
            let related: Vec<serde_json::Value> = group
                .iter()
                .map(|f| {
                    serde_json::json!({
                        "finding_type": f.finding_type,
                        "severity": f.severity,
                        "line": f.line,
                        "message": f.message,
                    })
                })
                .collect();

            if !related.is_empty() {
                let mut evidence = if best.evidence.is_object() {
                    best.evidence.clone()
                } else {
                    serde_json::json!({})
                };
                evidence["related_findings"] = serde_json::Value::Array(related);
                best.evidence = evidence;
            }

            result.push(best);
        }
    }

    // Phase 7: Sort by severity (desc), then file (asc), then line (asc)
    result.sort_by(|a, b| {
        severity_rank(&b.severity)
            .cmp(&severity_rank(&a.severity))
            .then(a.file.cmp(&b.file))
            .then(a.line.cmp(&b.line))
    });

    // Phase 8: Truncate at max (0 = unlimited)
    if max > 0 && result.len() > max {
        result.truncate(max);
    }

    result
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

    /// CK-4.1: Findings at the same file+function+nearby lines get grouped together.
    #[test]
    fn test_dedup_groups_by_file_function_line() {
        let findings = vec![
            make_finding("signature-regression", "high", "src/lib.rs", "foo", 10),
            make_finding("born-dead", "low", "src/lib.rs", "foo", 12),  // within 3 lines
            make_finding("complexity-increase", "medium", "src/other.rs", "bar", 50),
        ];

        let result = dedup_and_prioritize(findings, 0);

        // The born-dead in foo should suppress the signature-regression (born-dead rule),
        // and complexity-increase in bar should remain separate
        assert!(result.iter().any(|f| f.finding_type == "born-dead"));
        assert!(result.iter().any(|f| f.finding_type == "complexity-increase"));
        // signature-regression should be suppressed by born-dead
        let lib_rs = std::path::Path::new("src/lib.rs");
        assert!(!result.iter().any(|f| {
            f.finding_type == "signature-regression"
                && f.file == lib_rs
                && f.function == "foo"
        }));
    }

    /// CK-4.2: Within a group, the highest severity finding wins.
    #[test]
    fn test_dedup_keeps_highest_severity() {
        let findings = vec![
            make_finding("complexity-increase", "low", "src/lib.rs", "foo", 10),
            make_finding("contract-regression", "high", "src/lib.rs", "foo", 11),
            make_finding("dead-store", "medium", "src/lib.rs", "foo", 12),
        ];

        let result = dedup_and_prioritize(findings, 0);

        // Only one finding should remain for foo (the highest severity one)
        let foo_findings: Vec<&BugbotFinding> = result
            .iter()
            .filter(|f| f.function == "foo")
            .collect();
        assert_eq!(foo_findings.len(), 1);
        assert_eq!(foo_findings[0].severity, "high");
        assert_eq!(foo_findings[0].finding_type, "contract-regression");

        // Check that related_findings are stored in evidence
        let evidence = &foo_findings[0].evidence;
        let related = evidence.get("related_findings").expect("should have related_findings");
        assert!(related.is_array());
        assert_eq!(related.as_array().unwrap().len(), 2);
    }

    /// CK-4.3: born-dead suppresses ALL other findings for the same function.
    #[test]
    fn test_dedup_born_dead_suppresses_all() {
        let findings = vec![
            make_finding("signature-regression", "high", "src/lib.rs", "foo", 10),
            make_finding("complexity-increase", "medium", "src/lib.rs", "foo", 20),
            make_finding("born-dead", "low", "src/lib.rs", "foo", 15),
            make_finding("dead-store", "high", "src/other.rs", "bar", 5),
        ];

        let result = dedup_and_prioritize(findings, 0);

        // foo should only have born-dead
        let foo_findings: Vec<&BugbotFinding> = result
            .iter()
            .filter(|f| f.function == "foo")
            .collect();
        assert_eq!(foo_findings.len(), 1);
        assert_eq!(foo_findings[0].finding_type, "born-dead");

        // bar's finding should remain
        assert!(result.iter().any(|f| f.function == "bar"));
    }

    /// CK-4.4: taint-flow beats vulnerability at the same location.
    #[test]
    fn test_dedup_taint_flow_beats_vulnerability() {
        let findings = vec![
            make_finding("vulnerability", "high", "src/api.rs", "handle", 42),
            make_finding("taint-flow", "high", "src/api.rs", "handle", 43),
        ];

        let result = dedup_and_prioritize(findings, 0);

        // vulnerability should be suppressed, taint-flow kept
        assert!(result.iter().any(|f| f.finding_type == "taint-flow"));
        assert!(!result.iter().any(|f| f.finding_type == "vulnerability"));
    }

    /// CK-4.5: uninitialized-use is kept as root cause, downstream findings suppressed.
    #[test]
    fn test_dedup_uninitialized_root_cause() {
        let findings = vec![
            make_finding("uninitialized-use", "high", "src/lib.rs", "process", 10),
            make_finding("null-deref", "high", "src/lib.rs", "process", 20),
            make_finding("div-zero", "medium", "src/lib.rs", "process", 30),
        ];

        let result = dedup_and_prioritize(findings, 0);

        // Only uninitialized-use should remain for process()
        let process_findings: Vec<&BugbotFinding> = result
            .iter()
            .filter(|f| f.function == "process")
            .collect();
        assert_eq!(process_findings.len(), 1);
        assert_eq!(process_findings[0].finding_type, "uninitialized-use");
    }

    /// CK-4.6: Sort order is critical > high > medium > low > info, then file, then line.
    #[test]
    fn test_dedup_sort_order() {
        let findings = vec![
            make_finding("a", "low", "z.rs", "z", 100),
            make_finding("b", "critical", "a.rs", "a", 1),
            make_finding("c", "medium", "b.rs", "b", 50),
            make_finding("d", "high", "c.rs", "c", 25),
            make_finding("e", "info", "d.rs", "d", 75),
        ];

        let result = dedup_and_prioritize(findings, 0);

        assert_eq!(result.len(), 5);
        assert_eq!(result[0].severity, "critical");
        assert_eq!(result[1].severity, "high");
        assert_eq!(result[2].severity, "medium");
        assert_eq!(result[3].severity, "low");
        assert_eq!(result[4].severity, "info");
    }

    /// CK-4.7: max > 0 truncates after dedup.
    #[test]
    fn test_dedup_truncation() {
        let findings = vec![
            make_finding("a", "high", "a.rs", "a", 1),
            make_finding("b", "medium", "b.rs", "b", 2),
            make_finding("c", "low", "c.rs", "c", 3),
            make_finding("d", "info", "d.rs", "d", 4),
            make_finding("e", "high", "e.rs", "e", 5),
        ];

        let result = dedup_and_prioritize(findings, 3);

        assert_eq!(result.len(), 3);
        // Should keep highest severity first
        assert_eq!(result[0].severity, "high");
        assert_eq!(result[1].severity, "high");
        assert_eq!(result[2].severity, "medium");
    }

    /// CK-4.8: max=0 means unlimited.
    #[test]
    fn test_dedup_zero_max_no_truncation() {
        let findings: Vec<BugbotFinding> = (0..20)
            .map(|i| make_finding("test", "low", &format!("f{}.rs", i), &format!("fn_{}", i), i))
            .collect();

        let result = dedup_and_prioritize(findings, 0);

        assert_eq!(result.len(), 20);
    }

    /// CK-4.9: Empty input returns empty output.
    #[test]
    fn test_dedup_empty_input() {
        let result = dedup_and_prioritize(Vec::new(), 0);
        assert!(result.is_empty());
    }

    /// CK-4.10: Findings far apart in line number stay separate.
    #[test]
    fn test_dedup_no_overlap_no_grouping() {
        let findings = vec![
            make_finding("a", "high", "src/lib.rs", "foo", 10),
            make_finding("b", "high", "src/lib.rs", "foo", 100),
            make_finding("c", "high", "src/lib.rs", "foo", 200),
        ];

        let result = dedup_and_prioritize(findings, 0);

        // All three should remain (lines too far apart)
        assert_eq!(result.len(), 3);
    }
}
