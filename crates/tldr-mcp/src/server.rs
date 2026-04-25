//! MCP stdio server loop

use crate::protocol::{
    parse_request, serialize_response, InitializeParams, InitializeResult, JsonRpcError,
    JsonRpcRequest, JsonRpcResponse, ToolsCallParams, ToolsListResult, JSONRPC_VERSION,
};
use crate::tools::ToolRegistry;

use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

/// Run the MCP stdio server (blocking).
pub fn run() {
    let registry = ToolRegistry::new();
    eprintln!("TLDR MCP server ready ({} tools)", registry.tool_count());

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut stdout_handle = stdout.lock();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("Error reading stdin: {}", e);
                continue;
            }
        };

        if line.trim().is_empty() {
            continue;
        }

        // `process_request` returns `None` for JSON-RPC notifications
        // (frames without `id`) — those MUST NOT receive a response per
        // JSON-RPC 2.0 §4.1, so we silently skip writing a frame.
        if let Some(response) = process_request(&line, &registry) {
            if let Err(e) = writeln!(stdout_handle, "{}", response) {
                eprintln!("Error writing to stdout: {}", e);
            }
            if let Err(e) = stdout_handle.flush() {
                eprintln!("Error flushing stdout: {}", e);
            }
        }
    }
}

/// Dispatch a single JSON-RPC frame.
///
/// Returns `Some(serialized_response)` for requests (frames carrying an `id`)
/// and `None` for notifications (frames without `id`). Per JSON-RPC 2.0 §4.1
/// and MCP 2024-11-05 lifecycle, the server MUST NOT emit a response frame
/// for notifications such as `notifications/initialized`.
///
/// If the frame fails to parse as JSON, a parse-error response with
/// `id: null` is emitted (per JSON-RPC 2.0, parse errors return `id: null`
/// because the id is unknown). An `invalid_request` error response is
/// emitted when a request frame uses the wrong `jsonrpc` version. A
/// notification frame with the wrong `jsonrpc` version is silently dropped
/// (the client is not waiting for a response by definition).
pub fn process_request(input: &str, registry: &ToolRegistry) -> Option<String> {
    let request = match parse_request(input) {
        Ok(req) => req,
        Err(err) => {
            // Parse errors lose the id (frame may be unparseable), so we
            // cannot tell if the sender intended a notification. Per JSON-RPC
            // 2.0, parse errors are returned with id:null. We do so.
            return Some(serialize_response(&JsonRpcResponse::error(
                Value::Null,
                err,
            )));
        }
    };

    let is_notification = request.id.is_none();

    if request.jsonrpc != JSONRPC_VERSION {
        if is_notification {
            // Notifications never receive a response, even on error.
            return None;
        }
        return Some(serialize_response(&JsonRpcResponse::error(
            request.id.clone().unwrap_or(Value::Null),
            JsonRpcError::invalid_request(format!(
                "Invalid JSON-RPC version: expected {}, got {}",
                JSONRPC_VERSION, request.jsonrpc
            )),
        )));
    }

    let result = match request.method.as_str() {
        "initialize" => handle_initialize(&request),
        // MCP 2024-11-05 spec uses the namespaced method name
        // `notifications/initialized` for the post-handshake client → server
        // notification. The legacy bare `"initialized"` route was never
        // spec-correct and is removed; spec-compliant clients use the
        // namespaced form.
        "notifications/initialized" => handle_initialized(&request),
        "tools/list" => handle_tools_list(&request, registry),
        "tools/call" => handle_tools_call(&request, registry),
        "shutdown" => handle_shutdown(&request),
        _ => Err(JsonRpcError::method_not_found(&request.method)),
    };

    if is_notification {
        // Per JSON-RPC 2.0 §4.1: a server MUST NOT reply to a notification,
        // even when the dispatched handler returned an error. Side-effects
        // (e.g. logging in `handle_initialized`) have already executed above.
        return None;
    }

    // Safety: `is_notification == false` ⟹ `request.id.is_some()`.
    let id = request.id.unwrap_or(Value::Null);
    Some(match result {
        Ok(value) => serialize_response(&JsonRpcResponse::success(id, value)),
        Err(err) => serialize_response(&JsonRpcResponse::error(id, err)),
    })
}

fn handle_initialize(request: &JsonRpcRequest) -> Result<Value, JsonRpcError> {
    let params: Option<InitializeParams> = request
        .params
        .as_ref()
        .and_then(|p| serde_json::from_value(p.clone()).ok());

    if let Some(ref p) = params {
        if let Some(ref info) = p.client_info {
            eprintln!(
                "MCP client: {} v{}",
                info.name,
                info.version.as_deref().unwrap_or("unknown")
            );
        }
        if let Some(ref ver) = p.protocol_version {
            eprintln!("MCP protocol version: {}", ver);
        }
        if let Some(ref caps) = p.capabilities {
            if caps.experimental.is_some() {
                eprintln!("Client has experimental capabilities");
            }
        }
    }

    let result = InitializeResult::default();
    serde_json::to_value(result).map_err(|e| JsonRpcError::internal_error(e.to_string()))
}

fn handle_initialized(_request: &JsonRpcRequest) -> Result<Value, JsonRpcError> {
    Ok(json!({}))
}

fn handle_tools_list(
    _request: &JsonRpcRequest,
    registry: &ToolRegistry,
) -> Result<Value, JsonRpcError> {
    let result = ToolsListResult {
        tools: registry.list_tools(),
    };
    serde_json::to_value(result).map_err(|e| JsonRpcError::internal_error(e.to_string()))
}

fn handle_tools_call(
    request: &JsonRpcRequest,
    registry: &ToolRegistry,
) -> Result<Value, JsonRpcError> {
    let params: ToolsCallParams = request
        .params
        .as_ref()
        .ok_or_else(|| JsonRpcError::invalid_params("Missing params"))?
        .clone()
        .try_into()
        .map_err(|_| JsonRpcError::invalid_params("Invalid params format"))?;

    let result = registry.call_tool(&params.name, params.arguments);

    serde_json::to_value(result).map_err(|e| {
        JsonRpcError::with_data(
            -32603,
            "Failed to serialize tool result",
            json!({ "detail": e.to_string() }),
        )
    })
}

fn handle_shutdown(_request: &JsonRpcRequest) -> Result<Value, JsonRpcError> {
    Ok(json!(null))
}
