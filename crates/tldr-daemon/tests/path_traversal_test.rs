//! Reproduction test for VAL-001 / GitHub issue #5:
//! Daemon IPC handlers must reject absolute paths that resolve outside the
//! project root via `tldr_core::validate_file_path`.
//!
//! On unfixed code, the `secrets` and `vuln` handlers in
//! `crates/tldr-daemon/src/handlers/security.rs` accept any absolute path
//! and read the file's content. This test demonstrates the leak by creating
//! a "victim" file outside the project root containing the canary string
//! `root:` plus a hardcoded password (which triggers the Password regex,
//! causing `SecretFinding::line_content` to leak the line into the JSON
//! response).
//!
//! After the fix, the handler must return a `BAD_REQUEST` error and the
//! response JSON must not contain the canary string anywhere.

use std::path::PathBuf;
use std::sync::Arc;

use axum::{extract::State, Json};
use serde_json::Value;
use tempfile::TempDir;

use tldr_daemon::handlers::security::{secrets, vuln, SecretsRequest, VulnRequest};
use tldr_daemon::server::compute_socket_path;
use tldr_daemon::state::DaemonState;

/// Recursively walk a JSON value and return true if any string field anywhere
/// (in arrays, objects, nested combinations) contains `needle` as a substring.
fn json_contains_substring(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(s) => s.contains(needle),
        Value::Array(items) => items.iter().any(|v| json_contains_substring(v, needle)),
        Value::Object(map) => {
            // Also check keys, just to be safe — a leaked path could end up as a key.
            map.iter()
                .any(|(k, v)| k.contains(needle) || json_contains_substring(v, needle))
        }
        _ => false,
    }
}

/// Build a `DaemonState` rooted at a freshly created temp directory.
/// The state is wrapped in `Arc` to match the handler's `State` extractor.
fn make_state(project_root: PathBuf) -> Arc<DaemonState> {
    let socket = compute_socket_path(&project_root, "1.0");
    Arc::new(DaemonState::new(project_root, socket))
}

/// Materialize a victim file in `victim_dir` containing the canary string
/// `root:` and a hardcoded password sink that the secrets scanner picks up.
/// Returns the absolute path to the file.
fn write_victim_file(victim_dir: &std::path::Path) -> PathBuf {
    let victim_file = victim_dir.join("victim.env");
    // Two distinct triggers:
    //  - `password = "..."` matches the Password regex → SecretFinding with line_content
    //  - The literal "root:" canary that we will assert is leaked
    let content = "\
# Sensitive credential file (simulating /etc/passwd-like leak)
root:VictimSuperSecretValue_canary_xyz_42
password=\"VictimSuperSecretValue_canary_xyz_42\"
";
    std::fs::write(&victim_file, content).expect("write victim file");
    victim_file
}

#[tokio::test]
async fn secrets_handler_rejects_absolute_path_outside_project() {
    // Project root: a tempdir with at least one file (so canonicalize works).
    let project_dir = TempDir::new().expect("project tempdir");
    std::fs::write(project_dir.path().join("inside.txt"), "harmless")
        .expect("write inside file");

    // Victim location: a SEPARATE tempdir (outside project_dir).
    // This simulates "/etc/passwd" — an absolute path the daemon should refuse.
    let victim_dir = TempDir::new().expect("victim tempdir");
    let victim_path = write_victim_file(victim_dir.path());

    // Sanity check: the victim file is outside the project root.
    assert!(
        !victim_path.starts_with(project_dir.path()),
        "test setup error: victim should be outside project root"
    );

    let state = make_state(project_dir.path().to_path_buf());

    let request = SecretsRequest {
        path: Some(victim_path.to_string_lossy().to_string()),
        entropy_threshold: 4.5,
        include_test: true,
        severity_filter: None,
    };

    // Drive the handler in-process exactly the way Axum would.
    let result = secrets(State(state), Json(request)).await;

    // After-fix behaviour: handler returns Err (HandlerError -> BAD_REQUEST).
    // Before-fix behaviour: handler returns Ok with a SecretsReport whose
    // findings include `line_content` containing the canary "root:".
    match result {
        Err(_handler_error) => {
            // Fixed path: handler refused the absolute path. Good.
        }
        Ok(Json(response)) => {
            // Unfixed path: the file was read. Serialize the response and walk
            // the JSON for any leaked content from the victim file.
            let serialized =
                serde_json::to_value(&response).expect("serialize daemon response");

            // The canary substring "root:" MUST NOT appear anywhere in the
            // response. If it does, the file content leaked through the
            // unguarded handler.
            let leaked_root = json_contains_substring(&serialized, "root:");
            // Also check for the unique victim canary token to make this
            // test robust against any incidental "root:" elsewhere.
            let leaked_canary =
                json_contains_substring(&serialized, "VictimSuperSecretValue_canary_xyz_42");

            assert!(
                !leaked_root && !leaked_canary,
                "secrets handler leaked victim file content (root:/canary present in response): {}",
                serde_json::to_string_pretty(&serialized).unwrap_or_default()
            );
        }
    }
}

#[tokio::test]
async fn vuln_handler_rejects_absolute_path_outside_project() {
    // Same harness as the secrets test, but exercising the `vuln` handler.
    // The vuln scanner walks files looking for taint flows; even when no
    // findings fire on /etc/passwd-like content, a fixed handler must reject
    // the absolute path BEFORE invoking the scanner. The strongest signal is
    // that the response is an error (path validation refused the input).
    let project_dir = TempDir::new().expect("project tempdir");
    std::fs::write(project_dir.path().join("inside.txt"), "harmless")
        .expect("write inside file");

    let victim_dir = TempDir::new().expect("victim tempdir");
    let victim_path = write_victim_file(victim_dir.path());

    assert!(
        !victim_path.starts_with(project_dir.path()),
        "test setup error: victim should be outside project root"
    );

    let state = make_state(project_dir.path().to_path_buf());

    let request = VulnRequest {
        path: Some(victim_path.to_string_lossy().to_string()),
        language: None,
        vuln_type: None,
    };

    let result = vuln(State(state), Json(request)).await;

    // Spec invariant (VAL-001): handler MUST refuse absolute paths that resolve
    // outside the project root. The vuln scanner often produces no findings on
    // an unrelated victim file, so checking for leaked findings is too weak —
    // the strong assertion is that the handler returned an error BEFORE doing
    // any filesystem work on the out-of-project path.
    let response_value = match result {
        Err(_handler_error) => {
            // Fixed path: handler refused the absolute path. Done.
            return;
        }
        Ok(Json(response)) => serde_json::to_value(&response).expect("serialize daemon response"),
    };

    // If we got here, the handler accepted an out-of-project absolute path.
    // That is itself the bug, regardless of whether visible content leaked.
    panic!(
        "vuln handler accepted out-of-project absolute path {:?} (expected validation error). \
         Response body: {}",
        victim_path,
        serde_json::to_string_pretty(&response_value).unwrap_or_default()
    );
}
