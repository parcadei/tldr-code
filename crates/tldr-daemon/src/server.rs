//! Axum HTTP server for the daemon
//!
//! This module sets up the HTTP server using axum with all command handlers.
//!
//! # Mitigations Addressed
//!
//! - **M4**: Socket Race Condition - Lock file on startup
//! - **M10**: Async Runtime Issues - `spawn_blocking` for CPU work
//! - **M14**: Socket/FD Leak - `catch_unwind` in handlers
//! - **M17**: Socket Cleanup - Stale socket detection
//! - **M21**: Upgrade Path - Version in socket path

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::{http::StatusCode, response::IntoResponse, routing::post, Json, Router};
#[cfg(unix)]
use tokio::net::UnixListener;
use tracing::info;
#[cfg(unix)]
use tracing::{error, warn};

use crate::handlers;
use crate::state::DaemonState;

/// Daemon configuration
#[derive(Debug, Clone)]
pub struct DaemonConfig {
    /// Project root directory
    pub project: PathBuf,
    /// Idle timeout (default: 5 minutes)
    pub idle_timeout: Duration,
    /// Log level (from TLDR_LOG env var)
    pub log_level: String,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            project: std::env::current_dir().unwrap_or_default(),
            idle_timeout: Duration::from_secs(300),
            log_level: std::env::var("TLDR_LOG").unwrap_or_else(|_| "info".to_string()),
        }
    }
}

/// Compute socket path for a project (spec Section 4.1)
///
/// Unix: `/tmp/tldr-{hash}-v{version}.sock`
/// Windows: TCP on `127.0.0.1:{49152 + hash % 10000}`
///
/// Hash is MD5 of resolved project path (first 8 hex chars)
pub fn compute_socket_path(project: &Path, version: &str) -> PathBuf {
    // Normalize path
    let canonical = dunce::canonicalize(project).unwrap_or_else(|_| project.to_path_buf());
    let path_str = canonical.to_string_lossy();

    // Compute MD5 hash
    let digest = md5::compute(path_str.as_bytes());
    let hash = format!("{:x}", digest);
    let hash_prefix = &hash[..8];

    // Socket path with version (M21)
    let socket_dir = std::env::var("TLDR_SOCKET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());

    socket_dir.join(format!("tldr-{}-v{}.sock", hash_prefix, version))
}

/// Compute TCP port for Windows (or when Unix socket unavailable)
pub fn compute_tcp_port(project: &Path) -> u16 {
    let canonical = dunce::canonicalize(project).unwrap_or_else(|_| project.to_path_buf());
    let path_str = canonical.to_string_lossy();
    let digest = md5::compute(path_str.as_bytes());
    let hash_bytes: [u8; 16] = digest.into();

    // Use first 4 bytes to compute port offset
    let hash_u32 = u32::from_le_bytes([hash_bytes[0], hash_bytes[1], hash_bytes[2], hash_bytes[3]]);
    let port_offset = (hash_u32 % 10000) as u16;
    49152 + port_offset
}

/// Check if a socket is stale (M17)
///
/// A socket is stale if:
/// 1. The file exists but no daemon is listening
/// 2. Connection attempt fails
#[cfg(unix)]
pub async fn is_socket_stale(socket_path: &Path) -> bool {
    if !socket_path.exists() {
        return false; // Not stale, just doesn't exist
    }

    // Try to connect
    match tokio::net::UnixStream::connect(socket_path).await {
        Ok(_stream) => {
            // Daemon is running
            false
        }
        Err(_) => {
            // Connection failed - socket is stale
            warn!("Detected stale socket at {:?}", socket_path);
            true
        }
    }
}

/// Remove stale socket file (M17)
#[cfg(unix)]
pub fn remove_stale_socket(socket_path: &Path) -> std::io::Result<()> {
    if socket_path.exists() {
        std::fs::remove_file(socket_path)?;
        info!("Removed stale socket: {:?}", socket_path);
    }
    Ok(())
}

/// Build the axum router with all handlers
pub fn build_router(state: Arc<DaemonState>) -> Router {
    Router::new()
        // Core protocol endpoints
        .route("/ping", post(handlers::ping))
        .route("/status", post(handlers::status))
        // AST Layer (L1)
        .route("/tree", post(handlers::ast::tree))
        .route("/structure", post(handlers::ast::structure))
        .route("/extract", post(handlers::ast::extract))
        .route("/imports", post(handlers::ast::imports))
        // Call Graph Layer (L2)
        .route("/calls", post(handlers::callgraph::calls))
        .route("/impact", post(handlers::callgraph::impact))
        .route("/dead", post(handlers::callgraph::dead))
        .route("/importers", post(handlers::callgraph::importers))
        .route("/arch", post(handlers::callgraph::arch))
        // Flow Analysis (L3/L4/L5)
        .route("/cfg", post(handlers::flow::cfg))
        .route("/dfg", post(handlers::flow::dfg))
        .route("/slice", post(handlers::flow::slice))
        .route("/complexity", post(handlers::flow::complexity))
        // Search
        .route("/search", post(handlers::search::search))
        .route("/context", post(handlers::search::context))
        // Quality
        .route("/smells", post(handlers::quality::smells))
        .route("/maintainability", post(handlers::quality::maintainability))
        // Security
        .route("/secrets", post(handlers::security::secrets))
        .route("/vuln", post(handlers::security::vuln))
        // Shared state
        .with_state(state)
}

/// Start the daemon server on a Unix socket
#[cfg(unix)]
pub async fn run_unix_socket(socket_path: &Path, state: Arc<DaemonState>) -> anyhow::Result<()> {
    // Check for and remove stale socket (M17)
    if is_socket_stale(socket_path).await {
        remove_stale_socket(socket_path)?;
    }

    // Create socket
    let listener = UnixListener::bind(socket_path)?;
    info!("Daemon listening on {:?}", socket_path);

    let app = build_router(state.clone());

    // Spawn idle timeout checker
    let state_clone = state.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            if state_clone.is_idle() {
                info!("Idle timeout reached, shutting down");
                std::process::exit(0);
            }
        }
    });

    // Use axum's serve with Unix socket
    loop {
        let (stream, _addr) = listener.accept().await?;
        let app = app.clone();
        let state = state.clone();

        tokio::spawn(async move {
            // Update activity timestamp
            state.touch();
            state.record_request();

            // Use hyper-util's TowerToHyperService for compatibility
            let io = hyper_util::rt::TokioIo::new(stream);
            let service = hyper_util::service::TowerToHyperService::new(app);

            if let Err(e) = hyper::server::conn::http1::Builder::new()
                .serve_connection(io, service)
                .await
            {
                error!("Connection error: {}", e);
                state.record_error();
            }
        });
    }
}

/// Start the daemon server on TCP (Windows fallback)
pub async fn run_tcp(addr: SocketAddr, state: Arc<DaemonState>) -> anyhow::Result<()> {
    let app = build_router(state.clone());

    info!("Daemon listening on {}", addr);

    // Spawn idle timeout checker
    let state_clone = state.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            if state_clone.is_idle() {
                info!("Idle timeout reached, shutting down");
                std::process::exit(0);
            }
        }
    });

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// Standard JSON response wrapper
#[derive(Debug, serde::Serialize)]
pub struct DaemonResponse<T: serde::Serialize> {
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl<T: serde::Serialize> DaemonResponse<T> {
    /// Create a successful response with a result
    pub fn ok(result: T) -> Self {
        Self {
            status: "ok",
            result: Some(result),
            error: None,
        }
    }
}

impl DaemonResponse<()> {
    /// Create an error response
    pub fn error(message: impl Into<String>) -> Self {
        DaemonResponse {
            status: "error",
            result: None,
            error: Some(message.into()),
        }
    }
}

/// Error response type for handlers
pub struct HandlerError(pub StatusCode, pub String);

impl IntoResponse for HandlerError {
    fn into_response(self) -> axum::response::Response {
        let body = serde_json::json!({
            "status": "error",
            "error": self.1
        });
        (self.0, Json(body)).into_response()
    }
}

impl<E: std::error::Error> From<E> for HandlerError {
    fn from(err: E) -> Self {
        HandlerError(StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_socket_path() {
        let path = compute_socket_path(Path::new("/tmp/test-project"), "1.0");
        assert!(path.to_string_lossy().contains("tldr-"));
        assert!(path.to_string_lossy().contains("-v1.0.sock"));
    }

    #[test]
    fn test_compute_tcp_port() {
        let port = compute_tcp_port(Path::new("/tmp/test-project"));
        assert!(port >= 49152);
        assert!(port < 59152);
    }

    #[test]
    fn test_daemon_response_ok() {
        let response = DaemonResponse::ok("pong");
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"status\":\"ok\""));
        assert!(json.contains("\"result\":\"pong\""));
    }

    #[test]
    fn test_daemon_response_error() {
        let response: DaemonResponse<()> = DaemonResponse::error("Something went wrong");
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"status\":\"error\""));
        assert!(json.contains("Something went wrong"));
    }
}
