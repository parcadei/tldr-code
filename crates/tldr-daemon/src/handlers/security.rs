//! Security handlers: secrets, vuln
//!
//! These handlers provide security analysis including secrets scanning
//! and vulnerability detection via taint analysis.

use std::path::PathBuf;
use std::sync::Arc;

use axum::{extract::State, Json};
use serde::Deserialize;

use crate::server::{DaemonResponse, HandlerError};
use crate::state::DaemonState;

use tldr_core::{
    scan_secrets, scan_vulnerabilities, Language, SecretsReport, Severity, VulnReport, VulnType,
};

// =============================================================================
// Secrets Handler
// =============================================================================

/// Secrets request parameters
#[derive(Debug, Deserialize)]
pub struct SecretsRequest {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default = "default_entropy_threshold")]
    pub entropy_threshold: f64,
    #[serde(default)]
    pub include_test: bool,
    #[serde(default)]
    pub severity_filter: Option<String>,
}

fn default_entropy_threshold() -> f64 {
    4.5
}

/// Secrets handler - scans for hardcoded secrets
pub async fn secrets(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<SecretsRequest>,
) -> Result<Json<DaemonResponse<SecretsReport>>, HandlerError> {
    state.touch();

    let project = state.project().clone();
    let path = if let Some(p) = &request.path {
        if PathBuf::from(p).is_absolute() {
            PathBuf::from(p)
        } else {
            project.join(p)
        }
    } else {
        project
    };

    let entropy_threshold = request.entropy_threshold;
    let include_test = request.include_test;

    // Parse severity filter
    let severity_filter: Option<Severity> =
        request
            .severity_filter
            .as_deref()
            .and_then(|s| match s.to_lowercase().as_str() {
                "low" => Some(Severity::Low),
                "medium" => Some(Severity::Medium),
                "high" => Some(Severity::High),
                "critical" => Some(Severity::Critical),
                _ => None,
            });

    // Run in blocking context (M10)
    let result = tokio::task::spawn_blocking(move || {
        scan_secrets(&path, entropy_threshold, include_test, severity_filter)
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
// Vuln Handler
// =============================================================================

/// Vuln request parameters
#[derive(Debug, Deserialize)]
pub struct VulnRequest {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub vuln_type: Option<String>,
}

/// Vuln handler - detects vulnerabilities via taint analysis
pub async fn vuln(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<VulnRequest>,
) -> Result<Json<DaemonResponse<VulnReport>>, HandlerError> {
    state.touch();

    let project = state.project().clone();
    let path = if let Some(p) = &request.path {
        if PathBuf::from(p).is_absolute() {
            PathBuf::from(p)
        } else {
            project.join(p)
        }
    } else {
        project
    };

    // Parse optional language
    let language: Option<Language> = request.language.as_deref().and_then(|s| s.parse().ok());

    // Parse vuln type filter
    let vuln_type: Option<VulnType> =
        request
            .vuln_type
            .as_deref()
            .and_then(|s| match s.to_lowercase().as_str() {
                "sqlinjection" | "sql_injection" | "sql" => Some(VulnType::SqlInjection),
                "xss" => Some(VulnType::Xss),
                "commandinjection" | "command_injection" | "command" => {
                    Some(VulnType::CommandInjection)
                }
                "pathtraversal" | "path_traversal" | "path" => Some(VulnType::PathTraversal),
                "ssrf" => Some(VulnType::Ssrf),
                "deserialization" => Some(VulnType::Deserialization),
                _ => None,
            });

    // Run in blocking context (M10)
    let result =
        tokio::task::spawn_blocking(move || scan_vulnerabilities(&path, language, vuln_type))
            .await
            .map_err(|e| {
                HandlerError(
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Task join error: {}", e),
                )
            })?
            .map_err(|e| {
                HandlerError(axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
            })?;

    Ok(Json(DaemonResponse::ok(result)))
}
