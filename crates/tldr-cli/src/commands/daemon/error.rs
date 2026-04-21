//! Daemon-specific error types
//!
//! Errors for daemon lifecycle, IPC, and cache operations.

use std::io;
use std::path::PathBuf;

use thiserror::Error;

/// Daemon-specific errors
#[derive(Debug, Error)]
pub enum DaemonError {
    /// Daemon is already running for this project
    #[error("daemon already running (PID: {pid})")]
    AlreadyRunning { pid: u32 },

    /// Daemon is not running for this project
    #[error("daemon not running")]
    NotRunning,

    /// Failed to acquire PID file lock
    #[error("failed to acquire PID file lock: {0}")]
    LockFailed(io::Error),

    /// Failed to bind to socket
    #[error("failed to bind socket: {0}")]
    SocketBindFailed(io::Error),

    /// Address/socket is already in use
    #[error("address already in use: {addr}")]
    AddressInUse { addr: String },

    /// Connection to daemon was refused
    #[error("connection refused")]
    ConnectionRefused,

    /// Connection to daemon timed out
    #[error("connection timeout after {timeout_secs}s")]
    ConnectionTimeout { timeout_secs: u64 },

    /// Invalid IPC message received
    #[error("invalid IPC message: {0}")]
    InvalidMessage(String),

    /// Unknown command received
    #[error("unknown command: {cmd}")]
    UnknownCommand { cmd: String },

    /// Required parameter is missing
    #[error("missing required parameter: {param}")]
    MissingParameter { param: String },

    /// Permission denied for path
    #[error("permission denied: {}", path.display())]
    PermissionDenied { path: PathBuf },

    /// PID file exists but process is not running
    #[error("stale PID file (process {pid} not running)")]
    StalePidFile { pid: u32 },

    /// Generic IO error
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    /// JSON serialization/deserialization error
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Result type for daemon operations
pub type DaemonResult<T> = Result<T, DaemonError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_daemon_error_display() {
        let err = DaemonError::AlreadyRunning { pid: 12345 };
        assert_eq!(err.to_string(), "daemon already running (PID: 12345)");

        let err = DaemonError::NotRunning;
        assert_eq!(err.to_string(), "daemon not running");

        let err = DaemonError::ConnectionRefused;
        assert_eq!(err.to_string(), "connection refused");

        let err = DaemonError::ConnectionTimeout { timeout_secs: 5 };
        assert_eq!(err.to_string(), "connection timeout after 5s");

        let err = DaemonError::UnknownCommand {
            cmd: "foo".to_string(),
        };
        assert_eq!(err.to_string(), "unknown command: foo");

        let err = DaemonError::MissingParameter {
            param: "file".to_string(),
        };
        assert_eq!(err.to_string(), "missing required parameter: file");

        let err = DaemonError::PermissionDenied {
            path: PathBuf::from("/tmp/test"),
        };
        assert_eq!(err.to_string(), "permission denied: /tmp/test");

        let err = DaemonError::StalePidFile { pid: 99999 };
        assert_eq!(
            err.to_string(),
            "stale PID file (process 99999 not running)"
        );

        let err = DaemonError::AddressInUse {
            addr: "/tmp/test.sock".to_string(),
        };
        assert_eq!(err.to_string(), "address already in use: /tmp/test.sock");

        let err = DaemonError::InvalidMessage("bad json".to_string());
        assert_eq!(err.to_string(), "invalid IPC message: bad json");
    }

    #[test]
    fn test_daemon_error_io_from() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "file not found");
        let daemon_err: DaemonError = io_err.into();
        assert!(matches!(daemon_err, DaemonError::Io(_)));
    }
}
