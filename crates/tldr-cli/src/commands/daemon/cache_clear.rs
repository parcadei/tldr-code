//! Cache clear command implementation
//!
//! CLI command: `tldr cache clear [--project PATH]`
//!
//! Clears the cache for a TLDR project:
//! 1. If daemon is running, stops it first (or sends Clear command)
//! 2. Deletes cache files in `.tldr/cache/`
//! 3. Reports cleared size
//!
//! Files removed:
//! - salsa_cache.bin (Salsa query cache)
//! - salsa_stats.json (legacy stats file)
//! - call_graph.json (call graph cache)
//! - *.pkl files (pickle files, if any)
//! - Any other files in the cache directory

use std::fs;
use std::path::{Path, PathBuf};

use clap::Args;
use serde::Serialize;

use crate::output::OutputFormat;

use super::error::DaemonResult;
use super::ipc::send_command;
use super::types::DaemonCommand;

// =============================================================================
// CLI Arguments
// =============================================================================

/// Arguments for the `cache clear` command.
#[derive(Debug, Clone, Args)]
pub struct CacheClearArgs {
    /// Project root directory (default: current directory)
    #[arg(long, short = 'p', default_value = ".")]
    pub project: PathBuf,
}

// =============================================================================
// Output Types
// =============================================================================

/// Output structure for cache clear command.
#[derive(Debug, Clone, Serialize)]
pub struct CacheClearOutput {
    /// Status of the operation
    pub status: String,
    /// Number of files removed
    pub files_removed: usize,
    /// Bytes freed
    pub bytes_freed: u64,
    /// Human-readable size freed
    pub size_freed_human: String,
    /// Optional message
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

// =============================================================================
// Command Implementation
// =============================================================================

impl CacheClearArgs {
    /// Run the cache clear command.
    pub fn run(&self, format: OutputFormat, quiet: bool) -> anyhow::Result<()> {
        // Create a new tokio runtime for the async operations
        let runtime = tokio::runtime::Runtime::new()?;
        runtime.block_on(self.run_async(format, quiet))
    }

    /// Async implementation of the cache clear command.
    async fn run_async(&self, format: OutputFormat, quiet: bool) -> anyhow::Result<()> {
        // Resolve project path to absolute
        let project = self.project.canonicalize().unwrap_or_else(|_| {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(&self.project)
        });

        // Try to stop daemon first if it's running
        // This ensures the daemon doesn't continue writing to cache files
        self.try_stop_daemon(&project).await;

        // Clear cache files
        let (files_removed, bytes_freed) = self.clear_cache_files(&project)?;

        let output = if files_removed == 0 {
            CacheClearOutput {
                status: "ok".to_string(),
                files_removed: 0,
                bytes_freed: 0,
                size_freed_human: "0 B".to_string(),
                message: Some("No cache directory found".to_string()),
            }
        } else {
            CacheClearOutput {
                status: "ok".to_string(),
                files_removed,
                bytes_freed,
                size_freed_human: format_bytes(bytes_freed),
                message: Some(format!("Cache cleared: {} file(s) removed", files_removed)),
            }
        };

        self.print_output(&output, format, quiet)
    }

    /// Try to stop the daemon if it's running.
    async fn try_stop_daemon(&self, project: &Path) {
        let cmd = DaemonCommand::Shutdown;
        // Ignore errors - daemon might not be running
        let _ = send_command(project, &cmd).await;
    }

    /// Clear all cache files in the project's .tldr/cache/ directory.
    fn clear_cache_files(&self, project: &Path) -> DaemonResult<(usize, u64)> {
        let cache_dir = project.join(".tldr").join("cache");

        if !cache_dir.exists() {
            return Ok((0, 0));
        }

        let mut files_removed = 0;
        let mut bytes_freed = 0u64;

        // Collect files to remove
        let entries: Vec<_> = fs::read_dir(&cache_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.metadata().map(|m| m.is_file()).unwrap_or(false))
            .collect();

        // Remove each file
        for entry in entries {
            let path = entry.path();
            if let Ok(metadata) = entry.metadata() {
                bytes_freed += metadata.len();
            }
            if fs::remove_file(&path).is_ok() {
                files_removed += 1;
            }
        }

        Ok((files_removed, bytes_freed))
    }

    /// Print output in the requested format.
    fn print_output(
        &self,
        output: &CacheClearOutput,
        format: OutputFormat,
        quiet: bool,
    ) -> anyhow::Result<()> {
        if quiet {
            return Ok(());
        }

        match format {
            OutputFormat::Json | OutputFormat::Compact => {
                println!("{}", serde_json::to_string_pretty(output)?);
            }
            OutputFormat::Text | OutputFormat::Sarif | OutputFormat::Dot => {
                if output.files_removed == 0 {
                    println!("No cache directory found");
                } else {
                    println!(
                        "Cache cleared: {} file(s) removed ({})",
                        output.files_removed, output.size_freed_human
                    );
                }
            }
        }

        Ok(())
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Format bytes as human-readable string.
fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
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
    fn test_cache_clear_args_default() {
        let args = CacheClearArgs {
            project: PathBuf::from("."),
        };
        assert_eq!(args.project, PathBuf::from("."));
    }

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(1048576), "1.0 MB");
        assert_eq!(format_bytes(1073741824), "1.0 GB");
    }

    #[test]
    fn test_cache_clear_output_serialization() {
        let output = CacheClearOutput {
            status: "ok".to_string(),
            files_removed: 26,
            bytes_freed: 1048576,
            size_freed_human: "1.0 MB".to_string(),
            message: Some("Cache cleared: 26 file(s) removed".to_string()),
        };

        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("ok"));
        assert!(json.contains("26"));
        assert!(json.contains("1048576"));
        assert!(json.contains("1.0 MB"));
    }

    #[test]
    fn test_cache_clear_output_empty() {
        let output = CacheClearOutput {
            status: "ok".to_string(),
            files_removed: 0,
            bytes_freed: 0,
            size_freed_human: "0 B".to_string(),
            message: Some("No cache directory found".to_string()),
        };

        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("No cache directory found"));
    }

    #[test]
    fn test_clear_cache_files_no_cache_dir() {
        let temp = TempDir::new().unwrap();
        let args = CacheClearArgs {
            project: temp.path().to_path_buf(),
        };

        let result = args.clear_cache_files(temp.path());
        assert!(result.is_ok());
        let (files, bytes) = result.unwrap();
        assert_eq!(files, 0);
        assert_eq!(bytes, 0);
    }

    #[test]
    fn test_clear_cache_files_with_files() {
        let temp = TempDir::new().unwrap();
        let cache_dir = temp.path().join(".tldr").join("cache");
        fs::create_dir_all(&cache_dir).unwrap();

        // Create some test files
        fs::write(cache_dir.join("salsa_cache.bin"), "test data 1").unwrap();
        fs::write(cache_dir.join("call_graph.json"), r#"{"edges":[]}"#).unwrap();
        fs::write(cache_dir.join("test.pkl"), "pickle data").unwrap();

        let args = CacheClearArgs {
            project: temp.path().to_path_buf(),
        };

        let result = args.clear_cache_files(temp.path());
        assert!(result.is_ok());
        let (files, bytes) = result.unwrap();
        assert_eq!(files, 3);
        assert!(bytes > 0);

        // Verify files are gone
        assert!(!cache_dir.join("salsa_cache.bin").exists());
        assert!(!cache_dir.join("call_graph.json").exists());
        assert!(!cache_dir.join("test.pkl").exists());
    }

    #[test]
    fn test_clear_cache_files_preserves_directory() {
        let temp = TempDir::new().unwrap();
        let cache_dir = temp.path().join(".tldr").join("cache");
        fs::create_dir_all(&cache_dir).unwrap();
        fs::write(cache_dir.join("test.bin"), "data").unwrap();

        let args = CacheClearArgs {
            project: temp.path().to_path_buf(),
        };

        args.clear_cache_files(temp.path()).unwrap();

        // Cache directory should still exist (only files removed)
        assert!(cache_dir.exists());
    }

    #[tokio::test]
    async fn test_cache_clear_no_cache() {
        let temp = TempDir::new().unwrap();
        let args = CacheClearArgs {
            project: temp.path().to_path_buf(),
        };

        // Should succeed even with no cache
        let result = args.run_async(OutputFormat::Json, true).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_cache_clear_with_files() {
        let temp = TempDir::new().unwrap();
        let cache_dir = temp.path().join(".tldr").join("cache");
        fs::create_dir_all(&cache_dir).unwrap();
        fs::write(cache_dir.join("test.bin"), "test data").unwrap();

        let args = CacheClearArgs {
            project: temp.path().to_path_buf(),
        };

        let result = args.run_async(OutputFormat::Json, true).await;
        assert!(result.is_ok());

        // File should be removed
        assert!(!cache_dir.join("test.bin").exists());
    }
}
