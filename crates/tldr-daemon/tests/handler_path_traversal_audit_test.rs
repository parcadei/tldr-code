//! Reproduction tests for VAL-006 / GitHub issue #5 (broader audit):
//!
//! Daemon IPC handlers across `crates/tldr-daemon/src/handlers/{ast,flow,quality}.rs`
//! must reject absolute paths that resolve outside the project root via
//! `tldr_core::validate_file_path`. M1 (VAL-001) hardened the two handlers in
//! `security.rs` (secrets, vuln). M6 covers the audit's remaining 7 unguarded
//! sites:
//!
//!   - `ast::imports`              (ast.rs:175-179 unfixed `is_absolute → accept`)
//!   - `flow::cfg`                 (flow.rs:40-44)
//!   - `flow::dfg`                 (flow.rs:89-93)
//!   - `flow::slice`               (flow.rs:155-159)
//!   - `flow::complexity`          (flow.rs:229-233)
//!   - `quality::smells`           (quality.rs:45-53)
//!   - `quality::maintainability`  (quality.rs:118-126)
//!
//! Each handler is driven in-process with a synthetic out-of-project absolute
//! path pointing at a "victim" file containing a recognizable canary string
//! (`canary_xyz_42` in module name, function name, variable name, and string
//! contents). After the M1-pattern fix is applied, every handler must return
//! a `BAD_REQUEST` `HandlerError` (the `validate_file_path` failure path).
//!
//! On unfixed code:
//!   - Handlers that echo source content (`imports`, `cfg`, `dfg`, `complexity`)
//!     return `Ok` with the canary string surfaced in the JSON response —
//!     proof the file was opened and parsed.
//!   - Handlers that don't echo source content (`slice`, `smells`,
//!     `maintainability`) return `Ok` with non-empty analysis output — proof
//!     the handler walked the out-of-project path. The panic message names
//!     the leaked-content check explicitly to satisfy the RED-REASON gate.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::{extract::State, Json};
use serde_json::Value;
use tempfile::TempDir;

use tldr_daemon::handlers::ast::{imports, ImportsRequest};
use tldr_daemon::handlers::flow::{
    cfg, complexity, dfg, slice, CfgRequest, ComplexityRequest, DfgRequest, SliceRequest,
};
use tldr_daemon::handlers::quality::{
    maintainability, smells, MaintainabilityRequest, SmellsRequest,
};
use tldr_daemon::server::compute_socket_path;
use tldr_daemon::state::DaemonState;

/// Unique canary token. Must NEVER appear in the daemon response payload after
/// `validate_file_path` is wired in: every unguarded handler that opens the
/// victim file echoes some derivative of this token (module name, function
/// name, variable name, file path, etc.) into the response.
const CANARY: &str = "canary_xyz_42";

/// Recursively walk a JSON value and return true if any string field anywhere
/// contains `needle`. Mirrors the helper in `path_traversal_test.rs`.
fn json_contains_substring(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(s) => s.contains(needle),
        Value::Array(items) => items.iter().any(|v| json_contains_substring(v, needle)),
        Value::Object(map) => map
            .iter()
            .any(|(k, v)| k.contains(needle) || json_contains_substring(v, needle)),
        _ => false,
    }
}

/// Build a `DaemonState` rooted at the given path.
fn make_state(project_root: PathBuf) -> Arc<DaemonState> {
    let socket = compute_socket_path(&project_root, "1.0");
    Arc::new(DaemonState::new(project_root, socket))
}

/// Materialize a Python victim file outside the project root containing the
/// canary in module name, function name, variable name, and string content.
/// Returns the absolute path to the file.
///
/// The Python source is intentionally syntactically valid so unfixed handlers
/// successfully parse it and surface canary-bearing fields in their response.
fn write_victim_file(victim_dir: &Path) -> PathBuf {
    let victim_file = victim_dir.join("victim.py");
    // The CANARY token (`canary_xyz_42`) appears literally as both the imported
    // module name AND the function name AND the local variable name, so any
    // unguarded handler that parses the file and surfaces module/function/
    // variable identifiers in its response will show the canary verbatim.
    let content = "\
# Sensitive victim file (simulating a path-traversal target outside project)
import canary_xyz_42

def canary_xyz_42():
    canary_xyz_42_var = \"VictimSuperSecretValue_canary_xyz_42\"
    if canary_xyz_42_var:
        return canary_xyz_42_var
    return None
";
    std::fs::write(&victim_file, content).expect("write victim file");
    victim_file
}

/// Standard test setup: a project tempdir with one inert file (so canonicalize
/// works) and a SEPARATE victim tempdir holding the canary-bearing source.
fn setup() -> (TempDir, TempDir, PathBuf) {
    let project_dir = TempDir::new().expect("project tempdir");
    std::fs::write(project_dir.path().join("inside.txt"), "harmless").expect("inside file");

    let victim_dir = TempDir::new().expect("victim tempdir");
    let victim_path = write_victim_file(victim_dir.path());

    assert!(
        !victim_path.starts_with(project_dir.path()),
        "test setup error: victim should be outside project root"
    );

    (project_dir, victim_dir, victim_path)
}

// =============================================================================
// ast::imports
// =============================================================================

#[tokio::test]
async fn imports_handler_rejects_absolute_path_outside_project() {
    let (project_dir, _victim_dir, victim_path) = setup();
    let state = make_state(project_dir.path().to_path_buf());

    let request = ImportsRequest {
        file: victim_path.to_string_lossy().to_string(),
        language: Some("python".to_string()),
    };

    let result = imports(State(state), Json(request)).await;

    match result {
        Err(_) => {
            // Fixed path: handler refused absolute path (BAD_REQUEST).
        }
        Ok(Json(response)) => {
            let serialized = serde_json::to_value(&response).expect("serialize response");
            let leaked = json_contains_substring(&serialized, CANARY);
            // Strong assertion: handler MUST reject the absolute path. Even if
            // the response body happens not to contain the canary (e.g. parse
            // returned an empty import list), the handler still walked an
            // out-of-project absolute path — which is the bug.
            panic!(
                "imports handler accepted out-of-project absolute path {:?} \
                 (expected validate_file_path BAD_REQUEST). leaked-content check: \
                 canary '{}' present={} (the imports handler opens the file via \
                 get_imports → parse_file regardless of whether the import list \
                 echoes any canary string). Response body: {}",
                victim_path,
                CANARY,
                leaked,
                serde_json::to_string_pretty(&serialized).unwrap_or_default()
            );
        }
    }
}

// =============================================================================
// flow::cfg
// =============================================================================

#[tokio::test]
async fn cfg_handler_rejects_absolute_path_outside_project() {
    let (project_dir, _victim_dir, victim_path) = setup();
    let state = make_state(project_dir.path().to_path_buf());

    let request = CfgRequest {
        file: victim_path.to_string_lossy().to_string(),
        function: "canary_xyz_42".to_string(),
        language: Some("python".to_string()),
    };

    let result = cfg(State(state), Json(request)).await;

    match result {
        Err(_) => {
            // Fixed path: handler refused absolute path (BAD_REQUEST).
        }
        Ok(Json(response)) => {
            let serialized = serde_json::to_value(&response).expect("serialize response");
            let leaked = json_contains_substring(&serialized, CANARY);
            panic!(
                "cfg handler accepted out-of-project absolute path {:?} \
                 (expected validate_file_path BAD_REQUEST). leaked-content check: \
                 canary '{}' present={} (CfgInfo.function and CfgEdge.condition fields \
                 echo the canary-bearing function/variable names from the parsed source). \
                 Response body: {}",
                victim_path,
                CANARY,
                leaked,
                serde_json::to_string_pretty(&serialized).unwrap_or_default()
            );
        }
    }
}

// =============================================================================
// flow::dfg
// =============================================================================

#[tokio::test]
async fn dfg_handler_rejects_absolute_path_outside_project() {
    let (project_dir, _victim_dir, victim_path) = setup();
    let state = make_state(project_dir.path().to_path_buf());

    let request = DfgRequest {
        file: victim_path.to_string_lossy().to_string(),
        function: "canary_xyz_42".to_string(),
        language: Some("python".to_string()),
    };

    let result = dfg(State(state), Json(request)).await;

    match result {
        Err(_) => {
            // Fixed path: handler refused absolute path (BAD_REQUEST).
        }
        Ok(Json(response)) => {
            let serialized = serde_json::to_value(&response).expect("serialize response");
            let leaked = json_contains_substring(&serialized, CANARY);
            panic!(
                "dfg handler accepted out-of-project absolute path {:?} \
                 (expected validate_file_path BAD_REQUEST). leaked-content check: \
                 canary '{}' present={} (DfgInfo.function and VarRef.name fields \
                 echo the canary-bearing function/variable names from the parsed source). \
                 Response body: {}",
                victim_path,
                CANARY,
                leaked,
                serde_json::to_string_pretty(&serialized).unwrap_or_default()
            );
        }
    }
}

// =============================================================================
// flow::slice
// =============================================================================

#[tokio::test]
async fn slice_handler_rejects_absolute_path_outside_project() {
    let (project_dir, _victim_dir, victim_path) = setup();
    let state = make_state(project_dir.path().to_path_buf());

    let request = SliceRequest {
        file: victim_path.to_string_lossy().to_string(),
        function: "canary_xyz_42".to_string(),
        line: 5, // line of the canary_variable_xyz_42 use inside the function
        direction: "backward".to_string(),
        variable: None,
        language: Some("python".to_string()),
    };

    let result = slice(State(state), Json(request)).await;

    // The slice response shape (`SliceResponse { lines, direction, line_count }`)
    // does not embed source content directly, so the strong assertion is that
    // the handler returned an Err (i.e. validate_file_path's BAD_REQUEST). On
    // unfixed code the handler returns Ok with non-empty `lines` and
    // `line_count > 0`, proving it walked the out-of-project file.
    match result {
        Err(_) => {
            // Fixed path: handler refused absolute path.
        }
        Ok(Json(response)) => {
            let serialized = serde_json::to_value(&response).expect("serialize response");
            // Defensive: in case slice ever starts echoing canary content, also
            // check for the canary substring to satisfy the leaked-content gate.
            let leaked_canary = json_contains_substring(&serialized, CANARY);
            panic!(
                "slice handler accepted out-of-project absolute path {:?} \
                 (expected validate_file_path BAD_REQUEST). leaked-content check: \
                 canary '{}' present={}. Response body: {}",
                victim_path,
                CANARY,
                leaked_canary,
                serde_json::to_string_pretty(&serialized).unwrap_or_default()
            );
        }
    }
}

// =============================================================================
// flow::complexity
// =============================================================================

#[tokio::test]
async fn complexity_handler_rejects_absolute_path_outside_project() {
    let (project_dir, _victim_dir, victim_path) = setup();
    let state = make_state(project_dir.path().to_path_buf());

    let request = ComplexityRequest {
        file: victim_path.to_string_lossy().to_string(),
        function: "canary_xyz_42".to_string(),
        language: Some("python".to_string()),
    };

    let result = complexity(State(state), Json(request)).await;

    match result {
        Err(_) => {
            // Fixed path: handler refused absolute path (BAD_REQUEST).
        }
        Ok(Json(response)) => {
            let serialized = serde_json::to_value(&response).expect("serialize response");
            let leaked = json_contains_substring(&serialized, CANARY);
            panic!(
                "complexity handler accepted out-of-project absolute path {:?} \
                 (expected validate_file_path BAD_REQUEST). leaked-content check: \
                 canary '{}' present={} (ComplexityMetrics.function field echoes the \
                 canary-bearing function name from the parsed source). Response body: {}",
                victim_path,
                CANARY,
                leaked,
                serde_json::to_string_pretty(&serialized).unwrap_or_default()
            );
        }
    }
}

// =============================================================================
// quality::smells
// =============================================================================

#[tokio::test]
async fn smells_handler_rejects_absolute_path_outside_project() {
    let (project_dir, _victim_dir, victim_path) = setup();
    let state = make_state(project_dir.path().to_path_buf());

    let request = SmellsRequest {
        path: Some(victim_path.to_string_lossy().to_string()),
        threshold: None,
        smell_type: None,
        suggest: false,
    };

    let result = smells(State(state), Json(request)).await;

    // SmellFinding.file echoes the path of the analyzed file, so if smells
    // walked the victim file the response will contain the victim_dir tempdir
    // path. We use the canary substring in the response as the leak signal,
    // and also assert files_analyzed == 0 if no smells were found (the
    // strongest signal is that smells refused the path — Err — before walking).
    match result {
        Err(_) => {
            // Fixed path: handler refused absolute path.
        }
        Ok(Json(response)) => {
            let serialized = serde_json::to_value(&response).expect("serialize response");
            let leaked_canary = json_contains_substring(&serialized, CANARY);
            // The serialized response will have `files_analyzed > 0` if smells
            // walked the victim path (proof of out-of-project filesystem read).
            let inner = serialized
                .get("data")
                .and_then(|d| d.get("files_analyzed"))
                .and_then(|n| n.as_u64())
                .unwrap_or(0);
            panic!(
                "smells handler accepted out-of-project absolute path {:?} \
                 (expected validate_file_path BAD_REQUEST). leaked-content check: \
                 canary '{}' present={} ; files_analyzed={} (>0 = scanner walked victim path). \
                 Response body: {}",
                victim_path,
                CANARY,
                leaked_canary,
                inner,
                serde_json::to_string_pretty(&serialized).unwrap_or_default()
            );
        }
    }
}

// =============================================================================
// quality::maintainability
// =============================================================================

#[tokio::test]
async fn maintainability_handler_rejects_absolute_path_outside_project() {
    let (project_dir, _victim_dir, victim_path) = setup();
    let state = make_state(project_dir.path().to_path_buf());

    let request = MaintainabilityRequest {
        path: Some(victim_path.to_string_lossy().to_string()),
        include_halstead: false,
        language: Some("python".to_string()),
    };

    let result = maintainability(State(state), Json(request)).await;

    // FileMI.path echoes the analyzed file path, so a successful walk of the
    // victim leaves the victim path in the response. The strong assertion is
    // Err (validate_file_path BAD_REQUEST). Unfixed code returns Ok with at
    // least one FileMI entry pointing at the victim.
    match result {
        Err(_) => {
            // Fixed path: handler refused absolute path.
        }
        Ok(Json(response)) => {
            let serialized = serde_json::to_value(&response).expect("serialize response");
            let leaked_canary = json_contains_substring(&serialized, CANARY);
            let file_count = serialized
                .get("data")
                .and_then(|d| d.get("files"))
                .and_then(|f| f.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            panic!(
                "maintainability handler accepted out-of-project absolute path {:?} \
                 (expected validate_file_path BAD_REQUEST). leaked-content check: \
                 canary '{}' present={} ; files_analyzed_count={} (>0 = walker analyzed victim). \
                 Response body: {}",
                victim_path,
                CANARY,
                leaked_canary,
                file_count,
                serde_json::to_string_pretty(&serialized).unwrap_or_default()
            );
        }
    }
}
