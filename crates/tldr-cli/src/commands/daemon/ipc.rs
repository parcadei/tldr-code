//! Cross-platform IPC layer for daemon communication
//!
//! This module provides socket-based IPC for the TLDR daemon using:
//! - Unix domain sockets on Unix systems (Linux, macOS)
//! - TCP localhost connections on Windows
//!
//! # Security Mitigations
//!
//! - TIGER-P3-01: Socket path validation (no temp dir escapes)
//! - TIGER-P3-03: Message size limits (10MB max) to prevent OOM
//! - TIGER-P3-04: Symlink rejection at socket path
//! - Unix sockets created with 0600 permissions (owner-only)
//!
//! # Protocol
//!
//! Newline-delimited JSON:
//! - Client sends: `{"cmd": "...", ...}\n`
//! - Server responds: `{...}\n`

use std::io;
use std::path::{Path, PathBuf};

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use crate::commands::daemon::error::{DaemonError, DaemonResult};
use crate::commands::daemon::pid::compute_hash;
use crate::commands::daemon::types::{DaemonCommand, DaemonResponse};

// =============================================================================
// Constants
// =============================================================================

/// Maximum message size in bytes (10MB)
/// This prevents malicious clients from causing OOM via oversized messages.
/// (TIGER-P3-03)
pub const MAX_MESSAGE_SIZE: usize = 10 * 1024 * 1024;

/// Connection timeout in seconds
pub const CONNECTION_TIMEOUT_SECS: u64 = 5;

/// Read timeout in seconds
pub const READ_TIMEOUT_SECS: u64 = 30;

// =============================================================================
// Path/Port Computation
// =============================================================================

/// Compute the socket path for a project (Unix).
///
/// Path format: `{temp_dir}/tldr-{hash}.sock`
/// Uses same hash as PID file for consistency.
///
/// # Security (TIGER-P3-01)
///
/// The path is validated to ensure it stays within the temp directory
/// and doesn't escape via symlinks or path traversal.
#[cfg(unix)]
pub fn compute_socket_path(project: &Path) -> PathBuf {
    let hash = compute_hash(project);
    let tmp_dir = std::env::temp_dir();
    tmp_dir.join(format!("tldr-{}.sock", hash))
}

/// Compute the TCP port for a project (Windows).
///
/// Port range: 49152-59151 (dynamic/private port range)
/// Uses hash to deterministically map project to port.
#[cfg(windows)]
pub fn compute_tcp_port(project: &Path) -> u16 {
    let hash = compute_hash(project);
    let hash_int = u64::from_str_radix(&hash, 16).unwrap_or(0);
    49152 + (hash_int % 10000) as u16
}

// For cross-platform code that needs socket path on all platforms
#[cfg(not(unix))]
pub fn compute_socket_path(project: &Path) -> PathBuf {
    // On Windows, return a path that won't be used (TCP is used instead)
    let hash = compute_hash(project);
    let tmp_dir = std::env::temp_dir();
    tmp_dir.join(format!("tldr-{}.sock", hash))
}

#[cfg(not(windows))]
pub fn compute_tcp_port(project: &Path) -> u16 {
    // On Unix, return a port that won't be used (Unix socket is used instead)
    let hash = compute_hash(project);
    let hash_int = u64::from_str_radix(&hash, 16).unwrap_or(0);
    49152 + (hash_int % 10000) as u16
}

// =============================================================================
// Security Validation
// =============================================================================

/// Validate that a socket path is safe to use.
///
/// # Security Checks (TIGER-P3-01, TIGER-P3-04)
///
/// 1. Path must be within the system temp directory
/// 2. Path must not contain symlinks
/// 3. Path must not escape temp dir via `..` traversal
pub fn validate_socket_path(socket_path: &Path) -> DaemonResult<()> {
    let tmp_dir = std::env::temp_dir();

    // Canonicalize temp dir (resolve symlinks in temp dir itself)
    let canonical_tmp = tmp_dir.canonicalize().unwrap_or(tmp_dir);

    // Check that socket path starts with temp dir
    // We use the parent directory since the socket file doesn't exist yet
    let socket_parent = socket_path.parent().unwrap_or(socket_path);

    // Canonicalize parent if it exists
    let canonical_parent = socket_parent
        .canonicalize()
        .unwrap_or_else(|_| socket_parent.to_path_buf());

    if !canonical_parent.starts_with(&canonical_tmp) {
        return Err(DaemonError::PermissionDenied {
            path: socket_path.to_path_buf(),
        });
    }

    // Check for path traversal attempts in the filename
    if let Some(filename) = socket_path.file_name() {
        let filename_str = filename.to_string_lossy();
        if filename_str.contains("..") || filename_str.contains('/') || filename_str.contains('\\')
        {
            return Err(DaemonError::PermissionDenied {
                path: socket_path.to_path_buf(),
            });
        }
    }

    Ok(())
}

/// Check if a path is a symlink.
///
/// # Security (TIGER-P3-04)
///
/// Rejects symlinks at socket path to prevent symlink attacks.
#[cfg(unix)]
pub fn check_not_symlink(path: &Path) -> DaemonResult<()> {
    if let Ok(metadata) = std::fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() {
            return Err(DaemonError::PermissionDenied {
                path: path.to_path_buf(),
            });
        }
    }
    Ok(())
}

#[cfg(not(unix))]
pub fn check_not_symlink(path: &Path) -> DaemonResult<()> {
    // Windows symlink check
    if let Ok(metadata) = std::fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() {
            return Err(DaemonError::PermissionDenied {
                path: path.to_path_buf(),
            });
        }
    }
    Ok(())
}

// =============================================================================
// Socket path resolution (registry-first)
// =============================================================================

/// Resolve a project's socket path, preferring the TMPDIR-independent daemon
/// registry over the local TMPDIR-derived path.
///
/// The daemon binds its socket under *its own* `TMPDIR` (launchd inherits a
/// different `TMPDIR` than interactive shells), so a client that recomputes the
/// path from its own `TMPDIR` can diverge. The registry records the actual
/// socket path, so we consult it first and fall back to `compute_socket_path`
/// only when no live entry exists.
///
/// Returns `(path, from_registry)`.
fn resolve_socket_path(project: &Path) -> (PathBuf, bool) {
    match super::daemon_registry::find_entry(project) {
        Some(entry) => (entry.socket, true),
        None => (compute_socket_path(project), false),
    }
}

/// A registry-sourced socket path is trusted only if its file name matches the
/// deterministic `tldr-{hash}.sock` we would compute for this project. This
/// binds the registry entry to the project and prevents a poisoned/corrupt
/// registry from redirecting a connect or a *deletion* to an arbitrary file.
fn registry_socket_name_matches(project: &Path, socket_path: &Path) -> bool {
    compute_socket_path(project).file_name() == socket_path.file_name()
}

/// Resolve the socket path to delete during cleanup.
///
/// Unlike [`resolve_socket_path`] (which prunes dead entries via `find_entry`),
/// cleanup runs *precisely* when the daemon is dead, so it must consult the
/// registry WITHOUT pruning — otherwise a crashed cross-TMPDIR daemon's record
/// is dropped before we can read its socket path, and its socket is orphaned
/// (W6).
///
/// The registry-recorded path is honored only when (a) its filename matches
/// this project's deterministic socket name (poison guard, see
/// [`registry_socket_name_matches`]) and (b) the recorded PID is dead. A *live*
/// PID falls back to the local path, which spares an in-use socket recorded
/// under a *different* TMPDIR than this caller's (e.g. a re-registration swap).
/// A same-TMPDIR live socket equals the local path and is still removed — but
/// cleanup is teardown-only, so the meaningful guarantee is the cross-TMPDIR
/// one. On a daemon's own-exit cleanup the recorded socket likewise equals
/// `compute_socket_path(project)`, so the fallback removes the same path.
fn resolve_socket_path_for_cleanup(project: &Path) -> PathBuf {
    if let Some(entry) = super::daemon_registry::find_entry_unpruned(project) {
        if registry_socket_name_matches(project, &entry.socket)
            && !super::daemon_registry::is_pid_alive(entry.pid)
        {
            return entry.socket;
        }
    }
    compute_socket_path(project)
}

// =============================================================================
// IpcListener - Server Side
// =============================================================================

/// Platform-agnostic IPC listener
pub struct IpcListener {
    #[cfg(unix)]
    inner: tokio::net::UnixListener,
    #[cfg(windows)]
    inner: tokio::net::TcpListener,
    /// Path to socket file (for cleanup)
    #[allow(dead_code)]
    socket_path: PathBuf,
}

impl IpcListener {
    /// Bind a new IPC listener for the given project.
    ///
    /// # Unix
    /// Creates a Unix domain socket at `/tmp/tldr-{hash}.sock`
    /// with permissions 0600 (owner-only).
    ///
    /// # Windows
    /// Binds to TCP localhost on a deterministic port.
    ///
    /// # Security
    /// - TIGER-P3-01: Validates socket path stays in temp dir
    /// - TIGER-P3-04: Rejects symlinks at socket path
    pub async fn bind(project: &Path) -> DaemonResult<Self> {
        #[cfg(unix)]
        {
            Self::bind_unix(project).await
        }
        #[cfg(windows)]
        {
            Self::bind_tcp(project).await
        }
    }

    #[cfg(unix)]
    async fn bind_unix(project: &Path) -> DaemonResult<Self> {
        use std::os::unix::fs::PermissionsExt;

        let socket_path = compute_socket_path(project);

        // Validate socket path security
        validate_socket_path(&socket_path)?;

        // Check for existing symlink (TIGER-P3-04)
        check_not_symlink(&socket_path)?;

        // Issue #14 (TOCTOU fix): do NOT silently unlink an existing socket
        // here. Pre-fix, a second concurrent start could observe the file
        // existing, remove it, and bind a fresh socket — clobbering a live
        // first daemon's IPC endpoint. Instead, attempt the bind directly;
        // if a socket is already present we surface `AddressInUse`.
        // Stale-socket cleanup is the responsibility of the caller (start.rs)
        // after a liveness probe via `check_socket_alive`.
        let listener = tokio::net::UnixListener::bind(&socket_path).map_err(|e| {
            if e.kind() == io::ErrorKind::AddrInUse {
                DaemonError::AddressInUse {
                    addr: socket_path.display().to_string(),
                }
            } else {
                DaemonError::SocketBindFailed(e)
            }
        })?;

        // Set socket permissions to 0600 (owner-only) - TIGER-P3-01
        let permissions = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(&socket_path, permissions)
            .map_err(DaemonError::SocketBindFailed)?;

        Ok(Self {
            inner: listener,
            socket_path,
        })
    }

    #[cfg(windows)]
    async fn bind_tcp(project: &Path) -> DaemonResult<Self> {
        let socket_path = compute_socket_path(project); // For reference only
        let port = compute_tcp_port(project);
        let addr = format!("127.0.0.1:{}", port);

        let listener = tokio::net::TcpListener::bind(&addr).await.map_err(|e| {
            if e.kind() == io::ErrorKind::AddrInUse {
                DaemonError::AddressInUse { addr }
            } else {
                DaemonError::SocketBindFailed(e)
            }
        })?;

        Ok(Self {
            inner: listener,
            socket_path,
        })
    }

    /// Accept a new connection.
    ///
    /// Returns an `IpcStream` that can be used for bidirectional communication.
    pub async fn accept(&self) -> DaemonResult<IpcStream> {
        #[cfg(unix)]
        {
            let (stream, _addr) = self.inner.accept().await.map_err(DaemonError::Io)?;
            Ok(IpcStream {
                inner: IpcStreamInner::Unix(stream),
            })
        }
        #[cfg(windows)]
        {
            let (stream, _addr) = self.inner.accept().await.map_err(DaemonError::Io)?;
            Ok(IpcStream {
                inner: IpcStreamInner::Tcp(stream),
            })
        }
    }
}

// =============================================================================
// IpcStream - Bidirectional Communication
// =============================================================================

/// Inner stream type for platform abstraction
enum IpcStreamInner {
    #[cfg(unix)]
    Unix(tokio::net::UnixStream),
    #[cfg(windows)]
    Tcp(tokio::net::TcpStream),
    // Allow both variants on all platforms for testing
    #[cfg(all(not(unix), not(windows)))]
    Dummy,
}

/// Platform-agnostic IPC stream for bidirectional communication.
pub struct IpcStream {
    inner: IpcStreamInner,
}

impl IpcStream {
    /// Connect to a daemon for the given project.
    ///
    /// # Unix
    /// Resolves the socket path from the daemon registry first (TMPDIR-independent),
    /// falling back to `{temp_dir}/tldr-{hash}.sock` when no registry entry exists.
    ///
    /// # Windows
    /// Connects to TCP localhost on a deterministic port.
    pub async fn connect(project: &Path) -> DaemonResult<Self> {
        #[cfg(unix)]
        {
            Self::connect_unix(project).await
        }
        #[cfg(windows)]
        {
            Self::connect_tcp(project).await
        }
    }

    #[cfg(unix)]
    async fn connect_unix(project: &Path) -> DaemonResult<Self> {
        // Resolve socket path via the daemon registry first (TMPDIR-independent;
        // see `resolve_socket_path`). Fall back to the TMPDIR-derived path for
        // the single-daemon / no-registry case.
        let (socket_path, from_registry) = resolve_socket_path(project);

        // Registry path came from our cache-dir registry file — skip
        // TMPDIR-containment (the daemon's TMPDIR differs from ours) but
        // verify the filename matches what we'd compute for this project.
        if from_registry {
            if !registry_socket_name_matches(project, &socket_path) {
                return Err(DaemonError::PermissionDenied {
                    path: socket_path.clone(),
                });
            }
        } else {
            // W1/W2: a registry miss silently re-introduces the original
            // cross-TMPDIR bug if the daemon bound its socket under a different
            // TMPDIR. Surface it under TLDR_DEBUG so the failure mode is
            // diagnosable instead of a bare "not running".
            if std::env::var_os("TLDR_DEBUG").is_some() {
                eprintln!(
                    "[tldr-debug] daemon registry miss for {}; falling back to \
                     TMPDIR-derived socket {}",
                    project.display(),
                    socket_path.display()
                );
            }
            validate_socket_path(&socket_path)?;
        }

        // Check socket exists
        if !socket_path.exists() {
            return Err(DaemonError::NotRunning);
        }

        // Symlink check applies regardless of source (TIGER-P3-04)
        check_not_symlink(&socket_path)?;

        // Connect with timeout
        let connect_future = tokio::net::UnixStream::connect(&socket_path);
        let timeout = tokio::time::Duration::from_secs(CONNECTION_TIMEOUT_SECS);

        match tokio::time::timeout(timeout, connect_future).await {
            Ok(Ok(stream)) => Ok(Self {
                inner: IpcStreamInner::Unix(stream),
            }),
            Ok(Err(e)) if e.kind() == io::ErrorKind::ConnectionRefused => {
                Err(DaemonError::ConnectionRefused)
            }
            Ok(Err(e)) if e.kind() == io::ErrorKind::NotFound => Err(DaemonError::NotRunning),
            Ok(Err(e)) => Err(DaemonError::Io(e)),
            Err(_) => Err(DaemonError::ConnectionTimeout {
                timeout_secs: CONNECTION_TIMEOUT_SECS,
            }),
        }
    }

    #[cfg(windows)]
    async fn connect_tcp(project: &Path) -> DaemonResult<Self> {
        let port = compute_tcp_port(project);
        let addr = format!("127.0.0.1:{}", port);

        // Connect with timeout
        let connect_future = tokio::net::TcpStream::connect(&addr);
        let timeout = tokio::time::Duration::from_secs(CONNECTION_TIMEOUT_SECS);

        match tokio::time::timeout(timeout, connect_future).await {
            Ok(Ok(stream)) => Ok(Self {
                inner: IpcStreamInner::Tcp(stream),
            }),
            Ok(Err(e)) if e.kind() == io::ErrorKind::ConnectionRefused => {
                Err(DaemonError::ConnectionRefused)
            }
            Ok(Err(e)) => Err(DaemonError::Io(e)),
            Err(_) => Err(DaemonError::ConnectionTimeout {
                timeout_secs: CONNECTION_TIMEOUT_SECS,
            }),
        }
    }

    /// Send a command to the daemon.
    ///
    /// Serializes the command to JSON and sends with a newline delimiter.
    pub async fn send_command(&mut self, cmd: &DaemonCommand) -> DaemonResult<()> {
        let json = serde_json::to_string(cmd)?;
        self.send_raw(&json).await
    }

    /// Send a raw JSON string to the daemon.
    ///
    /// Adds newline delimiter automatically.
    pub async fn send_raw(&mut self, json: &str) -> DaemonResult<()> {
        // Check message size (TIGER-P3-03)
        if json.len() > MAX_MESSAGE_SIZE {
            return Err(DaemonError::InvalidMessage(format!(
                "message too large: {} bytes (max {})",
                json.len(),
                MAX_MESSAGE_SIZE
            )));
        }

        let mut message = json.to_string();
        message.push('\n');

        match &mut self.inner {
            #[cfg(unix)]
            IpcStreamInner::Unix(stream) => {
                stream.write_all(message.as_bytes()).await?;
                stream.flush().await?;
            }
            #[cfg(windows)]
            IpcStreamInner::Tcp(stream) => {
                stream.write_all(message.as_bytes()).await?;
                stream.flush().await?;
            }
            #[cfg(all(not(unix), not(windows)))]
            IpcStreamInner::Dummy => {}
        }

        Ok(())
    }

    /// Receive a response from the daemon.
    ///
    /// Reads a newline-delimited JSON response and deserializes it.
    pub async fn recv_response(&mut self) -> DaemonResult<DaemonResponse> {
        let json = self.recv_raw().await?;
        let response: DaemonResponse = serde_json::from_str(&json)?;
        Ok(response)
    }

    /// Receive a raw JSON string from the daemon.
    ///
    /// Reads until newline delimiter. Returns
    /// `DaemonError::InvalidMessage` if no newline is found within
    /// `MAX_MESSAGE_SIZE` bytes, preventing OOM/DoS by bounding the
    /// allocation BEFORE `read_line` consumes the stream
    /// (TIGER-P3-03; closes #17 + #25). Both Unix and Windows arms
    /// delegate to the shared `recv_raw_from` helper.
    pub async fn recv_raw(&mut self) -> DaemonResult<String> {
        let timeout = tokio::time::Duration::from_secs(READ_TIMEOUT_SECS);
        // limit = MAX + 1 so reading exactly MAX_MESSAGE_SIZE payload bytes
        // followed by the `\n` delimiter still fits within the bounded reader.
        let limit = (MAX_MESSAGE_SIZE + 1) as u64;

        match &mut self.inner {
            #[cfg(unix)]
            IpcStreamInner::Unix(stream) => recv_raw_from(stream, limit, timeout).await,
            #[cfg(windows)]
            IpcStreamInner::Tcp(stream) => recv_raw_from(stream, limit, timeout).await,
            #[cfg(all(not(unix), not(windows)))]
            IpcStreamInner::Dummy => Err(DaemonError::NotRunning),
        }
    }
}

/// Shared helper: read a newline-delimited string from any `AsyncRead` stream,
/// rejecting payloads that exceed `limit` bytes BEFORE allocation grows
/// unbounded.
///
/// `tokio::io::AsyncReadExt::take(stream, limit)` returns a `Take<&mut R>` that
/// reports EOF once `limit` bytes have been read. If `read_line` returns
/// without consuming a `\n`, we treat that as a size-limit violation. The
/// mutable borrow of `stream` is released when the `Take` adapter is dropped at
/// the end of this helper's scope, so the caller retains exclusive access to
/// the original stream after `recv_raw_from` returns.
async fn recv_raw_from<R>(
    stream: &mut R,
    limit: u64,
    timeout: tokio::time::Duration,
) -> DaemonResult<String>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let limited = AsyncReadExt::take(stream, limit);
    let mut reader = BufReader::new(limited);
    let mut line = String::new();

    let read_future = reader.read_line(&mut line);

    match tokio::time::timeout(timeout, read_future).await {
        // True EOF before any bytes were read → connection closed.
        Ok(Ok(0)) if line.is_empty() => Err(DaemonError::ConnectionRefused),
        // Any read that does not end in `\n` means we hit the bounded reader's
        // EOF without seeing the delimiter — i.e. the payload exceeds the
        // size limit. (Includes the `0` byte case where `line` is non-empty
        // because BufReader buffered partial bytes; still oversized.)
        Ok(Ok(_)) if !line.ends_with('\n') => Err(DaemonError::InvalidMessage(format!(
            "message exceeds size limit of {} bytes",
            MAX_MESSAGE_SIZE
        ))),
        Ok(Ok(_)) => Ok(line.trim_end().to_string()),
        Ok(Err(e)) => Err(DaemonError::Io(e)),
        Err(_) => Err(DaemonError::ConnectionTimeout {
            timeout_secs: READ_TIMEOUT_SECS,
        }),
    }
}

// =============================================================================
// Server-side message handling
// =============================================================================

/// Read a command from a client connection.
///
/// Used by the daemon to receive commands from clients. Size enforcement
/// (TIGER-P3-03) happens upstream inside `IpcStream::recv_raw`, which now
/// bounds the read with `AsyncReadExt::take(MAX_MESSAGE_SIZE + 1)` BEFORE
/// allocation. Any payload exceeding the limit is rejected with
/// `DaemonError::InvalidMessage` before this function runs, so no
/// post-allocation re-check is needed here (#17, #25).
pub async fn read_command(stream: &mut IpcStream) -> DaemonResult<DaemonCommand> {
    let json = stream.recv_raw().await?;
    let cmd: DaemonCommand = serde_json::from_str(&json)?;
    Ok(cmd)
}

/// Send a response to a client connection.
///
/// Used by the daemon to respond to client commands.
pub async fn send_response(stream: &mut IpcStream, response: &DaemonResponse) -> DaemonResult<()> {
    let json = serde_json::to_string(response)?;
    stream.send_raw(&json).await
}

// =============================================================================
// Cleanup
// =============================================================================

/// Clean up the socket file for a project.
///
/// Safe to call even if socket doesn't exist.
///
/// # W6 (cross-TMPDIR cleanup)
///
/// The socket is resolved via the daemon registry first, mirroring
/// `connect_unix`. Computing the path from the *caller's* `TMPDIR` would
/// no-op when the daemon bound its socket under a different `TMPDIR`, orphaning
/// the real socket file. Callers (`stop.rs`) invoke this BEFORE `remove_entry`,
/// so the registry entry is still live here.
///
/// Cleanup is best-effort: a registry path whose filename does not match this
/// project's deterministic socket name (poisoned/corrupt registry) is NOT
/// deleted — we fall back to the local TMPDIR-derived path. This also keeps the
/// one `?`-propagating caller (`start.rs` stale-socket cleanup) from aborting
/// startup over a bad registry entry.
pub fn cleanup_socket(project: &Path) -> DaemonResult<()> {
    let socket_path = resolve_socket_path_for_cleanup(project);
    cleanup_socket_at(&socket_path)
}

/// Remove the socket file at an explicit path. Use this when the caller has
/// already resolved the path (e.g. via [`snapshot_socket_path`]) to avoid a
/// second registry lookup that may see pruned state.
pub fn cleanup_socket_at(socket_path: &Path) -> DaemonResult<()> {
    if socket_path.exists() {
        check_not_symlink(socket_path)?;
        std::fs::remove_file(socket_path)?;
    }
    Ok(())
}

/// Snapshot the socket path from the unpruned registry BEFORE any operation
/// that might trigger a pruning `read_registry()` call (e.g.
/// `check_socket_alive`, `send_command`). The returned path is safe to pass
/// to [`cleanup_socket_at`] later, even if the registry entry has been pruned
/// in the meantime.
///
/// Unlike [`resolve_socket_path_for_cleanup`], this does NOT gate on
/// `!is_pid_alive` — the caller knows it is about to kill the daemon, so
/// the PID will be dead by the time cleanup runs.
pub fn snapshot_socket_path(project: &Path) -> PathBuf {
    if let Some(entry) = super::daemon_registry::find_entry_unpruned(project) {
        if registry_socket_name_matches(project, &entry.socket) {
            return entry.socket;
        }
    }
    compute_socket_path(project)
}

/// Check if a socket exists and is connectable.
///
/// Used to detect stale sockets that can be cleaned up.
pub async fn check_socket_alive(project: &Path) -> bool {
    (IpcStream::connect(project).await).is_ok()
}

// =============================================================================
// High-level client functions
// =============================================================================

/// Send a command to the daemon and receive a response.
///
/// Convenience function that handles connection, send, and receive.
pub async fn send_command(project: &Path, cmd: &DaemonCommand) -> DaemonResult<DaemonResponse> {
    let mut stream = IpcStream::connect(project).await?;
    stream.send_command(cmd).await?;
    stream.recv_response().await
}

/// Send a raw JSON command to the daemon and receive a raw response.
///
/// Useful for low-level debugging or custom commands.
pub async fn send_raw_command(project: &Path, json: &str) -> DaemonResult<String> {
    let mut stream = IpcStream::connect(project).await?;
    stream.send_raw(json).await?;
    stream.recv_raw().await
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[test]
    fn test_compute_socket_path_format() {
        let project = PathBuf::from("/test/project");
        let socket_path = compute_socket_path(&project);

        let filename = socket_path.file_name().unwrap().to_str().unwrap();
        assert!(filename.starts_with("tldr-"));
        assert!(filename.ends_with(".sock"));
    }

    #[test]
    fn test_compute_socket_path_deterministic() {
        let project = PathBuf::from("/test/project");
        let path1 = compute_socket_path(&project);
        let path2 = compute_socket_path(&project);
        assert_eq!(path1, path2);
    }

    #[test]
    fn test_compute_socket_path_different_projects() {
        let project1 = PathBuf::from("/test/project1");
        let project2 = PathBuf::from("/test/project2");
        let path1 = compute_socket_path(&project1);
        let path2 = compute_socket_path(&project2);
        assert_ne!(path1, path2);
    }

    #[test]
    fn test_compute_tcp_port_range() {
        let project = PathBuf::from("/test/project");
        let port = compute_tcp_port(&project);
        assert!(port >= 49152);
        assert!(port < 59152);
    }

    #[test]
    fn test_compute_tcp_port_deterministic() {
        let project = PathBuf::from("/test/project");
        let port1 = compute_tcp_port(&project);
        let port2 = compute_tcp_port(&project);
        assert_eq!(port1, port2);
    }

    #[test]
    fn test_validate_socket_path_valid() {
        let tmp_dir = std::env::temp_dir();
        let socket_path = tmp_dir.join("tldr-test.sock");
        assert!(validate_socket_path(&socket_path).is_ok());
    }

    #[test]
    fn test_validate_socket_path_traversal() {
        let tmp_dir = std::env::temp_dir();
        let socket_path = tmp_dir.join("../etc/passwd");
        // This should fail because the canonicalized path escapes temp dir
        // Note: behavior depends on whether /etc exists and is a directory
        let result = validate_socket_path(&socket_path);
        // Should fail either due to path validation or filename check
        // The exact error depends on the system
        assert!(result.is_err() || !socket_path.starts_with(&tmp_dir));
    }

    #[test]
    fn test_validate_socket_path_bad_filename() {
        let tmp_dir = std::env::temp_dir();
        // Create a path with .. in filename (not directory traversal)
        let socket_path = tmp_dir.join("test..sock");
        assert!(validate_socket_path(&socket_path).is_err());
    }

    #[test]
    fn test_max_message_size_constant() {
        // Verify 10MB limit
        assert_eq!(MAX_MESSAGE_SIZE, 10 * 1024 * 1024);
    }

    #[test]
    fn test_cleanup_socket_nonexistent() {
        use crate::commands::daemon::daemon_registry::test_support::with_registry_dir;
        // Redirect the registry to a temp dir: cleanup_socket now reads the
        // registry, and we must not touch the developer's real registry.
        with_registry_dir(|dir| {
            let project = dir.join("nonexistent");

            // Should not error on nonexistent socket
            let result = cleanup_socket(&project);
            assert!(result.is_ok());
        });
    }

    /// W6: a crashed cross-TMPDIR daemon leaves its socket orphaned under its
    /// own TMPDIR, with a DEAD PID in the registry. `cleanup_socket` must remove
    /// that socket rather than no-op on the caller's (non-existent) local path.
    /// The dead PID is the load-bearing detail: `find_entry` prunes dead entries,
    /// so cleanup must use an unpruned lookup.
    #[cfg(unix)]
    #[test]
    fn test_cleanup_socket_removes_dead_daemon_registry_path() {
        use crate::commands::daemon::daemon_registry::{add_entry, test_support::with_registry_dir};
        with_registry_dir(|dir| {
            // A real, canonicalizable project dir.
            let project = dir.join("proj");
            std::fs::create_dir_all(&project).unwrap();

            // Socket lives under `dir` (stand-in for the daemon's TMPDIR), not
            // the system temp dir that `compute_socket_path` would yield. The
            // filename still matches this project's deterministic socket name.
            let sock_name = compute_socket_path(&project).file_name().unwrap().to_owned();
            let sock = dir.join(&sock_name);
            std::fs::write(&sock, b"").unwrap();
            assert_ne!(
                sock,
                compute_socket_path(&project),
                "test premise: registry socket must differ from local path"
            );

            // Spawn `true` and reap → PID is definitely dead (the real W6
            // scenario: a crashed daemon's orphaned socket).
            let mut child = std::process::Command::new("true")
                .spawn()
                .expect("spawn true");
            let dead_pid = child.id();
            let _ = child.wait();

            add_entry(&project, dead_pid, &sock).expect("add");
            assert!(sock.exists());

            cleanup_socket(&project).expect("cleanup");
            assert!(
                !sock.exists(),
                "orphaned socket of dead cross-TMPDIR daemon should be removed"
            );
        });
    }

    /// W3: `snapshot_socket_path` must return the registry-recorded path even
    /// when the daemon is still alive. The caller (stop.rs) captures this before
    /// sending shutdown — by the time `cleanup_socket_at` runs the PID is dead,
    /// but the snapshot was taken while it was alive.
    #[cfg(unix)]
    #[test]
    fn test_snapshot_socket_path_returns_registry_path_for_live_daemon() {
        use crate::commands::daemon::daemon_registry::{add_entry, test_support::with_registry_dir};
        with_registry_dir(|dir| {
            let project = dir.join("proj-snap");
            std::fs::create_dir_all(&project).unwrap();

            let sock_name = compute_socket_path(&project).file_name().unwrap().to_owned();
            let registry_sock = dir.join(&sock_name);

            // Live PID (this test process) — simulates snapshotting before shutdown.
            add_entry(&project, std::process::id(), &registry_sock).expect("add");

            let snapped = snapshot_socket_path(&project);
            assert_eq!(
                snapped, registry_sock,
                "snapshot must return the registry path even for a live daemon"
            );
            assert_ne!(
                snapped,
                compute_socket_path(&project),
                "snapshot must NOT fall back to local TMPDIR when a registry entry exists"
            );
        });
    }

    /// W6 safety: a *live* daemon's registry socket must never be deleted by a
    /// cleanup from a different session (e.g. after a project-key re-registration
    /// swapped in a new live daemon). Cleanup falls back to the local path.
    #[cfg(unix)]
    #[test]
    fn test_cleanup_socket_spares_live_daemon_registry_path() {
        use crate::commands::daemon::daemon_registry::{add_entry, test_support::with_registry_dir};
        with_registry_dir(|dir| {
            let project = dir.join("proj-live");
            std::fs::create_dir_all(&project).unwrap();

            let sock_name = compute_socket_path(&project).file_name().unwrap().to_owned();
            let sock = dir.join(&sock_name);
            std::fs::write(&sock, b"").unwrap();

            // Live PID (this test process) → cleanup must NOT touch the socket.
            add_entry(&project, std::process::id(), &sock).expect("add");

            cleanup_socket(&project).expect("cleanup");
            assert!(
                sock.exists(),
                "a live daemon's socket must not be removed by cleanup"
            );
        });
    }

    #[cfg(unix)]
    #[test]
    fn test_check_not_symlink_regular_file() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("regular.txt");
        std::fs::write(&file_path, "test").unwrap();

        assert!(check_not_symlink(&file_path).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn test_check_not_symlink_symlink() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("regular.txt");
        let link_path = temp.path().join("symlink.txt");

        std::fs::write(&file_path, "test").unwrap();
        std::os::unix::fs::symlink(&file_path, &link_path).unwrap();

        assert!(check_not_symlink(&link_path).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn test_check_not_symlink_nonexistent() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("nonexistent");

        // Nonexistent path should be OK (nothing to check)
        assert!(check_not_symlink(&path).is_ok());
    }

    #[tokio::test]
    async fn test_connect_nonexistent_daemon() {
        use crate::commands::daemon::daemon_registry::test_support::REGISTRY_ENV_LOCK;
        // connect resolves via the registry first; isolate it from the real one.
        // `with_registry_dir` takes a sync closure, so hold the env override
        // manually across the await. tokio::test is current-thread, so holding
        // the !Send guard across `.await` is fine.
        let _guard = REGISTRY_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let temp = TempDir::new().unwrap();
        std::env::set_var("TLDR_DAEMON_REGISTRY_DIR", temp.path());
        let project = temp.path().join("nonexistent");

        let result = IpcStream::connect(&project).await;
        std::env::remove_var("TLDR_DAEMON_REGISTRY_DIR");
        assert!(matches!(result, Err(DaemonError::NotRunning)));
    }

    // Integration tests for listener/stream would require a running daemon
    // Those are tested in daemon_test.rs Phase 5+
}
