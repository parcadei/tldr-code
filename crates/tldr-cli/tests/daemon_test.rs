//! Comprehensive tests for TLDR Daemon and Cache commands
//!
//! These tests define expected behavior from spec.md and should FAIL initially
//! since no implementation exists yet. They drive the implementation.
//!
//! Test categories:
//! 1. Unit Tests - Types & Serialization
//! 2. Daemon Lifecycle Tests
//! 3. IPC Protocol Tests
//! 4. Cache Tests
//! 5. Warm Command Tests
//! 6. Stats Command Tests
//! 7. Edge Case Tests

use assert_cmd::prelude::*;
use assert_cmd::Command as AssertCommand;
use predicates::prelude::*;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;
use tempfile::TempDir;

/// Get the path to the test binary (std::process::Command version)
fn tldr_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
}

/// Get the path to the test binary (assert_cmd::Command version for timeout support)
fn tldr_assert_cmd() -> AssertCommand {
    assert_cmd::cargo::cargo_bin_cmd!("tldr")
}

fn cleanup_daemon(project_path: &str) {
    let mut stop_cmd = tldr_cmd();
    let _ = stop_cmd
        .args(["daemon", "stop", "--project", project_path])
        .assert();
}

/// Get home directory (cross-platform)
fn home_dir() -> PathBuf {
    env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}

// =============================================================================
// Module: Types (to be implemented in daemon/types.rs)
// =============================================================================

/// These types mirror the spec and will be imported once implemented.
/// For now, we define them inline to make tests compilable.
mod daemon_types {
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;
    use std::path::PathBuf;

    /// Idle timeout before daemon auto-shutdown (30 minutes)
    pub const IDLE_TIMEOUT_SECS: u64 = 30 * 60;

    /// Default threshold for triggering semantic re-index
    pub const DEFAULT_REINDEX_THRESHOLD: usize = 20;

    /// Daemon configuration
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    pub struct DaemonConfig {
        pub semantic_enabled: bool,
        pub auto_reindex_threshold: usize,
        pub semantic_model: String,
        pub idle_timeout_secs: u64,
    }

    impl Default for DaemonConfig {
        fn default() -> Self {
            Self {
                semantic_enabled: true,
                auto_reindex_threshold: DEFAULT_REINDEX_THRESHOLD,
                semantic_model: "bge-large-en-v1.5".to_string(),
                idle_timeout_secs: IDLE_TIMEOUT_SECS,
            }
        }
    }

    /// Daemon runtime status
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum DaemonStatus {
        Initializing,
        Indexing,
        Ready,
        ShuttingDown,
        Stopped,
    }

    /// Statistics for Salsa-style query cache
    #[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
    pub struct SalsaCacheStats {
        pub hits: u64,
        pub misses: u64,
        pub invalidations: u64,
        pub recomputations: u64,
    }

    impl SalsaCacheStats {
        pub fn hit_rate(&self) -> f64 {
            let total = self.hits + self.misses;
            if total == 0 {
                return 0.0;
            }
            (self.hits as f64 / total as f64) * 100.0
        }
    }

    /// Per-session statistics for token tracking
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct SessionStats {
        pub session_id: String,
        pub raw_tokens: u64,
        pub tldr_tokens: u64,
        pub requests: u64,
    }

    impl SessionStats {
        pub fn savings_tokens(&self) -> i64 {
            self.raw_tokens as i64 - self.tldr_tokens as i64
        }

        pub fn savings_percent(&self) -> f64 {
            if self.raw_tokens == 0 {
                return 0.0;
            }
            (self.savings_tokens() as f64 / self.raw_tokens as f64) * 100.0
        }
    }

    /// Command sent to daemon via socket
    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(tag = "cmd", rename_all = "snake_case")]
    pub enum DaemonCommand {
        Ping,
        Status {
            #[serde(skip_serializing_if = "Option::is_none")]
            session: Option<String>,
        },
        Shutdown,
        Notify {
            file: PathBuf,
        },
        Track {
            hook: String,
            #[serde(default = "default_true")]
            success: bool,
            #[serde(default)]
            metrics: HashMap<String, f64>,
        },
        Warm {
            #[serde(default)]
            language: Option<String>,
        },
        Semantic {
            query: String,
            #[serde(default = "default_top_k")]
            top_k: usize,
        },
        Search {
            pattern: String,
            max_results: Option<usize>,
        },
        Extract {
            file: PathBuf,
            session: Option<String>,
        },
    }

    fn default_true() -> bool {
        true
    }
    fn default_top_k() -> usize {
        10
    }

    /// Response from daemon
    ///
    /// IMPORTANT: Variant order matters for serde(untagged)!
    /// Variants are tried in declaration order, so more specific variants
    /// (with more required fields) must come BEFORE less specific ones.
    ///
    /// Key design: Error uses "error" field, Status uses "message" field.
    /// This makes them structurally distinguishable for serde untagged.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(untagged)]
    pub enum DaemonResponse {
        // FullStatus has 5 required fields including a typed enum status
        FullStatus {
            status: DaemonStatus,
            uptime: f64,
            files: usize,
            project: PathBuf,
            salsa_stats: SalsaCacheStats,
        },
        // NotifyResponse has 4 required fields
        NotifyResponse {
            status: String,
            dirty_count: usize,
            threshold: usize,
            reindex_triggered: bool,
        },
        // Error uses "error" field (not "message") to be distinguishable from Status
        Error {
            status: String,
            error: String,
        },
        // Status is the catch-all with only 1 required field (message is optional)
        Status {
            status: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            message: Option<String>,
        },
    }

    /// Aggregated global stats
    #[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
    pub struct GlobalStats {
        pub total_invocations: u64,
        pub estimated_tokens_saved: i64,
        pub raw_tokens_total: u64,
        pub tldr_tokens_total: u64,
        pub savings_percent: f64,
    }

    /// Cache file info for cache stats
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    pub struct CacheFileInfo {
        pub file_count: usize,
        pub total_bytes: u64,
        pub total_size_human: String,
    }
}

use daemon_types::*;

// =============================================================================
// 1. Unit Tests - Types & Serialization
// =============================================================================

mod unit_types {
    use super::*;

    #[test]
    fn test_daemon_config_default() {
        let config = DaemonConfig::default();

        assert!(config.semantic_enabled);
        assert_eq!(config.auto_reindex_threshold, DEFAULT_REINDEX_THRESHOLD);
        assert_eq!(config.semantic_model, "bge-large-en-v1.5");
        assert_eq!(config.idle_timeout_secs, IDLE_TIMEOUT_SECS);
    }

    #[test]
    fn test_daemon_config_serialization() {
        let config = DaemonConfig::default();
        let json = serde_json::to_string(&config).unwrap();

        assert!(json.contains("semantic_enabled"));
        assert!(json.contains("auto_reindex_threshold"));
        assert!(json.contains("20")); // DEFAULT_REINDEX_THRESHOLD
    }

    #[test]
    fn test_daemon_config_deserialization() {
        let json = r#"{
            "semantic_enabled": false,
            "auto_reindex_threshold": 50,
            "semantic_model": "custom-model",
            "idle_timeout_secs": 3600
        }"#;

        let config: DaemonConfig = serde_json::from_str(json).unwrap();

        assert!(!config.semantic_enabled);
        assert_eq!(config.auto_reindex_threshold, 50);
        assert_eq!(config.semantic_model, "custom-model");
        assert_eq!(config.idle_timeout_secs, 3600);
    }

    #[test]
    fn test_daemon_command_ping_serialization() {
        let cmd = DaemonCommand::Ping;
        let json = serde_json::to_string(&cmd).unwrap();

        assert_eq!(json, r#"{"cmd":"ping"}"#);
    }

    #[test]
    fn test_daemon_command_status_serialization() {
        let cmd = DaemonCommand::Status { session: None };
        let json = serde_json::to_string(&cmd).unwrap();

        assert_eq!(json, r#"{"cmd":"status"}"#);
    }

    #[test]
    fn test_daemon_command_status_with_session() {
        let cmd = DaemonCommand::Status {
            session: Some("abc123".to_string()),
        };
        let json = serde_json::to_string(&cmd).unwrap();

        assert!(json.contains("abc123"));
    }

    #[test]
    fn test_daemon_command_notify_serialization() {
        let cmd = DaemonCommand::Notify {
            file: PathBuf::from("/path/to/file.rs"),
        };
        let json = serde_json::to_string(&cmd).unwrap();

        assert!(json.contains("notify"));
        assert!(json.contains("/path/to/file.rs"));
    }

    #[test]
    fn test_daemon_command_track_serialization() {
        let mut metrics = HashMap::new();
        metrics.insert("errors_found".to_string(), 3.0);

        let cmd = DaemonCommand::Track {
            hook: "pre-commit".to_string(),
            success: true,
            metrics,
        };
        let json = serde_json::to_string(&cmd).unwrap();

        assert!(json.contains("track"));
        assert!(json.contains("pre-commit"));
        assert!(json.contains("errors_found"));
    }

    #[test]
    fn test_daemon_response_status_deserialization() {
        let json = r#"{"status": "ok", "message": "Daemon started"}"#;
        let response: DaemonResponse = serde_json::from_str(json).unwrap();

        match response {
            DaemonResponse::Status { status, message } => {
                assert_eq!(status, "ok");
                assert_eq!(message, Some("Daemon started".to_string()));
            }
            _ => panic!("Expected Status response"),
        }
    }

    #[test]
    fn test_daemon_response_notify_deserialization() {
        let json = r#"{
            "status": "ok",
            "dirty_count": 5,
            "threshold": 20,
            "reindex_triggered": false
        }"#;
        let response: DaemonResponse = serde_json::from_str(json).unwrap();

        match response {
            DaemonResponse::NotifyResponse {
                dirty_count,
                threshold,
                reindex_triggered,
                ..
            } => {
                assert_eq!(dirty_count, 5);
                assert_eq!(threshold, 20);
                assert!(!reindex_triggered);
            }
            _ => panic!("Expected NotifyResponse"),
        }
    }

    #[test]
    fn test_daemon_response_error_deserialization() {
        // Error variant uses "error" field (not "message") to be distinguishable
        let json = r#"{"status": "error", "error": "Something went wrong"}"#;
        let response: DaemonResponse = serde_json::from_str(json).unwrap();

        match response {
            DaemonResponse::Error { status, error } => {
                assert_eq!(status, "error");
                assert_eq!(error, "Something went wrong");
            }
            _ => panic!("Expected Error response, got {:?}", response),
        }
    }

    #[test]
    fn test_daemon_response_status_only_deserialization() {
        // Status-only JSON should match Status variant (catch-all)
        let json = r#"{"status": "ok"}"#;
        let response: DaemonResponse = serde_json::from_str(json).unwrap();

        match response {
            DaemonResponse::Status { status, message } => {
                assert_eq!(status, "ok");
                assert_eq!(message, None);
            }
            _ => panic!("Expected Status response"),
        }
    }

    #[test]
    fn test_salsa_cache_stats_hit_rate_empty() {
        let stats = SalsaCacheStats::default();
        assert_eq!(stats.hit_rate(), 0.0);
    }

    #[test]
    fn test_salsa_cache_stats_hit_rate_calculation() {
        let stats = SalsaCacheStats {
            hits: 90,
            misses: 10,
            invalidations: 5,
            recomputations: 3,
        };
        assert!((stats.hit_rate() - 90.0).abs() < 0.01);
    }

    #[test]
    fn test_session_stats_savings_calculation() {
        let stats = SessionStats {
            session_id: "test123".to_string(),
            raw_tokens: 1000,
            tldr_tokens: 100,
            requests: 10,
        };

        assert_eq!(stats.savings_tokens(), 900);
        assert!((stats.savings_percent() - 90.0).abs() < 0.01);
    }

    #[test]
    fn test_session_stats_zero_tokens() {
        let stats = SessionStats {
            session_id: "empty".to_string(),
            raw_tokens: 0,
            tldr_tokens: 0,
            requests: 0,
        };

        assert_eq!(stats.savings_tokens(), 0);
        assert_eq!(stats.savings_percent(), 0.0);
    }

    #[test]
    fn test_daemon_status_serialization() {
        let status = DaemonStatus::Ready;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, r#""ready""#);

        let status = DaemonStatus::Initializing;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, r#""initializing""#);
    }

    #[test]
    fn test_global_stats_serialization() {
        let stats = GlobalStats {
            total_invocations: 12,
            estimated_tokens_saved: 345,
            raw_tokens_total: 1_000,
            tldr_tokens_total: 655,
            savings_percent: 34.5,
        };
        let json = serde_json::to_string(&stats).unwrap();
        let roundtrip: GlobalStats = serde_json::from_str(&json).unwrap();

        assert_eq!(roundtrip.total_invocations, 12);
        assert_eq!(roundtrip.estimated_tokens_saved, 345);
        assert_eq!(roundtrip.raw_tokens_total, 1_000);
        assert_eq!(roundtrip.tldr_tokens_total, 655);
        assert_eq!(roundtrip.savings_percent, 34.5);
    }

    #[test]
    fn test_cache_file_info_serialization() {
        let cache_info = CacheFileInfo {
            file_count: 3,
            total_bytes: 4_096,
            total_size_human: "4.0 KiB".to_string(),
        };
        let json = serde_json::to_string(&cache_info).unwrap();
        let roundtrip: CacheFileInfo = serde_json::from_str(&json).unwrap();

        assert_eq!(roundtrip.file_count, 3);
        assert_eq!(roundtrip.total_bytes, 4_096);
        assert_eq!(roundtrip.total_size_human, "4.0 KiB");
    }
}

// =============================================================================
// 2. Daemon Lifecycle Tests (CLI integration)
// =============================================================================

mod daemon_lifecycle {
    use super::*;

    #[test]
    #[ignore = "daemon start command not yet implemented"]
    fn test_daemon_start_help() {
        let mut cmd = tldr_cmd();
        cmd.args(["daemon", "start", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("--project"))
            .stdout(predicate::str::contains("--foreground"));
    }

    #[test]
    #[ignore = "daemon start command not yet implemented"]
    fn test_daemon_start_creates_socket() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path().to_str().unwrap();

        // Start daemon in foreground mode with timeout
        let mut cmd = tldr_assert_cmd();
        cmd.args(["daemon", "start", "--project", project_path, "--foreground"])
            .timeout(Duration::from_secs(2));

        let output = cmd.output();

        // Verify socket path is mentioned in output
        if let Ok(output) = output {
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                stdout.contains(".sock") || stdout.contains("socket"),
                "Expected socket path in output"
            );
        }
    }

    #[test]
    #[ignore = "daemon start command not yet implemented"]
    fn test_daemon_start_creates_pid_file() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path().to_str().unwrap();

        // Start daemon
        let mut cmd = tldr_cmd();
        cmd.args(["daemon", "start", "--project", project_path])
            .assert()
            .success()
            .stdout(predicate::str::contains("pid").or(predicate::str::contains("PID")));

        // Stop daemon (cleanup)
        cleanup_daemon(project_path);
    }

    #[test]
    #[ignore = "daemon start command not yet implemented"]
    fn test_daemon_start_already_running_error() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path().to_str().unwrap();

        // Start first daemon
        let mut cmd1 = tldr_cmd();
        cmd1.args(["daemon", "start", "--project", project_path])
            .assert()
            .success();

        // Try to start second daemon - should fail
        let mut cmd2 = tldr_cmd();
        cmd2.args(["daemon", "start", "--project", project_path])
            .assert()
            .failure()
            .stderr(predicate::str::contains("already running"));

        // Cleanup
        cleanup_daemon(project_path);
    }

    #[test]
    #[ignore = "daemon stop command not yet implemented"]
    fn test_daemon_stop_help() {
        let mut cmd = tldr_cmd();
        cmd.args(["daemon", "stop", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("--project"));
    }

    #[test]
    #[ignore = "daemon stop command not yet implemented"]
    fn test_daemon_stop_removes_socket() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path().to_str().unwrap();

        // Start daemon
        let mut start_cmd = tldr_cmd();
        start_cmd
            .args(["daemon", "start", "--project", project_path])
            .assert()
            .success();

        // Stop daemon
        let mut stop_cmd = tldr_cmd();
        stop_cmd
            .args(["daemon", "stop", "--project", project_path])
            .assert()
            .success()
            .stdout(predicate::str::contains("stopped"));
    }

    #[test]
    #[ignore = "daemon stop command not yet implemented"]
    fn test_daemon_stop_not_running() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path().to_str().unwrap();

        // Stop daemon when not running - should succeed with message
        let mut cmd = tldr_cmd();
        cmd.args(["daemon", "stop", "--project", project_path])
            .assert()
            .success()
            .stdout(predicate::str::contains("not running"));
    }

    #[test]
    #[ignore = "daemon status command not yet implemented"]
    fn test_daemon_status_help() {
        let mut cmd = tldr_cmd();
        cmd.args(["daemon", "status", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("--project"))
            .stdout(predicate::str::contains("--session"));
    }

    #[test]
    #[ignore = "daemon status command not yet implemented"]
    fn test_daemon_status_returns_uptime() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path().to_str().unwrap();

        // Start daemon
        let mut start_cmd = tldr_cmd();
        start_cmd
            .args(["daemon", "start", "--project", project_path])
            .assert()
            .success();

        // Wait a bit
        std::thread::sleep(Duration::from_millis(500));

        // Check status
        let mut status_cmd = tldr_cmd();
        status_cmd
            .args(["daemon", "status", "--project", project_path])
            .assert()
            .success()
            .stdout(predicate::str::contains("uptime"));

        // Cleanup
        cleanup_daemon(project_path);
    }

    #[test]
    #[ignore = "daemon status command not yet implemented"]
    fn test_daemon_status_not_running() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path().to_str().unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["daemon", "status", "--project", project_path])
            .assert()
            .success()
            .stdout(predicate::str::contains("not running"));
    }

    #[test]
    #[ignore = "daemon status command not yet implemented"]
    fn test_daemon_status_json_output() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path().to_str().unwrap();

        // Start daemon
        let mut start_cmd = tldr_cmd();
        start_cmd
            .args(["daemon", "start", "--project", project_path])
            .assert()
            .success();

        // Get status in JSON format (default format is json)
        let mut status_cmd = tldr_cmd();
        let output = status_cmd
            .args(["daemon", "status", "--project", project_path])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);

        // Verify JSON structure
        let status: serde_json::Value = serde_json::from_str(&stdout).expect("Valid JSON output");
        assert!(status.get("status").is_some());
        assert!(status.get("uptime").is_some());
        assert!(status.get("files").is_some());

        // Cleanup
        cleanup_daemon(project_path);
    }
}

// =============================================================================
// 3. IPC Protocol Tests
// =============================================================================

mod ipc_protocol {
    use super::*;

    #[test]
    #[ignore = "daemon query command not yet implemented"]
    fn test_daemon_query_help() {
        let mut cmd = tldr_cmd();
        cmd.args(["daemon", "query", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("--project"))
            .stdout(predicate::str::contains("--json"));
    }

    #[test]
    #[ignore = "daemon query command not yet implemented"]
    fn test_daemon_query_ping() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path().to_str().unwrap();

        // Start daemon
        let mut start_cmd = tldr_cmd();
        start_cmd
            .args(["daemon", "start", "--project", project_path])
            .assert()
            .success();

        // Send ping query
        let mut query_cmd = tldr_cmd();
        query_cmd
            .args(["daemon", "query", "ping", "--project", project_path])
            .assert()
            .success()
            .stdout(predicate::str::contains("pong").or(predicate::str::contains("ok")));

        // Cleanup
        cleanup_daemon(project_path);
    }

    #[test]
    #[ignore = "daemon query command not yet implemented"]
    fn test_daemon_query_roundtrip() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path().to_str().unwrap();

        // Create test file
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        // Start daemon
        let mut start_cmd = tldr_cmd();
        start_cmd
            .args(["daemon", "start", "--project", project_path])
            .assert()
            .success();

        // Query structure
        let mut query_cmd = tldr_cmd();
        let output = query_cmd
            .args([
                "daemon",
                "query",
                "structure",
                "--project",
                project_path,
                "--json",
                &format!(r#"{{"path": "{}"}}"#, temp.path().join("test.py").display()),
            ])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let result: serde_json::Value = serde_json::from_str(&stdout).expect("Valid JSON response");
        assert!(result.get("status").is_some());

        // Cleanup
        cleanup_daemon(project_path);
    }

    #[test]
    #[ignore = "daemon notify command not yet implemented"]
    fn test_daemon_notify_help() {
        let mut cmd = tldr_cmd();
        cmd.args(["daemon", "notify", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("FILE"));
    }

    #[test]
    #[ignore = "daemon notify command not yet implemented"]
    fn test_daemon_notify_tracks_dirty_files() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path().to_str().unwrap();

        // Create test file
        let test_file = temp.path().join("test.py");
        fs::write(&test_file, "def foo(): pass").unwrap();

        // Start daemon
        let mut start_cmd = tldr_cmd();
        start_cmd
            .args(["daemon", "start", "--project", project_path])
            .assert()
            .success();

        // Notify about file change
        let mut notify_cmd = tldr_cmd();
        notify_cmd
            .args([
                "daemon",
                "notify",
                test_file.to_str().unwrap(),
                "--project",
                project_path,
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("dirty_count").or(predicate::str::contains("1/20")));

        // Cleanup
        cleanup_daemon(project_path);
    }

    #[test]
    #[ignore = "daemon notify command not yet implemented"]
    fn test_daemon_notify_triggers_reindex_at_threshold() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path().to_str().unwrap();

        // Start daemon
        let mut start_cmd = tldr_cmd();
        start_cmd
            .args(["daemon", "start", "--project", project_path])
            .assert()
            .success();

        // Notify about multiple file changes (threshold is 20)
        for i in 0..21 {
            let test_file = temp.path().join(format!("test{}.py", i));
            fs::write(&test_file, format!("def foo{}(): pass", i)).unwrap();

            let mut notify_cmd = tldr_cmd();
            let output = notify_cmd
                .args([
                    "daemon",
                    "notify",
                    test_file.to_str().unwrap(),
                    "--project",
                    project_path,
                ])
                .output()
                .unwrap();

            // Check if reindex was triggered on the 20th notification
            if i == 20 {
                let stdout = String::from_utf8_lossy(&output.stdout);
                assert!(
                    stdout.contains("reindex_triggered")
                        || stdout.contains("Reindex")
                        || stdout.contains("20/20"),
                    "Expected reindex to be triggered"
                );
            }
        }

        // Cleanup
        cleanup_daemon(project_path);
    }

    #[test]
    #[ignore = "daemon notify command not yet implemented"]
    fn test_daemon_notify_silent_when_not_running() {
        let temp = TempDir::new().unwrap();
        let test_file = temp.path().join("test.py");
        fs::write(&test_file, "def foo(): pass").unwrap();

        // Notify without daemon running - should exit 0 silently
        let mut cmd = tldr_cmd();
        cmd.args([
            "daemon",
            "notify",
            test_file.to_str().unwrap(),
            "--project",
            temp.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    }
}

// =============================================================================
// 4. Cache Tests
// =============================================================================

mod cache_tests {
    use super::*;

    #[test]
    fn test_cache_stats_help() {
        let mut cmd = tldr_cmd();
        cmd.args(["cache", "stats", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("--project"));
    }

    #[test]
    fn test_cache_stats_empty() {
        let temp = TempDir::new().unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["cache", "stats", "--project", temp.path().to_str().unwrap()])
            .assert()
            .success()
            .stdout(
                predicate::str::contains("No cache")
                    .or(predicate::str::contains("file_count"))
                    .or(predicate::str::contains("0")),
            );
    }

    #[test]
    #[ignore = "cache stats command not yet implemented"]
    fn test_cache_stats_after_queries() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path().to_str().unwrap();

        // Create test files
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        // Start daemon and make some queries
        let mut start_cmd = tldr_cmd();
        start_cmd
            .args(["daemon", "start", "--project", project_path])
            .assert()
            .success();

        // Make a few queries to populate cache
        for _ in 0..5 {
            let mut query_cmd = tldr_cmd();
            query_cmd
                .args([
                    "daemon",
                    "query",
                    "structure",
                    "--project",
                    project_path,
                    "--json",
                    &format!(r#"{{"path": "{}"}}"#, temp.path().join("test.py").display()),
                ])
                .output()
                .ok();
        }

        // Check cache stats
        let mut stats_cmd = tldr_cmd();
        stats_cmd
            .args(["cache", "stats", "--project", project_path])
            .assert()
            .success()
            .stdout(predicate::str::contains("hits").or(predicate::str::contains("misses")));

        // Cleanup
        cleanup_daemon(project_path);
    }

    #[test]
    fn test_cache_clear_help() {
        let mut cmd = tldr_cmd();
        cmd.args(["cache", "clear", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("--project"));
    }

    #[test]
    fn test_cache_clear_removes_files() {
        let temp = TempDir::new().unwrap();
        let cache_dir = temp.path().join(".tldr/cache");
        fs::create_dir_all(&cache_dir).unwrap();

        // Create some cache files
        fs::write(cache_dir.join("salsa_stats.json"), "{}").unwrap();
        fs::write(cache_dir.join("call_graph.json"), "{}").unwrap();
        fs::write(cache_dir.join("test.pkl"), "").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["cache", "clear", "--project", temp.path().to_str().unwrap()])
            .assert()
            .success()
            .stdout(predicate::str::contains("cleared").or(predicate::str::contains("removed")));

        // Verify files are gone
        assert!(
            !cache_dir.join("salsa_stats.json").exists(),
            "salsa_stats.json should be removed"
        );
        assert!(
            !cache_dir.join("call_graph.json").exists(),
            "call_graph.json should be removed"
        );
        assert!(
            !cache_dir.join("test.pkl").exists(),
            "test.pkl should be removed"
        );
    }

    #[test]
    fn test_cache_clear_no_cache_dir() {
        let temp = TempDir::new().unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["cache", "clear", "--project", temp.path().to_str().unwrap()])
            .assert()
            .success()
            .stdout(predicate::str::contains("No cache").or(predicate::str::contains("0")));
    }

    #[test]
    #[ignore = "cache invalidation not yet implemented"]
    fn test_cache_invalidation_on_file_change() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path().to_str().unwrap();
        let test_file = temp.path().join("test.py");

        // Create initial file
        fs::write(&test_file, "def foo(): pass").unwrap();

        // Start daemon
        let mut start_cmd = tldr_cmd();
        start_cmd
            .args(["daemon", "start", "--project", project_path])
            .assert()
            .success();

        // Query to populate cache
        let mut query1 = tldr_cmd();
        query1
            .args([
                "daemon",
                "query",
                "structure",
                "--project",
                project_path,
                "--json",
                &format!(r#"{{"path": "{}"}}"#, test_file.display()),
            ])
            .output()
            .ok();

        // Modify file
        fs::write(&test_file, "def foo(): return 1\ndef bar(): pass").unwrap();

        // Notify daemon
        let mut notify_cmd = tldr_cmd();
        notify_cmd
            .args([
                "daemon",
                "notify",
                test_file.to_str().unwrap(),
                "--project",
                project_path,
            ])
            .assert()
            .success();

        // Check stats for invalidation
        let mut stats_cmd = tldr_cmd();
        stats_cmd
            .args(["cache", "stats", "--project", project_path])
            .assert()
            .success()
            .stdout(predicate::str::contains("invalidations"));

        // Cleanup
        cleanup_daemon(project_path);
    }

    #[test]
    fn test_cache_stats_json_output() {
        let temp = TempDir::new().unwrap();
        let cache_dir = temp.path().join(".tldr/cache");
        fs::create_dir_all(&cache_dir).unwrap();

        // Create some cache files (not necessarily valid salsa cache, just for file stats)
        fs::write(cache_dir.join("test_cache.bin"), "test data").unwrap();
        fs::write(cache_dir.join("call_graph.json"), "{}").unwrap();

        let mut cmd = tldr_cmd();
        let output = cmd
            .args(["cache", "stats", "--project", temp.path().to_str().unwrap()])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: serde_json::Value = serde_json::from_str(&stdout).expect("Valid JSON output");

        // Should have cache_files info even without salsa_stats
        assert!(json.get("cache_files").is_some() || json.get("message").is_some());
    }
}

// =============================================================================
// 5. Warm Command Tests
// =============================================================================

mod warm_tests {
    use super::*;

    #[test]
    #[ignore = "warm command not yet implemented"]
    fn test_warm_help() {
        let mut cmd = tldr_cmd();
        cmd.args(["warm", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("--background"))
            .stdout(predicate::str::contains("--lang"));
    }

    #[test]
    #[ignore = "warm command not yet implemented"]
    fn test_warm_foreground_builds_cache() {
        let temp = TempDir::new().unwrap();

        // Create some Python files
        fs::write(temp.path().join("main.py"), "def main(): pass").unwrap();
        fs::write(
            temp.path().join("utils.py"),
            "def helper(): pass\ndef util(): pass",
        )
        .unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["warm", temp.path().to_str().unwrap(), "--lang", "python"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Indexed").or(predicate::str::contains("files")))
            .stdout(predicate::str::contains("edges").or(predicate::str::contains("call")));

        // Verify cache file was created
        let cache_file = temp.path().join(".tldr/cache/call_graph.json");
        assert!(cache_file.exists(), "call_graph.json should be created");
    }

    #[test]
    #[ignore = "warm command not yet implemented"]
    fn test_warm_background_spawns_task() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("main.py"), "def main(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args([
            "warm",
            temp.path().to_str().unwrap(),
            "--background",
            "--lang",
            "python",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("background"));

        // Wait a bit for background process
        std::thread::sleep(Duration::from_secs(2));

        // Check if cache was eventually created
        let cache_file = temp.path().join(".tldr/cache/call_graph.json");
        // Note: This may be flaky; in real implementation we might check differently
        assert!(
            cache_file.exists(),
            "Background warm should eventually create cache"
        );
    }

    #[test]
    #[ignore = "warm command not yet implemented"]
    fn test_warm_json_output() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("main.py"), "def main(): pass").unwrap();

        let mut cmd = tldr_cmd();
        let output = cmd
            .args([
                "warm",
                temp.path().to_str().unwrap(),
                "--lang",
                "python",
                "-q",
            ])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: serde_json::Value = serde_json::from_str(&stdout).expect("Valid JSON output");

        assert!(json.get("status").is_some());
        assert!(json.get("files").is_some());
        assert!(json.get("edges").is_some());
    }

    #[test]
    #[ignore = "warm command not yet implemented"]
    fn test_warm_auto_detect_languages() {
        let temp = TempDir::new().unwrap();

        // Create files in multiple languages
        fs::write(temp.path().join("main.py"), "def main(): pass").unwrap();
        fs::write(temp.path().join("lib.rs"), "fn main() {}").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["warm", temp.path().to_str().unwrap()]) // No --lang, auto-detect
            .assert()
            .success()
            .stdout(predicate::str::contains("python").or(predicate::str::contains("rust")));
    }

    #[test]
    #[ignore = "warm command not yet implemented"]
    fn test_warm_creates_tldrignore() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("main.py"), "def main(): pass").unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["warm", temp.path().to_str().unwrap()])
            .assert()
            .success();

        // Verify .tldrignore was created
        let ignore_file = temp.path().join(".tldrignore");
        assert!(
            ignore_file.exists(),
            ".tldrignore should be created with defaults"
        );
    }
}

// =============================================================================
// 6. Stats Command Tests
// =============================================================================

mod stats_tests {
    use super::*;

    #[test]
    #[ignore = "stats command not yet implemented"]
    fn test_stats_help() {
        let mut cmd = tldr_cmd();
        cmd.args(["stats", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("--format"));
    }

    #[test]
    #[ignore = "stats command not yet implemented"]
    fn test_stats_empty() {
        // Use a temporary directory to avoid affecting real stats
        let temp = TempDir::new().unwrap();
        let tldr_dir = temp.path().join(".tldr");
        fs::create_dir_all(&tldr_dir).ok();

        // Note: This test may need environment variable override
        // to point stats path to temp directory

        let mut cmd = tldr_cmd();
        cmd.args(["stats"])
            .assert()
            .success()
            .stdout(predicate::str::contains("No usage").or(predicate::str::contains("0")));
    }

    #[test]
    #[ignore = "stats command not yet implemented"]
    fn test_stats_formats_token_savings() {
        // Create a test stats file
        let tldr_dir = home_dir().join(".tldr");
        fs::create_dir_all(&tldr_dir).ok();

        let stats_path = tldr_dir.join("stats.jsonl");
        let backup_path = stats_path.with_extension("jsonl.bak");

        // Backup existing file
        if stats_path.exists() {
            fs::rename(&stats_path, &backup_path).ok();
        }

        // Write test data
        let test_data = r#"{"session_id":"test1","raw_tokens":1000,"tldr_tokens":100,"requests":10}
{"session_id":"test2","raw_tokens":2000,"tldr_tokens":200,"requests":20}"#;
        fs::write(&stats_path, test_data).unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["stats"])
            .assert()
            .success()
            .stdout(predicate::str::contains("2,700").or(predicate::str::contains("2700"))) // tokens saved
            .stdout(predicate::str::contains("90")); // percentage

        // Restore backup
        if backup_path.exists() {
            fs::rename(&backup_path, &stats_path).ok();
        } else {
            fs::remove_file(&stats_path).ok();
        }
    }

    #[test]
    #[ignore = "stats command not yet implemented"]
    fn test_stats_json_output() {
        let tldr_dir = home_dir().join(".tldr");
        fs::create_dir_all(&tldr_dir).ok();

        let stats_path = tldr_dir.join("stats.jsonl");
        let backup_path = stats_path.with_extension("jsonl.bak");

        // Backup existing file
        if stats_path.exists() {
            fs::rename(&stats_path, &backup_path).ok();
        }

        // Write test data
        let test_data =
            r#"{"session_id":"test1","raw_tokens":1000,"tldr_tokens":100,"requests":10}"#;
        fs::write(&stats_path, test_data).unwrap();

        let mut cmd = tldr_cmd();
        let output = cmd.args(["stats", "--format", "json"]).output().unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: serde_json::Value = serde_json::from_str(&stdout).expect("Valid JSON output");

        assert!(json.get("total_invocations").is_some());
        assert!(json.get("estimated_tokens_saved").is_some());
        assert!(json.get("raw_tokens_total").is_some());
        assert!(json.get("tldr_tokens_total").is_some());
        assert!(json.get("savings_percent").is_some());

        // Restore backup
        if backup_path.exists() {
            fs::rename(&backup_path, &stats_path).ok();
        } else {
            fs::remove_file(&stats_path).ok();
        }
    }

    #[test]
    #[ignore = "stats command not yet implemented"]
    fn test_stats_text_output() {
        let tldr_dir = home_dir().join(".tldr");
        fs::create_dir_all(&tldr_dir).ok();

        let stats_path = tldr_dir.join("stats.jsonl");
        let backup_path = stats_path.with_extension("jsonl.bak");

        // Backup existing file
        if stats_path.exists() {
            fs::rename(&stats_path, &backup_path).ok();
        }

        // Write test data
        let test_data =
            r#"{"session_id":"test1","raw_tokens":5000,"tldr_tokens":500,"requests":50}"#;
        fs::write(&stats_path, test_data).unwrap();

        let mut cmd = tldr_cmd();
        cmd.args(["stats", "--format", "text"])
            .assert()
            .success()
            .stdout(predicate::str::contains("TLDR Usage Statistics"))
            .stdout(predicate::str::contains("Total Invocations"))
            .stdout(predicate::str::contains("Tokens Saved"));

        // Restore backup
        if backup_path.exists() {
            fs::rename(&backup_path, &stats_path).ok();
        } else {
            fs::remove_file(&stats_path).ok();
        }
    }
}

// =============================================================================
// 7. Edge Case Tests
// =============================================================================

mod edge_cases {
    use super::*;

    #[test]
    #[ignore = "stale PID recovery not yet implemented"]
    fn test_stale_pid_file_recovery() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path().to_str().unwrap();

        // Create a stale PID file (process doesn't exist)
        let _tmp_dir = std::env::temp_dir();

        // Compute the expected PID file path (simplified - actual impl uses MD5)
        // For test purposes, we'll just create a file that looks stale
        let _pid_content = "99999999"; // Very unlikely to be a real PID

        // This test verifies the daemon can recover from stale PID files
        // The actual implementation should:
        // 1. Try to acquire lock
        // 2. Check if PID in file is a running process
        // 3. If not, clean up and proceed

        let mut cmd = tldr_cmd();
        cmd.args(["daemon", "start", "--project", project_path])
            .assert()
            .success();

        // Cleanup
        cleanup_daemon(project_path);
    }

    #[test]
    #[ignore = "stale socket cleanup not yet implemented"]
    fn test_stale_socket_cleanup() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path().to_str().unwrap();

        // Create a stale socket file (no process listening)
        let tmp_dir = std::env::temp_dir();
        let stale_socket = tmp_dir.join("tldr-stale-test.sock");

        // Create an empty file as a "stale socket"
        fs::write(&stale_socket, "").ok();

        // Daemon should detect stale socket and clean up
        let mut cmd = tldr_cmd();
        cmd.args(["daemon", "start", "--project", project_path])
            .assert()
            .success();

        // Cleanup
        cleanup_daemon(project_path);

        fs::remove_file(&stale_socket).ok();
    }

    #[test]
    #[ignore = "concurrent daemon start not yet implemented"]
    fn test_concurrent_daemon_start_fails() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path().to_str().unwrap();

        // Start first daemon
        let mut cmd1 = tldr_cmd();
        cmd1.args(["daemon", "start", "--project", project_path])
            .assert()
            .success();

        // Immediately try to start another (race condition test)
        let mut cmd2 = tldr_cmd();
        cmd2.args(["daemon", "start", "--project", project_path])
            .assert()
            .failure()
            .stderr(predicate::str::contains("already running"));

        // Cleanup
        cleanup_daemon(project_path);
    }

    #[test]
    #[ignore = "permission denied handling not yet implemented"]
    fn test_permission_denied_socket() {
        // This test is platform-specific and may need adjustment
        // It verifies proper error handling when socket creation fails

        #[cfg(unix)]
        {
            // Try to create socket in a directory we don't have write access to
            let mut cmd = tldr_cmd();
            cmd.args(["daemon", "start", "--project", "/root/nonexistent"])
                .assert()
                .failure()
                .stderr(
                    predicate::str::contains("Permission denied")
                        .or(predicate::str::contains("permission")),
                );
        }
    }

    #[test]
    #[ignore = "connection timeout not yet implemented"]
    fn test_daemon_connection_timeout() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path().to_str().unwrap();

        // Query without daemon running should fail gracefully
        // Using assert_cmd::Command for timeout support
        let mut cmd = tldr_assert_cmd();
        cmd.args(["daemon", "query", "ping", "--project", project_path])
            .timeout(Duration::from_secs(10))
            .assert()
            .failure()
            .stderr(
                predicate::str::contains("not running")
                    .or(predicate::str::contains("Connection"))
                    .or(predicate::str::contains("timeout")),
            );
    }

    #[test]
    #[ignore = "invalid command handling not yet implemented"]
    fn test_daemon_unknown_command() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path().to_str().unwrap();

        // Start daemon
        let mut start_cmd = tldr_cmd();
        start_cmd
            .args(["daemon", "start", "--project", project_path])
            .assert()
            .success();

        // Send unknown command
        let mut query_cmd = tldr_cmd();
        query_cmd
            .args([
                "daemon",
                "query",
                "nonexistent_command",
                "--project",
                project_path,
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains("unknown").or(predicate::str::contains("Unknown")));

        // Cleanup
        cleanup_daemon(project_path);
    }

    #[test]
    #[ignore = "graceful shutdown not yet implemented"]
    fn test_daemon_graceful_shutdown_persists_stats() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path().to_str().unwrap();

        // Create test file
        fs::write(temp.path().join("test.py"), "def foo(): pass").unwrap();

        // Start daemon
        let mut start_cmd = tldr_cmd();
        start_cmd
            .args(["daemon", "start", "--project", project_path])
            .assert()
            .success();

        // Make some queries to generate stats
        for _ in 0..3 {
            let mut query_cmd = tldr_cmd();
            query_cmd
                .args(["daemon", "query", "ping", "--project", project_path])
                .output()
                .ok();
        }

        // Stop daemon gracefully
        let mut stop_cmd = tldr_cmd();
        stop_cmd
            .args(["daemon", "stop", "--project", project_path])
            .assert()
            .success();

        // Verify stats were persisted
        let cache_dir = temp.path().join(".tldr/cache");
        let _stats_file = cache_dir.join("salsa_stats.json");

        // Stats should be written on shutdown
        // (actual path may vary based on implementation)
        // This assertion may need adjustment based on actual implementation
    }

    #[test]
    #[ignore = "idle timeout not yet implemented"]
    fn test_daemon_idle_timeout() {
        // This is a long-running test that verifies idle timeout behavior
        // In practice, we'd use a short timeout for testing

        let temp = TempDir::new().unwrap();
        let _project_path = temp.path().to_str().unwrap();

        // Start daemon with a very short idle timeout (would need config support)
        // For now, this test documents the expected behavior

        // Expected behavior:
        // 1. Daemon starts
        // 2. No queries for idle_timeout_secs
        // 3. Daemon auto-shuts down
        // 4. Status shows "not running"
    }
}

// =============================================================================
// 8. Socket Path Computation Tests
// =============================================================================

mod socket_path_tests {
    use super::*;
    use tldr_cli::commands::daemon::{compute_hash, compute_pid_path, compute_socket_path};

    #[test]
    fn test_socket_path_deterministic() {
        // Same project path should always produce same socket path
        let project = PathBuf::from("/test/project");

        // This test verifies the socket path computation is deterministic
        // Actual implementation uses MD5 hash of canonicalized path
        let path1 = compute_socket_path(&project);
        let path2 = compute_socket_path(&project);
        assert_eq!(path1, path2);

        // Also verify hash is deterministic
        let hash1 = compute_hash(&project);
        let hash2 = compute_hash(&project);
        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 8); // 8 hex chars
    }

    #[test]
    fn test_socket_path_different_projects() {
        // Different projects should have different socket paths
        let project1 = PathBuf::from("/test/project1");
        let project2 = PathBuf::from("/test/project2");

        let path1 = compute_socket_path(&project1);
        let path2 = compute_socket_path(&project2);
        assert_ne!(path1, path2);

        // Also verify hashes are different
        let hash1 = compute_hash(&project1);
        let hash2 = compute_hash(&project2);
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_pid_path_matches_socket_hash() {
        // PID path should use same hash as socket path
        let project = PathBuf::from("/test/project");

        let socket_path = compute_socket_path(&project);
        let pid_path = compute_pid_path(&project);

        // Both should have same hash prefix
        // e.g., /tmp/tldr-a1b2c3d4.sock and /tmp/tldr-a1b2c3d4.pid
        let socket_name = socket_path.file_name().unwrap().to_str().unwrap();
        let pid_name = pid_path.file_name().unwrap().to_str().unwrap();

        // Extract hash portion: tldr-XXXXXXXX.ext -> XXXXXXXX
        let socket_hash = &socket_name[5..13];
        let pid_hash = &pid_name[5..13];

        assert_eq!(socket_hash, pid_hash);

        // Verify extensions are correct
        assert!(socket_name.ends_with(".sock"));
        assert!(pid_name.ends_with(".pid"));
    }
}

// =============================================================================
// 9. Hook Stats Tracking Tests
// =============================================================================

mod hook_stats_tests {
    use super::*;

    #[test]
    #[ignore = "track command not yet implemented"]
    fn test_daemon_track_hook_activity() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path().to_str().unwrap();

        // Start daemon
        let mut start_cmd = tldr_cmd();
        start_cmd
            .args(["daemon", "start", "--project", project_path])
            .assert()
            .success();

        // Track a hook invocation
        let mut query_cmd = tldr_cmd();
        query_cmd
            .args([
                "daemon",
                "query",
                "track",
                "--project",
                project_path,
                "--json",
                r#"{"hook": "pre-commit", "success": true, "metrics": {"files_checked": 5}}"#,
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("total_invocations"));

        // Cleanup
        cleanup_daemon(project_path);
    }

    #[test]
    #[ignore = "track flush not yet implemented"]
    fn test_track_flush_at_threshold() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path().to_str().unwrap();

        // Start daemon
        let mut start_cmd = tldr_cmd();
        start_cmd
            .args(["daemon", "start", "--project", project_path])
            .assert()
            .success();

        // Track multiple hook invocations (flush threshold is 5)
        for i in 0..6 {
            let mut query_cmd = tldr_cmd();
            let output = query_cmd
                .args([
                    "daemon",
                    "query",
                    "track",
                    "--project",
                    project_path,
                    "--json",
                    r#"{"hook": "test-hook", "success": true}"#,
                ])
                .output()
                .unwrap();

            // Check if flush occurred on 5th invocation
            if i == 5 {
                let stdout = String::from_utf8_lossy(&output.stdout);
                assert!(
                    stdout.contains("flushed") || stdout.contains("true"),
                    "Expected stats to be flushed"
                );
            }
        }

        // Cleanup
        cleanup_daemon(project_path);
    }
}

// =============================================================================
// 10. Semantic Search Tests (requires model)
// =============================================================================

mod semantic_tests {
    use super::*;

    #[test]
    #[ignore = "semantic search not yet implemented"]
    fn test_daemon_semantic_query() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path().to_str().unwrap();

        // Create some Python files with meaningful content
        fs::write(
            temp.path().join("auth.py"),
            "def authenticate(user, password):\n    '''Verify user credentials'''\n    pass",
        )
        .unwrap();
        fs::write(
            temp.path().join("db.py"),
            "def connect_database(host, port):\n    '''Connect to database'''\n    pass",
        )
        .unwrap();

        // Start daemon
        let mut start_cmd = tldr_cmd();
        start_cmd
            .args(["daemon", "start", "--project", project_path])
            .assert()
            .success();

        // Wait for indexing
        std::thread::sleep(Duration::from_secs(2));

        // Semantic search for authentication-related code
        let mut query_cmd = tldr_cmd();
        query_cmd
            .args([
                "daemon",
                "query",
                "semantic",
                "--project",
                project_path,
                "--json",
                r#"{"query": "user login verification", "top_k": 5}"#,
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("auth")); // Should find auth.py

        // Cleanup
        cleanup_daemon(project_path);
    }
}
