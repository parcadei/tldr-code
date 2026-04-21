//! Daemon status command implementation
//!
//! CLI command: `tldr daemon status [--project PATH] [--session SESSION_ID]`
//!
//! This module provides status information about a running daemon:
//! - Current status (initializing, indexing, ready, shutting_down)
//! - Uptime
//! - Number of indexed files
//! - Cache statistics (hits, misses, hit rate, invalidations)
//! - Session statistics (if requested)
//! - Hook activity statistics

use std::path::PathBuf;

use clap::Args;
use serde::Serialize;

use crate::output::OutputFormat;

use super::error::DaemonError;
use super::ipc::send_command;
use super::types::{DaemonCommand, DaemonResponse, DaemonStatus, SalsaCacheStats};

// =============================================================================
// CLI Arguments
// =============================================================================

/// Arguments for the `daemon status` command.
#[derive(Debug, Clone, Args)]
pub struct DaemonStatusArgs {
    /// Project root directory (default: current directory)
    #[arg(long, short = 'p', default_value = ".")]
    pub project: PathBuf,

    /// Session ID to get session-specific stats
    #[arg(long, short = 's')]
    pub session: Option<String>,
}

// =============================================================================
// Output Types
// =============================================================================

/// Output structure for daemon status when running.
#[derive(Debug, Clone, Serialize)]
pub struct DaemonStatusOutput {
    /// Current status
    pub status: String,
    /// Uptime in seconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uptime: Option<f64>,
    /// Human-readable uptime
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uptime_human: Option<String>,
    /// Number of indexed files
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<usize>,
    /// Project path
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<PathBuf>,
    /// Cache statistics
    #[serde(skip_serializing_if = "Option::is_none")]
    pub salsa_stats: Option<SalsaCacheStats>,
    /// Optional message
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

// =============================================================================
// Command Implementation
// =============================================================================

impl DaemonStatusArgs {
    /// Run the daemon status command.
    pub fn run(&self, format: OutputFormat, quiet: bool) -> anyhow::Result<()> {
        // Create a new tokio runtime for the async operations
        let runtime = tokio::runtime::Runtime::new()?;
        runtime.block_on(self.run_async(format, quiet))
    }

    /// Async implementation of the daemon status command.
    async fn run_async(&self, format: OutputFormat, quiet: bool) -> anyhow::Result<()> {
        // Resolve project path to absolute
        let project = self.project.canonicalize().unwrap_or_else(|_| {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(&self.project)
        });

        // Send status command
        let cmd = DaemonCommand::Status {
            session: self.session.clone(),
        };

        match send_command(&project, &cmd).await {
            Ok(response) => self.handle_response(response, format, quiet),
            Err(DaemonError::NotRunning) | Err(DaemonError::ConnectionRefused) => {
                // Daemon not running
                let output = DaemonStatusOutput {
                    status: "not_running".to_string(),
                    uptime: None,
                    uptime_human: None,
                    files: None,
                    project: None,
                    salsa_stats: None,
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

                Ok(())
            }
            Err(e) => Err(anyhow::anyhow!("Failed to get daemon status: {}", e)),
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
            DaemonResponse::FullStatus {
                status,
                uptime,
                files,
                project,
                salsa_stats,
                ..
            } => {
                let status_str = format_status(status);
                let uptime_human = format_uptime(uptime);

                let output = DaemonStatusOutput {
                    status: status_str.clone(),
                    uptime: Some(uptime),
                    uptime_human: Some(uptime_human.clone()),
                    files: Some(files),
                    project: Some(project.clone()),
                    salsa_stats: Some(salsa_stats.clone()),
                    message: None,
                };

                if !quiet {
                    match format {
                        OutputFormat::Json | OutputFormat::Compact => {
                            println!("{}", serde_json::to_string_pretty(&output)?);
                        }
                        OutputFormat::Text | OutputFormat::Sarif | OutputFormat::Dot => {
                            println!("TLDR Daemon Status");
                            println!("==================");
                            println!("Status:  {}", status_str);
                            println!("Uptime:  {}", uptime_human);
                            println!("Project: {}", project.display());
                            println!("Files:   {}", files);
                            println!();
                            println!("Cache Statistics");
                            println!("----------------");
                            println!("Hits:          {}", format_number(salsa_stats.hits));
                            println!("Misses:        {}", format_number(salsa_stats.misses));
                            println!("Hit Rate:      {:.2}%", salsa_stats.hit_rate());
                            println!(
                                "Invalidations: {}",
                                format_number(salsa_stats.invalidations)
                            );
                        }
                    }
                }

                Ok(())
            }
            DaemonResponse::Status { status, message } => {
                let output = DaemonStatusOutput {
                    status: status.clone(),
                    uptime: None,
                    uptime_human: None,
                    files: None,
                    project: None,
                    salsa_stats: None,
                    message,
                };

                if !quiet {
                    match format {
                        OutputFormat::Json | OutputFormat::Compact => {
                            println!("{}", serde_json::to_string_pretty(&output)?);
                        }
                        OutputFormat::Text | OutputFormat::Sarif | OutputFormat::Dot => {
                            println!("Status: {}", status);
                            if let Some(msg) = &output.message {
                                println!("{}", msg);
                            }
                        }
                    }
                }

                Ok(())
            }
            DaemonResponse::Error { error, .. } => Err(anyhow::anyhow!("Daemon error: {}", error)),
            _ => Err(anyhow::anyhow!("Unexpected response from daemon")),
        }
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Format DaemonStatus as a string.
fn format_status(status: DaemonStatus) -> String {
    match status {
        DaemonStatus::Initializing => "initializing".to_string(),
        DaemonStatus::Indexing => "indexing".to_string(),
        DaemonStatus::Ready => "running".to_string(),
        DaemonStatus::ShuttingDown => "shutting_down".to_string(),
        DaemonStatus::Stopped => "stopped".to_string(),
    }
}

/// Format uptime seconds as human-readable string.
fn format_uptime(secs: f64) -> String {
    let total_secs = secs as u64;
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;
    format!("{}h {}m {}s", hours, minutes, seconds)
}

/// Format a number with thousands separators.
fn format_number(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut result = String::new();
    let len = bytes.len();

    for (i, &b) in bytes.iter().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            result.push(',');
        }
        result.push(b as char);
    }

    result
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_daemon_status_args_default() {
        let args = DaemonStatusArgs {
            project: PathBuf::from("."),
            session: None,
        };

        assert_eq!(args.project, PathBuf::from("."));
        assert!(args.session.is_none());
    }

    #[test]
    fn test_daemon_status_args_with_session() {
        let args = DaemonStatusArgs {
            project: PathBuf::from("/test/project"),
            session: Some("test-session".to_string()),
        };

        assert_eq!(args.session, Some("test-session".to_string()));
    }

    #[test]
    fn test_format_status() {
        assert_eq!(format_status(DaemonStatus::Ready), "running");
        assert_eq!(format_status(DaemonStatus::Initializing), "initializing");
        assert_eq!(format_status(DaemonStatus::Indexing), "indexing");
        assert_eq!(format_status(DaemonStatus::ShuttingDown), "shutting_down");
        assert_eq!(format_status(DaemonStatus::Stopped), "stopped");
    }

    #[test]
    fn test_format_uptime() {
        assert_eq!(format_uptime(0.0), "0h 0m 0s");
        assert_eq!(format_uptime(61.0), "0h 1m 1s");
        assert_eq!(format_uptime(3661.0), "1h 1m 1s");
        assert_eq!(format_uptime(7200.0), "2h 0m 0s");
    }

    #[test]
    fn test_format_number() {
        assert_eq!(format_number(0), "0");
        assert_eq!(format_number(999), "999");
        assert_eq!(format_number(1000), "1,000");
        assert_eq!(format_number(1234567), "1,234,567");
    }

    #[test]
    fn test_daemon_status_output_serialization() {
        let output = DaemonStatusOutput {
            status: "running".to_string(),
            uptime: Some(3600.0),
            uptime_human: Some("1h 0m 0s".to_string()),
            files: Some(100),
            project: Some(PathBuf::from("/test/project")),
            salsa_stats: Some(SalsaCacheStats {
                hits: 90,
                misses: 10,
                invalidations: 5,
                recomputations: 3,
            }),
            message: None,
        };

        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("running"));
        assert!(json.contains("3600"));
        assert!(json.contains("hits"));
    }

    #[test]
    fn test_daemon_status_output_not_running() {
        let output = DaemonStatusOutput {
            status: "not_running".to_string(),
            uptime: None,
            uptime_human: None,
            files: None,
            project: None,
            salsa_stats: None,
            message: Some("Daemon not running".to_string()),
        };

        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("not_running"));
        assert!(json.contains("not running"));
    }

    #[tokio::test]
    async fn test_daemon_status_not_running() {
        let temp = TempDir::new().unwrap();
        let args = DaemonStatusArgs {
            project: temp.path().to_path_buf(),
            session: None,
        };

        // Should succeed when daemon is not running (reports not_running)
        let result = args.run_async(OutputFormat::Json, true).await;
        assert!(result.is_ok());
    }
}
