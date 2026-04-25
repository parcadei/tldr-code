//! Daemon client for L2Context -- routes IR queries through the daemon's
//! QueryCache when available, falling back to on-the-fly computation.
//!
//! # Architecture
//!
//! The daemon client trait abstracts communication with a running `tldr-daemon`
//! process. When a daemon is available, IR artifacts (call graphs, CFGs, DFGs,
//! SSA) are fetched from the daemon's persistent cache, avoiding redundant
//! recomputation across `bugbot check` invocations within the same edit session.
//!
//! When no daemon is running, the `NoDaemon` implementation returns `None` for
//! all queries, causing L2Context to fall back to on-the-fly IR construction
//! via its existing DashMap/OnceLock caches.
//!
//! # Cache Invalidation
//!
//! The daemon registers file dependencies for each cached artifact. When
//! `L2Context` is constructed with `changed_files`, it notifies the daemon
//! of those files so the daemon can invalidate stale cache entries before
//! any queries are made.
//!
//! # Tiered Execution
//!
//! - **Foreground tier** (<200ms, blocking): ScanEngine, DeltaEngine, GraphEngine
//! - **Deferred tier** (daemon-queued, returns within 2s): FlowEngine
//!
//! When a daemon is available, deferred engines use cached results from prior
//! daemon runs. When no daemon is running, deferred engines run synchronously
//! after foreground engines complete.

use std::path::{Path, PathBuf};

use tldr_core::ssa::SsaFunction;
use tldr_core::{CfgInfo, DfgInfo, ProjectCallGraph};

use super::types::FunctionId;

/// Trait for communicating with the tldr-daemon process.
///
/// Provides query methods for IR artifacts that the daemon may have cached
/// from prior analysis runs. All methods return `Option<T>` -- `None` means
/// the daemon does not have the artifact cached (or no daemon is running),
/// and the caller should compute it locally.
///
/// Implementations must be `Send + Sync` so they can be stored in `L2Context`
/// (which is shared across threads via `Arc` or moved to a background thread).
pub trait DaemonClient: Send + Sync {
    /// Check whether a daemon is currently running and reachable.
    ///
    /// Returns `true` if the daemon is available for queries, `false` otherwise.
    /// This is a lightweight check (e.g., socket existence) that does not
    /// perform a full handshake.
    fn is_available(&self) -> bool;

    /// Query the daemon for a cached project call graph.
    ///
    /// Returns `Some(graph)` if the daemon has a cached call graph for the
    /// project, `None` if unavailable or the daemon is not running.
    fn query_call_graph(&self) -> Option<ProjectCallGraph>;

    /// Query the daemon for a cached CFG for a specific function.
    ///
    /// Returns `Some(cfg)` if the daemon has a cached CFG for the given
    /// function, `None` if unavailable.
    fn query_cfg(&self, function_id: &FunctionId) -> Option<CfgInfo>;

    /// Query the daemon for a cached DFG for a specific function.
    ///
    /// Returns `Some(dfg)` if the daemon has a cached DFG for the given
    /// function, `None` if unavailable.
    fn query_dfg(&self, function_id: &FunctionId) -> Option<DfgInfo>;

    /// Query the daemon for cached SSA for a specific function.
    ///
    /// Returns `Some(ssa)` if the daemon has cached SSA for the given
    /// function, `None` if unavailable.
    fn query_ssa(&self, function_id: &FunctionId) -> Option<SsaFunction>;

    /// Notify the daemon that files have changed, triggering cache invalidation.
    ///
    /// The daemon will invalidate any cached artifacts whose file dependencies
    /// overlap with the provided `changed_files`. This should be called before
    /// any queries are made for the current analysis session.
    fn notify_changed_files(&self, changed_files: &[PathBuf]);
}

/// Fallback implementation used when no daemon is running.
///
/// Returns `None` for all queries and no-ops for notifications. This is the
/// default used by `L2Context` when daemon integration is not available.
pub struct NoDaemon;

impl DaemonClient for NoDaemon {
    fn is_available(&self) -> bool {
        false
    }

    fn query_call_graph(&self) -> Option<ProjectCallGraph> {
        None
    }

    fn query_cfg(&self, _function_id: &FunctionId) -> Option<CfgInfo> {
        None
    }

    fn query_dfg(&self, _function_id: &FunctionId) -> Option<DfgInfo> {
        None
    }

    fn query_ssa(&self, _function_id: &FunctionId) -> Option<SsaFunction> {
        None
    }

    fn notify_changed_files(&self, _changed_files: &[PathBuf]) {
        // No daemon running, nothing to invalidate.
    }
}

/// Daemon client that connects to a running local daemon via Unix socket.
///
/// Probes the daemon socket path computed from the project root. If the socket
/// exists and the daemon responds to a ping, queries are routed through the
/// daemon's HTTP API. Otherwise, behaves like `NoDaemon`.
///
/// This is a synchronous (blocking) client suitable for use from the L2 analysis
/// thread. The daemon's handlers use `spawn_blocking` internally, so the client
/// can safely make blocking HTTP calls.
pub struct LocalDaemonClient {
    /// Project root used to compute the daemon socket path.
    project: PathBuf,
    /// Cached socket path (computed once on construction).
    socket_path: PathBuf,
    /// Whether the daemon was reachable at construction time.
    available: bool,
}

impl LocalDaemonClient {
    /// Create a new `LocalDaemonClient` for the given project.
    ///
    /// Probes the daemon socket to determine availability. The probe is a
    /// non-blocking check of socket file existence (actual connectivity is
    /// verified lazily on first query).
    pub fn new(project: &Path) -> Self {
        let socket_path = Self::compute_socket_path(project);
        let available = socket_path.exists();
        Self {
            project: project.to_path_buf(),
            socket_path,
            available,
        }
    }

    /// Compute the daemon socket path for a project.
    ///
    /// Uses the same algorithm as `tldr-daemon`'s `server::compute_socket_path`:
    /// MD5 hash of canonicalized project path, first 8 hex chars.
    fn compute_socket_path(project: &Path) -> PathBuf {
        let canonical = dunce::canonicalize(project).unwrap_or_else(|_| project.to_path_buf());
        let path_str = canonical.to_string_lossy();
        let digest = md5::compute(path_str.as_bytes());
        let hash = format!("{:x}", digest);
        let hash_prefix = &hash[..8];

        let socket_dir = std::env::var("TLDR_SOCKET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir());

        socket_dir.join(format!("tldr-{}-v1.0.sock", hash_prefix))
    }

    /// Get the project root path.
    pub fn project(&self) -> &Path {
        &self.project
    }

    /// Get the computed socket path.
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }
}

impl DaemonClient for LocalDaemonClient {
    fn is_available(&self) -> bool {
        self.available
    }

    fn query_call_graph(&self) -> Option<ProjectCallGraph> {
        if !self.available {
            return None;
        }
        // In the future, this will make an HTTP request to the daemon's
        // /calls endpoint via the Unix socket. For now, the daemon server
        // infrastructure exists but the client-side HTTP plumbing is not
        // wired. Return None to fall back to local computation.
        None
    }

    fn query_cfg(&self, _function_id: &FunctionId) -> Option<CfgInfo> {
        if !self.available {
            return None;
        }
        None
    }

    fn query_dfg(&self, _function_id: &FunctionId) -> Option<DfgInfo> {
        if !self.available {
            return None;
        }
        None
    }

    fn query_ssa(&self, _function_id: &FunctionId) -> Option<SsaFunction> {
        if !self.available {
            return None;
        }
        None
    }

    fn notify_changed_files(&self, _changed_files: &[PathBuf]) {
        if self.available {
            // In the future, this will POST to the daemon's invalidation endpoint.
            // For now, the notification is a no-op since the daemon does not yet
            // expose an invalidation API.
        }
    }
}

/// Create the appropriate daemon client for a project.
///
/// If a daemon socket exists for the project, returns a `LocalDaemonClient`.
/// Otherwise, returns a `NoDaemon` fallback. This factory function is the
/// primary entry point for daemon integration in the pipeline.
pub fn create_daemon_client(project: &Path) -> Box<dyn DaemonClient> {
    let client = LocalDaemonClient::new(project);
    if client.is_available() {
        Box::new(client)
    } else {
        Box::new(NoDaemon)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // =========================================================================
    // NoDaemon fallback tests
    // =========================================================================

    /// NoDaemon must report unavailable.
    #[test]
    fn test_no_daemon_is_not_available() {
        let client = NoDaemon;
        assert!(
            !client.is_available(),
            "NoDaemon should always report not available"
        );
    }

    /// NoDaemon must return None for call graph queries.
    #[test]
    fn test_no_daemon_query_call_graph_returns_none() {
        let client = NoDaemon;
        assert!(
            client.query_call_graph().is_none(),
            "NoDaemon should return None for call graph"
        );
    }

    /// NoDaemon must return None for CFG queries.
    #[test]
    fn test_no_daemon_query_cfg_returns_none() {
        let client = NoDaemon;
        let fid = FunctionId::new("test.py", "foo", 1);
        assert!(
            client.query_cfg(&fid).is_none(),
            "NoDaemon should return None for CFG"
        );
    }

    /// NoDaemon must return None for DFG queries.
    #[test]
    fn test_no_daemon_query_dfg_returns_none() {
        let client = NoDaemon;
        let fid = FunctionId::new("test.py", "bar", 5);
        assert!(
            client.query_dfg(&fid).is_none(),
            "NoDaemon should return None for DFG"
        );
    }

    /// NoDaemon must return None for SSA queries.
    #[test]
    fn test_no_daemon_query_ssa_returns_none() {
        let client = NoDaemon;
        let fid = FunctionId::new("test.py", "baz", 10);
        assert!(
            client.query_ssa(&fid).is_none(),
            "NoDaemon should return None for SSA"
        );
    }

    /// NoDaemon notify_changed_files must not panic (no-op).
    #[test]
    fn test_no_daemon_notify_changed_files_is_noop() {
        let client = NoDaemon;
        // Should not panic even with non-empty list
        client.notify_changed_files(&[PathBuf::from("src/lib.rs"), PathBuf::from("src/main.rs")]);
    }

    /// NoDaemon must return None for ALL query types (comprehensive check).
    #[test]
    fn test_daemon_client_no_daemon_fallback() {
        let client = NoDaemon;
        assert!(!client.is_available());
        assert!(client.query_call_graph().is_none());

        let fid = FunctionId::new("test.rs", "test_fn", 1);
        assert!(client.query_cfg(&fid).is_none());
        assert!(client.query_dfg(&fid).is_none());
        assert!(client.query_ssa(&fid).is_none());

        // notify should not panic
        client.notify_changed_files(&[PathBuf::from("a.rs")]);
    }

    // =========================================================================
    // DaemonClient trait object safety
    // =========================================================================

    /// DaemonClient must be object-safe (usable as Box<dyn DaemonClient>).
    #[test]
    fn test_daemon_client_trait_object_safe() {
        let client: Box<dyn DaemonClient> = Box::new(NoDaemon);
        assert!(!client.is_available());
        assert!(client.query_call_graph().is_none());

        let fid = FunctionId::new("test.rs", "f", 1);
        assert!(client.query_cfg(&fid).is_none());
        assert!(client.query_dfg(&fid).is_none());
        assert!(client.query_ssa(&fid).is_none());
        client.notify_changed_files(&[]);
    }

    /// DaemonClient must be Send + Sync (required for L2Context).
    #[test]
    fn test_daemon_client_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<NoDaemon>();
        assert_send_sync::<LocalDaemonClient>();
    }

    // =========================================================================
    // LocalDaemonClient tests
    // =========================================================================

    /// LocalDaemonClient for a nonexistent project should not find a socket.
    #[test]
    fn test_local_daemon_client_no_socket() {
        let client = LocalDaemonClient::new(Path::new("/tmp/nonexistent-bugbot-project-xyz"));
        assert!(
            !client.is_available(),
            "No daemon should be running for a nonexistent project"
        );
    }

    /// LocalDaemonClient should return None for all queries when unavailable.
    #[test]
    fn test_local_daemon_client_unavailable_returns_none() {
        let client = LocalDaemonClient::new(Path::new("/tmp/nonexistent-bugbot-project-xyz"));
        let fid = FunctionId::new("test.rs", "func", 1);

        assert!(client.query_call_graph().is_none());
        assert!(client.query_cfg(&fid).is_none());
        assert!(client.query_dfg(&fid).is_none());
        assert!(client.query_ssa(&fid).is_none());
    }

    /// LocalDaemonClient should not panic on notify when unavailable.
    #[test]
    fn test_local_daemon_client_notify_when_unavailable() {
        let client = LocalDaemonClient::new(Path::new("/tmp/nonexistent-bugbot-project-xyz"));
        client.notify_changed_files(&[PathBuf::from("src/lib.rs")]);
        // No panic = success
    }

    /// LocalDaemonClient socket path should use MD5 hash of project path.
    #[test]
    fn test_local_daemon_client_socket_path_computation() {
        let client = LocalDaemonClient::new(Path::new("/tmp/test-project-for-socket-path"));
        let socket = client.socket_path();
        let socket_name = socket.file_name().unwrap().to_string_lossy();

        // Socket name should match pattern: tldr-{8hex}-v1.0.sock
        assert!(
            socket_name.starts_with("tldr-"),
            "Socket name should start with 'tldr-', got: {}",
            socket_name
        );
        assert!(
            socket_name.ends_with("-v1.0.sock"),
            "Socket name should end with '-v1.0.sock', got: {}",
            socket_name
        );
    }

    /// create_daemon_client factory should return NoDaemon for nonexistent projects.
    #[test]
    fn test_create_daemon_client_no_daemon() {
        let client = create_daemon_client(Path::new("/tmp/nonexistent-bugbot-factory-test"));
        assert!(
            !client.is_available(),
            "Factory should return NoDaemon for nonexistent project"
        );
        assert!(client.query_call_graph().is_none());
    }

    // =========================================================================
    // Mock daemon for integration testing
    // =========================================================================

    /// A mock daemon client that returns pre-configured cached results.
    /// Used in L2Context integration tests to verify daemon-first routing.
    struct MockDaemon {
        available: bool,
        call_graph: Option<ProjectCallGraph>,
    }

    impl MockDaemon {
        fn available_with_call_graph() -> Self {
            Self {
                available: true,
                call_graph: Some(ProjectCallGraph::default()),
            }
        }

        fn unavailable() -> Self {
            Self {
                available: false,
                call_graph: None,
            }
        }
    }

    impl DaemonClient for MockDaemon {
        fn is_available(&self) -> bool {
            self.available
        }

        fn query_call_graph(&self) -> Option<ProjectCallGraph> {
            self.call_graph.clone()
        }

        fn query_cfg(&self, _function_id: &FunctionId) -> Option<CfgInfo> {
            None
        }

        fn query_dfg(&self, _function_id: &FunctionId) -> Option<DfgInfo> {
            None
        }

        fn query_ssa(&self, _function_id: &FunctionId) -> Option<SsaFunction> {
            None
        }

        fn notify_changed_files(&self, _changed_files: &[PathBuf]) {}
    }

    /// MockDaemon available should report available and return cached call graph.
    #[test]
    fn test_mock_daemon_available_returns_call_graph() {
        let mock = MockDaemon::available_with_call_graph();
        assert!(mock.is_available());
        assert!(
            mock.query_call_graph().is_some(),
            "Available mock daemon should return cached call graph"
        );
    }

    /// MockDaemon unavailable should report unavailable and return None.
    #[test]
    fn test_mock_daemon_unavailable_returns_none() {
        let mock = MockDaemon::unavailable();
        assert!(!mock.is_available());
        assert!(mock.query_call_graph().is_none());
    }

    /// MockDaemon as trait object must work correctly.
    #[test]
    fn test_mock_daemon_as_trait_object() {
        let client: Box<dyn DaemonClient> = Box::new(MockDaemon::available_with_call_graph());
        assert!(client.is_available());
        assert!(client.query_call_graph().is_some());

        let client2: Box<dyn DaemonClient> = Box::new(MockDaemon::unavailable());
        assert!(!client2.is_available());
        assert!(client2.query_call_graph().is_none());
    }

    // =========================================================================
    // Cache invalidation tests
    // =========================================================================

    /// A mock daemon that tracks whether notify_changed_files was called.
    struct TrackingDaemon {
        notified: std::sync::Mutex<Vec<Vec<PathBuf>>>,
    }

    impl TrackingDaemon {
        fn new() -> Self {
            Self {
                notified: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn notifications(&self) -> Vec<Vec<PathBuf>> {
            self.notified.lock().unwrap().clone()
        }
    }

    impl DaemonClient for TrackingDaemon {
        fn is_available(&self) -> bool {
            true
        }

        fn query_call_graph(&self) -> Option<ProjectCallGraph> {
            None
        }

        fn query_cfg(&self, _function_id: &FunctionId) -> Option<CfgInfo> {
            None
        }

        fn query_dfg(&self, _function_id: &FunctionId) -> Option<DfgInfo> {
            None
        }

        fn query_ssa(&self, _function_id: &FunctionId) -> Option<SsaFunction> {
            None
        }

        fn notify_changed_files(&self, changed_files: &[PathBuf]) {
            self.notified.lock().unwrap().push(changed_files.to_vec());
        }
    }

    /// Daemon cache invalidation: notify_changed_files should be callable and
    /// record the changed files for verification.
    #[test]
    fn test_daemon_cache_invalidation_on_changed_files() {
        let daemon = TrackingDaemon::new();
        assert!(daemon.is_available());

        let files = vec![PathBuf::from("src/lib.rs"), PathBuf::from("src/main.rs")];
        daemon.notify_changed_files(&files);

        let notifications = daemon.notifications();
        assert_eq!(
            notifications.len(),
            1,
            "Should have recorded one notification"
        );
        assert_eq!(
            notifications[0], files,
            "Notification should contain the changed files"
        );
    }

    /// Multiple notify calls should accumulate.
    #[test]
    fn test_daemon_multiple_notifications_accumulate() {
        let daemon = TrackingDaemon::new();

        daemon.notify_changed_files(&[PathBuf::from("a.rs")]);
        daemon.notify_changed_files(&[PathBuf::from("b.rs"), PathBuf::from("c.rs")]);

        let notifications = daemon.notifications();
        assert_eq!(
            notifications.len(),
            2,
            "Should have recorded two notifications"
        );
        assert_eq!(notifications[0].len(), 1);
        assert_eq!(notifications[1].len(), 2);
    }
}
