//! Flow Analysis handlers: cfg, dfg, slice, complexity (L3/L4/L5)
//!
//! These handlers provide control flow, data flow, and program slicing analysis.

use std::sync::Arc;

use axum::{extract::State, Json};
use serde::Deserialize;

use crate::server::{DaemonResponse, HandlerError};
use crate::state::DaemonState;

use tldr_core::{
    calculate_complexity, detect_or_parse_language, get_cfg_context, get_dfg_context, get_slice,
    validate_file_path, CfgInfo, ComplexityMetrics, DfgInfo, SliceDirection,
};

// =============================================================================
// CFG Handler
// =============================================================================

/// CFG request parameters
#[derive(Debug, Deserialize)]
pub struct CfgRequest {
    pub file: String,
    pub function: String,
    #[serde(default)]
    pub language: Option<String>,
}

/// CFG handler - extracts control flow graph for a function
pub async fn cfg(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<CfgRequest>,
) -> Result<Json<DaemonResponse<CfgInfo>>, HandlerError> {
    state.touch();

    let project = state.project().clone();
    // VAL-006 / issue #5 (broader audit): validate caller-supplied path stays
    // inside the project root before any filesystem read. Mirrors the M1 fix
    // pattern in `handlers/security.rs::secrets`.
    let file_path = validate_file_path(&request.file, Some(&project))
        .map_err(|e| HandlerError(axum::http::StatusCode::BAD_REQUEST, e.to_string()))?;

    // Detect language (using shared validator)
    let language = detect_or_parse_language(request.language.as_deref(), &file_path)
        .map_err(|e| HandlerError(axum::http::StatusCode::BAD_REQUEST, e.to_string()))?;

    let function = request.function;

    // Run in blocking context (M10)
    let result = tokio::task::spawn_blocking(move || {
        get_cfg_context(&file_path.to_string_lossy(), &function, language)
    })
    .await
    .map_err(|e| {
        HandlerError(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Task join error: {}", e),
        )
    })?
    .map_err(|e| HandlerError(axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(DaemonResponse::ok(result)))
}

// =============================================================================
// DFG Handler
// =============================================================================

/// DFG request parameters
#[derive(Debug, Deserialize)]
pub struct DfgRequest {
    pub file: String,
    pub function: String,
    #[serde(default)]
    pub language: Option<String>,
}

/// DFG handler - extracts data flow graph for a function
pub async fn dfg(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<DfgRequest>,
) -> Result<Json<DaemonResponse<DfgInfo>>, HandlerError> {
    state.touch();

    let project = state.project().clone();
    // VAL-006 / issue #5 (broader audit): validate caller-supplied path stays
    // inside the project root before any filesystem read. Mirrors the M1 fix
    // pattern in `handlers/security.rs::secrets`.
    let file_path = validate_file_path(&request.file, Some(&project))
        .map_err(|e| HandlerError(axum::http::StatusCode::BAD_REQUEST, e.to_string()))?;

    // Detect language (using shared validator)
    let language = detect_or_parse_language(request.language.as_deref(), &file_path)
        .map_err(|e| HandlerError(axum::http::StatusCode::BAD_REQUEST, e.to_string()))?;

    let function = request.function;

    // Run in blocking context (M10)
    let result = tokio::task::spawn_blocking(move || {
        get_dfg_context(&file_path.to_string_lossy(), &function, language)
    })
    .await
    .map_err(|e| {
        HandlerError(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Task join error: {}", e),
        )
    })?
    .map_err(|e| HandlerError(axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(DaemonResponse::ok(result)))
}

// =============================================================================
// Slice Handler
// =============================================================================

/// Slice request parameters
#[derive(Debug, Deserialize)]
pub struct SliceRequest {
    pub file: String,
    pub function: String,
    pub line: u32,
    #[serde(default = "default_direction")]
    pub direction: String,
    #[serde(default)]
    pub variable: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
}

fn default_direction() -> String {
    "backward".to_string()
}

/// Slice response
#[derive(Debug, serde::Serialize)]
pub struct SliceResponse {
    pub lines: Vec<u32>,
    pub direction: String,
    pub line_count: usize,
}

/// Slice handler - computes program slice from a line
pub async fn slice(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<SliceRequest>,
) -> Result<Json<DaemonResponse<SliceResponse>>, HandlerError> {
    state.touch();

    let project = state.project().clone();
    // VAL-006 / issue #5 (broader audit): validate caller-supplied path stays
    // inside the project root before any filesystem read. Mirrors the M1 fix
    // pattern in `handlers/security.rs::secrets`.
    let file_path = validate_file_path(&request.file, Some(&project))
        .map_err(|e| HandlerError(axum::http::StatusCode::BAD_REQUEST, e.to_string()))?;

    // Detect language (using shared validator)
    let language = detect_or_parse_language(request.language.as_deref(), &file_path)
        .map_err(|e| HandlerError(axum::http::StatusCode::BAD_REQUEST, e.to_string()))?;

    // Parse direction
    let direction: SliceDirection = request
        .direction
        .parse()
        .map_err(|e: String| HandlerError(axum::http::StatusCode::BAD_REQUEST, e))?;

    let function = request.function;
    let line = request.line;
    let variable = request.variable;
    let direction_str = request.direction.clone();

    // Run in blocking context (M10)
    let result = tokio::task::spawn_blocking(move || {
        get_slice(
            &file_path.to_string_lossy(),
            &function,
            line,
            direction,
            variable.as_deref(),
            language,
        )
    })
    .await
    .map_err(|e| {
        HandlerError(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Task join error: {}", e),
        )
    })?
    .map_err(|e| HandlerError(axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut lines: Vec<u32> = result.into_iter().collect();
    lines.sort();

    let response = SliceResponse {
        line_count: lines.len(),
        direction: direction_str,
        lines,
    };

    Ok(Json(DaemonResponse::ok(response)))
}

// =============================================================================
// Complexity Handler
// =============================================================================

/// Complexity request parameters
#[derive(Debug, Deserialize)]
pub struct ComplexityRequest {
    pub file: String,
    pub function: String,
    #[serde(default)]
    pub language: Option<String>,
}

/// Complexity handler - calculates cyclomatic and cognitive complexity
pub async fn complexity(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<ComplexityRequest>,
) -> Result<Json<DaemonResponse<ComplexityMetrics>>, HandlerError> {
    state.touch();

    let project = state.project().clone();
    // VAL-006 / issue #5 (broader audit): validate caller-supplied path stays
    // inside the project root before any filesystem read. Mirrors the M1 fix
    // pattern in `handlers/security.rs::secrets`.
    let file_path = validate_file_path(&request.file, Some(&project))
        .map_err(|e| HandlerError(axum::http::StatusCode::BAD_REQUEST, e.to_string()))?;

    // Detect language (using shared validator)
    let language = detect_or_parse_language(request.language.as_deref(), &file_path)
        .map_err(|e| HandlerError(axum::http::StatusCode::BAD_REQUEST, e.to_string()))?;

    let function = request.function;

    // Run in blocking context (M10)
    let result = tokio::task::spawn_blocking(move || {
        calculate_complexity(&file_path.to_string_lossy(), &function, language)
    })
    .await
    .map_err(|e| {
        HandlerError(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Task join error: {}", e),
        )
    })?
    .map_err(|e| HandlerError(axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(DaemonResponse::ok(result)))
}
