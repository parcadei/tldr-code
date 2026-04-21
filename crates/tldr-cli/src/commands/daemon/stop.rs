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

use super::error::DaemonError;
use super::ipc::{check_socket_alive, cleanup_socket, send_command};
use super::pid::{cleanup_stale_pid, compute_pid_path};
use super::types::DaemonCommand;

// =============================================================================
// CLI Arguments
// =============================================================================

/// Arguments for the `daemon stop` command.
#[derive(Debug, Clone, Args)]
pub struct DaemonStopArgs {
    /// Project root directory (default: current directory)
    #[arg(long, short = 'p', default_value = ".")]
    pub project: PathBuf,
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
        // Resolve project path to absolute
        let project = self.project.canonicalize().unwrap_or_else(|_| {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(&self.project)
        });

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

            // Clean up any stale files
            let pid_path = compute_pid_path(&project);
            let _ = cleanup_stale_pid(&pid_path);
            let _ = cleanup_socket(&project);

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

                // Clean up files
                let _ = cleanup_socket(&project);
                let pid_path = compute_pid_path(&project);
                let _ = cleanup_stale_pid(&pid_path);

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

                // Clean up any stale files
                let _ = cleanup_socket(&project);
                let pid_path = compute_pid_path(&project);
                let _ = cleanup_stale_pid(&pid_path);

                Ok(())
            }
            Err(e) => Err(anyhow::anyhow!("Failed to stop daemon: {}", e)),
        }
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
        };

        assert_eq!(args.project, PathBuf::from("."));
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
        };

        // Should succeed when daemon is not running
        let result = args.run_async(OutputFormat::Json, true).await;
        assert!(result.is_ok());
    }
}
