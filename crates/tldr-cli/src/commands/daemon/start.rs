//! Daemon start command implementation
//!
//! CLI command: `tldr daemon start [--project PATH] [--foreground]`
//!
//! This module handles starting the TLDR daemon process with:
//! - PID file locking to ensure single instance per project
//! - Daemonization (background mode) or foreground mode
//! - Socket binding for IPC communication
//!
//! # Security Mitigations
//!
//! - TIGER-P1-01: Exclusive file lock on PID file prevents race conditions
//! - TIGER-P2-02: Stale socket cleanup on startup

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::Args;
use serde::Serialize;

use crate::output::OutputFormat;

use super::daemon::{start_daemon_background, wait_for_daemon, TLDRDaemon};
use super::error::DaemonError;
use super::ipc::{check_socket_alive, cleanup_socket, compute_socket_path, IpcListener};
use super::pid::{check_stale_pid, cleanup_stale_pid, compute_pid_path, try_acquire_lock};
use super::types::DaemonConfig;

// =============================================================================
// CLI Arguments
// =============================================================================

/// Arguments for the `daemon start` command.
#[derive(Debug, Clone, Args)]
pub struct DaemonStartArgs {
    /// Project root directory (default: current directory)
    #[arg(long, short = 'p', default_value = ".")]
    pub project: PathBuf,

    /// Run daemon in foreground (don't daemonize)
    #[arg(long)]
    pub foreground: bool,
}

// =============================================================================
// Output Types
// =============================================================================

/// Output structure for successful daemon start.
#[derive(Debug, Clone, Serialize)]
pub struct DaemonStartOutput {
    /// Status message
    pub status: String,
    /// PID of the daemon process
    pub pid: u32,
    /// Path to the socket file
    pub socket: PathBuf,
    /// Optional message
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

// =============================================================================
// Command Implementation
// =============================================================================

impl DaemonStartArgs {
    /// Run the daemon start command.
    pub fn run(&self, format: OutputFormat, quiet: bool) -> anyhow::Result<()> {
        // Create a new tokio runtime for the async operations
        let runtime = tokio::runtime::Runtime::new()?;
        runtime.block_on(self.run_async(format, quiet))
    }

    /// Async implementation of the daemon start command.
    async fn run_async(&self, format: OutputFormat, quiet: bool) -> anyhow::Result<()> {
        // Resolve project path to absolute
        let project = self.project.canonicalize().unwrap_or_else(|_| {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(&self.project)
        });

        // Check for stale PID file and clean up
        let pid_path = compute_pid_path(&project);
        if check_stale_pid(&pid_path)? {
            cleanup_stale_pid(&pid_path)?;
        }

        // Check for stale socket and clean up
        let socket_path = compute_socket_path(&project);
        if socket_path.exists() && !check_socket_alive(&project).await {
            // Socket exists but daemon is not responding - stale
            cleanup_socket(&project)?;
        }

        if self.foreground {
            // Run in foreground
            self.run_foreground(&project, format, quiet).await
        } else {
            // Run in background
            self.run_background(&project, format, quiet).await
        }
    }

    /// Run the daemon in foreground mode.
    async fn run_foreground(
        &self,
        project: &Path,
        format: OutputFormat,
        quiet: bool,
    ) -> anyhow::Result<()> {
        // Try to acquire PID lock
        let pid_path = compute_pid_path(project);
        let _pid_guard = try_acquire_lock(&pid_path).map_err(|e| match e {
            DaemonError::AlreadyRunning { pid } => {
                anyhow::anyhow!("Daemon already running (PID: {})", pid)
            }
            DaemonError::StalePidFile { pid } => {
                anyhow::anyhow!("Stale PID file (process {} not running)", pid)
            }
            other => anyhow::anyhow!("Failed to acquire lock: {}", other),
        })?;

        // Bind IPC listener
        let listener = IpcListener::bind(project).await.map_err(|e| match e {
            DaemonError::AddressInUse { addr } => {
                anyhow::anyhow!("Address already in use: {}", addr)
            }
            DaemonError::SocketBindFailed(io_err) => {
                anyhow::anyhow!("Failed to bind socket: {}", io_err)
            }
            other => anyhow::anyhow!("Socket error: {}", other),
        })?;

        let socket_path = compute_socket_path(project);
        let our_pid = std::process::id();

        // Print startup message
        let output = DaemonStartOutput {
            status: "ok".to_string(),
            pid: our_pid,
            socket: socket_path.clone(),
            message: Some("Daemon started in foreground".to_string()),
        };

        if !quiet {
            match format {
                OutputFormat::Json | OutputFormat::Compact => {
                    println!("{}", serde_json::to_string_pretty(&output)?);
                }
                OutputFormat::Text | OutputFormat::Sarif | OutputFormat::Dot => {
                    println!("Daemon started with PID {}", our_pid);
                    println!("Socket: {}", socket_path.display());
                }
            }
        }

        // Create and run daemon
        let config = DaemonConfig::default();
        let daemon = Arc::new(TLDRDaemon::new(project.to_path_buf(), config));
        daemon.run(listener).await?;

        // Cleanup socket on exit
        let _ = cleanup_socket(project);

        Ok(())
    }

    /// Run the daemon in background mode.
    async fn run_background(
        &self,
        project: &Path,
        format: OutputFormat,
        quiet: bool,
    ) -> anyhow::Result<()> {
        // First check if daemon is already running
        if check_socket_alive(project).await {
            // Try to get PID from PID file
            let pid_path = compute_pid_path(project);
            let pid = std::fs::read_to_string(&pid_path)
                .ok()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0);

            return Err(anyhow::anyhow!("Daemon already running (PID: {})", pid));
        }

        // Start the daemon in background
        let pid = start_daemon_background(project).await?;

        // Wait for daemon to become ready
        wait_for_daemon(project, 10)
            .await
            .map_err(|_| anyhow::anyhow!("Daemon failed to start within timeout"))?;

        let socket_path = compute_socket_path(project);

        // Print output
        let output = DaemonStartOutput {
            status: "ok".to_string(),
            pid,
            socket: socket_path.clone(),
            message: Some("Daemon started".to_string()),
        };

        if !quiet {
            match format {
                OutputFormat::Json | OutputFormat::Compact => {
                    println!("{}", serde_json::to_string_pretty(&output)?);
                }
                OutputFormat::Text | OutputFormat::Sarif | OutputFormat::Dot => {
                    println!("Daemon started with PID {}", pid);
                    println!("Socket: {}", socket_path.display());
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
    

    #[test]
    fn test_daemon_start_args_default() {
        let args = DaemonStartArgs {
            project: PathBuf::from("."),
            foreground: false,
        };

        assert_eq!(args.project, PathBuf::from("."));
        assert!(!args.foreground);
    }

    #[test]
    fn test_daemon_start_args_foreground() {
        let args = DaemonStartArgs {
            project: PathBuf::from("/test/project"),
            foreground: true,
        };

        assert!(args.foreground);
    }

    #[test]
    fn test_daemon_start_output_serialization() {
        let output = DaemonStartOutput {
            status: "ok".to_string(),
            pid: 12345,
            socket: PathBuf::from("/tmp/tldr-abc123.sock"),
            message: Some("Daemon started".to_string()),
        };

        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("ok"));
        assert!(json.contains("12345"));
        assert!(json.contains("tldr-abc123.sock"));
    }
}
