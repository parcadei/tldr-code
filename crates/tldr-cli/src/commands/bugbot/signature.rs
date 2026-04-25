//! Signature regression detection for bugbot
//!
//! Detects when a function's signature changes (parameters, return type,
//! generics) between baseline and current. This is the primary regression
//! signal for typed languages: if a public function's signature changed,
//! every call site must be updated or the build breaks.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::types::BugbotFinding;
use crate::commands::remaining::types::{ASTChange, ChangeType, NodeKind};

/// Evidence attached to a signature regression finding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignatureRegressionEvidence {
    /// The function signature before the change
    pub before_signature: String,
    /// The function signature after the change
    pub after_signature: String,
    /// Individual changes detected between old and new signatures
    pub changes: Vec<SignatureChange>,
}

/// A single change within a function signature.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SignatureChange {
    /// Category of change: "param_removed", "param_added", "param_type_changed",
    /// "param_renamed", "return_type_changed", "generic_changed"
    pub change_type: String,
    /// Human-readable description, e.g. "removed parameter: y: String"
    pub detail: String,
}

/// Analyze AST diff changes for signature regressions.
///
/// Takes a map of file path -> changes (from the diff phase) and produces
/// findings for any function whose signature changed between baseline and current.
pub fn compose_signature_regression(
    file_diffs: &HashMap<PathBuf, Vec<ASTChange>>,
    project_root: &Path,
) -> Vec<BugbotFinding> {
    let mut findings = Vec::new();

    for (file, changes) in file_diffs {
        for change in changes {
            if !is_update_function(change) {
                continue;
            }

            let old_text = change.old_text.as_deref().unwrap_or("");
            let new_text = change.new_text.as_deref().unwrap_or("");

            let old_sig = extract_signature(old_text);
            let new_sig = extract_signature(new_text);

            if old_sig == new_sig {
                continue; // Body-only change, no signature regression
            }

            let sig_changes = diff_signatures(&old_sig, &new_sig);
            if sig_changes.is_empty() {
                continue;
            }

            // Guard against AST differ mismatches: when multiple functions share
            // the same unqualified name (e.g. `new` on different impl blocks),
            // the differ can pair the wrong old/new functions. Detect this by
            // checking for visibility change + all-params-removed/added, which
            // almost never happens in a real refactor.
            if is_likely_differ_mismatch(&old_sig, &new_sig, &sig_changes) {
                continue;
            }

            let severity = max_severity(&sig_changes);
            let func_name = change.name.clone().unwrap_or_else(|| "unknown".to_string());
            let relative_file = file.strip_prefix(project_root).unwrap_or(file);

            let evidence = SignatureRegressionEvidence {
                before_signature: old_sig,
                after_signature: new_sig,
                changes: sig_changes.clone(),
            };

            let line = change
                .new_location
                .as_ref()
                .map(|loc| loc.line as usize)
                .unwrap_or(0);

            findings.push(BugbotFinding {
                finding_type: "signature-regression".to_string(),
                severity: severity.to_string(),
                file: relative_file.to_path_buf(),
                function: func_name,
                line,
                message: format_message(&sig_changes),
                evidence: serde_json::to_value(&evidence).unwrap_or_default(),
                confidence: None,
                finding_id: None,
            });
        }
    }

    findings
}

/// Returns true if the change is an Update to a Function or Method node.
fn is_update_function(change: &ASTChange) -> bool {
    matches!(change.change_type, ChangeType::Update)
        && matches!(change.node_kind, NodeKind::Function | NodeKind::Method)
}

/// Detect likely AST differ mismatch when multiple functions share the same
/// unqualified name (e.g., `new` on `impl Foo` vs `impl Bar`).
///
/// Heuristic: if the visibility changed (pub ↔ non-pub) AND all signature
/// changes are pure param additions or removals (no type changes or renames),
/// the differ probably paired the wrong functions.
fn is_likely_differ_mismatch(old_sig: &str, new_sig: &str, changes: &[SignatureChange]) -> bool {
    let old_is_pub = old_sig.trim_start().starts_with("pub ");
    let new_is_pub = new_sig.trim_start().starts_with("pub ");

    // Visibility must differ
    if old_is_pub == new_is_pub {
        return false;
    }

    // All changes must be pure param additions or removals
    changes
        .iter()
        .all(|c| c.change_type == "param_removed" || c.change_type == "param_added")
}

/// Extract the function signature from the full function text.
///
/// For Rust, the signature is everything from the start of the text up to
/// (but not including) the opening `{`. This captures visibility modifiers,
/// `fn` keyword, name, generics, parameters, return type, and where clauses.
pub fn extract_signature(function_text: &str) -> String {
    if let Some(brace_pos) = find_top_level_brace(function_text) {
        function_text[..brace_pos].trim().to_string()
    } else {
        // No opening brace found (e.g. trait method declaration)
        function_text.trim().to_string()
    }
}

/// Find the position of the first top-level `{` that is not inside angle brackets
/// or parentheses. This handles cases like `fn foo(x: HashMap<K, V>) {`.
fn find_top_level_brace(text: &str) -> Option<usize> {
    let mut angle_depth: i32 = 0;
    let mut paren_depth: i32 = 0;

    for (i, ch) in text.char_indices() {
        match ch {
            '<' => angle_depth += 1,
            '>' if angle_depth > 0 => angle_depth -= 1,
            '(' => paren_depth += 1,
            ')' if paren_depth > 0 => paren_depth -= 1,
            '{' if angle_depth == 0 && paren_depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

/// Compare old and new signatures to find specific changes.
///
/// Uses positional parameter matching first (since Rust params are positional),
/// then falls back to name-based matching for remaining unmatched params.
/// This avoids false positives when a parameter is simply renamed without
/// changing its type -- a non-breaking change in Rust.
pub fn diff_signatures(old_sig: &str, new_sig: &str) -> Vec<SignatureChange> {
    let mut changes = Vec::new();

    let old_params = parse_params(old_sig);
    let new_params = parse_params(new_sig);
    let old_return = parse_return_type(old_sig);
    let new_return = parse_return_type(new_sig);
    let old_generics = parse_generics(old_sig);
    let new_generics = parse_generics(new_sig);

    // Phase 1: Positional comparison for overlapping indices
    let common_len = old_params.len().min(new_params.len());
    let mut old_matched = vec![false; old_params.len()];
    let mut new_matched = vec![false; new_params.len()];

    for i in 0..common_len {
        let old_name = param_name(&old_params[i]);
        let new_name = param_name(&new_params[i]);
        let old_type = param_type(&old_params[i]);
        let new_type = param_type(&new_params[i]);

        if old_name == new_name && old_type == new_type {
            // Identical parameter at this position -- no change
            old_matched[i] = true;
            new_matched[i] = true;
        } else if old_name == new_name && old_type != new_type {
            // Same name, different type -- type changed
            changes.push(SignatureChange {
                change_type: "param_type_changed".to_string(),
                detail: format!(
                    "parameter type changed: {} -> {}",
                    old_params[i].trim(),
                    new_params[i].trim()
                ),
            });
            old_matched[i] = true;
            new_matched[i] = true;
        } else if old_name != new_name && old_type == new_type {
            // Different name, same type -- just a rename (not breaking in Rust)
            changes.push(SignatureChange {
                change_type: "param_renamed".to_string(),
                detail: format!(
                    "parameter renamed: {} -> {} (type unchanged: {})",
                    old_name, new_name, old_type
                ),
            });
            old_matched[i] = true;
            new_matched[i] = true;
        } else {
            // Different name AND different type at the same position.
            // In Rust, this means the parameter at position i changed both
            // name and type. Since params are positional, this is effectively
            // a type change at that position (the rename is incidental).
            changes.push(SignatureChange {
                change_type: "param_type_changed".to_string(),
                detail: format!(
                    "parameter type changed: {} -> {}",
                    old_params[i].trim(),
                    new_params[i].trim()
                ),
            });
            old_matched[i] = true;
            new_matched[i] = true;
        }
    }

    // Phase 2: Name-based matching for unmatched params (handles reordering,
    // true additions, and true removals).
    for (i, param) in old_params.iter().enumerate() {
        if old_matched[i] {
            continue;
        }
        let old_name = param_name(param);
        if let Some(j) = new_params
            .iter()
            .enumerate()
            .position(|(j, p)| !new_matched[j] && param_name(p) == old_name)
        {
            // Same name found elsewhere -- check if type changed
            new_matched[j] = true;
            old_matched[i] = true;
            if normalize_whitespace(param) != normalize_whitespace(&new_params[j]) {
                changes.push(SignatureChange {
                    change_type: "param_type_changed".to_string(),
                    detail: format!(
                        "parameter type changed: {} -> {}",
                        param.trim(),
                        new_params[j].trim()
                    ),
                });
            }
        } else {
            changes.push(SignatureChange {
                change_type: "param_removed".to_string(),
                detail: format!("removed parameter: {}", param.trim()),
            });
        }
    }

    for (j, param) in new_params.iter().enumerate() {
        if new_matched[j] {
            continue;
        }
        changes.push(SignatureChange {
            change_type: "param_added".to_string(),
            detail: format!("added parameter: {}", param.trim()),
        });
    }

    // Check return type
    if old_return != new_return {
        changes.push(SignatureChange {
            change_type: "return_type_changed".to_string(),
            detail: format!(
                "return type: {} -> {}",
                old_return.as_deref().unwrap_or("()"),
                new_return.as_deref().unwrap_or("()")
            ),
        });
    }

    // Check generics
    if old_generics != new_generics {
        changes.push(SignatureChange {
            change_type: "generic_changed".to_string(),
            detail: format!(
                "generics: {} -> {}",
                old_generics.as_deref().unwrap_or("none"),
                new_generics.as_deref().unwrap_or("none")
            ),
        });
    }

    changes
}

/// Extract the parameter name from a "name: type" pair.
/// Returns the trimmed name portion before the first `:`.
fn param_name(param: &str) -> String {
    let trimmed = param.trim();
    if let Some(colon_pos) = trimmed.find(':') {
        trimmed[..colon_pos].trim().to_string()
    } else {
        trimmed.to_string()
    }
}

/// Extract the parameter type from a "name: type" pair.
/// Returns the normalized (whitespace-collapsed) type portion after the first `:`.
fn param_type(param: &str) -> String {
    let trimmed = param.trim();
    if let Some(colon_pos) = trimmed.find(':') {
        normalize_whitespace(trimmed[colon_pos + 1..].trim())
    } else {
        String::new()
    }
}

/// Normalize whitespace in a string to a single space for comparison.
fn normalize_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Parse the parameter list from a Rust function signature.
///
/// Extracts the content between the first `(` and its matching `)`,
/// then splits on top-level commas (not inside `<>` or `()`).
/// Filters out `self`, `&self`, and `&mut self` parameters since
/// receiver changes are not signature regressions.
pub fn parse_params(sig: &str) -> Vec<String> {
    let param_content = match extract_paren_content(sig) {
        Some(content) => content,
        None => return Vec::new(),
    };

    if param_content.trim().is_empty() {
        return Vec::new();
    }

    let parts = split_top_level(&param_content, ',');
    parts
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .filter(|s| !is_self_param(s))
        .collect()
}

/// Returns true if the parameter string is a self receiver (`self`, `&self`,
/// `&mut self`, `mut self`).
fn is_self_param(param: &str) -> bool {
    let trimmed = param.trim();
    matches!(trimmed, "self" | "&self" | "&mut self" | "mut self")
}

/// Extract the content between the first `(` and its matching `)`.
fn extract_paren_content(sig: &str) -> Option<String> {
    let open = sig.find('(')?;
    let mut depth: i32 = 0;
    for (i, ch) in sig[open..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(sig[open + 1..open + i].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

/// Parse the return type from a Rust function signature.
///
/// Looks for `->` after the closing `)` of the parameter list.
/// Returns `None` if there is no return type (implicit `()`).
pub fn parse_return_type(sig: &str) -> Option<String> {
    // Find the closing paren of the param list
    let close_paren = find_matching_close_paren(sig)?;
    let after_params = &sig[close_paren + 1..];

    // Look for `->` in the remaining text
    if let Some(arrow_pos) = after_params.find("->") {
        let ret_type = after_params[arrow_pos + 2..].trim();
        // Strip any where clause: everything after `where` keyword at word boundary
        let ret_type = strip_where_clause(ret_type);
        if ret_type.is_empty() {
            None
        } else {
            Some(ret_type.to_string())
        }
    } else {
        None
    }
}

/// Find the position of the closing `)` matching the first `(` in the signature.
fn find_matching_close_paren(sig: &str) -> Option<usize> {
    let open = sig.find('(')?;
    let mut depth: i32 = 0;
    for (i, ch) in sig[open..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Parse generic parameters from the function signature.
///
/// Extracts the content between `<` and `>` immediately after the function name
/// (before the parameter list `(`). Returns `None` if no generics are present.
pub fn parse_generics(sig: &str) -> Option<String> {
    // Find the fn keyword and name, then look for `<` before `(`
    let fn_pos = sig.find("fn ")?;
    let after_fn = &sig[fn_pos + 3..];
    let paren_pos = after_fn.find('(')?;
    let before_paren = &after_fn[..paren_pos];

    // Look for generics: the part between < and > in the name section
    let angle_open = before_paren.find('<')?;
    let mut depth: i32 = 0;
    for (i, ch) in before_paren[angle_open..].char_indices() {
        match ch {
            '<' => depth += 1,
            '>' => {
                depth -= 1;
                if depth == 0 {
                    return Some(before_paren[angle_open..angle_open + i + 1].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

/// Strip a `where` clause from the return type text.
///
/// Recognizes `where` as a keyword when preceded by whitespace or at the start.
fn strip_where_clause(text: &str) -> String {
    // Split on ` where ` or leading `where `
    if let Some(pos) = find_where_keyword(text) {
        text[..pos].trim().to_string()
    } else {
        text.trim().to_string()
    }
}

/// Find the position of the `where` keyword in text, ensuring it is a word boundary.
fn find_where_keyword(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let pattern = b"where";
    for i in 0..text.len().saturating_sub(4) {
        if &bytes[i..i + 5] == pattern {
            // Check that it is at a word boundary
            let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
            let after_ok = i + 5 >= bytes.len() || !bytes[i + 5].is_ascii_alphanumeric();
            if before_ok && after_ok {
                return Some(i);
            }
        }
    }
    None
}

/// Split a string on a delimiter, respecting nested `<>` and `()`.
fn split_top_level(text: &str, delimiter: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut angle_depth: i32 = 0;
    let mut paren_depth: i32 = 0;

    for ch in text.chars() {
        match ch {
            '<' => {
                angle_depth += 1;
                current.push(ch);
            }
            '>' if angle_depth > 0 => {
                angle_depth -= 1;
                current.push(ch);
            }
            '(' => {
                paren_depth += 1;
                current.push(ch);
            }
            ')' if paren_depth > 0 => {
                paren_depth -= 1;
                current.push(ch);
            }
            c if c == delimiter && angle_depth == 0 && paren_depth == 0 => {
                parts.push(current.clone());
                current.clear();
            }
            _ => {
                current.push(ch);
            }
        }
    }
    if !current.is_empty() || !parts.is_empty() {
        parts.push(current);
    }
    parts
}

/// Determine the maximum severity from a set of signature changes.
///
/// - `param_removed` or `return_type_changed` => "high"
/// - `param_added`, `param_type_changed`, or `generic_changed` => "medium"
/// - `param_renamed` => "low" (not a breaking change in positional languages)
/// - everything else => "low"
fn max_severity(changes: &[SignatureChange]) -> &str {
    if changes
        .iter()
        .any(|c| c.change_type == "param_removed" || c.change_type == "return_type_changed")
    {
        "high"
    } else if changes.iter().any(|c| {
        c.change_type == "param_added"
            || c.change_type == "param_type_changed"
            || c.change_type == "generic_changed"
    }) {
        "medium"
    } else if changes.iter().all(|c| c.change_type == "param_renamed") {
        // Pure param renames are non-breaking in Rust (positional args).
        // Report as info for awareness, not as actionable regression.
        "info"
    } else {
        "low"
    }
}

/// Format a human-readable message summarizing the signature changes.
fn format_message(changes: &[SignatureChange]) -> String {
    if changes.len() == 1 {
        format!("Signature regression: {}", changes[0].detail)
    } else {
        let details: Vec<&str> = changes.iter().map(|c| c.detail.as_str()).collect();
        format!(
            "Signature regression ({} changes): {}",
            changes.len(),
            details.join("; ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::remaining::types::{ASTChange, ChangeType, NodeKind};

    /// Helper to build a minimal Update ASTChange for a function with old/new text.
    fn make_update(name: &str, old_text: &str, new_text: &str) -> ASTChange {
        use crate::commands::remaining::types::Location;
        ASTChange {
            change_type: ChangeType::Update,
            node_kind: NodeKind::Function,
            name: Some(name.to_string()),
            old_location: None,
            new_location: Some(Location::new("test.rs", 10)),
            old_text: Some(old_text.to_string()),
            new_text: Some(new_text.to_string()),
            similarity: None,
            children: None,
            base_changes: None,
        }
    }

    /// Helper to run compose on a single change and return findings.
    fn compose_one(change: ASTChange) -> Vec<BugbotFinding> {
        let mut file_diffs = HashMap::new();
        file_diffs.insert(PathBuf::from("/project/src/lib.rs"), vec![change]);
        compose_signature_regression(&file_diffs, Path::new("/project"))
    }

    // =========================================================================
    // Test 1: Identical signatures produce no findings
    // =========================================================================
    #[test]
    fn test_no_regression_identical_signatures() {
        let old = "fn compute(x: i32, y: i32) -> i32 {\n    x + y\n}";
        let new = "fn compute(x: i32, y: i32) -> i32 {\n    x * y\n}";
        let change = make_update("compute", old, new);
        let findings = compose_one(change);
        assert!(
            findings.is_empty(),
            "Body-only change should produce 0 findings, got: {:?}",
            findings
        );
    }

    // =========================================================================
    // Test 2: Param removed is detected as HIGH severity
    // =========================================================================
    #[test]
    fn test_param_removed_detected() {
        let old = "fn process(x: i32, y: String) -> bool {\n    true\n}";
        let new = "fn process(x: i32) -> bool {\n    true\n}";
        let change = make_update("process", old, new);
        let findings = compose_one(change);

        assert_eq!(findings.len(), 1, "Should detect exactly 1 finding");
        assert_eq!(findings[0].finding_type, "signature-regression");
        assert_eq!(findings[0].severity, "high");
        assert_eq!(findings[0].function, "process");

        let evidence: SignatureRegressionEvidence =
            serde_json::from_value(findings[0].evidence.clone()).expect("valid evidence");
        assert!(
            evidence
                .changes
                .iter()
                .any(|c| c.change_type == "param_removed"),
            "Should contain a param_removed change, got: {:?}",
            evidence.changes
        );
    }

    // =========================================================================
    // Test 3: Return type changed is detected as HIGH severity
    // =========================================================================
    #[test]
    fn test_return_type_changed() {
        let old = "fn fetch(url: &str) -> String {\n    String::new()\n}";
        let new = "fn fetch(url: &str) -> Result<String, Error> {\n    Ok(String::new())\n}";
        let change = make_update("fetch", old, new);
        let findings = compose_one(change);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "high");

        let evidence: SignatureRegressionEvidence =
            serde_json::from_value(findings[0].evidence.clone()).expect("valid evidence");
        assert!(
            evidence
                .changes
                .iter()
                .any(|c| c.change_type == "return_type_changed"),
            "Should detect return type change, got: {:?}",
            evidence.changes
        );
    }

    // =========================================================================
    // Test 4: Param type changed is detected
    // =========================================================================
    #[test]
    fn test_param_type_changed() {
        let old = "fn send(data: Vec<u8>) {\n    // send\n}";
        let new = "fn send(data: &[u8]) {\n    // send\n}";
        let change = make_update("send", old, new);
        let findings = compose_one(change);

        assert_eq!(findings.len(), 1);

        let evidence: SignatureRegressionEvidence =
            serde_json::from_value(findings[0].evidence.clone()).expect("valid evidence");
        assert!(
            evidence
                .changes
                .iter()
                .any(|c| c.change_type == "param_type_changed"),
            "Should detect param type change, got: {:?}",
            evidence.changes
        );
    }

    // =========================================================================
    // Test 5: Param added is detected as MEDIUM severity
    // =========================================================================
    #[test]
    fn test_param_added() {
        let old = "fn create(name: &str) -> Item {\n    Item::new(name)\n}";
        let new = "fn create(name: &str, count: usize) -> Item {\n    Item::new(name)\n}";
        let change = make_update("create", old, new);
        let findings = compose_one(change);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "medium");

        let evidence: SignatureRegressionEvidence =
            serde_json::from_value(findings[0].evidence.clone()).expect("valid evidence");
        assert!(
            evidence
                .changes
                .iter()
                .any(|c| c.change_type == "param_added"),
            "Should detect param added, got: {:?}",
            evidence.changes
        );
    }

    // =========================================================================
    // Test 6: Body-only change with same signature produces no findings
    // =========================================================================
    #[test]
    fn test_body_only_change_no_finding() {
        let old = r#"pub fn calculate(a: f64, b: f64) -> f64 {
    a + b
}"#;
        let new = r#"pub fn calculate(a: f64, b: f64) -> f64 {
    let result = a + b;
    log::debug!("result = {}", result);
    result
}"#;
        let change = make_update("calculate", old, new);
        let findings = compose_one(change);
        assert!(
            findings.is_empty(),
            "Body-only change should produce no findings, got: {:?}",
            findings
        );
    }

    // =========================================================================
    // Test 7: Generic change is detected as MEDIUM severity
    // =========================================================================
    #[test]
    fn test_generic_change() {
        let old = "fn transform<T: Clone>(items: Vec<T>) -> Vec<T> {\n    items\n}";
        let new = "fn transform<T: Clone + Send>(items: Vec<T>) -> Vec<T> {\n    items\n}";
        let change = make_update("transform", old, new);
        let findings = compose_one(change);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "medium");

        let evidence: SignatureRegressionEvidence =
            serde_json::from_value(findings[0].evidence.clone()).expect("valid evidence");
        assert!(
            evidence
                .changes
                .iter()
                .any(|c| c.change_type == "generic_changed"),
            "Should detect generic change, got: {:?}",
            evidence.changes
        );
    }

    // =========================================================================
    // Test 8: Multiple signature changes produce multiple evidence entries
    // =========================================================================
    #[test]
    fn test_multiple_changes() {
        let old = "fn handle(req: Request, ctx: Context) -> Response {\n    Response::ok()\n}";
        let new = "fn handle(req: Request) -> Result<Response, Error> {\n    Ok(Response::ok())\n}";
        let change = make_update("handle", old, new);
        let findings = compose_one(change);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "high");

        let evidence: SignatureRegressionEvidence =
            serde_json::from_value(findings[0].evidence.clone()).expect("valid evidence");
        assert!(
            evidence.changes.len() >= 2,
            "Should have at least 2 changes (param removed + return type changed), got: {:?}",
            evidence.changes
        );
    }

    // =========================================================================
    // Test 9: Self param changes are ignored (not treated as param_removed)
    // =========================================================================
    #[test]
    fn test_self_param_ignored() {
        let old = "fn method(&self, x: i32) -> i32 {\n    x\n}";
        let new = "fn method(&mut self, x: i32) -> i32 {\n    x\n}";
        // Only the self receiver changed, params are the same
        let change = make_update("method", old, new);
        let findings = compose_one(change);

        // The signatures differ (due to &self vs &mut self in the text before {)
        // but there should be NO param_removed or param_added for self
        for finding in &findings {
            let evidence: SignatureRegressionEvidence =
                serde_json::from_value(finding.evidence.clone()).expect("valid evidence");
            for sc in &evidence.changes {
                assert_ne!(
                    sc.change_type, "param_removed",
                    "self receiver change should not be reported as param_removed: {:?}",
                    sc
                );
                assert_ne!(
                    sc.change_type, "param_added",
                    "self receiver change should not be reported as param_added: {:?}",
                    sc
                );
            }
        }
    }

    // =========================================================================
    // Test 10: Extract signature with where clause
    // =========================================================================
    #[test]
    fn test_extract_signature_with_where_clause() {
        let function_text = r#"pub fn serialize<T>(value: &T) -> String
where
    T: Serialize + Debug,
{
    serde_json::to_string(value).unwrap()
}"#;
        let sig = extract_signature(function_text);
        assert!(
            sig.contains("fn serialize"),
            "Signature should contain function name, got: {:?}",
            sig
        );
        assert!(
            sig.contains("where"),
            "Signature should include where clause, got: {:?}",
            sig
        );
        assert!(
            !sig.contains("serde_json"),
            "Signature should NOT include body, got: {:?}",
            sig
        );
    }

    // =========================================================================
    // Additional unit tests for helper functions
    // =========================================================================

    #[test]
    fn test_extract_signature_simple() {
        let sig = extract_signature("fn add(a: i32, b: i32) -> i32 { a + b }");
        assert_eq!(sig, "fn add(a: i32, b: i32) -> i32");
    }

    #[test]
    fn test_extract_signature_no_brace() {
        let sig = extract_signature("fn abstract_method(x: i32) -> bool;");
        assert_eq!(sig, "fn abstract_method(x: i32) -> bool;");
    }

    #[test]
    fn test_parse_params_simple() {
        let params = parse_params("fn foo(x: i32, y: String)");
        assert_eq!(params, vec!["x: i32", "y: String"]);
    }

    #[test]
    fn test_parse_params_nested_generics() {
        let params = parse_params("fn foo(map: HashMap<String, Vec<i32>>, count: usize)");
        assert_eq!(
            params,
            vec!["map: HashMap<String, Vec<i32>>", "count: usize"]
        );
    }

    #[test]
    fn test_parse_params_empty() {
        let params = parse_params("fn foo()");
        assert!(params.is_empty());
    }

    #[test]
    fn test_parse_params_self_filtered() {
        let params = parse_params("fn method(&self, x: i32, y: bool)");
        assert_eq!(params, vec!["x: i32", "y: bool"]);
    }

    #[test]
    fn test_parse_return_type_present() {
        let ret = parse_return_type("fn foo(x: i32) -> String");
        assert_eq!(ret.as_deref(), Some("String"));
    }

    #[test]
    fn test_parse_return_type_absent() {
        let ret = parse_return_type("fn foo(x: i32)");
        assert_eq!(ret, None);
    }

    #[test]
    fn test_parse_return_type_with_where() {
        let ret = parse_return_type("fn foo<T>(x: T) -> Vec<T> where T: Clone");
        assert_eq!(ret.as_deref(), Some("Vec<T>"));
    }

    #[test]
    fn test_parse_generics_present() {
        let gen = parse_generics("fn foo<T: Clone + Send>(x: T) -> T");
        assert_eq!(gen.as_deref(), Some("<T: Clone + Send>"));
    }

    #[test]
    fn test_parse_generics_absent() {
        let gen = parse_generics("fn foo(x: i32) -> i32");
        assert_eq!(gen, None);
    }

    #[test]
    fn test_parse_generics_multiple() {
        let gen = parse_generics("fn foo<A, B: Debug>(a: A, b: B)");
        assert_eq!(gen.as_deref(), Some("<A, B: Debug>"));
    }

    #[test]
    fn test_severity_high_for_param_removed() {
        let changes = vec![SignatureChange {
            change_type: "param_removed".to_string(),
            detail: "removed parameter: x: i32".to_string(),
        }];
        assert_eq!(max_severity(&changes), "high");
    }

    #[test]
    fn test_severity_high_for_return_type_changed() {
        let changes = vec![SignatureChange {
            change_type: "return_type_changed".to_string(),
            detail: "return type: i32 -> String".to_string(),
        }];
        assert_eq!(max_severity(&changes), "high");
    }

    #[test]
    fn test_severity_medium_for_param_added() {
        let changes = vec![SignatureChange {
            change_type: "param_added".to_string(),
            detail: "added parameter: z: bool".to_string(),
        }];
        assert_eq!(max_severity(&changes), "medium");
    }

    #[test]
    fn test_is_update_function_true() {
        let change = ASTChange {
            change_type: ChangeType::Update,
            node_kind: NodeKind::Function,
            name: None,
            old_location: None,
            new_location: None,
            old_text: None,
            new_text: None,
            similarity: None,
            children: None,
            base_changes: None,
        };
        assert!(is_update_function(&change));
    }

    #[test]
    fn test_is_update_function_false_for_insert() {
        let change = ASTChange {
            change_type: ChangeType::Insert,
            node_kind: NodeKind::Function,
            name: None,
            old_location: None,
            new_location: None,
            old_text: None,
            new_text: None,
            similarity: None,
            children: None,
            base_changes: None,
        };
        assert!(!is_update_function(&change));
    }

    #[test]
    fn test_is_update_function_false_for_class() {
        let change = ASTChange {
            change_type: ChangeType::Update,
            node_kind: NodeKind::Class,
            name: None,
            old_location: None,
            new_location: None,
            old_text: None,
            new_text: None,
            similarity: None,
            children: None,
            base_changes: None,
        };
        assert!(!is_update_function(&change));
    }

    #[test]
    fn test_is_update_method_true() {
        let change = ASTChange {
            change_type: ChangeType::Update,
            node_kind: NodeKind::Method,
            name: None,
            old_location: None,
            new_location: None,
            old_text: None,
            new_text: None,
            similarity: None,
            children: None,
            base_changes: None,
        };
        assert!(is_update_function(&change));
    }

    #[test]
    fn test_format_message_single() {
        let changes = vec![SignatureChange {
            change_type: "param_removed".to_string(),
            detail: "removed parameter: x: i32".to_string(),
        }];
        let msg = format_message(&changes);
        assert!(msg.contains("removed parameter: x: i32"));
        assert!(!msg.contains("changes)"));
    }

    #[test]
    fn test_format_message_multiple() {
        let changes = vec![
            SignatureChange {
                change_type: "param_removed".to_string(),
                detail: "removed parameter: y: String".to_string(),
            },
            SignatureChange {
                change_type: "return_type_changed".to_string(),
                detail: "return type: i32 -> bool".to_string(),
            },
        ];
        let msg = format_message(&changes);
        assert!(msg.contains("2 changes"));
    }

    #[test]
    fn test_file_path_made_relative() {
        let old = "fn foo(x: i32) -> i32 { x }";
        let new = "fn foo(x: i32, y: i32) -> i32 { x + y }";
        let change = make_update("foo", old, new);

        let mut file_diffs = HashMap::new();
        file_diffs.insert(PathBuf::from("/myproject/src/lib.rs"), vec![change]);
        let findings = compose_signature_regression(&file_diffs, Path::new("/myproject"));

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].file, PathBuf::from("src/lib.rs"));
    }

    #[test]
    fn test_nested_type_params_not_split() {
        let params = parse_params("fn foo(a: Option<(i32, i32)>, b: Vec<String>)");
        assert_eq!(params.len(), 2);
        assert_eq!(params[0], "a: Option<(i32, i32)>");
        assert_eq!(params[1], "b: Vec<String>");
    }

    #[test]
    fn test_pub_fn_signature_extracted() {
        let sig = extract_signature("pub fn public_api(x: u32) -> bool { x > 0 }");
        assert_eq!(sig, "pub fn public_api(x: u32) -> bool");
    }

    #[test]
    fn test_pub_crate_fn_signature_extracted() {
        let sig = extract_signature("pub(crate) fn internal(x: u32) -> bool { x > 0 }");
        assert_eq!(sig, "pub(crate) fn internal(x: u32) -> bool");
    }

    // =========================================================================
    // Test: Positional param rename (same type) should be INFO severity
    // =========================================================================
    #[test]
    fn test_param_renamed_same_type_info_severity() {
        // Rename `input` to `data` but keep the same type `&str`.
        // In Rust, parameters are positional -- this is NOT a breaking change
        // at the call site. Pure param renames should produce "param_renamed"
        // at INFO severity (non-breaking, awareness only).
        let old = "fn process(input: &str) -> bool {\n    true\n}";
        let new = "fn process(data: &str) -> bool {\n    true\n}";
        let change = make_update("process", old, new);
        let findings = compose_one(change);

        assert_eq!(findings.len(), 1, "Should detect exactly 1 finding");
        assert_eq!(
            findings[0].severity, "info",
            "Pure param rename should be INFO severity, got: {}",
            findings[0].severity
        );

        let evidence: SignatureRegressionEvidence =
            serde_json::from_value(findings[0].evidence.clone()).expect("valid evidence");
        assert!(
            evidence
                .changes
                .iter()
                .any(|c| c.change_type == "param_renamed"),
            "Should contain a param_renamed change, got: {:?}",
            evidence.changes
        );
        assert!(
            !evidence
                .changes
                .iter()
                .any(|c| c.change_type == "param_removed"),
            "Should NOT contain param_removed for a rename, got: {:?}",
            evidence.changes
        );
        assert!(
            !evidence
                .changes
                .iter()
                .any(|c| c.change_type == "param_added"),
            "Should NOT contain param_added for a rename, got: {:?}",
            evidence.changes
        );
    }

    // =========================================================================
    // Test: Positional param rename with different type should be MEDIUM
    // =========================================================================
    #[test]
    fn test_param_renamed_different_type_still_medium() {
        // Rename `input` to `data` AND change type from `&str` to `String`.
        // Both name and type changed at the same position -- this is a real
        // breaking change. Should be treated as param_type_changed (MEDIUM).
        let old = "fn process(input: &str) -> bool {\n    true\n}";
        let new = "fn process(data: String) -> bool {\n    true\n}";
        let change = make_update("process", old, new);
        let findings = compose_one(change);

        assert_eq!(findings.len(), 1, "Should detect exactly 1 finding");
        assert_eq!(
            findings[0].severity, "medium",
            "Param rename + type change should be MEDIUM severity, got: {}",
            findings[0].severity
        );

        let evidence: SignatureRegressionEvidence =
            serde_json::from_value(findings[0].evidence.clone()).expect("valid evidence");
        // Should have param_type_changed, not param_removed + param_added
        assert!(
            evidence
                .changes
                .iter()
                .any(|c| c.change_type == "param_type_changed"),
            "Should contain param_type_changed, got: {:?}",
            evidence.changes
        );
        assert!(
            !evidence
                .changes
                .iter()
                .any(|c| c.change_type == "param_removed"),
            "Should NOT contain param_removed for positional type change, got: {:?}",
            evidence.changes
        );
    }

    #[test]
    fn test_differ_mismatch_pub_to_private_all_params_removed() {
        // pub fn new(a: i32, b: String) -> Self  matched against  fn new() -> Self
        // This is a differ mismatch (two different impl blocks), not a real regression.
        let old_sig = "pub fn new(a: i32, b: String) -> Self";
        let new_sig = "fn new() -> Self";
        let changes = diff_signatures(old_sig, new_sig);
        assert!(
            is_likely_differ_mismatch(old_sig, new_sig, &changes),
            "Should detect mismatch: pub->private with all params removed"
        );
    }

    #[test]
    fn test_differ_mismatch_private_to_pub_all_params_added() {
        let old_sig = "fn new() -> Self";
        let new_sig = "pub fn new(a: i32, b: String) -> Self";
        let changes = diff_signatures(old_sig, new_sig);
        assert!(
            is_likely_differ_mismatch(old_sig, new_sig, &changes),
            "Should detect mismatch: private->pub with all params added"
        );
    }

    #[test]
    fn test_real_regression_not_suppressed_same_visibility() {
        // Same visibility, param removed — this is a real regression
        let old_sig = "pub fn process(data: Vec<u8>, timeout: u64) -> Result<()>";
        let new_sig = "pub fn process(data: Vec<u8>) -> Result<()>";
        let changes = diff_signatures(old_sig, new_sig);
        assert!(
            !is_likely_differ_mismatch(old_sig, new_sig, &changes),
            "Same visibility = real regression, should NOT be suppressed"
        );
    }

    #[test]
    fn test_real_regression_visibility_change_with_type_change() {
        // Visibility changed but has a type change (not pure add/remove) — could be real
        let old_sig = "pub fn transform(input: &str) -> String";
        let new_sig = "fn transform(input: Vec<u8>) -> String";
        let changes = diff_signatures(old_sig, new_sig);
        assert!(
            !is_likely_differ_mismatch(old_sig, new_sig, &changes),
            "Type change present = might be real, should NOT be suppressed"
        );
    }
}
