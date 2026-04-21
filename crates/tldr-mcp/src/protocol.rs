//! JSON-RPC 2.0 protocol handling for MCP
//!
//! This module implements the JSON-RPC protocol layer for the Model Context Protocol (MCP).
//! It handles request parsing, response formatting, and error codes per the MCP specification.
//!
//! # Protocol Overview
//!
//! MCP uses JSON-RPC 2.0 over stdio with the following methods:
//! - `initialize` - Initial handshake with capabilities
//! - `tools/list` - List all available tools
//! - `tools/call` - Execute a tool
//!
//! # Mitigation: M14 (MCP JSON-RPC Protocol Compliance)
//!
//! This implementation follows the MCP specification exactly to ensure compatibility
//! with Claude Code and other MCP clients.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// JSON-RPC version string
pub const JSONRPC_VERSION: &str = "2.0";

/// JSON-RPC request structure
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
    pub id: Value,
}

/// JSON-RPC response structure
#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
    pub id: Value,
}

impl JsonRpcResponse {
    /// Create a success response
    pub fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_string(),
            result: Some(result),
            error: None,
            id,
        }
    }

    /// Create an error response
    pub fn error(id: Value, error: JsonRpcError) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_string(),
            result: None,
            error: Some(error),
            id,
        }
    }
}

/// JSON-RPC error structure
#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcError {
    /// Create a new error
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    /// Create error with additional data
    pub fn with_data(code: i32, message: impl Into<String>, data: Value) -> Self {
        Self {
            code,
            message: message.into(),
            data: Some(data),
        }
    }

    // Standard JSON-RPC error codes

    /// Parse error (-32700)
    pub fn parse_error(message: impl Into<String>) -> Self {
        Self::new(-32700, message)
    }

    /// Invalid request (-32600)
    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(-32600, message)
    }

    /// Method not found (-32601)
    pub fn method_not_found(method: &str) -> Self {
        Self::new(-32601, format!("Method not found: {}", method))
    }

    /// Invalid params (-32602)
    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(-32602, message)
    }

    /// Internal error (-32603)
    pub fn internal_error(message: impl Into<String>) -> Self {
        Self::new(-32603, message)
    }
}

/// MCP initialize request parameters
#[derive(Debug, Clone, Deserialize)]
pub struct InitializeParams {
    #[serde(default)]
    pub protocol_version: Option<String>,
    #[serde(default)]
    pub capabilities: Option<ClientCapabilities>,
    #[serde(default)]
    pub client_info: Option<ClientInfo>,
}

/// Client capabilities
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ClientCapabilities {
    #[serde(default)]
    pub experimental: Option<Value>,
}

/// Client information
#[derive(Debug, Clone, Deserialize)]
pub struct ClientInfo {
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
}

/// MCP initialize response result
#[derive(Debug, Clone, Serialize)]
pub struct InitializeResult {
    pub protocol_version: String,
    pub capabilities: ServerCapabilities,
    pub server_info: ServerInfo,
}

/// Server capabilities
#[derive(Debug, Clone, Serialize)]
pub struct ServerCapabilities {
    pub tools: ToolsCapability,
}

/// Tools capability
#[derive(Debug, Clone, Serialize)]
pub struct ToolsCapability {
    #[serde(rename = "listChanged")]
    pub list_changed: bool,
}

/// Server information
#[derive(Debug, Clone, Serialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

impl Default for InitializeResult {
    fn default() -> Self {
        Self {
            protocol_version: "2024-11-05".to_string(),
            capabilities: ServerCapabilities {
                tools: ToolsCapability {
                    list_changed: false,
                },
            },
            server_info: ServerInfo {
                name: "tldr-mcp".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
        }
    }
}

/// MCP tools/list response
#[derive(Debug, Clone, Serialize)]
pub struct ToolsListResult {
    pub tools: Vec<ToolDefinition>,
}

/// Tool definition for tools/list response
#[derive(Debug, Clone, Serialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

/// MCP tools/call request parameters
#[derive(Debug, Clone, Deserialize)]
pub struct ToolsCallParams {
    pub name: String,
    #[serde(default)]
    pub arguments: Value,
}

/// MCP tools/call response result
#[derive(Debug, Clone, Serialize)]
pub struct ToolsCallResult {
    pub content: Vec<ContentItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "isError")]
    pub is_error: Option<bool>,
}

impl ToolsCallResult {
    /// Create a text result
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![ContentItem::text(text)],
            is_error: None,
        }
    }

    /// Create an error result
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            content: vec![ContentItem::text(message)],
            is_error: Some(true),
        }
    }
}

/// Content item in tool result
#[derive(Debug, Clone, Serialize)]
pub struct ContentItem {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: String,
}

impl ContentItem {
    /// Create a text content item
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content_type: "text".to_string(),
            text: text.into(),
        }
    }
}

/// Convert a JSON `Value` into `ToolsCallParams` for use in `tools/call` dispatch.
impl TryFrom<Value> for ToolsCallParams {
    type Error = ();

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        serde_json::from_value(value).map_err(|_| ())
    }
}

/// Parse a JSON-RPC request from a string
pub fn parse_request(input: &str) -> Result<JsonRpcRequest, JsonRpcError> {
    serde_json::from_str(input).map_err(|e| JsonRpcError::parse_error(e.to_string()))
}

/// Serialize a JSON-RPC response to a string
pub fn serialize_response(response: &JsonRpcResponse) -> String {
    serde_json::to_string(response).unwrap_or_else(|e| {
        // Fallback error response if serialization fails
        format!(
            r#"{{"jsonrpc":"2.0","error":{{"code":-32603,"message":"Serialization error: {}"}},"id":null}}"#,
            e
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_request() {
        let input = r#"{"jsonrpc":"2.0","method":"tools/list","id":1}"#;
        let request = parse_request(input).unwrap();
        assert_eq!(request.jsonrpc, "2.0");
        assert_eq!(request.method, "tools/list");
        assert_eq!(request.id, serde_json::json!(1));
    }

    #[test]
    fn test_parse_request_with_params() {
        let input = r#"{"jsonrpc":"2.0","method":"tools/call","params":{"name":"tree","arguments":{}},"id":"abc"}"#;
        let request = parse_request(input).unwrap();
        assert_eq!(request.method, "tools/call");
        assert!(request.params.is_some());
    }

    #[test]
    fn test_parse_invalid_json() {
        let input = r#"{"jsonrpc":"2.0","method":"#;
        let err = parse_request(input).unwrap_err();
        assert_eq!(err.code, -32700);
    }

    #[test]
    fn test_success_response() {
        let response =
            JsonRpcResponse::success(serde_json::json!(1), serde_json::json!({"result": "ok"}));
        assert_eq!(response.jsonrpc, "2.0");
        assert!(response.result.is_some());
        assert!(response.error.is_none());
    }

    #[test]
    fn test_error_response() {
        let response = JsonRpcResponse::error(
            serde_json::json!(1),
            JsonRpcError::method_not_found("unknown"),
        );
        assert_eq!(response.jsonrpc, "2.0");
        assert!(response.result.is_none());
        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32601);
    }

    #[test]
    fn test_tools_call_result_text() {
        let result = ToolsCallResult::text("Hello, world!");
        assert_eq!(result.content.len(), 1);
        assert_eq!(result.content[0].content_type, "text");
        assert!(result.is_error.is_none());
    }

    #[test]
    fn test_tools_call_result_error() {
        let result = ToolsCallResult::error("Something went wrong");
        assert!(result.is_error == Some(true));
    }
}
