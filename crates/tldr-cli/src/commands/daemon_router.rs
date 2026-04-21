//! Daemon Router - Auto-route CLI commands through daemon cache
//!
//! This module provides transparent routing of CLI commands through the daemon
//! when it's running, falling back to direct compute when the daemon is unavailable.
//!
//! # Design
//!
//! Each command can call `try_daemon_route()` before doing direct compute.
//! If the daemon is running and responds successfully, the cached result is returned.
//! Otherwise, the command falls back to computing the result directly.
//!
//! # Performance
//!
//! The daemon maintains Salsa-style query memoization, providing ~35x speedup
//! on cache hits compared to direct computation.

use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;

use crate::commands::daemon::error::DaemonError;
use crate::commands::daemon::ipc::send_raw_command;

// =============================================================================
// Core Router Function
// =============================================================================

/// Try to route a command through the daemon.
///
/// Returns `Some(result)` if the daemon is running and responds successfully.
/// Returns `None` if the daemon is not running or an error occurs (caller should fallback).
///
/// # Arguments
///
/// * `project` - Project root directory (used to find the correct daemon)
/// * `endpoint` - Command name (e.g., "calls", "impact", "structure")
/// * `params` - Additional JSON parameters for the command
///
/// # Example
///
/// ```ignore
/// if let Some(result) = try_daemon_route::<CallGraphOutput>(
///     &self.path,
///     "calls",
///     json!({"language": language.to_string()})
/// ) {
///     return writer.write(&result);
/// }
/// // Fallback to direct compute...
/// ```
pub fn try_daemon_route<T: DeserializeOwned>(
    project: &Path,
    endpoint: &str,
    params: serde_json::Value,
) -> Option<T> {
    // Use blocking runtime for sync commands
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(_) => return None,
    };

    runtime.block_on(try_daemon_route_async(project, endpoint, params))
}

/// Async version of try_daemon_route.
///
/// Used internally and can be called directly from async contexts.
pub async fn try_daemon_route_async<T: DeserializeOwned>(
    project: &Path,
    endpoint: &str,
    params: serde_json::Value,
) -> Option<T> {
    // Resolve project path to absolute
    let project = project.canonicalize().unwrap_or_else(|_| {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(project)
    });

    // Build command JSON
    let mut cmd_obj = serde_json::json!({
        "cmd": endpoint.to_lowercase()
    });

    // Merge additional parameters
    if let serde_json::Value::Object(params_obj) = params {
        if let serde_json::Value::Object(ref mut cmd_map) = cmd_obj {
            for (key, value) in params_obj {
                cmd_map.insert(key, value);
            }
        }
    }

    let command_json = match serde_json::to_string(&cmd_obj) {
        Ok(json) => json,
        Err(_) => return None,
    };

    // Send to daemon
    let response = match send_raw_command(&project, &command_json).await {
        Ok(resp) => resp,
        Err(DaemonError::NotRunning) => return None,
        Err(DaemonError::ConnectionRefused) => return None,
        Err(_) => return None,
    };

    // Parse response - check for error response first
    let response_value: serde_json::Value = match serde_json::from_str(&response) {
        Ok(v) => v,
        Err(_) => return None,
    };

    // Check if response is an error
    if let Some(status) = response_value.get("status") {
        if status == "error" {
            return None;
        }
    }

    // If the response has a "result" field, extract it (daemon wraps results)
    let result_value = if response_value.get("result").is_some() {
        response_value
            .get("result")
            .cloned()
            .unwrap_or(response_value)
    } else {
        response_value
    };

    // Deserialize to target type
    serde_json::from_value(result_value).ok()
}

/// Check if the daemon is running for a project.
///
/// This is a lightweight check that doesn't send a command.
pub fn is_daemon_running(project: &Path) -> bool {
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(_) => return false,
    };

    runtime.block_on(is_daemon_running_async(project))
}

/// Async version of is_daemon_running.
pub async fn is_daemon_running_async(project: &Path) -> bool {
    use crate::commands::daemon::ipc::check_socket_alive;
    check_socket_alive(project).await
}

// =============================================================================
// Convenience Builders
// =============================================================================

/// Build JSON params with optional path.
pub fn params_with_path(path: Option<&Path>) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    if let Some(p) = path {
        obj.insert("path".to_string(), serde_json::json!(p));
    }
    serde_json::Value::Object(obj)
}

/// Build JSON params with file path.
pub fn params_with_file(file: &Path) -> serde_json::Value {
    serde_json::json!({
        "file": file
    })
}

/// Build JSON params with file and function.
pub fn params_with_file_function(file: &Path, function: &str) -> serde_json::Value {
    serde_json::json!({
        "file": file,
        "function": function
    })
}

/// Build JSON params with file, function, and line.
pub fn params_with_file_function_line(file: &Path, function: &str, line: u32) -> serde_json::Value {
    serde_json::json!({
        "file": file,
        "function": function,
        "line": line
    })
}

/// Build JSON params with function name and optional depth.
pub fn params_with_func_depth(func: &str, depth: Option<usize>) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    obj.insert("func".to_string(), serde_json::json!(func));
    if let Some(d) = depth {
        obj.insert("depth".to_string(), serde_json::json!(d));
    }
    serde_json::Value::Object(obj)
}

/// Build JSON params with module and optional path.
pub fn params_with_module(module: &str, path: Option<&Path>) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    obj.insert("module".to_string(), serde_json::json!(module));
    if let Some(p) = path {
        obj.insert("path".to_string(), serde_json::json!(p));
    }
    serde_json::Value::Object(obj)
}

/// Build JSON params with pattern and max_results.
pub fn params_with_pattern(pattern: &str, max_results: Option<usize>) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    obj.insert("pattern".to_string(), serde_json::json!(pattern));
    if let Some(m) = max_results {
        obj.insert("max_results".to_string(), serde_json::json!(m));
    }
    serde_json::Value::Object(obj)
}

/// Build JSON params with entry point and depth.
pub fn params_with_entry_depth(entry: &str, depth: Option<usize>) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    obj.insert("entry".to_string(), serde_json::json!(entry));
    if let Some(d) = depth {
        obj.insert("depth".to_string(), serde_json::json!(d));
    }
    serde_json::Value::Object(obj)
}

/// Build JSON params with path and lang.
pub fn params_with_path_lang(path: &Path, lang: Option<&str>) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    obj.insert("path".to_string(), serde_json::json!(path));
    if let Some(l) = lang {
        obj.insert("lang".to_string(), serde_json::json!(l));
    }
    serde_json::Value::Object(obj)
}

/// Build JSON params for dead code analysis.
pub fn params_for_dead(path: Option<&Path>, entry: Option<&[String]>) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    if let Some(p) = path {
        obj.insert("path".to_string(), serde_json::json!(p));
    }
    if let Some(e) = entry {
        obj.insert("entry".to_string(), serde_json::json!(e));
    }
    serde_json::Value::Object(obj)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_params_with_path() {
        let params = params_with_path(Some(Path::new("/test/path")));
        assert_eq!(params.get("path").unwrap(), "/test/path");
    }

    #[test]
    fn test_params_with_path_none() {
        let params = params_with_path(None);
        assert!(params.get("path").is_none());
    }

    #[test]
    fn test_params_with_file_function() {
        let params = params_with_file_function(Path::new("/test/file.py"), "my_func");
        assert_eq!(params.get("file").unwrap(), "/test/file.py");
        assert_eq!(params.get("function").unwrap(), "my_func");
    }

    #[test]
    fn test_params_with_file_function_line() {
        let params = params_with_file_function_line(Path::new("/test/file.py"), "my_func", 42);
        assert_eq!(params.get("file").unwrap(), "/test/file.py");
        assert_eq!(params.get("function").unwrap(), "my_func");
        assert_eq!(params.get("line").unwrap(), 42);
    }

    #[test]
    fn test_params_with_func_depth() {
        let params = params_with_func_depth("process_data", Some(5));
        assert_eq!(params.get("func").unwrap(), "process_data");
        assert_eq!(params.get("depth").unwrap(), 5);
    }

    #[test]
    fn test_params_with_pattern() {
        let params = params_with_pattern("fn main", Some(100));
        assert_eq!(params.get("pattern").unwrap(), "fn main");
        assert_eq!(params.get("max_results").unwrap(), 100);
    }

    #[test]
    fn test_is_daemon_running_no_daemon() {
        let temp = TempDir::new().unwrap();
        assert!(!is_daemon_running(temp.path()));
    }

    #[test]
    fn test_try_daemon_route_no_daemon() {
        let temp = TempDir::new().unwrap();
        let result: Option<serde_json::Value> =
            try_daemon_route(temp.path(), "ping", serde_json::json!({}));
        assert!(result.is_none());
    }
}
