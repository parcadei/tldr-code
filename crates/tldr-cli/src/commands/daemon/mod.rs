//! Daemon subsystem for TLDR CLI
//!
//! Provides a persistent background process that holds indexes in memory for fast
//! queries, implements Salsa-style query memoization, and tracks usage statistics.
//!
//! # Architecture
//!
//! ```text
//! +-----------------------------------------------------------------+
//! |                          CLI Layer                               |
//! |  daemon start | stop | status | query | notify | stats | warm   |
//! +-----------------------------------------------------------------+
//!                                |
//!                    +-----------v-----------+
//!                    |    IPC Transport      |
//!                    |  Unix Socket (Unix)   |
//!                    |  TCP Socket (Windows) |
//!                    +-----------+-----------+
//!                                |
//! +-----------------------------------------------------------------+
//! |                        TLDRDaemon                                |
//! |  +-------------+  +-------------+  +-------------+              |
//! |  | SalsaDB     |  | Dedup Index |  | Stats Store |              |
//! |  | (memoize)   |  | (content-   |  | (per-session|              |
//! |  |             |  |  hash)      |  |  tracking)  |              |
//! |  +-------------+  +-------------+  +-------------+              |
//! +-----------------------------------------------------------------+
//! ```
//!
//! # Modules
//!
//! - `types`: Core data types for configuration, status, and statistics
//! - `error`: Error types for daemon operations
//! - `pid`: PID file locking for daemon singleton enforcement
//! - `ipc`: IPC client/server for socket communication
//! - `salsa`: Salsa-style incremental computation cache
//! - `daemon`: Main daemon process and command handlers
//! - `start`: Daemon start command
//! - `stop`: Daemon stop command
//! - `status`: Daemon status command
//! - `query`: Low-level query passthrough command
//! - `notify`: File change notification command
//! - `warm`: Cache warming command
//! - `stats`: Usage statistics command
//! - `cache_stats`: Cache statistics command
//! - `cache_clear`: Cache clearing command

pub mod cache_clear;
pub mod cache_stats;
#[path = "daemon.rs"]
pub mod daemon_impl;
pub mod error;
pub mod ipc;
pub mod notify;
pub mod pid;
pub mod query;
pub mod salsa;
pub mod start;
pub mod stats;
pub mod status;
pub mod stop;
pub mod types;
pub mod warm;
pub use daemon_impl as daemon;

// Re-export core types for convenience
pub use error::{DaemonError, DaemonResult};
pub use ipc::{
    check_socket_alive, cleanup_socket, read_command, send_command, send_raw_command,
    send_response, validate_socket_path, IpcListener, IpcStream, CONNECTION_TIMEOUT_SECS,
    MAX_MESSAGE_SIZE, READ_TIMEOUT_SECS,
};
pub use pid::{
    check_stale_pid, cleanup_stale_pid, compute_hash, compute_pid_path, compute_socket_path,
    is_process_running, try_acquire_lock, PidGuard,
};
pub use salsa::{hash_args, hash_path, CacheEntry, QueryCache, QueryKey, DEFAULT_MAX_ENTRIES};
pub use types::{
    // Statistics
    AllSessionsSummary,
    CacheFileInfo,
    // IPC Messages
    DaemonCommand,
    // Configuration
    DaemonConfig,
    DaemonResponse,
    // Status
    DaemonStatus,
    DedupStats,
    GlobalStats,
    HookStats,
    SalsaCacheStats,
    SessionStats,
    // Constants
    DEFAULT_REINDEX_THRESHOLD,
    HOOK_FLUSH_THRESHOLD,
    IDLE_TIMEOUT,
    IDLE_TIMEOUT_SECS,
};

// Re-export daemon components
pub use daemon_impl::TLDRDaemon;

// Re-export CLI argument types for main.rs integration
pub use cache_clear::CacheClearArgs;
pub use cache_stats::CacheStatsArgs;
pub use notify::DaemonNotifyArgs;
pub use query::DaemonQueryArgs;
pub use start::DaemonStartArgs;
pub use stats::StatsArgs;
pub use status::DaemonStatusArgs;
pub use stop::DaemonStopArgs;
pub use warm::WarmArgs;
