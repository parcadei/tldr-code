//! Cache statistics command implementation
//!
//! CLI command: `tldr cache stats [--project PATH]`
//!
//! Displays cache statistics for a TLDR project:
//! - If daemon is running: queries cache stats via IPC
//! - If daemon is not running: reads cache files directly
//!
//! Statistics include:
//! - Salsa-style query cache: hits, misses, hit rate, invalidations
//! - Cache files: file count, total size on disk

use std::fs;
use std::path::{Path, PathBuf};

use clap::Args;
use serde::Serialize;

use crate::output::OutputFormat;

use super::error::{DaemonError, DaemonResult};
use super::ipc::send_command;
use super::salsa::QueryCache;
use super::types::{CacheFileInfo, DaemonCommand, DaemonResponse, SalsaCacheStats};

// =============================================================================
// CLI Arguments
// =============================================================================

/// Arguments for the `cache stats` command.
#[derive(Debug, Clone, Args)]
pub struct CacheStatsArgs {
    /// Project root directory (default: current directory)
    #[arg(long, short = 'p', default_value = ".")]
    pub project: PathBuf,
}

// =============================================================================
// Output Types
// =============================================================================

/// Output structure for cache stats command.
#[derive(Debug, Clone, Serialize)]
pub struct CacheStatsOutput {
    /// Salsa-style query cache statistics
    #[serde(skip_serializing_if = "Option::is_none")]
    pub salsa_stats: Option<SalsaCacheStats>,
    /// Cache file information
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_files: Option<CacheFileInfo>,
    /// Optional message (e.g., "No cache statistics found")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

// =============================================================================
// Command Implementation
// =============================================================================

impl CacheStatsArgs {
    /// Run the cache stats command.
    pub fn run(&self, format: OutputFormat, quiet: bool) -> anyhow::Result<()> {
        // Create a new tokio runtime for the async operations
        let runtime = tokio::runtime::Runtime::new()?;
        runtime.block_on(self.run_async(format, quiet))
    }

    /// Async implementation of the cache stats command.
    async fn run_async(&self, format: OutputFormat, quiet: bool) -> anyhow::Result<()> {
        // Resolve project path to absolute
        let project = self.project.canonicalize().unwrap_or_else(|_| {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(&self.project)
        });

        // Try to get stats from running daemon first
        let cmd = DaemonCommand::Status { session: None };

        match send_command(&project, &cmd).await {
            Ok(DaemonResponse::FullStatus { salsa_stats, .. }) => {
                // Daemon is running, use its stats
                let cache_files = scan_cache_files(&project)?;
                let output = CacheStatsOutput {
                    salsa_stats: Some(salsa_stats),
                    cache_files: Some(cache_files),
                    message: None,
                };
                self.print_output(&output, format, quiet)
            }
            Ok(_) | Err(DaemonError::NotRunning) | Err(DaemonError::ConnectionRefused) => {
                // Daemon not running, read from cache files directly
                self.read_cache_from_files(&project, format, quiet)
            }
            Err(e) => Err(anyhow::anyhow!("Failed to get cache stats: {}", e)),
        }
    }

    /// Read cache statistics from files when daemon is not running.
    fn read_cache_from_files(
        &self,
        project: &Path,
        format: OutputFormat,
        quiet: bool,
    ) -> anyhow::Result<()> {
        let cache_dir = project.join(".tldr").join("cache");

        // Check if cache directory exists
        if !cache_dir.exists() {
            let output = CacheStatsOutput {
                salsa_stats: None,
                cache_files: None,
                message: Some("No cache directory found".to_string()),
            };
            return self.print_output(&output, format, quiet);
        }

        // Try to load salsa stats from file
        let salsa_stats = self.load_salsa_stats(&cache_dir);

        // Scan cache files
        let cache_files = scan_cache_files(project)?;

        // Check if we have any cache data
        if salsa_stats.is_none() && cache_files.file_count == 0 {
            let output = CacheStatsOutput {
                salsa_stats: None,
                cache_files: Some(cache_files),
                message: Some("No cache statistics found".to_string()),
            };
            return self.print_output(&output, format, quiet);
        }

        let output = CacheStatsOutput {
            salsa_stats,
            cache_files: Some(cache_files),
            message: None,
        };

        self.print_output(&output, format, quiet)
    }

    /// Try to load salsa cache stats from persisted file.
    fn load_salsa_stats(&self, cache_dir: &Path) -> Option<SalsaCacheStats> {
        let salsa_cache_file = cache_dir.join("salsa_cache.bin");

        if !salsa_cache_file.exists() {
            return None;
        }

        // Try to load the cache and extract stats
        match QueryCache::load_from_file(&salsa_cache_file) {
            Ok(cache) => Some(cache.stats()),
            Err(_) => None,
        }
    }

    /// Print output in the requested format.
    fn print_output(
        &self,
        output: &CacheStatsOutput,
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
                if let Some(ref msg) = output.message {
                    println!("{}", msg);
                    return Ok(());
                }

                println!("Cache Statistics");
                println!("================");

                if let Some(ref stats) = output.salsa_stats {
                    println!();
                    println!("Salsa Cache:");
                    println!("  Hits:          {}", format_number(stats.hits));
                    println!("  Misses:        {}", format_number(stats.misses));
                    println!("  Hit Rate:      {:.2}%", stats.hit_rate());
                    println!("  Invalidations: {}", format_number(stats.invalidations));
                    println!("  Recomputations: {}", format_number(stats.recomputations));
                }

                if let Some(ref files) = output.cache_files {
                    println!();
                    println!("Cache Files:");
                    println!("  Count: {} files", files.file_count);
                    println!("  Size:  {}", files.total_size_human);
                }
            }
        }

        Ok(())
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Scan cache files in the project's .tldr/cache/ directory.
fn scan_cache_files(project: &Path) -> DaemonResult<CacheFileInfo> {
    let cache_dir = project.join(".tldr").join("cache");

    if !cache_dir.exists() {
        return Ok(CacheFileInfo {
            file_count: 0,
            total_bytes: 0,
            total_size_human: "0 B".to_string(),
        });
    }

    let mut file_count = 0;
    let mut total_bytes = 0u64;

    // Count all files in cache directory
    if let Ok(entries) = fs::read_dir(&cache_dir) {
        for entry in entries.flatten() {
            if let Ok(metadata) = entry.metadata() {
                if metadata.is_file() {
                    file_count += 1;
                    total_bytes += metadata.len();
                }
            }
        }
    }

    Ok(CacheFileInfo {
        file_count,
        total_bytes,
        total_size_human: format_bytes(total_bytes),
    })
}

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
    fn test_cache_stats_args_default() {
        let args = CacheStatsArgs {
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
        assert_eq!(format_bytes(1572864), "1.5 MB");
        assert_eq!(format_bytes(1073741824), "1.0 GB");
    }

    #[test]
    fn test_format_number() {
        assert_eq!(format_number(0), "0");
        assert_eq!(format_number(999), "999");
        assert_eq!(format_number(1000), "1,000");
        assert_eq!(format_number(1234567), "1,234,567");
    }

    #[test]
    fn test_scan_cache_files_no_cache_dir() {
        let temp = TempDir::new().unwrap();
        let result = scan_cache_files(temp.path()).unwrap();

        assert_eq!(result.file_count, 0);
        assert_eq!(result.total_bytes, 0);
        assert_eq!(result.total_size_human, "0 B");
    }

    #[test]
    fn test_scan_cache_files_with_files() {
        let temp = TempDir::new().unwrap();
        let cache_dir = temp.path().join(".tldr").join("cache");
        fs::create_dir_all(&cache_dir).unwrap();

        // Create some test files
        fs::write(cache_dir.join("file1.bin"), "hello").unwrap();
        fs::write(cache_dir.join("file2.json"), "world").unwrap();
        fs::write(cache_dir.join("call_graph.json"), r#"{"edges":[]}"#).unwrap();

        let result = scan_cache_files(temp.path()).unwrap();

        assert_eq!(result.file_count, 3);
        assert!(result.total_bytes > 0);
    }

    #[test]
    fn test_cache_stats_output_serialization() {
        let output = CacheStatsOutput {
            salsa_stats: Some(SalsaCacheStats {
                hits: 100,
                misses: 10,
                invalidations: 5,
                recomputations: 3,
            }),
            cache_files: Some(CacheFileInfo {
                file_count: 25,
                total_bytes: 1048576,
                total_size_human: "1.0 MB".to_string(),
            }),
            message: None,
        };

        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("hits"));
        assert!(json.contains("100"));
        assert!(json.contains("file_count"));
        assert!(json.contains("25"));
    }

    #[test]
    fn test_cache_stats_output_empty() {
        let output = CacheStatsOutput {
            salsa_stats: None,
            cache_files: None,
            message: Some("No cache statistics found".to_string()),
        };

        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("No cache statistics found"));
        assert!(!json.contains("salsa_stats"));
        assert!(!json.contains("cache_files"));
    }

    #[tokio::test]
    async fn test_cache_stats_no_cache() {
        let temp = TempDir::new().unwrap();
        let args = CacheStatsArgs {
            project: temp.path().to_path_buf(),
        };

        // Should succeed even with no cache
        let result = args.run_async(OutputFormat::Json, true).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_cache_stats_with_cache_dir() {
        let temp = TempDir::new().unwrap();
        let cache_dir = temp.path().join(".tldr").join("cache");
        fs::create_dir_all(&cache_dir).unwrap();
        fs::write(cache_dir.join("test.bin"), "test data").unwrap();

        let args = CacheStatsArgs {
            project: temp.path().to_path_buf(),
        };

        let result = args.run_async(OutputFormat::Json, true).await;
        assert!(result.is_ok());
    }
}
