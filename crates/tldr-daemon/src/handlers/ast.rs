//! AST Layer (L1) handlers: tree, structure, extract, imports
//!
//! These handlers provide file tree traversal and code structure extraction.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use axum::{extract::State, Json};
use serde::Deserialize;

use crate::server::{DaemonResponse, HandlerError};
use crate::state::DaemonState;

use tldr_core::{
    detect_or_parse_language, extract_file, get_code_structure, get_file_tree, get_imports,
    validate_file_path, CodeStructure, FileTree, ImportInfo, Language, ModuleInfo,
};

// =============================================================================
// Tree Handler
// =============================================================================

/// Tree request parameters
#[derive(Debug, Deserialize)]
pub struct TreeRequest {
    #[serde(default)]
    pub extensions: Option<Vec<String>>,
    #[serde(default = "default_true")]
    pub exclude_hidden: bool,
}

fn default_true() -> bool {
    true
}

/// Tree handler - returns file tree for the project
pub async fn tree(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<TreeRequest>,
) -> Result<Json<DaemonResponse<FileTree>>, HandlerError> {
    state.touch();

    let project = state.project().clone();

    // Convert extensions to HashSet
    let extensions: Option<HashSet<String>> = request.extensions.map(|exts| {
        exts.into_iter()
            .map(|e| {
                if e.starts_with('.') {
                    e
                } else {
                    format!(".{}", e)
                }
            })
            .collect()
    });

    // Run in blocking context for CPU-bound work (M10)
    let result = tokio::task::spawn_blocking(move || {
        get_file_tree(&project, extensions.as_ref(), request.exclude_hidden, None)
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
// Structure Handler
// =============================================================================

/// Structure request parameters
#[derive(Debug, Deserialize)]
pub struct StructureRequest {
    pub language: String,
    #[serde(default)]
    pub max_results: usize,
}

/// Structure handler - extracts code structure from all files
pub async fn structure(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<StructureRequest>,
) -> Result<Json<DaemonResponse<CodeStructure>>, HandlerError> {
    state.touch();

    let project = state.project().clone();
    let language: Language = request
        .language
        .parse()
        .map_err(|e: String| HandlerError(axum::http::StatusCode::BAD_REQUEST, e))?;
    let max_results = request.max_results;

    // Run in blocking context (M10)
    let result = tokio::task::spawn_blocking(move || {
        get_code_structure(&project, language, max_results, None)
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
// Extract Handler
// =============================================================================

/// Extract request parameters
#[derive(Debug, Deserialize)]
pub struct ExtractRequest {
    pub file: String,
}

/// Extract handler - extracts complete module info from a file
pub async fn extract(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<ExtractRequest>,
) -> Result<Json<DaemonResponse<ModuleInfo>>, HandlerError> {
    state.touch();

    let project = state.project().clone();
    let file_path = if PathBuf::from(&request.file).is_absolute() {
        PathBuf::from(&request.file)
    } else {
        project.join(&request.file)
    };

    // Run in blocking context (M10)
    let result = tokio::task::spawn_blocking(move || extract_file(&file_path, Some(&project)))
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
// Imports Handler
// =============================================================================

/// Imports request parameters
#[derive(Debug, Deserialize)]
pub struct ImportsRequest {
    pub file: String,
    #[serde(default)]
    pub language: Option<String>,
}

/// Imports handler - parses import statements from a file
pub async fn imports(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<ImportsRequest>,
) -> Result<Json<DaemonResponse<Vec<ImportInfo>>>, HandlerError> {
    state.touch();

    let project = state.project().clone();
    // VAL-006 / issue #5 (broader audit): validate caller-supplied path stays
    // inside the project root before any filesystem read. `validate_file_path`
    // resolves the path (absolute or relative) against `project` and returns
    // `PathTraversal` if the canonical form escapes. Mirrors the M1 fix in
    // `handlers/security.rs::secrets`.
    let file_path = validate_file_path(&request.file, Some(&project))
        .map_err(|e| HandlerError(axum::http::StatusCode::BAD_REQUEST, e.to_string()))?;

    // Detect language (using shared validator)
    let language = detect_or_parse_language(request.language.as_deref(), &file_path)
        .map_err(|e| HandlerError(axum::http::StatusCode::BAD_REQUEST, e.to_string()))?;

    // Run in blocking context (M10)
    let result = tokio::task::spawn_blocking(move || get_imports(&file_path, language))
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
