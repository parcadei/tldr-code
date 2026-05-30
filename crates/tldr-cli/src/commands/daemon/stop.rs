//! Daemon stop command implementation
//!
//! CLI command: `tldr daemon stop [--project PATH]`
//!
//! This module handles stopping the TLDR daemon gracefully by:
//! - Connecting to the daemon via IPC
//! - Sending a shutdown command
//! - Waiting for the daemon to exit
//! - Cleaning up socket and PID files

use std::path::PathBuf;

use clap::Args;
use serde::Serialize;

use crate::output::OutputFormat;

use super::daemon_active::remove_active;
use super::daemon_registry::{live_entries, remove_entry};
use super::error::DaemonError;
use super::ipc::{check_socket_alive, cleanup_socket_at, send_command, snapshot_socket_path};
use super::pid::{cleanup_stale_pid, compute_pid_path};
use super::types::DaemonCommand;

// =============================================================================
// CLI Arguments
// =============================================================================

/// Arguments for the `daemon stop` command.
#[derive(Debug, Clone, Args)]
pub struct DaemonStopArgs {
    /// Project root directory (default: current directory).
    ///
    /// Mutually exclusive with `--all`. When neither is set, the command
    /// targets the daemon for the current working directory.
    #[arg(long, short = 'p', default_value = ".")]
    pub project: PathBuf,

    /// Stop ALL running daemons known to the v0.3.0 multi-daemon registry.
    ///
    /// Mutually exclusive with `--project`. Iterates the registry, sends
    /// shutdown to each daemon, and removes the entry on success.
    #[arg(long, conflicts_with = "project")]
    pub all: bool,
}

// =============================================================================
// Output Types
// =============================================================================

/// Output structure for daemon stop result.
#[derive(Debug, Clone, Serialize)]
pub struct DaemonStopOutput {
    /// Status message
    pub status: String,
    /// Optional message
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

// =============================================================================
// Command Implementation
// =============================================================================

impl DaemonStopArgs {
    /// Run the daemon stop command.
    pub fn run(&self, format: OutputFormat, quiet: bool) -> anyhow::Result<()> {
        // Create a new tokio runtime for the async operations
        let runtime = tokio::runtime::Runtime::new()?;
        runtime.block_on(self.run_async(format, quiet))
    }

    /// Async implementation of the daemon stop command.
    async fn run_async(&self, format: OutputFormat, quiet: bool) -> anyhow::Result<()> {
        // --all: iterate the v0.3.0 registry and stop each known daemon.
        if self.all {
            return self.run_stop_all(format, quiet).await;
        }

        // Resolve project path to absolute
        let project = self.project.canonicalize().unwrap_or_else(|_| {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(&self.project)
        });

        // Snapshot the socket path from the unpruned registry BEFORE any
        // operation that triggers read_registry() (which prunes dead PIDs and
        // writes back). Without this, check_socket_alive -> connect ->
        // find_entry -> read_registry() drops the dead entry, and the later
        // cleanup_socket -> find_entry_unpruned finds nothing — orphaning a
        // cross-TMPDIR socket (W6/W3).
        let socket_snapshot = snapshot_socket_path(&project);

        // Check if daemon is running
        if !check_socket_alive(&project).await {
            // Daemon not running
            let output = DaemonStopOutput {
                status: "ok".to_string(),
                message: Some("Daemon not running".to_string()),
            };

            if !quiet {
                match format {
                    OutputFormat::Json | OutputFormat::Compact => {
                        println!("{}", serde_json::to_string_pretty(&output)?);
                    }
                    OutputFormat::Text | OutputFormat::Sarif | OutputFormat::Dot => {
                        println!("Daemon not running");
                    }
                }
            }

            // Clean up any stale files (legacy daemon-active.json + v0.3.0
            // registry entry).
            let pid_path = compute_pid_path(&project);
            let _ = cleanup_stale_pid(&pid_path);
            let _ = cleanup_socket_at(&socket_snapshot);
            let _ = remove_active();
            let _ = remove_entry(&project);

            return Ok(());
        }

        // Send shutdown command
        let cmd = DaemonCommand::Shutdown;
        match send_command(&project, &cmd).await {
            Ok(_response) => {
                // Wait for daemon to actually stop
                let mut retries = 0;
                while retries < 50 {
                    // 5 seconds max
                    if !check_socket_alive(&project).await {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    retries += 1;
                }

                // Clean up files (legacy daemon-active.json + v0.3.0 entry).
                let _ = cleanup_socket_at(&socket_snapshot);
                let pid_path = compute_pid_path(&project);
                let _ = cleanup_stale_pid(&pid_path);
                let _ = remove_active();
                let _ = remove_entry(&project);

                let output = DaemonStopOutput {
                    status: "ok".to_string(),
                    message: Some("Daemon stopped".to_string()),
                };

                if !quiet {
                    match format {
                        OutputFormat::Json | OutputFormat::Compact => {
                            println!("{}", serde_json::to_string_pretty(&output)?);
                        }
                        OutputFormat::Text | OutputFormat::Sarif | OutputFormat::Dot => {
                            println!("Daemon stopped");
                        }
                    }
                }

                Ok(())
            }
            Err(DaemonError::NotRunning) | Err(DaemonError::ConnectionRefused) => {
                // Daemon already stopped or not responding
                let output = DaemonStopOutput {
                    status: "ok".to_string(),
                    message: Some("Daemon not running".to_string()),
                };

                if !quiet {
                    match format {
                        OutputFormat::Json | OutputFormat::Compact => {
                            println!("{}", serde_json::to_string_pretty(&output)?);
                        }
                        OutputFormat::Text | OutputFormat::Sarif | OutputFormat::Dot => {
                            println!("Daemon not running");
                        }
                    }
                }

                // Clean up any stale files (legacy daemon-active.json +
                // v0.3.0 registry entry).
                let _ = cleanup_socket_at(&socket_snapshot);
                let pid_path = compute_pid_path(&project);
                let _ = cleanup_stale_pid(&pid_path);
                let _ = remove_active();
                let _ = remove_entry(&project);

                Ok(())
            }
            Err(e) => Err(anyhow::anyhow!("Failed to stop daemon: {}", e)),
        }
    }

    /// Implements `daemon stop --all`. Iterates the v0.3.0 registry and
    /// sends shutdown to each known daemon. Best-effort: a failure on one
    /// entry does not abort the iteration; failures are reported in the
    /// final output but the command exits 0 if at least the registry is
    /// reachable.
    async fn run_stop_all(&self, format: OutputFormat, quiet: bool) -> anyhow::Result<()> {
        let entries = live_entries();

        if entries.is_empty() {
            let output = DaemonStopOutput {
                status: "ok".to_string(),
                message: Some("No daemons running".to_string()),
            };
            if !quiet {
                match format {
                    OutputFormat::Json | OutputFormat::Compact => {
                        println!("{}", serde_json::to_string_pretty(&output)?);
                    }
                    OutputFormat::Text | OutputFormat::Sarif | OutputFormat::Dot => {
                        println!("No daemons running");
                    }
                }
            }
            // Defensive: ensure no legacy daemon-active.json lingers.
            let _ = remove_active();
            return Ok(());
        }

        let mut stopped = 0usize;
        let mut failed = 0usize;
        for entry in &entries {
            let project = &entry.project;
            let socket = snapshot_socket_path(project);
            let cmd = DaemonCommand::Shutdown;
            match send_command(project, &cmd).await {
                Ok(_response) => {
                    let mut retries = 0;
                    while retries < 50 {
                        if !check_socket_alive(project).await {
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        retries += 1;
                    }
                    let _ = cleanup_socket_at(&socket);
                    let pid_path = compute_pid_path(project);
                    let _ = cleanup_stale_pid(&pid_path);
                    let _ = remove_entry(project);
                    stopped += 1;
                }
                Err(DaemonError::NotRunning) | Err(DaemonError::ConnectionRefused) => {
                    let _ = cleanup_socket_at(&socket);
                    let pid_path = compute_pid_path(project);
                    let _ = cleanup_stale_pid(&pid_path);
                    let _ = remove_entry(project);
                    stopped += 1;
                }
                Err(_) => {
                    failed += 1;
                }
            }
        }
        // Defensive: drop any legacy single-slot record.
        let _ = remove_active();

        let summary = if failed == 0 {
            format!("Stopped {} daemon(s)", stopped)
        } else {
            format!("Stopped {} daemon(s); {} failed", stopped, failed)
        };
        let output = DaemonStopOutput {
            status: "ok".to_string(),
            message: Some(summary.clone()),
        };
        if !quiet {
            match format {
                OutputFormat::Json | OutputFormat::Compact => {
                    println!("{}", serde_json::to_string_pretty(&output)?);
                }
                OutputFormat::Text | OutputFormat::Sarif | OutputFormat::Dot => {
                    println!("{}", summary);
                }
            }
        }
        Ok(())
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_daemon_stop_args_default() {
        let args = DaemonStopArgs {
            project: PathBuf::from("."),
            all: false,
        };

        assert_eq!(args.project, PathBuf::from("."));
        assert!(!args.all);
    }

    #[test]
    fn test_daemon_stop_output_serialization() {
        let output = DaemonStopOutput {
            status: "ok".to_string(),
            message: Some("Daemon stopped".to_string()),
        };

        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("ok"));
        assert!(json.contains("Daemon stopped"));
    }

    #[test]
    fn test_daemon_stop_output_not_running() {
        let output = DaemonStopOutput {
            status: "ok".to_string(),
            message: Some("Daemon not running".to_string()),
        };

        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("ok"));
        assert!(json.contains("not running"));
    }

    #[tokio::test]
    async fn test_daemon_stop_not_running() {
        let temp = TempDir::new().unwrap();
        let args = DaemonStopArgs {
            project: temp.path().to_path_buf(),
            all: false,
        };

        // Should succeed when daemon is not running
        let result = args.run_async(OutputFormat::Json, true).await;
        assert!(result.is_ok());
    }
}
