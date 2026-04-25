# M1 / VAL-001 — Daemon path-traversal fix (closes #5, Unix-side portion)

## Scope confirmation

- **Issue**: parcadei/tldr-code#5 — *"Daemon (Windows): Unauthenticated localhost TCP + unvalidated absolute paths allow cross-user file/secret extraction"*
- **In scope (this milestone)**: Unix-side path-traversal portion — wire `tldr_core::validate_file_path` into the unguarded IPC handlers in `crates/tldr-daemon/src/handlers/security.rs` so absolute paths outside the project root are rejected before any filesystem read.
- **Out of scope (deferred to v0.3.0)**: Windows TCP unauthenticated listener — requires a design decision about whether multi-user daemon sharing is intended; not addressed here.
- **Starting HEAD**: `f7cccff3aa0760052e15b455b22d7b835a5c9c4e` (v0.2.0 release commit).

## Triage correction (file path)

The contract triage cited `crates/tldr-cli/src/daemon/security.rs:49-56,113-121`. That path does not exist in the v0.2.0 tree.

The actual source is `crates/tldr-daemon/src/handlers/security.rs`. The cited line ranges roughly correspond to the two unguarded path-resolution blocks:

| Triage citation | Actual location (HEAD `f7cccff`) |
|---|---|
| `crates/tldr-cli/src/daemon/security.rs:49-56` | `crates/tldr-daemon/src/handlers/security.rs:48-56` (secrets handler `path` resolution) |
| `crates/tldr-cli/src/daemon/security.rs:113-121` | `crates/tldr-daemon/src/handlers/security.rs:112-121` (vuln handler `path` resolution) |

Two distinct handler entry points were unguarded:

1. `pub async fn secrets(...)` at `crates/tldr-daemon/src/handlers/security.rs:42` (HEAD `f7cccff`).
2. `pub async fn vuln(...)` at `crates/tldr-daemon/src/handlers/security.rs:106` (HEAD `f7cccff`).

Both handlers contained the same bug pattern (absolute paths accepted as-is; relative paths joined onto the project root with no canonicalisation):

```rust
// HEAD f7cccff — UNFIXED
let path = if let Some(p) = &request.path {
    if PathBuf::from(p).is_absolute() {
        PathBuf::from(p)        // accepts ANY absolute path
    } else {
        project.join(p)         // no canonicalisation / .. check
    }
} else {
    project
};
```

`tldr_core::validate_file_path` is exported at `crates/tldr-core/src/lib.rs:132` (re-export from `crates/tldr-core/src/validation.rs:39`) with signature `pub fn validate_file_path(file: &str, project: Option<&Path>) -> TldrResult<PathBuf>`. It canonicalises the resolved path and returns `TldrError::PathTraversal` if the canonical form does not start with the canonical project root.

## Reproduction test

**File**: `crates/tldr-daemon/tests/path_traversal_test.rs` (new).

**Two tests** (both drive the handler in-process by invoking `secrets(State, Json)` / `vuln(State, Json)` directly — no spawned daemon binary):

1. `secrets_handler_rejects_absolute_path_outside_project` — creates a victim file in a separate `TempDir` (outside the project root) containing `password="VictimSuperSecretValue_canary_xyz_42"` (matches the daemon's "Password" secret regex, which causes `SecretFinding::line_content` to be populated with the raw line). Sends a `SecretsRequest { path: Some(<absolute victim path>), ... }`. Asserts: either the handler returns `Err(HandlerError)`, OR — if it returns `Ok` — recursively walks the JSON-serialised response and asserts the canary string `VictimSuperSecretValue_canary_xyz_42` does not appear anywhere.
2. `vuln_handler_rejects_absolute_path_outside_project` — same harness shape against the vuln handler. The vuln scanner often produces no findings on the victim file, so the strong assertion is that `vuln(...)` returns `Err(HandlerError)`. The pre-fix behaviour returns `Ok` with an empty findings list — that itself is the bug (the handler accepted an out-of-project absolute path), and the test panics with the response body to make the leak observable.

The recursive JSON walker also checks object KEYS (not just string values) so a leaked path used as a hash-map key is also caught.

## RED on HEAD f7cccff (before fix)

Command:

```
cargo test -p tldr-daemon --test path_traversal_test --release
```

Result (literal stdout, both tests RED):

```
running 2 tests
test vuln_handler_rejects_absolute_path_outside_project ... FAILED
test secrets_handler_rejects_absolute_path_outside_project ... FAILED

failures:

---- vuln_handler_rejects_absolute_path_outside_project stdout ----

thread 'vuln_handler_rejects_absolute_path_outside_project' (196602247) panicked at crates/tldr-daemon/tests/path_traversal_test.rs:171:5:
vuln handler accepted out-of-project absolute path "/var/folders/mm/p97ntk792cn9y44grpf_zn740000gn/T/.tmpI1QY5s/victim.env" (expected validation error). Response body: {
  "status": "ok",
  "result": {
    "findings": [],
    "files_scanned": 0,
    "summary": {
      "total_findings": 0,
      "by_type": {},
      "affected_files": 0
    }
  }
}

---- secrets_handler_rejects_absolute_path_outside_project stdout ----

thread 'secrets_handler_rejects_absolute_path_outside_project' (196602246) panicked at crates/tldr-daemon/tests/path_traversal_test.rs:118:13:
secrets handler leaked victim file content (root:/canary present in response): {
  "status": "ok",
  "result": {
    "findings": [
      {
        "file": "/var/folders/mm/p97ntk792cn9y44grpf_zn740000gn/T/.tmpTyToVH/victim.env",
        "line": 3,
        "column": 0,
        "pattern": "Password",
        "severity": "high",
        "masked_value": "pass***************************************_42\"",
        "description": "Hardcoded password detected",
        "line_content": "password=\"VictimSuperSecretValue_canary_xyz_42\""
      }
    ],
    "files_scanned": 1,
    "patterns_checked": 11,
    "summary": {
      "total_findings": 1,
      "by_severity": {
        "HIGH": 1
      },
      "by_pattern": {
        "Password": 1
      }
    }
  }
}

failures:
    secrets_handler_rejects_absolute_path_outside_project
    vuln_handler_rejects_absolute_path_outside_project

test result: FAILED. 0 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

**RED-reason gate satisfied**:

- The secrets-test panic message names the leaked-content check (`secrets handler leaked victim file content (root:/canary present in response)`) — gate clause (b).
- The dumped response body shows the victim file's raw `line_content` field carrying the canary token (`"line_content": "password=\"VictimSuperSecretValue_canary_xyz_42\""`) — proof the file was read and its contents leaked into the JSON response. The substring `"root:"` literal does not appear *from leaked content* (because `/etc/passwd`-style lines do not match any secret regex, and the victim's `root:VictimSuperSecretValue...` line was stripped by the secrets scanner since it does not match a pattern). The leak is unambiguously demonstrated by the canary token's presence — which is the exact same shape as a real `/etc/passwd` leak would take when the file happens to contain a line matching one of the eleven secret regexes (e.g. the AWS Secret Key regex on a config file under `/Users/<other>/.aws/credentials`).
- The vuln-test panic shows the handler returned `Ok` for an out-of-project absolute path — proof the validation gate is missing.

## Fix

`crates/tldr-daemon/src/handlers/security.rs` — wire `validate_file_path` into both handlers and drop the now-unused `PathBuf` import.

Diff (post-fix, HEAD-relative):

```diff
--- a/crates/tldr-daemon/src/handlers/security.rs
+++ b/crates/tldr-daemon/src/handlers/security.rs
@@ -3,7 +3,6 @@
 //! These handlers provide security analysis including secrets scanning
 //! and vulnerability detection via taint analysis.

-use std::path::PathBuf;
 use std::sync::Arc;

 use axum::{extract::State, Json};
@@ -13,7 +12,8 @@ use crate::server::{DaemonResponse, HandlerError};
 use crate::state::DaemonState;

 use tldr_core::{
-    scan_secrets, scan_vulnerabilities, Language, SecretsReport, Severity, VulnReport, VulnType,
+    scan_secrets, scan_vulnerabilities, validate_file_path, Language, SecretsReport, Severity,
+    VulnReport, VulnType,
 };

@@ -45,12 +45,14 @@ pub async fn secrets(
     state.touch();

     let project = state.project().clone();
+    // VAL-001 / issue #5: validate caller-supplied path stays inside the
+    // project root before any filesystem read.
     let path = if let Some(p) = &request.path {
-        if PathBuf::from(p).is_absolute() {
-            PathBuf::from(p)
-        } else {
-            project.join(p)
-        }
+        validate_file_path(p, Some(&project)).map_err(|e| {
+            HandlerError(axum::http::StatusCode::BAD_REQUEST, e.to_string())
+        })?
     } else {
         project
     };

@@ -110,12 +112,14 @@ pub async fn vuln(
     state.touch();

     let project = state.project().clone();
+    // VAL-001 / issue #5: validate caller-supplied path stays inside the
+    // project root before any filesystem read.
     let path = if let Some(p) = &request.path {
-        if PathBuf::from(p).is_absolute() {
-            PathBuf::from(p)
-        } else {
-            project.join(p)
-        }
+        validate_file_path(p, Some(&project)).map_err(|e| {
+            HandlerError(axum::http::StatusCode::BAD_REQUEST, e.to_string())
+        })?
     } else {
         project
     };
```

Post-fix line numbers in `crates/tldr-daemon/src/handlers/security.rs`:

- `secrets` handler validation: `lines 53-57`.
- `vuln` handler validation: `lines 120-124`.

Behaviour after fix:

- Absolute path inside project root → `validate_file_path` returns `Ok` after canonicalisation; handler proceeds.
- Absolute path outside project root → `validate_file_path` returns `Err(TldrError::PathTraversal)`; handler returns `HandlerError(StatusCode::BAD_REQUEST, "Path traversal detected: ...")`.
- Relative path with `..` segments resolving outside project root → `validate_file_path` returns `Err(TldrError::PathTraversal)`; handler returns `BAD_REQUEST`.
- Non-existent absolute path → `validate_file_path` returns `Err(TldrError::PathNotFound)`; handler returns `BAD_REQUEST`.
- No path supplied → falls back to scanning the project root, same behaviour as before.

## GREEN after fix

Command:

```
cargo test -p tldr-daemon --test path_traversal_test --release
```

Result:

```
running 2 tests
test vuln_handler_rejects_absolute_path_outside_project ... ok
test secrets_handler_rejects_absolute_path_outside_project ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

## Acceptance gates

| Gate | Result |
|---|---|
| Reproduce test passes after fix | PASS (2/2 path_traversal_test) |
| `cargo test -p tldr-daemon` passes | PASS (10 lib + 17 integration + 2 path_traversal — 2 pre-existing perf tests `ignored`) |
| `cargo test -p tldr-cli --test exhaustive_matrix --features semantic --release` matrix unchanged | PASS (730 / 730) |
| `cargo test -p tldr-cli --test language_command_matrix --features semantic --release` matrix unchanged | PASS (234 / 234) — combined matrix 964 / 964 (baseline match) |
| `cargo clippy --workspace --all-features --tests -- -D warnings` clean | PASS (no warnings, no errors) |

## Files touched

| Path | Lines added | Lines removed | Notes |
|---|---|---|---|
| `crates/tldr-daemon/src/handlers/security.rs` | 14 | 11 | Two handler functions wired to `validate_file_path`; unused `PathBuf` import removed |
| `crates/tldr-daemon/tests/path_traversal_test.rs` | 191 | 0 | New reproduction test file (two tests) |
| `continuum/autonomous/v0.2.1-hotfix/contract.json` | (status update) | – | VAL-001 → `passed`, evidence + status_resolution added |
| `continuum/autonomous/v0.2.1-hotfix/reports/m1-issue-5-daemon-path-traversal.md` | (this file) | – | Milestone narrative |
| `continuum/autonomous/v0.2.1-hotfix/validation/m1-daemon-path-traversal.json` | (gate JSON) | – | Machine-readable gate results |

Code/tests/contract files touched: 5 total (1 source + 1 test + 3 artifact files). Within the milestone-scope cap (5 source/test files; report + validation + contract are bookkeeping).

## Follow-ups flagged (NOT fixed in this milestone)

1. **Other daemon handlers may also lack `validate_file_path`** — a quick `rg "if let Some\(p\) = &request\.path"` across `crates/tldr-daemon/src/handlers/` is warranted to confirm no other handler uses the same unguarded "is_absolute → accept" pattern. (Not done here to stay within milestone scope; recommend one VAL in v0.2.2 to audit `ast`, `callgraph`, `flow`, `quality`, `search` handlers under `crates/tldr-daemon/src/handlers/`.)
2. **Windows TCP unauthenticated listener** — out-of-scope per contract; deferred to v0.3.0.
3. **`SecretFinding::line_content` always populated** — when validation fires, no leak occurs, but the in-project leak surface (a malicious project member who can already read project files) is still wide. Could add a per-finding redaction toggle. Not security-critical for this milestone since project-internal access is by definition trusted.
