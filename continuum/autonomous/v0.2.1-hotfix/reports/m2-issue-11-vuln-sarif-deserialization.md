# M2 / VAL-002: Close issue #11 — vuln SARIF Deserialization mislabel

**Status:** PASSED
**Issue:** parcadei/tldr-code#11 ("Vuln CLI misclassifies Deserialization findings as SQL injection, producing invalid SARIF")
**Worker:** kraken (M2 VAL-002)
**Starting HEAD:** f7cccff (v0.2.0)
**Test:** `crates/tldr-cli/tests/vuln_sarif_deserialization_test.rs`

## Triage verification

The contract cited the wildcard at `crates/tldr-cli/src/commands/remaining/vuln.rs:645-651`. Verified — the literal pre-fix block was:

```rust
let vuln_type = match format!("{:?}", f.vuln_type).as_str() {
    "SqlInjection" => VulnType::SqlInjection,
    "CommandInjection" => VulnType::CommandInjection,
    "Xss" => VulnType::Xss,
    "PathTraversal" => VulnType::PathTraversal,
    _ => VulnType::SqlInjection,
};
```

The full `tldr_core::security::vuln::VulnType` enum (verified at `crates/tldr-core/src/security/vuln.rs:38-51`) has six variants:

| Variant | CWE | Pre-fix label |
|---|---|---|
| `SqlInjection` | CWE-89 | SqlInjection (correct) |
| `Xss` | CWE-79 | Xss (correct) |
| `CommandInjection` | CWE-78 | CommandInjection (correct) |
| `PathTraversal` | CWE-22 | PathTraversal (correct) |
| `Ssrf` | CWE-918 | **SqlInjection (WRONG)** |
| `Deserialization` | CWE-502 | **SqlInjection (WRONG)** |

So the wildcard mislabels two variants — Deserialization (the user-facing #11 symptom) AND Ssrf.

The Java deserialization rule fires on `ObjectInputStream` and `readObject(` patterns (verified at `crates/tldr-core/src/security/vuln.rs:636-640`). Java taint sources include `request.getParameter(`, `request.getHeader(`, etc. (verified at `crates/tldr-core/src/security/vuln.rs:183-189`). The scanner requires the tainted variable to appear on the same line as a sink pattern (`scan_file_vulns`, `vuln.rs:887-933`), so the fixture writes a single-line expression where `payload` (from `request.getParameter`) flows into `new ObjectInputStream(...).readObject()`.

## Fixture

`crates/tldr-cli/tests/fixtures/deserialize_java/Vuln.java`:

```java
import java.io.ObjectInputStream;
import java.io.ByteArrayInputStream;
import javax.servlet.http.HttpServletRequest;

public class Vuln {
    public Object readUser(HttpServletRequest request) throws Exception {
        String payload = request.getParameter("data");
        Object result = new ObjectInputStream(new ByteArrayInputStream(payload.getBytes())).readObject();
        return result;
    }
}
```

The single-line sink ensures the scanner sees `payload` (tainted), `ObjectInputStream(`, and `readObject(` together — emitting one `VulnFinding { vuln_type: Deserialization, cwe_id: Some("CWE-502") }`.

## Test design

Two tests in the same file, one per output format:

1. `vuln_json_labels_deserialization_correctly` — invokes `tldr vuln <fixture> --lang java --format json --quiet`, parses stdout as JSON, asserts `findings[0].vuln_type == "deserialization"` and `!= "sql_injection"`.

2. `vuln_sarif_labels_deserialization_correctly` — invokes with `--format sarif`, parses stdout as JSON, asserts:
   - `runs[0].results[0].ruleId == "CWE-502"` and `!= "CWE-89"`
   - `runs[0].tool.driver.rules` array contains a rule with `id == "CWE-502"` and does NOT contain `CWE-89`. This catches the second half of the bug — pre-fix, results.ruleId came from the (correct) `cwe_id` field while rules came from the (misclassified) local vuln_type, producing an internally inconsistent SARIF document.

The vuln command exits with code 2 when findings are present (per spec), so the test uses `cmd.output()` instead of `.success()`.

## RED on f7cccff (before fix)

```
running 2 tests
test vuln_json_labels_deserialization_correctly ... FAILED
test vuln_sarif_labels_deserialization_correctly ... FAILED

failures:

---- vuln_json_labels_deserialization_correctly stdout ----

thread 'vuln_json_labels_deserialization_correctly' (196600960) panicked at crates/tldr-cli/tests/vuln_sarif_deserialization_test.rs:121:5:
assertion `left == right` failed: Java fixture with `ObjectInputStream(...).readObject()` should be labeled `deserialization` in JSON output. Got `sql_injection` instead. This is the VAL-002 / issue #11 mislabel: the wildcard match arm at crates/tldr-cli/src/commands/remaining/vuln.rs:650 (`_ => VulnType::SqlInjection`) silently relabels every Deserialization and Ssrf finding from tldr-core as SqlInjection.
  left: "sql_injection"
 right: "deserialization"

---- vuln_sarif_labels_deserialization_correctly stdout ----

thread 'vuln_sarif_labels_deserialization_correctly' (196600961) panicked at crates/tldr-cli/tests/vuln_sarif_deserialization_test.rs:201:5:
SARIF rules array must contain a rule with id `CWE-502` matching the deserialization finding's ruleId. Got rule ids: ["CWE-89"]. This is the second half of the VAL-002 bug: the rules array is built from the (misclassified) local vuln_type while results.ruleId is built from the (correct) cwe_id field, so they disagree and the SARIF document is invalid (results reference a rule not in the rules array).


failures:
    vuln_json_labels_deserialization_correctly
    vuln_sarif_labels_deserialization_correctly

test result: FAILED. 0 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.90s
```

**RED-REASON GATE check:**
- Substring `sql` (case-insensitive)? **YES** — `"sql_injection"` (left side of the JSON assertion) and `"CWE-89"` (in the SARIF rules array assertion).
- Substring matching expected deserialization value? **YES** — `"deserialization"` (right side of JSON assertion) and `"CWE-502"` (in SARIF assertion).

Both halves of the gate satisfied.

## Fix

Replaced the wildcard match arm with a free-function `map_core_vuln_type` that exhaustively maps every `tldr_core::security::vuln::VulnType` variant. No `_` arm — the Rust compiler now refuses to build if a future tldr-core variant is added without a CLI mapping.

Diff (only relevant excerpt, full diff in commit):

```rust
// crates/tldr-cli/src/commands/remaining/vuln.rs:644-645 (was 645-651)
- let vuln_type = match format!("{:?}", f.vuln_type).as_str() {
-     "SqlInjection" => VulnType::SqlInjection,
-     "CommandInjection" => VulnType::CommandInjection,
-     "Xss" => VulnType::Xss,
-     "PathTraversal" => VulnType::PathTraversal,
-     _ => VulnType::SqlInjection,
- };
+ let vuln_type = map_core_vuln_type(f.vuln_type);

// new function added at vuln.rs:705-715
+ fn map_core_vuln_type(core_ty: tldr_core::security::vuln::VulnType) -> VulnType {
+     use tldr_core::security::vuln::VulnType as CoreVulnType;
+     match core_ty {
+         CoreVulnType::SqlInjection => VulnType::SqlInjection,
+         CoreVulnType::Xss => VulnType::Xss,
+         CoreVulnType::CommandInjection => VulnType::CommandInjection,
+         CoreVulnType::PathTraversal => VulnType::PathTraversal,
+         CoreVulnType::Ssrf => VulnType::Ssrf,
+         CoreVulnType::Deserialization => VulnType::Deserialization,
+     }
+ }
```

**Variants mapped:** `SqlInjection`, `Xss`, `CommandInjection`, `PathTraversal`, `Ssrf`, `Deserialization` — all 6 of the tldr-core enum.

The local CLI `VulnType` (`crates/tldr-cli/src/commands/remaining/types.rs:1363-1377`) already had `Deserialization` and `Ssrf` variants with correct CWE mappings — only the conversion site was buggy.

## GREEN after fix

```
running 2 tests
test vuln_json_labels_deserialization_correctly ... ok
test vuln_sarif_labels_deserialization_correctly ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.95s
```

## Acceptance gates

| Gate | Result |
|---|---|
| Reproduce test passes after fix | PASS — 2/2 |
| `cargo test -p tldr-cli --test exhaustive_matrix --features semantic --release` | PASS — 730/730 |
| `cargo test -p tldr-cli --test language_command_matrix --features semantic --release` | PASS — 234/234 |
| Combined matrix (964/964 expected) | PASS — 730 + 234 = 964 |
| `cargo clippy --workspace --all-features --tests -- -D warnings` | PASS — clean (no new warnings) |
| Exhaustive match enforces compile-time completeness | PASS — no `_` arm |
| Files touched ≤ 5 (excl. contract/reports) | PASS — 3 files (vuln.rs, new test, new fixture) |

## Files modified (worker scope)

1. `crates/tldr-cli/src/commands/remaining/vuln.rs` — fix the wildcard mislabel
2. `crates/tldr-cli/tests/vuln_sarif_deserialization_test.rs` — new (reproduction test)
3. `crates/tldr-cli/tests/fixtures/deserialize_java/Vuln.java` — new (fixture)
4. `continuum/autonomous/v0.2.1-hotfix/contract.json` — VAL-002 status → passed
5. `continuum/autonomous/v0.2.1-hotfix/reports/m2-issue-11-vuln-sarif-deserialization.md` — this report
6. `continuum/autonomous/v0.2.1-hotfix/validation/m2-vuln-sarif-deserialization.json` — gate JSON

No tldr-core changes needed — the bug was strictly at the CLI conversion boundary.

## Out-of-scope artifacts NOT staged

The orchestrator session has the following uncommitted/untracked files outside M2 scope which the worker did NOT touch and did NOT stage:
- `Cargo.lock` (modified — pre-existing prior-session artifact)
- `continuum/autonomous/issue-1-bug-fixes/` (modified + untracked — prior-session artifacts)
- `crates/tldr-daemon/src/handlers/security.rs` (modified — M1 territory, parallel worker)
- `crates/tldr-daemon/tests/path_traversal_test.rs` (untracked — M1 territory)
- `crates/tldr-cli/META-INF/`, `MainKt.class`, `UtilKt.class` (untracked Kotlin compiler droppings — unrelated to this milestone)
- Various other untracked `continuum/autonomous/{abstract-interp-float-pi-fix,bump-v0.1.3,c-grammar-struct-emission,definitions-bugs,ts-abstract-methods}/` directories (prior-session scratch)
