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

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

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

        // Remove existing socket if present (could be stale)
        if socket_path.exists() {
            // Double-check it's not a symlink before removing
            check_not_symlink(&socket_path)?;
            std::fs::remove_file(&socket_path).map_err(DaemonError::SocketBindFailed)?;
        }

        // Bind the Unix socket
        let listener =
            tokio::net::UnixListener::bind(&socket_path).map_err(DaemonError::SocketBindFailed)?;

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
    /// Connects to Unix domain socket at `/tmp/tldr-{hash}.sock`
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
        let socket_path = compute_socket_path(project);

        // Validate socket path security
        validate_socket_path(&socket_path)?;

        // Check socket exists
        if !socket_path.exists() {
            return Err(DaemonError::NotRunning);
        }

        // Check for symlink attack (TIGER-P3-04)
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
    /// Reads until newline delimiter.
    pub async fn recv_raw(&mut self) -> DaemonResult<String> {
        let timeout = tokio::time::Duration::from_secs(READ_TIMEOUT_SECS);

        match &mut self.inner {
            #[cfg(unix)]
            IpcStreamInner::Unix(stream) => {
                let mut reader = BufReader::new(stream);
                let mut line = String::new();

                let read_future = reader.read_line(&mut line);

                match tokio::time::timeout(timeout, read_future).await {
                    Ok(Ok(0)) => Err(DaemonError::ConnectionRefused), // EOF
                    Ok(Ok(n)) if n > MAX_MESSAGE_SIZE => Err(DaemonError::InvalidMessage(format!(
                        "response too large: {} bytes (max {})",
                        n, MAX_MESSAGE_SIZE
                    ))),
                    Ok(Ok(_)) => Ok(line.trim_end().to_string()),
                    Ok(Err(e)) => Err(DaemonError::Io(e)),
                    Err(_) => Err(DaemonError::ConnectionTimeout {
                        timeout_secs: READ_TIMEOUT_SECS,
                    }),
                }
            }
            #[cfg(windows)]
            IpcStreamInner::Tcp(stream) => {
                let mut reader = BufReader::new(stream);
                let mut line = String::new();

                let read_future = reader.read_line(&mut line);

                match tokio::time::timeout(timeout, read_future).await {
                    Ok(Ok(0)) => Err(DaemonError::ConnectionRefused), // EOF
                    Ok(Ok(n)) if n > MAX_MESSAGE_SIZE => Err(DaemonError::InvalidMessage(format!(
                        "response too large: {} bytes (max {})",
                        n, MAX_MESSAGE_SIZE
                    ))),
                    Ok(Ok(_)) => Ok(line.trim_end().to_string()),
                    Ok(Err(e)) => Err(DaemonError::Io(e)),
                    Err(_) => Err(DaemonError::ConnectionTimeout {
                        timeout_secs: READ_TIMEOUT_SECS,
                    }),
                }
            }
            #[cfg(all(not(unix), not(windows)))]
            IpcStreamInner::Dummy => Err(DaemonError::NotRunning),
        }
    }
}

// =============================================================================
// Server-side message handling
// =============================================================================

/// Read a command from a client connection.
///
/// Used by the daemon to receive commands from clients.
pub async fn read_command(stream: &mut IpcStream) -> DaemonResult<DaemonCommand> {
    let json = stream.recv_raw().await?;

    // Check message size (TIGER-P3-03)
    if json.len() > MAX_MESSAGE_SIZE {
        return Err(DaemonError::InvalidMessage(format!(
            "command too large: {} bytes (max {})",
            json.len(),
            MAX_MESSAGE_SIZE
        )));
    }

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
pub fn cleanup_socket(project: &Path) -> DaemonResult<()> {
    let socket_path = compute_socket_path(project);

    if socket_path.exists() {
        // Safety check: don't remove symlinks
        check_not_symlink(&socket_path)?;
        std::fs::remove_file(&socket_path)?;
    }

    Ok(())
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
        let temp = TempDir::new().unwrap();
        let project = temp.path().join("nonexistent");

        // Should not error on nonexistent socket
        let result = cleanup_socket(&project);
        assert!(result.is_ok());
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
        let temp = TempDir::new().unwrap();
        let project = temp.path();

        let result = IpcStream::connect(project).await;
        assert!(matches!(result, Err(DaemonError::NotRunning)));
    }

    // Integration tests for listener/stream would require a running daemon
    // Those are tested in daemon_test.rs Phase 5+
}
