//! M3 VAL-003 — MCP lifecycle handshake integration test.
//!
//! Drives `process_request` (the per-frame JSON-RPC dispatcher used by the
//! stdio server loop) in-process with synthetic frames, verifying that the
//! tldr-mcp server speaks JSON-RPC 2.0 + MCP 2024-11-05 lifecycle correctly:
//!
//!   (a) requests with `id` receive a paired response,
//!   (b) notifications (no `id` field) MUST NOT receive a response,
//!   (c) the canonical client notification `notifications/initialized` is
//!       recognized as a valid (no-op) notification rather than rejected as
//!       an unknown method or as malformed JSON for missing `id`.
//!
//! Pre-fix this test fails at the notification step because:
//!   - `JsonRpcRequest.id: Value` is non-optional, so serde rejects the
//!     notification frame with `missing field \`id\`` (parse_error -32700);
//!   - the server then emits that parse-error response to stdout,
//!     producing 3 frames total instead of the spec-required 2.

use serde_json::{json, Value};
use tldr_mcp::server::process_request;
use tldr_mcp::tools::ToolRegistry;

const FRAME_A_INITIALIZE: &str = r#"{"jsonrpc":"2.0","method":"initialize","id":1,"params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}"#;

const FRAME_B_NOTIFICATION: &str =
    r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;

const FRAME_C_TOOLS_LIST: &str = r#"{"jsonrpc":"2.0","method":"tools/list","id":2}"#;

/// Drive a single frame through `process_request` and return the emitted
/// JSON frame as a string, or `None` if the dispatcher emitted nothing
/// (the spec-correct outcome for notifications).
fn dispatch(frame: &str, registry: &ToolRegistry) -> Option<String> {
    process_request(frame, registry)
}

/// VAL-003 isolated assertion: a notification frame (no `id`) MUST NOT
/// produce a response. On unfixed HEAD this fails with a parse-error
/// response containing the substring `missing field \`id\``.
#[test]
fn notification_frame_emits_no_response() {
    let registry = ToolRegistry::new();

    let response = dispatch(FRAME_B_NOTIFICATION, &registry);

    assert!(
        response.is_none(),
        "notification frame produced response (must be None per JSON-RPC 2.0 §4.1): {:?}",
        response
    );
}

/// VAL-003 full lifecycle handshake (per MCP 2024-11-05):
///   Frame A: `initialize` request, id=1            → exactly one response, id == 1
///   Frame B: `notifications/initialized` (no id)   → ZERO responses
///   Frame C: `tools/list` request, id=2            → exactly one response, id == 2
#[test]
fn lifecycle_handshake_three_frames() {
    let registry = ToolRegistry::new();

    let mut emitted: Vec<String> = Vec::new();
    for frame in [FRAME_A_INITIALIZE, FRAME_B_NOTIFICATION, FRAME_C_TOOLS_LIST] {
        if let Some(out) = dispatch(frame, &registry) {
            emitted.push(out);
        }
    }

    assert_eq!(
        emitted.len(),
        2,
        "expected exactly 2 emitted frames (initialize + tools/list, no response for notification), got {}: {:?}",
        emitted.len(),
        emitted
    );

    let parsed_init: Value = serde_json::from_str(&emitted[0])
        .expect("frame 0 (initialize response) must be valid JSON");
    let parsed_tools: Value = serde_json::from_str(&emitted[1])
        .expect("frame 1 (tools/list response) must be valid JSON");

    assert_eq!(
        parsed_init.get("id"),
        Some(&json!(1)),
        "first emitted frame must be the initialize response with id == 1, got: {}",
        emitted[0]
    );
    assert_eq!(
        parsed_tools.get("id"),
        Some(&json!(2)),
        "second emitted frame must be the tools/list response with id == 2, got: {}",
        emitted[1]
    );

    assert!(
        parsed_init.get("result").is_some() && parsed_init.get("error").is_none(),
        "initialize response must be success (result, no error), got: {}",
        emitted[0]
    );
    assert!(
        parsed_tools.get("result").is_some() && parsed_tools.get("error").is_none(),
        "tools/list response must be success (result, no error), got: {}",
        emitted[1]
    );
}

/// Even when a notification names an UNKNOWN method, the server must stay
/// silent (per JSON-RPC 2.0 §4.1 — notifications never receive responses).
/// This locks down behavior for the legacy bare `"initialized"` and any
/// other unknown notification name.
#[test]
fn unknown_notification_method_emits_no_response() {
    let registry = ToolRegistry::new();
    let frame = r#"{"jsonrpc":"2.0","method":"some/unknown/notification"}"#;

    let response = dispatch(frame, &registry);

    assert!(
        response.is_none(),
        "unknown-method notification must not receive a response, got: {:?}",
        response
    );
}

/// Contrast case: a request frame (with `id`) using an unknown method MUST
/// receive a `method_not_found` (-32601) response — ensures we did not
/// over-suppress responses while fixing the notification path.
#[test]
fn unknown_request_method_emits_method_not_found() {
    let registry = ToolRegistry::new();
    let frame = r#"{"jsonrpc":"2.0","method":"some/unknown/request","id":42}"#;

    let response = dispatch(frame, &registry).expect("request must produce a response");

    let parsed: Value = serde_json::from_str(&response).expect("response must be valid JSON");
    assert_eq!(parsed.get("id"), Some(&json!(42)));
    assert_eq!(
        parsed
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(|c| c.as_i64()),
        Some(-32601),
        "unknown request method must yield method_not_found (-32601), got: {}",
        response
    );
}
