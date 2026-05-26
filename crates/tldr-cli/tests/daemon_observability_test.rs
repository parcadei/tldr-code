//! Daemon observability contract tests (issue #67).
//!
//! Each test in this file locks down ONE externally-observable signal that the
//! daemon must keep emitting so callers can answer "what just happened?" from
//! logs alone. Tests are intentionally narrow — one signal per test — so a
//! regression points at the exact contract that broke.
//!
//! All tests in this file spawn a real daemon process (which writes to
//! `~/.tldr/registry.json`), so they are `#[ignore]` by default and run via
//! `cargo test -p tldr-cli --test daemon_observability_test -- --ignored`.
//! Same gating pattern as the daemon-spawn tests in `daemon_test.rs` (#68).

use std::path::Path;
use std::process::Command;
use std::time::Duration;
use tempfile::TempDir;

fn tldr_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
}

fn stop_daemon(project: &Path) {
    let _ = tldr_cmd()
        .args(["daemon", "stop", "--project"])
        .arg(project)
        .output();
}

/// Contract: `daemon start` (background mode) must create
/// `<project>/.tldr/daemon.log` and write at least one byte of daemon output
/// to it. Prevents regression to the previous behavior where the spawned
/// daemon's stdout and stderr were dropped to `/dev/null`, making any panic
/// or tracing message invisible after the parent CLI exited.
#[test]
#[ignore = "spawns a real daemon and writes to ~/.tldr/registry.json — run manually with `cargo test -- --ignored`"]
fn daemon_start_creates_nonempty_daemon_log() {
    let temp = TempDir::new().expect("temp dir");
    let project = temp.path();

    let start = tldr_cmd()
        .args(["daemon", "start", "--project"])
        .arg(project)
        .output()
        .expect("spawn daemon start");
    assert!(
        start.status.success(),
        "daemon start failed: stdout={:?} stderr={:?}",
        String::from_utf8_lossy(&start.stdout),
        String::from_utf8_lossy(&start.stderr),
    );

    // Give the detached child a brief moment to flush its first tracing line.
    std::thread::sleep(Duration::from_millis(500));

    let log_path = project.join(".tldr").join("daemon.log");
    let metadata = std::fs::metadata(&log_path).unwrap_or_else(|e| {
        stop_daemon(project);
        panic!("expected {:?} to exist after `daemon start`, got: {}", log_path, e);
    });
    let len = metadata.len();

    stop_daemon(project);

    assert!(
        len > 0,
        "expected {:?} to contain at least one byte of daemon output, was empty",
        log_path,
    );
}
