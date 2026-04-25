//! VAL-002 (issue parcadei/tldr-code#11): The `tldr vuln` command must
//! correctly label Deserialization findings as `deserialization` in JSON
//! output and as `CWE-502` in SARIF output — NOT as SqlInjection / `CWE-89`.
//!
//! Root cause confirmed at `crates/tldr-cli/src/commands/remaining/vuln.rs:645-651`:
//! a `match format!("{:?}", f.vuln_type).as_str()` covers SqlInjection /
//! CommandInjection / Xss / PathTraversal explicitly, then defaults via
//! `_ => VulnType::SqlInjection`. The full `tldr_core::security::vuln::VulnType`
//! enum has six variants — Ssrf and Deserialization fall through the wildcard
//! and are silently relabeled as SqlInjection.
//!
//! Reproduction:
//!   1. Create a Java fixture containing both a tainted source
//!      (`request.getParameter`) and a deserialization sink
//!      (`ObjectInputStream(...).readObject()`) on a line that mentions the
//!      tainted variable, so `tldr_core::security::vuln::scan_vulnerabilities`
//!      emits a `VulnFinding { vuln_type: Deserialization, cwe_id: Some("CWE-502") }`.
//!   2. Invoke `tldr vuln <fixture> --lang java --format json` and parse the
//!      JSON. Assert top-level `findings[0].vuln_type == "deserialization"`,
//!      not `"sql_injection"`.
//!   3. Invoke again with `--format sarif`. Assert
//!      `runs[0].results[0].ruleId == "CWE-502"`. Also assert the rules array
//!      contains a rule with `id == "CWE-502"` (this catches the second half
//!      of the SARIF bug — the rules section is built from the local
//!      misclassified `vuln_type`, while results use the correct `cwe_id`,
//!      producing an invalid SARIF document where `result.ruleId` references
//!      a rule that does not exist in `tool.driver.rules`).

use assert_cmd::Command;
use serde_json::Value;
use std::path::PathBuf;

fn tldr_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("deserialize_java")
        .join("Vuln.java")
}

/// Run `tldr vuln <fixture> --lang java --format <fmt> --quiet` and return
/// the JSON-parsed stdout. Note: the vuln command exits non-zero (code 2)
/// when findings are present (per spec) — we ignore the exit status and only
/// verify that stdout is well-formed JSON we can navigate.
fn run_vuln(format: &str) -> Value {
    let fixture = fixture_path();
    assert!(
        fixture.exists(),
        "fixture missing: {} — did you delete it?",
        fixture.display()
    );

    let mut cmd = tldr_cmd();
    cmd.arg("vuln")
        .arg(&fixture)
        .arg("--lang")
        .arg("java")
        .arg("--format")
        .arg(format)
        .arg("--quiet");

    // The vuln command exits 2 when findings are detected. Use `output()`
    // (not `success()`) so we still get stdout for parsing.
    let output = cmd.output().expect("failed to execute tldr vuln");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();

    serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "failed to parse `tldr vuln --format {}` stdout as JSON: {}\n--- stdout (len={}) ---\n{}\n--- stderr ---\n{}",
            format,
            e,
            stdout.len(),
            stdout,
            String::from_utf8_lossy(&output.stderr),
        )
    })
}

/// VAL-002 part A: JSON output must label the Deserialization finding as
/// `"deserialization"`, not `"sql_injection"`. On unfixed HEAD this assertion
/// fails because the wildcard match arm at vuln.rs:650 maps every non-
/// {Sql,Cmd,Xss,Path} variant (including Deserialization and Ssrf) to the
/// local `VulnType::SqlInjection`, which serializes as `"sql_injection"`.
#[test]
fn vuln_json_labels_deserialization_correctly() {
    let report = run_vuln("json");

    let findings = report
        .get("findings")
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| {
            panic!(
                "expected a `findings` array in JSON output; full report:\n{}",
                serde_json::to_string_pretty(&report).unwrap_or_default()
            )
        });

    assert!(
        !findings.is_empty(),
        "expected at least one finding for Java fixture with ObjectInputStream + tainted source; got zero. \
         The deserialization rule for Java fires on `ObjectInputStream` / `readObject(` \
         (see crates/tldr-core/src/security/vuln.rs:636-640). Full report:\n{}",
        serde_json::to_string_pretty(&report).unwrap_or_default()
    );

    let first = &findings[0];
    let vuln_type = first
        .get("vuln_type")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| {
            panic!(
                "finding[0] missing `vuln_type` string field; finding:\n{}",
                serde_json::to_string_pretty(first).unwrap_or_default()
            )
        });

    assert_eq!(
        vuln_type, "deserialization",
        "Java fixture with `ObjectInputStream(...).readObject()` should be labeled \
         `deserialization` in JSON output. Got `{}` instead. \
         This is the VAL-002 / issue #11 mislabel: the wildcard match arm at \
         crates/tldr-cli/src/commands/remaining/vuln.rs:650 \
         (`_ => VulnType::SqlInjection`) silently relabels every Deserialization \
         and Ssrf finding from tldr-core as SqlInjection.",
        vuln_type,
    );

    assert_ne!(
        vuln_type, "sql_injection",
        "Java deserialization finding must NOT be labeled `sql_injection`. \
         The wildcard match arm at vuln.rs:650 is the bug — replace with an \
         exhaustive From<tldr_core::security::vuln::VulnType> impl.",
    );
}

/// VAL-002 part B: SARIF output must use `CWE-502` as the rule id for
/// deserialization, AND the rules array must include a rule with that id
/// (otherwise `result.ruleId` references a non-existent rule and the SARIF
/// document is invalid for tooling like GitHub code scanning). On unfixed
/// HEAD, results use the correct CWE id from `f.cwe_id` (`CWE-502`) but
/// rules are built from the misclassified local vuln_type and emit
/// `id: "CWE-89"`, producing a broken SARIF document.
#[test]
fn vuln_sarif_labels_deserialization_correctly() {
    let sarif = run_vuln("sarif");

    let result = sarif
        .pointer("/runs/0/results/0")
        .unwrap_or_else(|| {
            panic!(
                "expected runs[0].results[0] in SARIF output; full SARIF:\n{}",
                serde_json::to_string_pretty(&sarif).unwrap_or_default()
            )
        });

    let rule_id = result
        .get("ruleId")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| {
            panic!(
                "results[0] missing `ruleId` field; result:\n{}",
                serde_json::to_string_pretty(result).unwrap_or_default()
            )
        });

    assert_eq!(
        rule_id, "CWE-502",
        "SARIF ruleId for Java `ObjectInputStream(...).readObject()` should be \
         CWE-502 (Deserialization of Untrusted Data). Got `{}` instead.",
        rule_id,
    );
    assert_ne!(
        rule_id, "CWE-89",
        "SARIF ruleId must NOT be CWE-89 (SQL injection) for a deserialization sink.",
    );

    // Cross-check: the rules array must contain a rule with id == ruleId,
    // otherwise the SARIF is internally inconsistent. The bug also manifests
    // here — pre-fix, results[0].ruleId is `CWE-502` (from f.cwe_id) but
    // the rules array entry is `CWE-89` (from the misclassified local
    // vuln_type), so this assertion fails too.
    let rules = sarif
        .pointer("/runs/0/tool/driver/rules")
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| {
            panic!(
                "expected runs[0].tool.driver.rules array in SARIF; full SARIF:\n{}",
                serde_json::to_string_pretty(&sarif).unwrap_or_default()
            )
        });

    let rule_ids: Vec<&str> = rules
        .iter()
        .filter_map(|r| r.get("id").and_then(|v| v.as_str()))
        .collect();

    assert!(
        rule_ids.contains(&"CWE-502"),
        "SARIF rules array must contain a rule with id `CWE-502` matching \
         the deserialization finding's ruleId. Got rule ids: {:?}. This is \
         the second half of the VAL-002 bug: the rules array is built from \
         the (misclassified) local vuln_type while results.ruleId is built \
         from the (correct) cwe_id field, so they disagree and the SARIF \
         document is invalid (results reference a rule not in the rules array).",
        rule_ids,
    );
    assert!(
        !rule_ids.contains(&"CWE-89"),
        "SARIF rules array must NOT contain a SQL-injection rule (CWE-89) \
         for a deserialization-only fixture. Got rule ids: {:?}.",
        rule_ids,
    );
}
