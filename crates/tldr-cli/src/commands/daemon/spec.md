# TLDR Daemon & Cache Commands - Behavioral Specification

**Version:** 1.0  
**Date:** 2026-02-03  
**Author:** architect-agent  
**Source Analysis:** Python implementation in `tldr/daemon/`, gap analysis in `thoughts/shared/gap-analysis/tldr-daemon-cache-gap.yaml`

## Overview

The daemon subsystem provides a persistent background process that holds indexes in memory for fast queries, implements Salsa-style query memoization, and tracks usage statistics. This specification covers 9 commands: 5 daemon lifecycle commands and 4 infrastructure commands.

### Architecture Summary

```
┌─────────────────────────────────────────────────────────────────────┐
│                           CLI Layer                                  │
│  daemon start | stop | status | query | notify | stats | warm | ... │
└───────────────────────────────┬─────────────────────────────────────┘
                                │
                    ┌───────────▼───────────┐
                    │    IPC Transport      │
                    │  Unix Socket (Unix)   │
                    │  TCP Socket (Windows) │
                    └───────────┬───────────┘
                                │
┌───────────────────────────────▼─────────────────────────────────────┐
│                        TLDRDaemon                                    │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐                  │
│  │ SalsaDB     │  │ Dedup Index │  │ Stats Store │                  │
│  │ (memoize)   │  │ (content-   │  │ (per-session│                  │
│  │             │  │  hash)      │  │  tracking)  │                  │
│  └─────────────┘  └─────────────┘  └─────────────┘                  │
│                                                                      │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │                    Command Handlers                           │   │
│  │  ping | status | shutdown | search | extract | impact | ...  │   │
│  └──────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────┘
```

## Types

### Core Types

```rust
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::RwLock;

/// Idle timeout before daemon auto-shutdown (30 minutes)
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Default threshold for triggering semantic re-index
pub const DEFAULT_REINDEX_THRESHOLD: usize = 20;

/// Default flush interval for hook stats (every N invocations)
pub const HOOK_FLUSH_THRESHOLD: usize = 5;

/// Daemon configuration loaded from .tldr/config.json or .claude/settings.json
#[derive(Debug, Clone, Serialize, Deserialize)]
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
            idle_timeout_secs: IDLE_TIMEOUT.as_secs(),
        }
    }
}

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

/// Statistics for Salsa-style query cache
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    
    /// When session started
    pub started_at: chrono::DateTime<chrono::Utc>,
}

impl SessionStats {
    pub fn new(session_id: String) -> Self {
        Self {
            session_id,
            raw_tokens: 0,
            tldr_tokens: 0,
            requests: 0,
            started_at: chrono::Utc::now(),
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
    pub metrics: HashMap<String, f64>,
    
    /// When tracking started
    pub started_at: chrono::DateTime<chrono::Utc>,
}

impl HookStats {
    pub fn new(hook_name: String) -> Self {
        Self {
            hook_name,
            invocations: 0,
            successes: 0,
            failures: 0,
            metrics: HashMap::new(),
            started_at: chrono::Utc::now(),
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheFileInfo {
    /// Number of cache files
    pub file_count: usize,
    
    /// Total size in bytes
    pub total_bytes: u64,
    
    /// Size formatted as human-readable
    pub total_size_human: String,
}
```

### Error Types

```rust
/// Daemon-specific errors
#[derive(Debug, Error)]
pub enum DaemonError {
    #[error("daemon already running (PID: {pid})")]
    AlreadyRunning { pid: u32 },
    
    #[error("daemon not running")]
    NotRunning,
    
    #[error("failed to acquire PID file lock: {0}")]
    LockFailed(io::Error),
    
    #[error("failed to bind socket: {0}")]
    SocketBindFailed(io::Error),
    
    #[error("address already in use: {addr}")]
    AddressInUse { addr: String },
    
    #[error("connection refused")]
    ConnectionRefused,
    
    #[error("connection timeout after {timeout_secs}s")]
    ConnectionTimeout { timeout_secs: u64 },
    
    #[error("invalid IPC message: {0}")]
    InvalidMessage(String),
    
    #[error("unknown command: {cmd}")]
    UnknownCommand { cmd: String },
    
    #[error("missing required parameter: {param}")]
    MissingParameter { param: String },
    
    #[error("permission denied: {path}")]
    PermissionDenied { path: PathBuf },
    
    #[error("stale PID file (process {pid} not running)")]
    StalePidFile { pid: u32 },
    
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Result type for daemon operations
pub type DaemonResult<T> = Result<T, DaemonError>;
```

### IPC Message Types

```rust
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
        file: PathBuf,
    },
    
    /// Track hook activity
    Track {
        hook: String,
        #[serde(default = "default_true")]
        success: bool,
        #[serde(default)]
        metrics: HashMap<String, f64>,
    },
    
    /// Warm call graph cache
    Warm {
        #[serde(default)]
        language: Option<String>,
    },
    
    /// Semantic search (if model loaded)
    Semantic {
        query: String,
        #[serde(default = "default_top_k")]
        top_k: usize,
    },
    
    // Pass-through analysis commands
    Search { pattern: String, max_results: Option<usize> },
    Extract { file: PathBuf, session: Option<String> },
    Tree { path: Option<PathBuf> },
    Structure { path: PathBuf, lang: Option<String> },
    Context { entry: String, depth: Option<usize> },
    Cfg { file: PathBuf, function: String },
    Dfg { file: PathBuf, function: String },
    Slice { file: PathBuf, function: String, line: usize },
    Calls { path: Option<PathBuf> },
    Impact { func: String, depth: Option<usize> },
    Dead { path: Option<PathBuf>, entry: Option<Vec<String>> },
    Arch { path: Option<PathBuf> },
    Imports { file: PathBuf },
    Importers { module: String, path: Option<PathBuf> },
    Diagnostics { path: PathBuf, project: Option<bool> },
    ChangeImpact { files: Option<Vec<PathBuf>>, session: Option<bool>, git: Option<bool> },
}

fn default_true() -> bool { true }
fn default_top_k() -> usize { 10 }

/// Response from daemon
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DaemonResponse {
    /// Simple status response
    Status {
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    
    /// Full status response
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
        all_sessions: AllSessionsSummary,
        hook_stats: HashMap<String, HookStats>,
    },
    
    /// Notify response
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
    
    /// Generic JSON result (for analysis commands)
    Result {
        status: String,
        #[serde(flatten)]
        data: serde_json::Value,
    },
    
    /// Error response
    Error {
        status: String,
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllSessionsSummary {
    pub active_sessions: usize,
    pub total_raw_tokens: u64,
    pub total_tldr_tokens: u64,
    pub total_requests: u64,
}
```

## Commands

### daemon start

**CLI:** `tldr daemon start [--project PATH] [--foreground]`

**Arguments:**
| Arg | Short | Type | Default | Description |
|-----|-------|------|---------|-------------|
| `--project` | `-p` | Path | `.` | Project root directory |
| `--foreground` | `-f` | bool | false | Run in foreground (don't daemonize) |

**Behavior:**

1. Resolve project path to absolute path
2. Compute deterministic paths:
   - PID file: `/tmp/tldr-{hash}.pid` where hash = MD5(project_path)[:8]
   - Socket: `/tmp/tldr-{hash}.sock` (Unix) or TCP port 49152+hash%10000 (Windows)
3. Try to acquire exclusive lock on PID file:
   - **Unix:** `fcntl::flock(LOCK_EX | LOCK_NB)`
   - **Windows:** `LockFileEx(LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY)`
4. If lock fails → print "Daemon already running", exit
5. Ensure `.tldrignore` exists (create with defaults if missing)
6. If `--foreground`:
   - Write PID to locked file
   - Run daemon main loop directly
7. Else (background mode):
   - **Unix:** Fork with `os::fork()`, child calls `setsid()`, runs daemon
   - **Windows:** Spawn detached subprocess via `Command::new().creation_flags(DETACHED_PROCESS)`
8. Wait up to 10s for socket to become connectable
9. Print status: "Daemon started with PID {pid}\nSocket: {socket_path}"

**Success Output:**
```json
{
  "status": "ok",
  "pid": 12345,
  "socket": "/tmp/tldr-a1b2c3d4.sock"
}
```

**Errors:**
| Error | Message | Exit Code |
|-------|---------|-----------|
| `AlreadyRunning` | "Daemon already running" | 1 |
| `LockFailed` | "Failed to acquire PID file lock: {reason}" | 1 |
| `SocketBindFailed` | "Failed to bind socket: {reason}" | 1 |
| `PermissionDenied` | "Permission denied: {path}" | 1 |

**Invariants:**
- Only one daemon per project root (enforced by PID file lock)
- Lock held for daemon's entire lifetime
- Socket file cleaned up on graceful shutdown

---

### daemon stop

**CLI:** `tldr daemon stop [--project PATH]`

**Arguments:**
| Arg | Short | Type | Default | Description |
|-----|-------|------|---------|-------------|
| `--project` | `-p` | Path | `.` | Project root directory |

**Behavior:**

1. Resolve project path
2. Compute socket path from project hash
3. Connect to daemon socket (timeout: 5s)
4. Send `{"cmd": "shutdown"}` message
5. Daemon responds with `{"status": "shutting_down"}`
6. Daemon persists all stats before exiting
7. Daemon releases PID file lock, removes socket file
8. Print "Daemon stopped"

**Success Output:**
```json
{
  "status": "ok",
  "message": "Daemon stopped"
}
```

**Errors:**
| Error | Message | Exit Code |
|-------|---------|-----------|
| `NotRunning` | "Daemon not running" | 0 (not an error) |
| `ConnectionTimeout` | "Connection timeout" | 1 |

**Text Output:**
```
Daemon stopped
```
or
```
Daemon not running
```

---

### daemon status

**CLI:** `tldr daemon status [--project PATH] [--session SESSION_ID]`

**Arguments:**
| Arg | Short | Type | Default | Description |
|-----|-------|------|---------|-------------|
| `--project` | `-p` | Path | `.` | Project root directory |
| `--session` | `-s` | String | None | Session ID for session-specific stats |

**Behavior:**

1. Resolve project path
2. Connect to daemon socket
3. Send `{"cmd": "status", "session": "<session_id>"}` if session provided
4. Receive and display:
   - `status`: "running" | "initializing" | "indexing"
   - `uptime`: seconds (formatted as Xh Xm Xs)
   - `files`: indexed file count
   - `salsa_stats`: cache hit/miss/invalidation counts
   - `dedup_stats`: deduplication metrics (if loaded)
   - `session_stats`: per-session token stats (if session_id provided)
   - `all_sessions`: summary across all sessions
   - `hook_stats`: per-hook activity metrics

**Success Output (JSON):**
```json
{
  "status": "ready",
  "uptime": 3600.5,
  "uptime_human": "1h 0m 0s",
  "files": 150,
  "project": "/path/to/project",
  "salsa_stats": {
    "hits": 1234,
    "misses": 56,
    "hit_rate": 95.67,
    "invalidations": 10,
    "recomputations": 8
  },
  "dedup_stats": {
    "unique_hashes": 500,
    "duplicates_avoided": 120,
    "bytes_saved": 1048576
  },
  "all_sessions": {
    "active_sessions": 3,
    "total_raw_tokens": 500000,
    "total_tldr_tokens": 50000,
    "total_requests": 200
  }
}
```

**Text Output:**
```
TLDR Daemon Status
==================
Status:  running
Uptime:  1h 0m 0s
Project: /path/to/project
Files:   150

Cache Statistics
----------------
Hits:          1,234
Misses:        56
Hit Rate:      95.67%
Invalidations: 10

Session Summary
---------------
Active Sessions:    3
Total Requests:     200
Tokens Saved:       450,000 (90.0%)
```

**Errors:**
| Error | Message | Exit Code |
|-------|---------|-----------|
| `NotRunning` | "Daemon not running" | 0 |

---

### daemon query

**CLI:** `tldr daemon query CMD [--project PATH] [--json PARAMS]`

**Arguments:**
| Arg | Short | Type | Default | Description |
|-----|-------|------|---------|-------------|
| `cmd` | - | String | required | Command name to send |
| `--project` | `-p` | Path | `.` | Project root directory |
| `--json` | `-j` | String | None | Additional JSON parameters |

**Behavior:**

1. Resolve project path
2. Connect to daemon socket
3. Build command: `{"cmd": "<cmd>", ...<parsed_json_params>}`
4. Send command, receive raw JSON response
5. Print response (pass-through)

**Supported Commands:**
- `ping` - health check
- `status` - daemon status
- `shutdown` - graceful shutdown
- `notify` - file change notification (requires `file` param)
- `track` - hook activity (requires `hook` param)
- `warm` - warm cache (optional `language` param)
- `semantic` - semantic search (requires `query` param)
- Plus all analysis commands: `search`, `extract`, `tree`, `structure`, `context`, `cfg`, `dfg`, `slice`, `calls`, `impact`, `dead`, `arch`, `imports`, `importers`, `diagnostics`, `change_impact`

**Example:**
```bash
tldr daemon query search --json '{"pattern": "fn main"}'
```

**Success Output:** Raw JSON response from daemon

**Errors:**
| Error | Message | Exit Code |
|-------|---------|-----------|
| `NotRunning` | "Error: Daemon not running" | 1 |
| `InvalidMessage` | "Invalid JSON parameters: {reason}" | 1 |

---

### daemon notify

**CLI:** `tldr daemon notify FILE [--project PATH]`

**Arguments:**
| Arg | Short | Type | Default | Description |
|-----|-------|------|---------|-------------|
| `file` | - | Path | required | Path to changed file |
| `--project` | `-p` | Path | `.` | Project root directory |

**Behavior:**

1. Resolve project and file paths to absolute
2. Connect to daemon socket (fail silently if daemon not running)
3. Send `{"cmd": "notify", "file": "<absolute_path>"}`
4. Daemon:
   - Adds file to dirty set (if not already tracked)
   - Increments dirty count
   - Invalidates Salsa cache entries for this file
   - Updates dedup index if loaded
5. If `dirty_count >= threshold` (default 20):
   - Triggers background semantic re-index
   - Clears dirty set after spawn
6. Return response with dirty count and threshold

**Success Output:**
```json
{
  "status": "ok",
  "dirty_count": 5,
  "threshold": 20,
  "reindex_triggered": false
}
```

**Text Output:**
```
Tracked: 5/20 files
```
or
```
Reindex triggered (20/20 files)
```

**Errors:**
- On `ConnectionRefused` or socket not found: silently return (exit 0)
- File edits should never fail due to daemon status

**Use Case:** Editor hooks call this on file save to keep daemon cache fresh.

---

### stats

**CLI:** `tldr stats [--format json|text]`

**Arguments:**
| Arg | Short | Type | Default | Description |
|-----|-------|------|---------|-------------|
| `--format` | `-f` | String | `json` | Output format |

**Behavior:**

1. Load stats from `~/.tldr/stats.jsonl`
2. Aggregate totals across all entries:
   - `total_invocations`: sum of `requests`
   - `raw_tokens_total`: sum of `raw_tokens`
   - `tldr_tokens_total`: sum of `tldr_tokens`
3. Calculate:
   - `estimated_tokens_saved = raw_tokens_total - tldr_tokens_total`
   - `savings_percent = (estimated_tokens_saved / raw_tokens_total) * 100`
4. Return aggregated stats

**Success Output (JSON):**
```json
{
  "total_invocations": 1500,
  "estimated_tokens_saved": 4500000,
  "raw_tokens_total": 5000000,
  "tldr_tokens_total": 500000,
  "savings_percent": 90.0
}
```

**Text Output:**
```
TLDR Usage Statistics
=====================
Total Invocations:     1,500
Tokens Saved:          4,500,000 (90.0%)
Raw Tokens Processed:  5,000,000
TLDR Tokens Returned:  500,000
```

**Empty State:**
```json
{
  "message": "No usage recorded yet"
}
```

---

### warm

**CLI:** `tldr warm PATH [--background] [--lang LANG]`

**Arguments:**
| Arg | Short | Type | Default | Description |
|-----|-------|------|---------|-------------|
| `path` | - | Path | required | Project root directory |
| `--background` | `-b` | bool | false | Run in background process |
| `--lang` | `-l` | String | `all` | Language to analyze |

**Behavior:**

1. If `--background`:
   - Spawn detached subprocess: `tldr warm <path> --lang <lang>`
   - Print "Warming cache in background..."
   - Return immediately
2. Foreground mode:
   - Ensure `.tldrignore` exists
   - If `lang == "all"`: auto-detect languages in project
   - For each language:
     - Scan project for files
     - Build call graph edges
   - Create `.tldr/cache/` directory
   - Write `call_graph.json`:
     ```json
     {
       "edges": [{"from_file": "...", "from_func": "...", "to_file": "...", "to_func": "..."}, ...],
       "languages": ["python", "typescript"],
       "timestamp": 1706918400
     }
     ```
   - Print summary: "Indexed {N} files, found {M} edges"

**Success Output:**
```json
{
  "status": "ok",
  "files": 150,
  "edges": 2500,
  "languages": ["python", "typescript"],
  "cache_path": ".tldr/cache/call_graph.json"
}
```

**Text Output:**
```
Warming call graph cache...
Indexed 150 files, found 2,500 edges
Languages: python, typescript
Cache written to: .tldr/cache/call_graph.json
```

---

### cache stats

**CLI:** `tldr cache stats [--project PATH]`

**Arguments:**
| Arg | Short | Type | Default | Description |
|-----|-------|------|---------|-------------|
| `--project` | `-p` | Path | `.` | Project root directory |

**Behavior:**

1. Resolve project path
2. Look for `.tldr/cache/salsa_stats.json`
3. If exists, read and display:
   - `cache_hits`
   - `cache_misses`
   - `hit_rate`
   - `invalidations`
   - `recomputations`
4. Scan `.tldr/cache/*.pkl` files:
   - Count files
   - Sum sizes
5. Return combined stats

**Success Output:**
```json
{
  "salsa_stats": {
    "hits": 1234,
    "misses": 56,
    "hit_rate": 95.67,
    "invalidations": 10,
    "recomputations": 8
  },
  "cache_files": {
    "file_count": 25,
    "total_bytes": 1048576,
    "total_size_human": "1.0 MB"
  }
}
```

**Text Output:**
```
Cache Statistics
================
Salsa Cache:
  Hits:          1,234
  Misses:        56
  Hit Rate:      95.67%
  Invalidations: 10
  Recomputations: 8

Cache Files:
  Count: 25 files
  Size:  1.0 MB
```

**Empty State:**
```
No cache statistics found
```

---

### cache clear

**CLI:** `tldr cache clear [--project PATH]`

**Arguments:**
| Arg | Short | Type | Default | Description |
|-----|-------|------|---------|-------------|
| `--project` | `-p` | Path | `.` | Project root directory |

**Behavior:**

1. Resolve project path
2. Locate `.tldr/cache/` directory
3. If exists:
   - Delete `salsa_stats.json` if exists
   - Delete all `*.pkl` files
   - Delete `call_graph.json` if exists
   - Count deleted files
4. Print result

**Success Output:**
```json
{
  "status": "ok",
  "files_removed": 26,
  "message": "Cache cleared: 26 file(s) removed"
}
```

**Text Output:**
```
Cache cleared: 26 file(s) removed
```

**Empty State:**
```
No cache directory found
```

---

## IPC Protocol

### Socket Path Computation

```rust
fn compute_socket_path(project: &Path) -> PathBuf {
    use md5::{Md5, Digest};
    
    let project_str = project.canonicalize()
        .unwrap_or_else(|_| project.to_path_buf())
        .to_string_lossy()
        .to_string();
    
    let mut hasher = Md5::new();
    hasher.update(project_str.as_bytes());
    let hash = hasher.finalize();
    let hash_str = format!("{:x}", hash)[..8].to_string();
    
    let tmp_dir = std::env::temp_dir();
    tmp_dir.join(format!("tldr-{}.sock", hash_str))
}

fn compute_pid_path(project: &Path) -> PathBuf {
    // Same hash computation
    let hash_str = /* ... */;
    let tmp_dir = std::env::temp_dir();
    tmp_dir.join(format!("tldr-{}.pid", hash_str))
}

fn compute_tcp_port(project: &Path) -> u16 {
    // For Windows
    let hash_str = /* ... */;
    let hash_int = u64::from_str_radix(&hash_str, 16).unwrap_or(0);
    49152 + (hash_int % 10000) as u16
}
```

### Message Format

- **Transport:** Newline-delimited JSON over Unix socket (Unix) or TCP (Windows)
- **Request:** `{"cmd": "...", ...params}\n`
- **Response:** `{...}\n`
- **Encoding:** UTF-8
- **Max message size:** 64KB (client recv buffer)

### Connection Handling

```rust
// Client side
async fn send_command(project: &Path, cmd: DaemonCommand) -> DaemonResult<DaemonResponse> {
    let socket_path = compute_socket_path(project);
    
    #[cfg(unix)]
    let stream = tokio::net::UnixStream::connect(&socket_path).await?;
    
    #[cfg(windows)]
    let stream = tokio::net::TcpStream::connect(("127.0.0.1", compute_tcp_port(project))).await?;
    
    let (reader, mut writer) = stream.into_split();
    
    // Send command
    let msg = serde_json::to_string(&cmd)?;
    writer.write_all(msg.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    
    // Read response
    let mut reader = tokio::io::BufReader::new(reader);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    
    let response: DaemonResponse = serde_json::from_str(&line)?;
    Ok(response)
}
```

---

## Cache Behavior

### Salsa-Style Memoization

The daemon implements Salsa-style incremental computation:

1. **Query Functions:** Each analysis (search, extract, etc.) is wrapped in a memoizing query function
2. **Input Tracking:** Queries declare which files they depend on
3. **Revision Tracking:** Each file has a revision number (incremented on change)
4. **Automatic Invalidation:** When a file's revision changes, dependent queries are invalidated
5. **Lazy Recomputation:** Invalidated queries are recomputed on next access

```rust
/// Salsa-style database for query memoization
pub struct SalsaDB {
    /// Cache of query results: (query_key, args_hash) -> (result, revision)
    cache: DashMap<(String, u64), (serde_json::Value, u64)>,
    
    /// File revisions: file_path -> revision
    file_revisions: DashMap<PathBuf, u64>,
    
    /// Reverse dependencies: file_path -> Set<(query_key, args_hash)>
    dependents: DashMap<PathBuf, HashSet<(String, u64)>>,
    
    /// Statistics
    stats: RwLock<SalsaCacheStats>,
}

impl SalsaDB {
    /// Execute a query with memoization
    pub fn query<F, R>(&self, key: &str, args: impl Hash, deps: &[PathBuf], f: F) -> R
    where
        F: FnOnce() -> R,
        R: Serialize + DeserializeOwned,
    {
        let args_hash = hash(args);
        let cache_key = (key.to_string(), args_hash);
        
        // Check cache
        if let Some(entry) = self.cache.get(&cache_key) {
            let (result, cached_rev) = entry.value();
            if self.is_valid(deps, *cached_rev) {
                self.stats.write().unwrap().hits += 1;
                return serde_json::from_value(result.clone()).unwrap();
            }
        }
        
        // Cache miss - compute
        self.stats.write().unwrap().misses += 1;
        let result = f();
        let current_rev = self.current_revision(deps);
        
        // Store in cache
        self.cache.insert(cache_key.clone(), (serde_json::to_value(&result).unwrap(), current_rev));
        
        // Track dependencies
        for dep in deps {
            self.dependents.entry(dep.clone()).or_default().insert(cache_key.clone());
        }
        
        result
    }
    
    /// Notify that a file has changed
    pub fn set_file(&self, path: &Path, _content: &str) {
        // Increment revision
        let new_rev = self.file_revisions.entry(path.to_path_buf())
            .and_modify(|r| *r += 1)
            .or_insert(1);
        
        // Invalidate dependents
        if let Some(deps) = self.dependents.get(path) {
            let mut stats = self.stats.write().unwrap();
            for cache_key in deps.value() {
                if self.cache.remove(cache_key).is_some() {
                    stats.invalidations += 1;
                }
            }
        }
    }
}
```

### Cache Persistence

- **Salsa stats:** `.tldr/cache/salsa_stats.json`
- **Call graph cache:** `.tldr/cache/call_graph.json`
- **Pickle files:** `.tldr/cache/*.pkl` (optional, for large results)

### Invalidation Rules

| Trigger | Action |
|---------|--------|
| File modified (notify command) | Increment file revision, invalidate dependent queries |
| File deleted | Remove from revision map, invalidate dependents |
| Cache clear command | Remove all cache files and reset stats |
| Daemon restart | In-memory cache cleared, stats persist |

---

## PID File Locking

### Lock Acquisition

```rust
#[cfg(unix)]
fn try_acquire_lock(pid_path: &Path) -> DaemonResult<std::fs::File> {
    use std::os::unix::io::AsRawFd;
    
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(pid_path)?;
    
    // Non-blocking exclusive lock
    let ret = unsafe {
        libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB)
    };
    
    if ret != 0 {
        return Err(DaemonError::AlreadyRunning { pid: read_pid(&file)? });
    }
    
    Ok(file)
}

#[cfg(windows)]
fn try_acquire_lock(pid_path: &Path) -> DaemonResult<std::fs::File> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::*;
    
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(pid_path)?;
    
    let mut overlapped = unsafe { std::mem::zeroed::<OVERLAPPED>() };
    let ret = unsafe {
        LockFileEx(
            file.as_raw_handle() as _,
            LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
            0,
            1,
            0,
            &mut overlapped,
        )
    };
    
    if ret == 0 {
        return Err(DaemonError::AlreadyRunning { pid: read_pid(&file)? });
    }
    
    Ok(file)
}
```

### Stale PID Detection

```rust
fn is_process_running(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // Signal 0 checks if process exists without sending signal
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
    
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::*;
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return false;
            }
            CloseHandle(handle);
            true
        }
    }
}

fn check_daemon_alive(project: &Path) -> DaemonResult<bool> {
    let pid_path = compute_pid_path(project);
    
    match try_acquire_lock(&pid_path) {
        Ok(file) => {
            // We got the lock - check for stale PID
            if let Ok(content) = std::fs::read_to_string(&pid_path) {
                if let Ok(pid) = content.trim().parse::<u32>() {
                    if is_process_running(pid) {
                        // Process exists but we got lock? Shouldn't happen.
                        // Release and report alive.
                        drop(file);
                        return Ok(true);
                    }
                }
            }
            // Stale PID or no PID - daemon not running
            drop(file);
            Ok(false)
        }
        Err(DaemonError::AlreadyRunning { .. }) => Ok(true),
        Err(e) => Err(e),
    }
}
```

---

## Edge Cases

### Daemon Crash Recovery

1. **Stale Socket File:**
   - On bind failure with EADDRINUSE, try to connect to existing socket
   - If connection refused → stale socket, unlink and retry bind
   - If connection succeeds → another daemon running, exit

2. **Stale PID File:**
   - Lock acquisition succeeds but PID file has content
   - Check if PID process is running
   - If not running → stale, overwrite with new PID
   - If running but we got lock → edge case, treat as not running

3. **Stats Persistence on Crash:**
   - atexit handler registered to persist stats
   - Also persist periodically (every 10 requests or 5 hook invocations)
   - Stats are append-only JSONL, so partial writes don't corrupt

### Concurrent Access

1. **Multiple Clients:**
   - Daemon uses single-threaded async loop (tokio)
   - Commands processed sequentially
   - ThreadPoolExecutor (4 workers) for CPU-intensive operations

2. **File Notifications During Query:**
   - SalsaDB uses DashMap for thread-safe access
   - Invalidation can happen between cache check and result return
   - Acceptable: query returns stale result, next query will recompute

### Cross-Platform Differences

| Feature | Unix (Linux/macOS) | Windows |
|---------|-------------------|---------|
| Socket | Unix domain socket | TCP localhost |
| Daemonize | `fork()` + `setsid()` | `DETACHED_PROCESS` |
| PID lock | `flock()` | `LockFileEx()` |
| Process check | `kill(pid, 0)` | `OpenProcess()` |
| Signal handling | SIGTERM, SIGINT | SIGINT only |

---

## Dependencies

### Crate Dependencies

```toml
[dependencies]
# Async runtime
tokio = { version = "1", features = ["full", "net", "io-util", "sync", "time", "signal"] }

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# CLI
clap = { version = "4", features = ["derive"] }

# Error handling
thiserror = "1"
anyhow = "1"

# Hashing
md5 = "0.7"

# Concurrent data structures
dashmap = "5"

# Time
chrono = { version = "0.4", features = ["serde"] }

# Platform-specific
[target.'cfg(unix)'.dependencies]
libc = "0.2"

[target.'cfg(windows)'.dependencies]
windows-sys = { version = "0.52", features = ["Win32_Storage_FileSystem", "Win32_System_Threading"] }
```

### Internal Dependencies

- `tldr-core`: Analysis functions (extract, structure, calls, etc.)
- Existing command modules for pass-through queries

---

## Testing Strategy

### Unit Tests

1. **Socket path computation:** Verify deterministic hashing
2. **PID file locking:** Test lock acquisition and release
3. **SalsaDB:** Test cache hit/miss, invalidation cascades
4. **Stats aggregation:** Test token counting and percentage calculation

### Integration Tests

1. **Daemon lifecycle:** start → status → stop
2. **Query routing:** Send each command type, verify response format
3. **Cache behavior:** Query, modify file, query again
4. **Concurrent clients:** Multiple connections simultaneously
5. **Crash recovery:** Kill daemon, verify restart works

### Platform Tests

1. **Unix socket binding:** macOS and Linux
2. **Windows TCP binding:** Windows
3. **Daemonization:** Background process spawning
4. **Signal handling:** SIGTERM graceful shutdown

---

## Implementation Phases

### Phase 1: Foundation (Types & IPC)
- [ ] Define all types in `types.rs`
- [ ] Implement socket path computation
- [ ] Implement IPC client (send_command)
- [ ] Basic daemon start (foreground only)

### Phase 2: Lifecycle Commands
- [ ] `daemon start` with full daemonization
- [ ] `daemon stop` with graceful shutdown
- [ ] `daemon status` with basic info
- [ ] PID file locking

### Phase 3: Query Infrastructure
- [ ] Command routing in daemon
- [ ] SalsaDB implementation
- [ ] Pass-through to tldr-core functions

### Phase 4: Stats & Monitoring
- [ ] SessionStats tracking
- [ ] HookStats tracking
- [ ] `stats` command
- [ ] `daemon notify`

### Phase 5: Cache Commands
- [ ] `warm` command
- [ ] `cache stats` command
- [ ] `cache clear` command
- [ ] Cache persistence

### Phase 6: Polish
- [ ] Windows support
- [ ] Cross-platform testing
- [ ] Documentation
- [ ] Performance optimization
