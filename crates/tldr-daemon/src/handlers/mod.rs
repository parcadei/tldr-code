//! Request handlers for daemon endpoints
//!
//! This module contains all HTTP handlers for the daemon's REST API.
//! Each submodule corresponds to a layer in the TLDR architecture.
//!
//! # Handler Organization
//!
//! - `ast`: Tree, structure, extract, imports (L1)
//! - `callgraph`: Calls, impact, dead, importers, arch (L2)
//! - `flow`: CFG, DFG, slice, complexity (L3/L4/L5)
//! - `search`: Regex search, context, semantic search
//! - `quality`: Code smells, maintainability index
//! - `security`: Secrets scanning, vulnerability detection

pub mod ast;
pub mod callgraph;
pub mod flow;
pub mod quality;
pub mod search;
pub mod security;

use std::sync::Arc;

use axum::{extract::State, Json};
use serde::Deserialize;

use crate::server::{DaemonResponse, HandlerError};
use crate::state::{DaemonState, DaemonStatus};

// =============================================================================
// Core Protocol Handlers
// =============================================================================

/// Ping request (health check)
#[derive(Debug, Deserialize)]
pub struct PingRequest {
    // Empty for now, but allows future extension
}

/// Ping handler - returns "pong" for health checks
pub async fn ping(
    State(state): State<Arc<DaemonState>>,
    Json(_request): Json<PingRequest>,
) -> Result<Json<DaemonResponse<&'static str>>, HandlerError> {
    state.touch();
    Ok(Json(DaemonResponse::ok("pong")))
}

/// Status request for monitoring (M22)
#[derive(Debug, Deserialize)]
pub struct StatusRequest {
    // Empty for now
}

/// Status handler - returns daemon metrics
pub async fn status(
    State(state): State<Arc<DaemonState>>,
    Json(_request): Json<StatusRequest>,
) -> Result<Json<DaemonResponse<DaemonStatus>>, HandlerError> {
    state.touch();
    let status = state.status().await;
    Ok(Json(DaemonResponse::ok(status)))
}

// =============================================================================
// Common Request/Response Types
// =============================================================================

/// Common fields for requests that operate on a file
#[derive(Debug, Deserialize)]
pub struct FileRequest {
    pub file: String,
    #[serde(default)]
    pub language: Option<String>,
}

/// Common fields for requests that operate on a function
#[derive(Debug, Deserialize)]
pub struct FunctionRequest {
    pub file: String,
    pub function: String,
    #[serde(default)]
    pub language: Option<String>,
}

/// Parse language from optional string
pub fn parse_language(lang: Option<&str>) -> Option<tldr_core::Language> {
    lang.and_then(|s| s.parse().ok())
}

/// Parse language or return error
pub fn require_language(lang: Option<&str>) -> Result<tldr_core::Language, HandlerError> {
    lang.and_then(|s| s.parse().ok()).ok_or_else(|| {
        HandlerError(
            axum::http::StatusCode::BAD_REQUEST,
            "Missing or invalid language parameter".to_string(),
        )
    })
}
