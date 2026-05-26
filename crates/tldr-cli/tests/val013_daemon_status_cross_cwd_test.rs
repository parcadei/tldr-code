//! VAL-013: Daemon status cross-cwd discovery (issue #20).
//!
//! Reproduces: `tldr daemon status` defaults `--project` to "." and computes a
//! socket-path hash from that. Invoked from a cwd different from the
//! `daemon start --project` cwd, the hash differs → connect fails →
//! status incorrectly reports `not_running`, even though the daemon IS alive.
//!
//! Live repro (orchestrator, 2026-04-27, HEAD 451036d):
//!   - `daemon start --project <fixture>` → PID 33467 alive
//!   - `daemon status` from `/tmp` → `{"status":"not_running"}`  ← BUG
//!   - `daemon status --project <fixture>` from `/tmp` → running ← workaround
//!
//! Acceptance (post-fix):
//!   1. `daemon status` invoked from a different cwd (no `--project`) reports
//!      `status == "running"` and `project == <fixture>`.
//!   2. `daemon status --project <fixture>` workaround keeps working.
//!
//! Fix strategy: write `~/Library/Caches/tldr/daemon-active.json` (or
//! XDG cache) atomically on successful bind containing `{project, pid, socket}`;
//! `daemon status` falls back to that file when `--project` is the default.

#![cfg(unix)]

use std::path::Path;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

static DAEMON_TEST_LOCK: Mutex<()> = Mutex::new(());

/// A scope guard that issues `daemon stop --project <fixture>` from the
/// fixture cwd on drop, ensuring the daemon process is cleaned up even if
/// the test panics.
struct DaemonStopGuard {
    project: std::path::PathBuf,
}

impl Drop for DaemonStopGuard {
    fn drop(&mut self) {
        // Best-effort stop. Allow up to 5 s for the daemon to exit cleanly.
        let _ = Command::new(env!("CARGO_BIN_EXE_tldr"))
            .args(["daemon", "stop", "--project"])
            .arg(&self.project)
            .output();
    }
}

/// Wait until the daemon answers a `status --project <fixture>` call with
/// `"status":"running"`. Caps at `timeout` and re-polls every 100 ms.
fn wait_for_daemon_running(project: &Path, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        let out = Command::new(env!("CARGO_BIN_EXE_tldr"))
            .args(["daemon", "status", "--project"])
            .arg(project)
            .output();
        if let Ok(out) = out {
            let stdout = String::from_utf8_lossy(&out.stdout);
            if stdout.contains("\"status\": \"running\"")
                || stdout.contains("\"status\":\"running\"")
            {
                return true;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// RED on HEAD 451036d, GREEN after fix.
///
/// Spawns a daemon for a fixture project; from `/tmp` (a different cwd, NOT
/// the fixture project), runs `daemon status` with no `--project` flag.
///
/// Pre-fix: status defaults `--project` to ".", which canonicalizes to
/// `/tmp` (or whatever cwd is), and produces a different socket hash.
/// `IpcStream::connect` fails → status returns `"not_running"`.
///
/// Post-fix: status reads the active-daemon discovery file as a fallback
/// when `--project` defaults to ".", looks up the alive daemon's project,
/// and reports `"running"` with the fixture project path.
#[test]
fn daemon_status_from_other_cwd_reports_running() {
    let _guard = DAEMON_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    // Pre-test cleanup: stop any daemons left over from previous test runs
    // so the cross-cwd discovery has a single canonical active daemon to
    // resolve. Without this, leftover daemons yield "multiple daemons
    // running" and `daemon status` errors to stderr instead of emitting
    // a JSON envelope.
    let stop_all = Command::new(env!("CARGO_BIN_EXE_tldr"))
        .args(["daemon", "stop", "--all"])
        .output();
    let _ = stop_all; // best-effort

    let fixture = tempfile::Builder::new()
        .prefix("val013-fixture-")
        .tempdir()
        .expect("tempdir");
    // Canonicalize so the path matches the daemon's resolved project.
    let fixture_path = fixture
        .path()
        .canonicalize()
        .expect("canonicalize fixture path");

    // Start daemon for the fixture project.
    let start = Command::new(env!("CARGO_BIN_EXE_tldr"))
        .args(["daemon", "start", "--project"])
        .arg(&fixture_path)
        .output()
        .expect("daemon start spawn");
    assert!(
        start.status.success(),
        "daemon start failed: stdout={} stderr={}",
        String::from_utf8_lossy(&start.stdout),
        String::from_utf8_lossy(&start.stderr)
    );

    // Ensure cleanup regardless of test outcome.
    let _stop_guard = DaemonStopGuard {
        project: fixture_path.clone(),
    };

    // Wait for the daemon to be ready (probed via the workaround path
    // which uses an explicit --project; this is unaffected by the bug).
    assert!(
        wait_for_daemon_running(&fixture_path, Duration::from_secs(10)),
        "daemon never became reachable via --project workaround within 10 s"
    );

    // CORE REPRODUCTION: invoke `daemon status` from a different cwd
    // (`/tmp`) with NO `--project` flag. Pre-fix this returns
    // `"status":"not_running"`. Post-fix it returns `"status":"running"`
    // with the fixture's project path.
    let status_other_cwd = Command::new(env!("CARGO_BIN_EXE_tldr"))
        .args(["daemon", "status"])
        .current_dir("/tmp")
        .output()
        .expect("daemon status spawn (from /tmp)");

    let stdout = String::from_utf8_lossy(&status_other_cwd.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&status_other_cwd.stderr).into_owned();

    // RED gate: pre-fix, stdout will contain `"status": "not_running"`.
    // We assert post-fix behavior.
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "[VAL-013] daemon status (from /tmp, no --project) did not emit valid JSON: \
             parse_err={} stdout={:?} stderr={:?}",
            e, stdout, stderr
        )
    });

    let status_field = parsed
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("<missing>");
    assert_eq!(
        status_field, "running",
        "[VAL-013 cross-cwd discovery] daemon status from /tmp (no --project) \
         reported status={:?}, but the daemon IS alive (verified via the \
         --project workaround). Expected status=\"running\". \
         RED proof keyword: not_running. Full stdout: {}",
        status_field, stdout
    );

    // Secondary gate: project path must be the fixture, not /tmp.
    let fixture_str = fixture_path.to_string_lossy();
    let project_field = parsed
        .get("project")
        .and_then(|v| v.as_str())
        .unwrap_or("<missing>");
    assert_eq!(
        project_field, fixture_str,
        "[VAL-013 cross-cwd discovery] daemon status reported project={:?}, \
         but the running daemon was started with --project={:?}. \
         The status discovery fallback must surface the active daemon's \
         project, not the caller's cwd.",
        project_field, fixture_str
    );
}

/// Regression guard: the existing `--project` workaround MUST keep working
/// after the fix. Pre-fix this already passes; we run it post-fix to ensure
/// the active-daemon fallback did not break the explicit-flag path.
#[test]
fn daemon_status_with_explicit_project_still_works_from_other_cwd() {
    let _guard = DAEMON_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let fixture = tempfile::Builder::new()
        .prefix("val013-workaround-")
        .tempdir()
        .expect("tempdir");
    let fixture_path = fixture
        .path()
        .canonicalize()
        .expect("canonicalize fixture path");

    let start = Command::new(env!("CARGO_BIN_EXE_tldr"))
        .args(["daemon", "start", "--project"])
        .arg(&fixture_path)
        .output()
        .expect("daemon start spawn");
    assert!(start.status.success(), "daemon start failed");

    let _stop_guard = DaemonStopGuard {
        project: fixture_path.clone(),
    };

    assert!(
        wait_for_daemon_running(&fixture_path, Duration::from_secs(10)),
        "daemon never became reachable"
    );

    // From /tmp, with an explicit --project, must still return running.
    let out = Command::new(env!("CARGO_BIN_EXE_tldr"))
        .args(["daemon", "status", "--project"])
        .arg(&fixture_path)
        .current_dir("/tmp")
        .output()
        .expect("daemon status spawn");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("invalid JSON: err={} stdout={}", e, stdout));
    let status_field = parsed
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("<missing>");
    assert_eq!(
        status_field, "running",
        "regression: explicit --project workaround broken; got {:?} \
         (full stdout: {})",
        status_field, stdout
    );
}
