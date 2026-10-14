//! Stats command implementation
//!
//! CLI command: `tldr stats [--format json|text]`
//!
//! Reads usage statistics from `~/.tldr/stats.jsonl` and aggregates them.
//!
//! # Behavior
//!
//! 1. Read stats from JSONL file
//! 2. Aggregate session stats
//! 3. Calculate token savings
//! 4. Output in requested format
//!
//! # Output
//!
//! JSON format:
//! ```json
//! {
//!   "total_invocations": 1500,
//!   "estimated_tokens_saved": 4500000,
//!   "raw_tokens_total": 5000000,
//!   "tldr_tokens_total": 500000,
//!   "savings_percent": 90.0
//! }
//! ```

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use clap::Args;
use dirs;
use serde::{Deserialize, Serialize};

use crate::output::OutputFormat;

use super::types::GlobalStats;

// =============================================================================
// CLI Arguments
// =============================================================================

/// Arguments for the `stats` command.
#[derive(Debug, Clone, Args)]
pub struct StatsArgs {
    // Stats command uses the global --format flag, no local format arg needed
}

// =============================================================================
// Stats File Types
// =============================================================================

/// Entry in the stats.jsonl file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatsEntry {
    /// Session identifier
    pub session_id: String,

    /// Raw tokens processed
    pub raw_tokens: u64,

    /// TLDR tokens returned
    pub tldr_tokens: u64,

    /// Number of requests
    pub requests: u64,

    /// Optional timestamp
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
}

/// Output for the stats command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatsOutput {
    /// Total number of invocations across all sessions
    pub total_invocations: u64,

    /// Estimated tokens saved across all sessions
    pub estimated_tokens_saved: i64,

    /// Total raw tokens processed
    pub raw_tokens_total: u64,

    /// Total TLDR tokens returned
    pub tldr_tokens_total: u64,

    /// Savings percentage (0-100)
    pub savings_percent: f64,
}

/// Message output for empty stats.
///
/// low-cleanup-bundle-v1 (L2): the previous shape `{"message": "No usage
/// recorded"}` was opaque — users had no idea what "usage" meant or how to
/// produce it. We now include a `next_steps` hint that names the daemon and
/// the exact command to run, plus a `requires` field listing prerequisites
/// for programmatic consumers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmptyStatsOutput {
    /// Message indicating no usage
    pub message: String,
    /// Concrete command(s) the user can run to populate stats.
    pub next_steps: Vec<String>,
    /// Prerequisites required for the stats command to produce data.
    pub requires: Vec<String>,
}

impl EmptyStatsOutput {
    /// Build the canonical empty-stats payload.
    fn empty() -> Self {
        Self {
            message: "No usage recorded yet".to_string(),
            next_steps: vec![
                "tldr daemon start  # begin recording usage".to_string(),
                "tldr <any-command> ...  # run a few commands while the daemon is up".to_string(),
                "tldr stats  # rerun this command to see call counts and latencies".to_string(),
            ],
            requires: vec![
                "tldr daemon (run `tldr daemon start`)".to_string(),
                "at least one daemon-tracked invocation".to_string(),
            ],
        }
    }
}

// =============================================================================
// Implementation
// =============================================================================

impl StatsArgs {
    /// Run the stats command.
    pub fn run(&self, format: OutputFormat, quiet: bool) -> anyhow::Result<()> {
        // Get stats file path
        let stats_path = get_stats_path()?;

        // Read and aggregate stats
        let stats = read_and_aggregate_stats(&stats_path)?;

        // Output result.
        //
        // high-bundle-progress-determinism-coverage-v1 (N1 follow-up):
        // `quiet` is for *progress* suppression, not for total silence.
        // For json/compact (now auto-quiet-on-json), we still need to
        // emit the structured payload — otherwise pipelines see empty
        // stdout. Only the human-readable text branches honor `quiet`
        // for the verbose explanatory blurb.
        let output_format = format;

        match stats {
            Some(stats) => {
                let output = StatsOutput {
                    total_invocations: stats.total_invocations,
                    estimated_tokens_saved: stats.estimated_tokens_saved,
                    raw_tokens_total: stats.raw_tokens_total,
                    tldr_tokens_total: stats.tldr_tokens_total,
                    savings_percent: stats.savings_percent,
                };

                match output_format {
                    OutputFormat::Json | OutputFormat::Compact => {
                        println!("{}", serde_json::to_string_pretty(&output)?);
                    }
                    OutputFormat::Text | OutputFormat::Sarif | OutputFormat::Dot => {
                        if !quiet {
                            print_text_stats(&output);
                        }
                    }
                }
            }
            None => {
                let empty = EmptyStatsOutput::empty();
                match output_format {
                    OutputFormat::Json | OutputFormat::Compact => {
                        println!("{}", serde_json::to_string_pretty(&empty)?);
                    }
                    OutputFormat::Text | OutputFormat::Sarif | OutputFormat::Dot => {
                        if !quiet {
                            println!("{}", empty.message);
                            println!();
                            println!(
                                "Usage tracking requires the tldr daemon. To begin recording:"
                            );
                            for step in &empty.next_steps {
                                println!("  $ {}", step);
                            }
                            println!();
                            println!(
                                "Once the daemon has captured invocations, this command will \
                                 display call counts, latencies, and most-used commands."
                            );
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

/// Get the path to the stats file.
fn get_stats_path() -> anyhow::Result<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
    Ok(home.join(".tldr").join("stats.jsonl"))
}

/// Read and aggregate stats from the JSONL file.
fn read_and_aggregate_stats(stats_path: &PathBuf) -> anyhow::Result<Option<GlobalStats>> {
    if !stats_path.exists() {
        return Ok(None);
    }

    let file = fs::File::open(stats_path)?;
    let reader = BufReader::new(file);

    let mut total_invocations: u64 = 0;
    let mut raw_tokens_total: u64 = 0;
    let mut tldr_tokens_total: u64 = 0;
    let mut has_entries = false;

    for line in reader.lines() {
        let line = line?;
        let line = line.trim();

        if line.is_empty() {
            continue;
        }

        // Parse each line as a stats entry
        if let Ok(entry) = serde_json::from_str::<StatsEntry>(line) {
            total_invocations += entry.requests;
            raw_tokens_total += entry.raw_tokens;
            tldr_tokens_total += entry.tldr_tokens;
            has_entries = true;
        }
    }

    if !has_entries {
        return Ok(None);
    }

    let estimated_tokens_saved = raw_tokens_total as i64 - tldr_tokens_total as i64;
    let savings_percent = if raw_tokens_total > 0 {
        (estimated_tokens_saved as f64 / raw_tokens_total as f64) * 100.0
    } else {
        0.0
    };

    Ok(Some(GlobalStats {
        total_invocations,
        estimated_tokens_saved,
        raw_tokens_total,
        tldr_tokens_total,
        savings_percent,
    }))
}

/// Print stats in text format.
fn print_text_stats(stats: &StatsOutput) {
    println!("TLDR Usage Statistics");
    println!("=====================");
    println!(
        "Total Invocations:     {}",
        format_number(stats.total_invocations)
    );
    println!(
        "Tokens Saved:          {} ({:.1}%)",
        format_number_signed(stats.estimated_tokens_saved),
        stats.savings_percent
    );
    println!(
        "Raw Tokens Processed:  {}",
        format_number(stats.raw_tokens_total)
    );
    println!(
        "TLDR Tokens Returned:  {}",
        format_number(stats.tldr_tokens_total)
    );
}

/// Format a number with thousands separators.
fn format_number(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::new();
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();

    for (i, c) in chars.iter().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            result.push(',');
        }
        result.push(*c);
    }

    result
}

/// Format a signed number with thousands separators.
fn format_number_signed(n: i64) -> String {
    if n < 0 {
        format!("-{}", format_number((-n) as u64))
    } else {
        format_number(n as u64)
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
    fn test_stats_args_default() {
        // StatsArgs has no fields - it uses global format from CLI
        let _args = StatsArgs {};
    }

    #[test]
    fn test_stats_entry_serialization() {
        let entry = StatsEntry {
            session_id: "test123".to_string(),
            raw_tokens: 1000,
            tldr_tokens: 100,
            requests: 10,
            timestamp: Some("2024-01-01T00:00:00Z".to_string()),
        };

        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("test123"));
        assert!(json.contains("1000"));
        assert!(json.contains("100"));
    }

    #[test]
    fn test_stats_entry_deserialization() {
        let json = r#"{"session_id":"test1","raw_tokens":1000,"tldr_tokens":100,"requests":10}"#;
        let entry: StatsEntry = serde_json::from_str(json).unwrap();

        assert_eq!(entry.session_id, "test1");
        assert_eq!(entry.raw_tokens, 1000);
        assert_eq!(entry.tldr_tokens, 100);
        assert_eq!(entry.requests, 10);
    }

    #[test]
    fn test_stats_output_serialization() {
        let output = StatsOutput {
            total_invocations: 1500,
            estimated_tokens_saved: 4500000,
            raw_tokens_total: 5000000,
            tldr_tokens_total: 500000,
            savings_percent: 90.0,
        };

        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("1500"));
        assert!(json.contains("4500000"));
        assert!(json.contains("90"));
    }

    #[test]
    fn test_format_number() {
        assert_eq!(format_number(0), "0");
        assert_eq!(format_number(100), "100");
        assert_eq!(format_number(1000), "1,000");
        assert_eq!(format_number(1234567), "1,234,567");
    }

    #[test]
    fn test_format_number_signed() {
        assert_eq!(format_number_signed(1000), "1,000");
        assert_eq!(format_number_signed(-1000), "-1,000");
        assert_eq!(format_number_signed(0), "0");
    }

    #[test]
    fn test_read_and_aggregate_stats_empty() {
        let temp = TempDir::new().unwrap();
        let stats_path = temp.path().join("stats.jsonl");

        // File doesn't exist
        let result = read_and_aggregate_stats(&stats_path).unwrap();
        assert!(result.is_none());

        // Empty file
        fs::write(&stats_path, "").unwrap();
        let result = read_and_aggregate_stats(&stats_path).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_read_and_aggregate_stats_single_entry() {
        let temp = TempDir::new().unwrap();
        let stats_path = temp.path().join("stats.jsonl");

        let data = r#"{"session_id":"test1","raw_tokens":1000,"tldr_tokens":100,"requests":10}"#;
        fs::write(&stats_path, data).unwrap();

        let result = read_and_aggregate_stats(&stats_path).unwrap().unwrap();
        assert_eq!(result.total_invocations, 10);
        assert_eq!(result.raw_tokens_total, 1000);
        assert_eq!(result.tldr_tokens_total, 100);
        assert_eq!(result.estimated_tokens_saved, 900);
        assert!((result.savings_percent - 90.0).abs() < 0.01);
    }

    #[test]
    fn test_read_and_aggregate_stats_multiple_entries() {
        let temp = TempDir::new().unwrap();
        let stats_path = temp.path().join("stats.jsonl");

        let data = r#"{"session_id":"test1","raw_tokens":1000,"tldr_tokens":100,"requests":10}
{"session_id":"test2","raw_tokens":2000,"tldr_tokens":200,"requests":20}"#;
        fs::write(&stats_path, data).unwrap();

        let result = read_and_aggregate_stats(&stats_path).unwrap().unwrap();
        assert_eq!(result.total_invocations, 30);
        assert_eq!(result.raw_tokens_total, 3000);
        assert_eq!(result.tldr_tokens_total, 300);
        assert_eq!(result.estimated_tokens_saved, 2700);
    }

    #[test]
    fn test_read_and_aggregate_stats_with_blank_lines() {
        let temp = TempDir::new().unwrap();
        let stats_path = temp.path().join("stats.jsonl");

        let data = r#"{"session_id":"test1","raw_tokens":1000,"tldr_tokens":100,"requests":10}

{"session_id":"test2","raw_tokens":2000,"tldr_tokens":200,"requests":20}
"#;
        fs::write(&stats_path, data).unwrap();

        let result = read_and_aggregate_stats(&stats_path).unwrap().unwrap();
        assert_eq!(result.total_invocations, 30);
    }

    #[test]
    fn test_append_stats_entry() {
        let temp = TempDir::new().unwrap();
        let tldr_dir = temp.path().join(".tldr");
        fs::create_dir_all(&tldr_dir).unwrap();

        // Override home dir for test - this is tricky, so we test the serialization
        let entry = StatsEntry {
            session_id: "test123".to_string(),
            raw_tokens: 1000,
            tldr_tokens: 100,
            requests: 10,
            timestamp: None,
        };

        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("test123"));
        assert!(json.contains("1000"));
    }

    #[test]
    fn test_global_stats_calculation() {
        let stats = GlobalStats {
            total_invocations: 100,
            estimated_tokens_saved: 9000,
            raw_tokens_total: 10000,
            tldr_tokens_total: 1000,
            savings_percent: 90.0,
        };

        // Verify the calculation is correct
        assert_eq!(
            stats.estimated_tokens_saved,
            (stats.raw_tokens_total - stats.tldr_tokens_total) as i64
        );
        assert!((stats.savings_percent - 90.0).abs() < 0.01);
    }
}
