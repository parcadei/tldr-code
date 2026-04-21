//! Core types for the TLDR daemon subsystem
//!
//! Types for daemon configuration, status, statistics, and IPC messages.
//! All types are serializable for JSON IPC communication.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

// =============================================================================
// Constants
// =============================================================================

/// Idle timeout before daemon auto-shutdown (30 minutes)
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Idle timeout in seconds for serialization
pub const IDLE_TIMEOUT_SECS: u64 = 30 * 60;

/// Default threshold for triggering semantic re-index
pub const DEFAULT_REINDEX_THRESHOLD: usize = 20;

/// Default flush interval for hook stats (every N invocations)
pub const HOOK_FLUSH_THRESHOLD: usize = 5;

// =============================================================================
// Configuration Types
// =============================================================================

/// Daemon configuration loaded from .tldr/config.json or .claude/settings.json
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DaemonConfig {
    /// Whether semantic search is enabled
    pub semantic_enabled: bool,

    /// Number of dirty files before auto re-index
    pub auto_reindex_threshold: usize,

    /// Embedding model for semantic search
    pub semantic_model: String,

    /// Idle timeout in seconds (default: 1800 = 30 min)
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

// =============================================================================
// Status Types
// =============================================================================

/// Daemon runtime status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DaemonStatus {
    /// Daemon is starting up, acquiring locks
    Initializing,
    /// Daemon is building initial indexes
    Indexing,
    /// Daemon is ready to accept queries
    Ready,
    /// Daemon is shutting down
    ShuttingDown,
    /// Daemon has stopped
    Stopped,
}

// =============================================================================
// Statistics Types
// =============================================================================

/// Statistics for Salsa-style query cache
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SalsaCacheStats {
    /// Number of cache hits (query result reused)
    pub hits: u64,

    /// Number of cache misses (query recomputed)
    pub misses: u64,

    /// Number of invalidations (file changed)
    pub invalidations: u64,

    /// Number of recomputations triggered by invalidation
    pub recomputations: u64,
}

impl SalsaCacheStats {
    /// Calculate hit rate as percentage (0-100)
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            return 0.0;
        }
        (self.hits as f64 / total as f64) * 100.0
    }
}

/// Statistics for content-hash deduplication
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DedupStats {
    /// Number of unique content hashes
    pub unique_hashes: usize,

    /// Number of duplicate content blocks avoided
    pub duplicates_avoided: usize,

    /// Bytes saved through deduplication
    pub bytes_saved: u64,
}

/// Per-session statistics for token tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStats {
    /// Session identifier (8-char truncated UUID)
    pub session_id: String,

    /// Raw tokens (what vanilla Claude would use)
    pub raw_tokens: u64,

    /// TLDR tokens (what was actually returned)
    pub tldr_tokens: u64,

    /// Number of requests in this session
    pub requests: u64,

    /// When session started (ISO 8601 timestamp)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl SessionStats {
    /// Create a new session with the given ID
    pub fn new(session_id: String) -> Self {
        Self {
            session_id,
            raw_tokens: 0,
            tldr_tokens: 0,
            requests: 0,
            started_at: Some(chrono::Utc::now()),
        }
    }

    /// Record a request's token usage
    pub fn record_request(&mut self, raw_tokens: u64, tldr_tokens: u64) {
        self.raw_tokens += raw_tokens;
        self.tldr_tokens += tldr_tokens;
        self.requests += 1;
    }

    /// Tokens saved
    pub fn savings_tokens(&self) -> i64 {
        self.raw_tokens as i64 - self.tldr_tokens as i64
    }

    /// Savings as percentage (0-100)
    pub fn savings_percent(&self) -> f64 {
        if self.raw_tokens == 0 {
            return 0.0;
        }
        (self.savings_tokens() as f64 / self.raw_tokens as f64) * 100.0
    }
}

/// Per-hook activity statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookStats {
    /// Hook name
    pub hook_name: String,

    /// Total invocations
    pub invocations: u64,

    /// Successful invocations
    pub successes: u64,

    /// Failed invocations
    pub failures: u64,

    /// Hook-specific metrics (e.g., errors_found, queries_routed)
    #[serde(default)]
    pub metrics: HashMap<String, f64>,

    /// When tracking started (ISO 8601 timestamp)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl HookStats {
    /// Create a new hook stats tracker
    pub fn new(hook_name: String) -> Self {
        Self {
            hook_name,
            invocations: 0,
            successes: 0,
            failures: 0,
            metrics: HashMap::new(),
            started_at: Some(chrono::Utc::now()),
        }
    }

    /// Record a hook invocation
    pub fn record_invocation(&mut self, success: bool, metrics: Option<HashMap<String, f64>>) {
        self.invocations += 1;
        if success {
            self.successes += 1;
        } else {
            self.failures += 1;
        }
        if let Some(m) = metrics {
            for (key, value) in m {
                *self.metrics.entry(key).or_insert(0.0) += value;
            }
        }
    }

    /// Success rate as percentage (0-100)
    pub fn success_rate(&self) -> f64 {
        if self.invocations == 0 {
            return 100.0;
        }
        (self.successes as f64 / self.invocations as f64) * 100.0
    }
}

/// Aggregated global stats (from JSONL store)
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct GlobalStats {
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

/// Cache file info for cache stats
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CacheFileInfo {
    /// Number of cache files
    pub file_count: usize,

    /// Total size in bytes
    pub total_bytes: u64,

    /// Size formatted as human-readable
    pub total_size_human: String,
}

/// Summary of all active sessions
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AllSessionsSummary {
    /// Number of active sessions
    pub active_sessions: usize,

    /// Total raw tokens across all sessions
    pub total_raw_tokens: u64,

    /// Total TLDR tokens across all sessions
    pub total_tldr_tokens: u64,

    /// Total requests across all sessions
    pub total_requests: u64,
}

// =============================================================================
// IPC Message Types
// =============================================================================

/// Command sent to daemon via socket
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum DaemonCommand {
    /// Health check
    Ping,

    /// Get daemon status
    Status {
        /// Optional session ID to get session-specific stats
        #[serde(skip_serializing_if = "Option::is_none")]
        session: Option<String>,
    },

    /// Graceful shutdown
    Shutdown,

    /// File change notification
    Notify {
        /// Path to the changed file
        file: PathBuf,
    },

    /// Track hook activity
    Track {
        /// Hook name
        hook: String,
        /// Whether invocation was successful
        #[serde(default = "default_true")]
        success: bool,
        /// Hook-specific metrics
        #[serde(default)]
        metrics: HashMap<String, f64>,
    },

    /// Warm call graph cache
    Warm {
        /// Optional language filter
        #[serde(default)]
        language: Option<String>,
    },

    /// Semantic search (if model loaded)
    Semantic {
        /// Search query
        query: String,
        /// Number of results to return
        #[serde(default = "default_top_k")]
        top_k: usize,
    },

    // Pass-through analysis commands
    /// Search for patterns in files
    Search {
        pattern: String,
        max_results: Option<usize>,
    },

    /// Extract file information
    Extract {
        file: PathBuf,
        session: Option<String>,
    },

    /// Get file tree
    Tree { path: Option<PathBuf> },

    /// Get code structure
    Structure { path: PathBuf, lang: Option<String> },

    /// Get context for entry point
    Context { entry: String, depth: Option<usize> },

    /// Get control flow graph
    Cfg { file: PathBuf, function: String },

    /// Get data flow graph
    Dfg { file: PathBuf, function: String },

    /// Get program slice
    Slice {
        file: PathBuf,
        function: String,
        line: usize,
    },

    /// Get call graph
    Calls { path: Option<PathBuf> },

    /// Get impact analysis
    Impact { func: String, depth: Option<usize> },

    /// Find dead code
    Dead {
        path: Option<PathBuf>,
        entry: Option<Vec<String>>,
    },

    /// Get architecture analysis
    Arch { path: Option<PathBuf> },

    /// Get imports for a file
    Imports { file: PathBuf },

    /// Find files that import a module
    Importers {
        module: String,
        path: Option<PathBuf>,
    },

    /// Run diagnostics
    Diagnostics {
        path: PathBuf,
        project: Option<bool>,
    },

    /// Analyze change impact
    ChangeImpact {
        files: Option<Vec<PathBuf>>,
        session: Option<bool>,
        git: Option<bool>,
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
    /// Full status response (5 required fields including typed enum status)
    FullStatus {
        status: DaemonStatus,
        uptime: f64,
        files: usize,
        project: PathBuf,
        salsa_stats: SalsaCacheStats,
        #[serde(skip_serializing_if = "Option::is_none")]
        dedup_stats: Option<DedupStats>,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_stats: Option<SessionStats>,
        #[serde(skip_serializing_if = "Option::is_none")]
        all_sessions: Option<AllSessionsSummary>,
        #[serde(skip_serializing_if = "Option::is_none")]
        hook_stats: Option<HashMap<String, HookStats>>,
    },

    /// Notify response (4 required fields)
    NotifyResponse {
        status: String,
        dirty_count: usize,
        threshold: usize,
        reindex_triggered: bool,
    },

    /// Track response
    TrackResponse {
        status: String,
        hook: String,
        total_invocations: u64,
        flushed: bool,
    },

    /// Error response (uses "error" field to distinguish from Status)
    Error { status: String, error: String },

    /// Simple status response (catch-all with only 1 required field)
    Status {
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// Generic JSON result (for analysis commands) - MUST be last (catch-all)
    Result(serde_json::Value),
}

#[cfg(test)]
mod tests {
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
    fn test_daemon_config_serialize_deserialize() {
        let config = DaemonConfig::default();
        let json = serde_json::to_string(&config).unwrap();

        assert!(json.contains("semantic_enabled"));
        assert!(json.contains("auto_reindex_threshold"));
        assert!(json.contains("20")); // DEFAULT_REINDEX_THRESHOLD

        // Deserialize back
        let parsed: DaemonConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config, parsed);
    }

    #[test]
    fn test_daemon_status_serialization() {
        let status = DaemonStatus::Ready;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, r#""ready""#);

        let status = DaemonStatus::Initializing;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, r#""initializing""#);

        let status = DaemonStatus::ShuttingDown;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, r#""shutting_down""#);
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
            started_at: None,
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
            started_at: None,
        };

        assert_eq!(stats.savings_tokens(), 0);
        assert_eq!(stats.savings_percent(), 0.0);
    }

    #[test]
    fn test_hook_stats_success_rate() {
        let mut stats = HookStats::new("test-hook".to_string());
        stats.record_invocation(true, None);
        stats.record_invocation(true, None);
        stats.record_invocation(false, None);

        assert_eq!(stats.invocations, 3);
        assert_eq!(stats.successes, 2);
        assert_eq!(stats.failures, 1);
        assert!((stats.success_rate() - 66.67).abs() < 0.1);
    }

    #[test]
    fn test_hook_stats_metrics_accumulation() {
        let mut stats = HookStats::new("test-hook".to_string());

        let mut metrics = HashMap::new();
        metrics.insert("errors_found".to_string(), 3.0);
        stats.record_invocation(true, Some(metrics));

        let mut metrics2 = HashMap::new();
        metrics2.insert("errors_found".to_string(), 2.0);
        stats.record_invocation(true, Some(metrics2));

        assert_eq!(*stats.metrics.get("errors_found").unwrap(), 5.0);
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
    fn test_cache_file_info_fields() {
        let info = CacheFileInfo {
            file_count: 25,
            total_bytes: 1048576,
            total_size_human: "1.0 MB".to_string(),
        };

        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("file_count"));
        assert!(json.contains("25"));
        assert!(json.contains("total_bytes"));
        assert!(json.contains("1.0 MB"));
    }

    #[test]
    fn test_global_stats_fields() {
        let stats = GlobalStats {
            total_invocations: 1500,
            estimated_tokens_saved: 4500000,
            raw_tokens_total: 5000000,
            tldr_tokens_total: 500000,
            savings_percent: 90.0,
        };

        let json = serde_json::to_string(&stats).unwrap();
        assert!(json.contains("total_invocations"));
        assert!(json.contains("estimated_tokens_saved"));
        assert!(json.contains("savings_percent"));
    }

    #[test]
    fn test_all_sessions_summary_fields() {
        let summary = AllSessionsSummary {
            active_sessions: 3,
            total_raw_tokens: 500000,
            total_tldr_tokens: 50000,
            total_requests: 200,
        };

        let json = serde_json::to_string(&summary).unwrap();
        assert!(json.contains("active_sessions"));
        assert!(json.contains("total_raw_tokens"));
        assert!(json.contains("total_requests"));
    }

    #[test]
    fn test_dedup_stats_fields() {
        let stats = DedupStats {
            unique_hashes: 500,
            duplicates_avoided: 120,
            bytes_saved: 1048576,
        };

        let json = serde_json::to_string(&stats).unwrap();
        assert!(json.contains("unique_hashes"));
        assert!(json.contains("duplicates_avoided"));
        assert!(json.contains("bytes_saved"));
    }
}
