//! Search handlers: regex search, context
//!
//! These handlers provide text search and LLM context extraction.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use axum::{extract::State, Json};
use serde::Deserialize;

use crate::server::{DaemonResponse, HandlerError};
use crate::state::DaemonState;

use tldr_core::{
    get_relevant_context, search as regex_search, Language, RelevantContext, SearchMatch,
};

// =============================================================================
// Search Handler
// =============================================================================

/// Search request parameters
#[derive(Debug, Deserialize)]
pub struct SearchRequest {
    pub pattern: String,
    #[serde(default)]
    pub extensions: Option<Vec<String>>,
    #[serde(default)]
    pub context_lines: Option<usize>,
    #[serde(default = "default_max_results")]
    pub max_results: usize,
    #[serde(default = "default_max_files")]
    pub max_files: usize,
}

fn default_max_results() -> usize {
    100
}

fn default_max_files() -> usize {
    50
}

/// Search response
#[derive(Debug, serde::Serialize)]
pub struct SearchResponse {
    pub matches: Vec<SearchMatch>,
    pub total_matches: usize,
    pub files_searched: usize,
}

/// Search handler - regex pattern search across files
pub async fn search(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<SearchRequest>,
) -> Result<Json<DaemonResponse<SearchResponse>>, HandlerError> {
    state.touch();

    let project = state.project().clone();
    let pattern = request.pattern;
    let context_lines = request.context_lines.unwrap_or(0);
    let max_results = request.max_results;
    let max_files = request.max_files;

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

    // Run in blocking context (M10)
    let result = tokio::task::spawn_blocking(move || {
        regex_search(
            &pattern,
            &project,
            extensions.as_ref(),
            context_lines,
            max_results,
            max_files,
            None,
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

    let response = SearchResponse {
        total_matches: result.len(),
        files_searched: result.iter().map(|m| &m.file).collect::<HashSet<_>>().len(),
        matches: result,
    };

    Ok(Json(DaemonResponse::ok(response)))
}

// =============================================================================
// Context Handler
// =============================================================================

/// Context request parameters
#[derive(Debug, Deserialize)]
pub struct ContextRequest {
    pub entry: String,
    #[serde(default = "default_depth")]
    pub depth: usize,
    pub language: String,
    #[serde(default)]
    pub include_docstrings: bool,
    /// Optional file path filter to disambiguate common function names
    #[serde(default)]
    pub file: Option<String>,
}

fn default_depth() -> usize {
    2
}

/// Context handler - extracts LLM-ready context from entry point
pub async fn context(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<ContextRequest>,
) -> Result<Json<DaemonResponse<ContextResponse>>, HandlerError> {
    state.touch();

    let language: Language = request
        .language
        .parse()
        .map_err(|e: String| HandlerError(axum::http::StatusCode::BAD_REQUEST, e))?;

    let project = state.project().clone();
    let entry = request.entry;
    let depth = request.depth;
    let include_docstrings = request.include_docstrings;
    let file_filter: Option<PathBuf> = request.file.map(PathBuf::from);

    // Run in blocking context (M10)
    let result = tokio::task::spawn_blocking(move || {
        get_relevant_context(
            &project,
            &entry,
            depth,
            language,
            include_docstrings,
            file_filter.as_deref(),
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

    let response = ContextResponse {
        entry_point: result.entry_point.clone(),
        depth: result.depth,
        function_count: result.functions.len(),
        context: result,
    };

    Ok(Json(DaemonResponse::ok(response)))
}

/// Context response with additional metadata
#[derive(Debug, serde::Serialize)]
pub struct ContextResponse {
    pub entry_point: String,
    pub depth: usize,
    pub function_count: usize,
    pub context: RelevantContext,
}
