//! Daemon notify command implementation
//!
//! CLI command: `tldr daemon notify FILE [--project PATH]`
//!
//! This module provides file change notifications to the daemon for:
//! - Cache invalidation
//! - Dirty file tracking
//! - Automatic re-indexing when threshold is reached
//!
//! # Security Mitigations
//!
//! - TIGER-P3-03: Validates file path is within project root
//! - TIGER-P3-05: Rate limiting handled in daemon (client just sends)
//!
//! # Use Case
//!
//! Editor hooks call this on file save to keep daemon cache fresh.

use std::path::PathBuf;

use clap::Args;
use serde::Serialize;

use crate::output::OutputFormat;

use super::error::{DaemonError, DaemonResult};
use super::ipc::send_command;
use super::types::{DaemonCommand, DaemonResponse};

// =============================================================================
// CLI Arguments
// =============================================================================

/// Arguments for the `daemon notify` command.
#[derive(Debug, Clone, Args)]
pub struct DaemonNotifyArgs {
    /// Path to the changed file
    pub file: PathBuf,

    /// Project root directory (default: current directory)
    #[arg(long, short = 'p', default_value = ".")]
    pub project: PathBuf,
}

// =============================================================================
// Output Types
// =============================================================================

/// Output structure for successful notify response.
#[derive(Debug, Clone, Serialize)]
pub struct DaemonNotifyOutput {
    /// Status (always "ok")
    pub status: String,
    /// Number of dirty files tracked
    pub dirty_count: usize,
    /// Threshold for triggering re-index
    pub threshold: usize,
    /// Whether re-index was triggered
    pub reindex_triggered: bool,
    /// Optional message
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Output structure for notify errors.
#[derive(Debug, Clone, Serialize)]
pub struct DaemonNotifyErrorOutput {
    /// Status (always "error")
    pub status: String,
    /// Error message
    pub error: String,
}

// =============================================================================
// Command Implementation
// =============================================================================

impl DaemonNotifyArgs {
    /// Run the daemon notify command.
    pub fn run(&self, format: OutputFormat, quiet: bool) -> anyhow::Result<()> {
        // Create a new tokio runtime for the async operations
        let runtime = tokio::runtime::Runtime::new()?;
        runtime.block_on(self.run_async(format, quiet))
    }

    /// Async implementation of the daemon notify command.
    async fn run_async(&self, format: OutputFormat, quiet: bool) -> anyhow::Result<()> {
        // Resolve project path to absolute
        let project = self.project.canonicalize().unwrap_or_else(|_| {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(&self.project)
        });

        // Resolve file path to absolute
        let file = self.file.canonicalize().unwrap_or_else(|_| {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(&self.file)
        });

        // TIGER-P3-03: Validate file path is within project root
        if !file.starts_with(&project) {
            let output = DaemonNotifyErrorOutput {
                status: "error".to_string(),
                error: format!(
                    "File '{}' is outside project root '{}'",
                    file.display(),
                    project.display()
                ),
            };

            if !quiet {
                match format {
                    OutputFormat::Json | OutputFormat::Compact => {
                        println!("{}", serde_json::to_string_pretty(&output)?);
                    }
                    OutputFormat::Text | OutputFormat::Sarif | OutputFormat::Dot => {
                        eprintln!("Error: File '{}' is outside project root", file.display());
                    }
                }
            }

            return Err(anyhow::anyhow!("File is outside project root"));
        }

        // Build notify command
        let cmd = DaemonCommand::Notify { file: file.clone() };

        // Send to daemon
        match send_command(&project, &cmd).await {
            Ok(response) => self.handle_response(response, format, quiet),
            Err(DaemonError::NotRunning) | Err(DaemonError::ConnectionRefused) => {
                // Daemon not running - silently succeed
                // File edits should never fail due to daemon status
                if !quiet {
                    match format {
                        OutputFormat::Json | OutputFormat::Compact => {
                            let output = DaemonNotifyOutput {
                                status: "ok".to_string(),
                                dirty_count: 0,
                                threshold: 20,
                                reindex_triggered: false,
                                message: Some(
                                    "Daemon not running (notification ignored)".to_string(),
                                ),
                            };
                            println!("{}", serde_json::to_string_pretty(&output)?);
                        }
                        OutputFormat::Text | OutputFormat::Sarif | OutputFormat::Dot => {
                            // Silent - don't interrupt editor workflow
                        }
                    }
                }
                Ok(())
            }
            Err(e) => {
                // Other errors - also silently succeed
                // File edits should never fail due to daemon issues
                if !quiet {
                    match format {
                        OutputFormat::Json | OutputFormat::Compact => {
                            let output = DaemonNotifyOutput {
                                status: "ok".to_string(),
                                dirty_count: 0,
                                threshold: 20,
                                reindex_triggered: false,
                                message: Some(format!("Notification failed: {} (ignored)", e)),
                            };
                            println!("{}", serde_json::to_string_pretty(&output)?);
                        }
                        OutputFormat::Text | OutputFormat::Sarif | OutputFormat::Dot => {
                            // Silent - don't interrupt editor workflow
                        }
                    }
                }
                Ok(())
            }
        }
    }

    /// Handle the daemon response.
    fn handle_response(
        &self,
        response: DaemonResponse,
        format: OutputFormat,
        quiet: bool,
    ) -> anyhow::Result<()> {
        match response {
            DaemonResponse::NotifyResponse {
                status,
                dirty_count,
                threshold,
                reindex_triggered,
            } => {
                let output = DaemonNotifyOutput {
                    status,
                    dirty_count,
                    threshold,
                    reindex_triggered,
                    message: None,
                };

                if !quiet {
                    match format {
                        OutputFormat::Json | OutputFormat::Compact => {
                            println!("{}", serde_json::to_string_pretty(&output)?);
                        }
                        OutputFormat::Text | OutputFormat::Sarif | OutputFormat::Dot => {
                            if reindex_triggered {
                                println!("Reindex triggered ({}/{} files)", dirty_count, threshold);
                            } else {
                                println!("Tracked: {}/{} files", dirty_count, threshold);
                            }
                        }
                    }
                }

                Ok(())
            }
            DaemonResponse::Status { status, message } => {
                // Simple status response (probably "ok")
                let output = DaemonNotifyOutput {
                    status: status.clone(),
                    dirty_count: 0,
                    threshold: 20,
                    reindex_triggered: false,
                    message,
                };

                if !quiet {
                    match format {
                        OutputFormat::Json | OutputFormat::Compact => {
                            println!("{}", serde_json::to_string_pretty(&output)?);
                        }
                        OutputFormat::Text | OutputFormat::Sarif | OutputFormat::Dot => {
                            println!("Status: {}", status);
                        }
                    }
                }

                Ok(())
            }
            DaemonResponse::Error { error, .. } => {
                let output = DaemonNotifyErrorOutput {
                    status: "error".to_string(),
                    error: error.clone(),
                };

                if !quiet {
                    match format {
                        OutputFormat::Json | OutputFormat::Compact => {
                            println!("{}", serde_json::to_string_pretty(&output)?);
                        }
                        OutputFormat::Text | OutputFormat::Sarif | OutputFormat::Dot => {
                            eprintln!("Error: {}", error);
                        }
                    }
                }

                // Don't fail - file edits should work even with daemon errors
                Ok(())
            }
            _ => {
                // Unexpected response - treat as success
                Ok(())
            }
        }
    }
}

/// Send a notify command to the daemon (async version).
///
/// Convenience function that validates and sends the notification.
///
/// # Security
///
/// - TIGER-P3-03: Validates file path is within project root
pub async fn cmd_notify(args: DaemonNotifyArgs) -> DaemonResult<()> {
    // Resolve project path to absolute
    let project = args.project.canonicalize().unwrap_or_else(|_| {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(&args.project)
    });

    // Resolve file path to absolute
    let file = args.file.canonicalize().unwrap_or_else(|_| {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(&args.file)
    });

    // TIGER-P3-03: Validate file path is within project root
    if !file.starts_with(&project) {
        return Err(DaemonError::PermissionDenied { path: file });
    }

    // Build notify command
    let cmd = DaemonCommand::Notify { file };

    // Send to daemon
    let response = send_command(&project, &cmd).await?;

    // Print response
    match response {
        DaemonResponse::NotifyResponse {
            dirty_count,
            threshold,
            reindex_triggered,
            ..
        } => {
            if reindex_triggered {
                println!("Reindex triggered ({}/{} files)", dirty_count, threshold);
            } else {
                println!("Tracked: {}/{} files", dirty_count, threshold);
            }
            Ok(())
        }
        DaemonResponse::Error { error, .. } => {
            eprintln!("Error: {}", error);
            Ok(()) // Don't fail - file edits should work
        }
        _ => Ok(()),
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_daemon_notify_args_default() {
        let args = DaemonNotifyArgs {
            file: PathBuf::from("test.rs"),
            project: PathBuf::from("."),
        };

        assert_eq!(args.file, PathBuf::from("test.rs"));
        assert_eq!(args.project, PathBuf::from("."));
    }

    #[test]
    fn test_daemon_notify_args_with_project() {
        let args = DaemonNotifyArgs {
            file: PathBuf::from("/test/project/src/main.rs"),
            project: PathBuf::from("/test/project"),
        };

        assert_eq!(args.file, PathBuf::from("/test/project/src/main.rs"));
        assert_eq!(args.project, PathBuf::from("/test/project"));
    }

    #[test]
    fn test_daemon_notify_output_serialization() {
        let output = DaemonNotifyOutput {
            status: "ok".to_string(),
            dirty_count: 5,
            threshold: 20,
            reindex_triggered: false,
            message: None,
        };

        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("ok"));
        assert!(json.contains("5"));
        assert!(json.contains("20"));
        assert!(json.contains("false"));
    }

    #[test]
    fn test_daemon_notify_output_reindex_triggered() {
        let output = DaemonNotifyOutput {
            status: "ok".to_string(),
            dirty_count: 20,
            threshold: 20,
            reindex_triggered: true,
            message: None,
        };

        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("true"));
    }

    #[test]
    fn test_daemon_notify_error_output_serialization() {
        let output = DaemonNotifyErrorOutput {
            status: "error".to_string(),
            error: "File outside project root".to_string(),
        };

        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("error"));
        assert!(json.contains("File outside project root"));
    }

    #[tokio::test]
    async fn test_daemon_notify_file_outside_project() {
        let temp = TempDir::new().unwrap();
        let outside_file = TempDir::new().unwrap();
        let test_file = outside_file.path().join("outside.rs");
        fs::write(&test_file, "fn main() {}").unwrap();

        let args = DaemonNotifyArgs {
            file: test_file.clone(),
            project: temp.path().to_path_buf(),
        };

        // Should fail because file is outside project
        let result = cmd_notify(args).await;
        assert!(result.is_err());
        assert!(matches!(result, Err(DaemonError::PermissionDenied { .. })));
    }

    #[tokio::test]
    async fn test_daemon_notify_file_inside_project() {
        let temp = TempDir::new().unwrap();
        let test_file = temp.path().join("test.rs");
        fs::write(&test_file, "fn main() {}").unwrap();

        let args = DaemonNotifyArgs {
            file: test_file.clone(),
            project: temp.path().to_path_buf(),
        };

        // Should fail because daemon is not running (but path validation passed)
        let result = cmd_notify(args).await;
        // NotRunning error means path validation passed
        assert!(result.is_err());
        assert!(matches!(result, Err(DaemonError::NotRunning)));
    }

    #[tokio::test]
    async fn test_daemon_notify_silent_when_not_running() {
        let temp = TempDir::new().unwrap();
        let test_file = temp.path().join("test.rs");
        fs::write(&test_file, "fn main() {}").unwrap();

        let args = DaemonNotifyArgs {
            file: test_file.clone(),
            project: temp.path().to_path_buf(),
        };

        // The run method should succeed even when daemon is not running
        let result = args.run_async(OutputFormat::Json, true).await;
        assert!(result.is_ok());
    }
}
