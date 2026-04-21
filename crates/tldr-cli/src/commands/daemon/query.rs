//! Daemon query command implementation
//!
//! CLI command: `tldr daemon query CMD [--project PATH] [--json PARAMS]`
//!
//! This module provides raw query passthrough to the daemon for:
//! - Low-level debugging
//! - Custom commands
//! - Direct access to daemon functionality
//!
//! # Security Mitigations
//!
//! - TIGER-P3-03: Message size limits enforced by IPC layer
//! - TIGER-P3-05: Rate limiting handled in daemon (client just sends)

use std::path::PathBuf;

use clap::Args;
use serde::Serialize;

use crate::output::OutputFormat;

use super::error::{DaemonError, DaemonResult};
use super::ipc::send_raw_command;

// =============================================================================
// CLI Arguments
// =============================================================================

/// Arguments for the `daemon query` command.
#[derive(Debug, Clone, Args)]
pub struct DaemonQueryArgs {
    /// Command name to send (e.g., ping, status, search, structure)
    pub cmd: String,

    /// Project root directory (default: current directory)
    #[arg(long, short = 'p', default_value = ".")]
    pub project: PathBuf,

    /// Additional JSON parameters for the command
    #[arg(long, short = 'j')]
    pub json: Option<String>,
}

// =============================================================================
// Output Types
// =============================================================================

/// Output structure for query errors.
#[derive(Debug, Clone, Serialize)]
pub struct DaemonQueryErrorOutput {
    /// Status (always "error")
    pub status: String,
    /// Error message
    pub error: String,
}

// =============================================================================
// Command Implementation
// =============================================================================

impl DaemonQueryArgs {
    /// Run the daemon query command.
    pub fn run(&self, format: OutputFormat, quiet: bool) -> anyhow::Result<()> {
        // Create a new tokio runtime for the async operations
        let runtime = tokio::runtime::Runtime::new()?;
        runtime.block_on(self.run_async(format, quiet))
    }

    /// Async implementation of the daemon query command.
    async fn run_async(&self, format: OutputFormat, quiet: bool) -> anyhow::Result<()> {
        // Resolve project path to absolute
        let project = self.project.canonicalize().unwrap_or_else(|_| {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(&self.project)
        });

        // Build the command JSON
        let command_json = self.build_command_json()?;

        // Send command to daemon
        match send_raw_command(&project, &command_json).await {
            Ok(response) => {
                // Print raw response (pass-through)
                if !quiet {
                    match format {
                        OutputFormat::Json => {
                            // Pretty-print JSON
                            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&response)
                            {
                                println!("{}", serde_json::to_string_pretty(&parsed)?);
                            } else {
                                println!("{}", response);
                            }
                        }
                        OutputFormat::Compact => {
                            // Raw JSON
                            println!("{}", response);
                        }
                        OutputFormat::Text | OutputFormat::Sarif | OutputFormat::Dot => {
                            // Try to format as text if possible
                            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&response)
                            {
                                self.print_text_output(&parsed);
                            } else {
                                println!("{}", response);
                            }
                        }
                    }
                }
                Ok(())
            }
            Err(DaemonError::NotRunning) | Err(DaemonError::ConnectionRefused) => {
                let output = DaemonQueryErrorOutput {
                    status: "error".to_string(),
                    error: "Daemon not running".to_string(),
                };

                if !quiet {
                    match format {
                        OutputFormat::Json | OutputFormat::Compact => {
                            println!("{}", serde_json::to_string_pretty(&output)?);
                        }
                        OutputFormat::Text | OutputFormat::Sarif | OutputFormat::Dot => {
                            eprintln!("Error: Daemon not running");
                        }
                    }
                }

                Err(anyhow::anyhow!("Daemon not running"))
            }
            Err(DaemonError::InvalidMessage(msg)) => {
                let output = DaemonQueryErrorOutput {
                    status: "error".to_string(),
                    error: format!("Invalid JSON parameters: {}", msg),
                };

                if !quiet {
                    match format {
                        OutputFormat::Json | OutputFormat::Compact => {
                            println!("{}", serde_json::to_string_pretty(&output)?);
                        }
                        OutputFormat::Text | OutputFormat::Sarif | OutputFormat::Dot => {
                            eprintln!("Error: Invalid JSON parameters: {}", msg);
                        }
                    }
                }

                Err(anyhow::anyhow!("Invalid JSON parameters: {}", msg))
            }
            Err(e) => {
                let output = DaemonQueryErrorOutput {
                    status: "error".to_string(),
                    error: e.to_string(),
                };

                if !quiet {
                    match format {
                        OutputFormat::Json | OutputFormat::Compact => {
                            println!("{}", serde_json::to_string_pretty(&output)?);
                        }
                        OutputFormat::Text | OutputFormat::Sarif | OutputFormat::Dot => {
                            eprintln!("Error: {}", e);
                        }
                    }
                }

                Err(anyhow::anyhow!("Query failed: {}", e))
            }
        }
    }

    /// Build the command JSON from the cmd string and optional JSON parameters.
    fn build_command_json(&self) -> anyhow::Result<String> {
        // Start with the base command
        let mut cmd_obj = serde_json::json!({
            "cmd": self.cmd.to_lowercase()
        });

        // Merge additional JSON parameters if provided
        if let Some(json_str) = &self.json {
            let params: serde_json::Value = serde_json::from_str(json_str)
                .map_err(|e| anyhow::anyhow!("Invalid JSON parameters: {}", e))?;

            if let serde_json::Value::Object(params_obj) = params {
                if let serde_json::Value::Object(ref mut cmd_map) = cmd_obj {
                    for (key, value) in params_obj {
                        cmd_map.insert(key, value);
                    }
                }
            }
        }

        Ok(serde_json::to_string(&cmd_obj)?)
    }

    /// Print text output for common response types.
    fn print_text_output(&self, response: &serde_json::Value) {
        // Check for error response
        if let Some(error) = response.get("error") {
            eprintln!("Error: {}", error);
            return;
        }

        // Check for status response
        if let Some(status) = response.get("status") {
            println!("Status: {}", status);
            if let Some(message) = response.get("message") {
                println!("{}", message);
            }
        }

        // For complex responses, just pretty-print the JSON
        if response.as_object().map(|o| o.len() > 2).unwrap_or(false) {
            println!(
                "{}",
                serde_json::to_string_pretty(response).unwrap_or_default()
            );
        }
    }
}

/// Send a typed DaemonCommand (async version).
///
/// Convenience function that builds the command and sends it.
pub async fn cmd_query(args: DaemonQueryArgs) -> DaemonResult<()> {
    // Resolve project path to absolute
    let project = args.project.canonicalize().unwrap_or_else(|_| {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(&args.project)
    });

    // Build the command JSON
    let mut cmd_obj = serde_json::json!({
        "cmd": args.cmd.to_lowercase()
    });

    // Merge additional JSON parameters if provided
    if let Some(json_str) = &args.json {
        let params: serde_json::Value = serde_json::from_str(json_str)
            .map_err(|e| DaemonError::InvalidMessage(format!("Invalid JSON: {}", e)))?;

        if let serde_json::Value::Object(params_obj) = params {
            if let serde_json::Value::Object(ref mut cmd_map) = cmd_obj {
                for (key, value) in params_obj {
                    cmd_map.insert(key, value);
                }
            }
        }
    }

    let command_json =
        serde_json::to_string(&cmd_obj).map_err(|e| DaemonError::InvalidMessage(e.to_string()))?;

    // Send command to daemon
    let response = send_raw_command(&project, &command_json).await?;

    // Print response
    println!("{}", response);

    Ok(())
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_daemon_query_args_default() {
        let args = DaemonQueryArgs {
            cmd: "ping".to_string(),
            project: PathBuf::from("."),
            json: None,
        };

        assert_eq!(args.cmd, "ping");
        assert_eq!(args.project, PathBuf::from("."));
        assert!(args.json.is_none());
    }

    #[test]
    fn test_daemon_query_args_with_json() {
        let args = DaemonQueryArgs {
            cmd: "search".to_string(),
            project: PathBuf::from("/test/project"),
            json: Some(r#"{"pattern": "fn main"}"#.to_string()),
        };

        assert_eq!(args.cmd, "search");
        assert!(args.json.is_some());
    }

    #[test]
    fn test_build_command_json_simple() {
        let args = DaemonQueryArgs {
            cmd: "ping".to_string(),
            project: PathBuf::from("."),
            json: None,
        };

        let json = args.build_command_json().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.get("cmd").unwrap(), "ping");
    }

    #[test]
    fn test_build_command_json_with_params() {
        let args = DaemonQueryArgs {
            cmd: "search".to_string(),
            project: PathBuf::from("."),
            json: Some(r#"{"pattern": "fn main", "max_results": 10}"#.to_string()),
        };

        let json = args.build_command_json().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.get("cmd").unwrap(), "search");
        assert_eq!(parsed.get("pattern").unwrap(), "fn main");
        assert_eq!(parsed.get("max_results").unwrap(), 10);
    }

    #[test]
    fn test_build_command_json_invalid_params() {
        let args = DaemonQueryArgs {
            cmd: "search".to_string(),
            project: PathBuf::from("."),
            json: Some("not valid json".to_string()),
        };

        let result = args.build_command_json();
        assert!(result.is_err());
    }

    #[test]
    fn test_build_command_json_lowercases_cmd() {
        let args = DaemonQueryArgs {
            cmd: "PING".to_string(),
            project: PathBuf::from("."),
            json: None,
        };

        let json = args.build_command_json().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.get("cmd").unwrap(), "ping");
    }

    #[test]
    fn test_daemon_query_error_output_serialization() {
        let output = DaemonQueryErrorOutput {
            status: "error".to_string(),
            error: "Daemon not running".to_string(),
        };

        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("error"));
        assert!(json.contains("Daemon not running"));
    }

    #[tokio::test]
    async fn test_daemon_query_not_running() {
        let temp = TempDir::new().unwrap();
        let args = DaemonQueryArgs {
            cmd: "ping".to_string(),
            project: temp.path().to_path_buf(),
            json: None,
        };

        // Should fail when daemon is not running
        let result = cmd_query(args).await;
        assert!(result.is_err());
    }
}
