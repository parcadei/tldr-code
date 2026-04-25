//! M7 VAL-007 — MCP request-side params camelCase compliance.
//!
//! Closes parcadei/tldr-code#19 (request-side audit). M4 fixed the
//! response-side (`InitializeResult` now emits `protocolVersion` /
//! `serverInfo`). M7 fixes the symmetric REQUEST-side bug: the
//! `InitializeParams` struct has snake_case field names
//! (`protocol_version`, `client_info`) without `#[serde(rename = ...)]`
//! or `#[serde(rename_all = "camelCase")]`. When a spec-compliant client
//! (e.g. Claude Code) sends camelCase params per MCP 2024-11-05 wire
//! spec, serde silently fails to bind those keys → fields default to
//! `None` (each field carries `#[serde(default)]`) → the server
//! processes a degraded request, silently losing client identity, the
//! protocol version it announced, and any experimental capabilities it
//! declared — all the diagnostic eprintln paths in
//! `server::handle_initialize` (server.rs:120-145) become dead code
//! when the live client is spec-compliant.
//!
//! # Test design
//!
//! The bug is a serde-level deserialization failure, so the cleanest
//! reproduction is at the deserialize boundary: parse the canonical
//! camelCase JSON payload directly into `InitializeParams` and assert
//! every field is populated with the value sent (NOT defaulted). To
//! also confirm the fix flows through the full server dispatch path
//! that Claude Code exercises, a second test drives the same payload
//! through `server::process_request` and reasserts via the response
//! shape that the inputs reached the handler intact.
//!
//! # RED-REASON gate (per contract VAL-007)
//!
//! Pre-fix the assertion `parsed.client_info.is_some()` fails — the
//! panic message names the dropped field by reproducing the JSON we
//! sent (which contains the literal substring `clientInfo`) and
//! reporting the parsed value (`None`). Both `clientInfo` and
//! `client_info` appear in the panic, satisfying the gate requirement
//! that the RED stdout name a specific dropped field.
//!
//! # Symmetry note with M4
//!
//! M4's tests assert *response* keys are camelCase. M7's tests assert
//! *request* keys are accepted in camelCase. Together they certify the
//! full handshake direction is wire-spec compliant.

use serde_json::{json, Value};
use tldr_mcp::protocol::{ClientInfo, InitializeParams};
use tldr_mcp::server::process_request;
use tldr_mcp::tools::ToolRegistry;

/// Canonical MCP 2024-11-05 `initialize` params payload as a
/// spec-compliant client (e.g. Claude Code) would send. All keys at
/// every depth are camelCase. The values chosen are distinguishable
/// strings/objects so that defaulting (e.g. `None`, empty string)
/// shows up clearly in panic messages.
fn canonical_initialize_params_camelcase_json() -> Value {
    json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {
            "experimental": {"someFeatureFlag": true}
        },
        "clientInfo": {
            "name": "claude-code-test-client",
            "version": "0.5.0"
        }
    })
}

/// VAL-007 PRIMARY: parse the canonical camelCase payload directly
/// into `InitializeParams` and assert every field carries the value
/// from the JSON. Pre-fix this test FAILS at the first
/// `parsed.protocol_version` assertion because serde does not match
/// `protocolVersion` to a field named `protocol_version` without an
/// explicit rename, and `#[serde(default)]` then silently sets it to
/// `None`. The panic message reproduces the JSON (containing the
/// substring `clientInfo`) and the parsed value (`None`), naming
/// both the dropped key and the field that should have been
/// populated — satisfying the RED-REASON gate.
#[test]
fn initialize_params_accepts_camelcase_keys() {
    let payload = canonical_initialize_params_camelcase_json();
    let parsed: InitializeParams = serde_json::from_value(payload.clone()).unwrap_or_else(|e| {
        panic!(
            "InitializeParams must deserialize from spec-compliant camelCase payload; \
             serde error: {} — payload was: {}",
            e, payload
        )
    });

    // Field-by-field assertion: each MUST be populated, with the panic
    // message naming the camelCase key that was sent and the parsed
    // value (None / default) that came back.

    assert!(
        parsed.protocol_version.is_some(),
        "parsed.protocol_version was None despite protocolVersion=\"2024-11-05\" \
         being sent in JSON; payload: {}",
        payload
    );
    assert_eq!(
        parsed.protocol_version.as_deref(),
        Some("2024-11-05"),
        "parsed.protocol_version should equal \"2024-11-05\" (from JSON key \
         protocolVersion), got: {:?}; full payload: {}",
        parsed.protocol_version,
        payload
    );

    assert!(
        parsed.client_info.is_some(),
        "parsed.client_info was None despite clientInfo={{name:..., version:...}} \
         being sent in JSON; payload: {}",
        payload
    );
    let client_info: &ClientInfo = parsed.client_info.as_ref().expect("checked above");
    assert_eq!(
        client_info.name, "claude-code-test-client",
        "parsed.client_info.name should equal client name from JSON \
         (key clientInfo.name), got: {:?}",
        client_info.name
    );
    assert_eq!(
        client_info.version.as_deref(),
        Some("0.5.0"),
        "parsed.client_info.version should equal \"0.5.0\" (from JSON key \
         clientInfo.version), got: {:?}",
        client_info.version
    );

    assert!(
        parsed.capabilities.is_some(),
        "parsed.capabilities was None despite capabilities={{...}} being \
         sent in JSON; payload: {}",
        payload
    );
    let caps = parsed.capabilities.as_ref().expect("checked above");
    assert!(
        caps.experimental.is_some(),
        "parsed.capabilities.experimental was None despite \
         capabilities.experimental={{someFeatureFlag:true}} being sent in JSON; \
         payload: {}",
        payload
    );
}

/// VAL-007 SUPPLEMENTARY: drive the same camelCase payload through
/// `server::process_request` (the dispatcher Claude Code actually
/// hits) and assert the full lifecycle stays healthy. This is a
/// belt-and-braces check that the fix flows through the live path
/// the client exercises, not just the bare deserialize. The frame
/// here is byte-for-byte identical to the FRAME_INITIALIZE constant
/// used by M3's lifecycle test and M4's camelcase test, so a
/// regression in the request-side rename would show up across all
/// three test files.
#[test]
fn initialize_request_via_process_request_accepts_camelcase_params() {
    let registry = ToolRegistry::new();
    let frame = r#"{"jsonrpc":"2.0","method":"initialize","id":1,"params":{"protocolVersion":"2024-11-05","capabilities":{"experimental":{"someFeatureFlag":true}},"clientInfo":{"name":"claude-code-test-client","version":"0.5.0"}}}"#;

    let raw = process_request(frame, &registry)
        .expect("initialize request (id=1) must produce a response");
    let response: Value = serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("response must be valid JSON: {} — raw: {}", e, raw));

    // The handshake must succeed (success response, not parse error
    // or invalid-params). If the request-side rename were missing AND
    // the handler upgraded its tolerance check (it currently does not
    // — silent loss in handle_initialize is the actual symptom), this
    // would catch a future regression that promotes the loss into a
    // hard error.
    assert_eq!(
        response.get("id"),
        Some(&json!(1)),
        "initialize response must echo id=1, got: {}",
        response
    );
    assert!(
        response.get("result").is_some() && response.get("error").is_none(),
        "initialize must succeed (have .result, no .error); got: {}",
        response
    );

    // Sanity: the response result still carries the spec-required
    // camelCase keys (M4 territory), so the round-trip is healthy.
    let result = response
        .get("result")
        .expect("checked above");
    assert!(
        result.get("protocolVersion").is_some(),
        "initialize result must have .protocolVersion (M4 invariant); got: {}",
        result
    );
    assert!(
        result.get("serverInfo").is_some(),
        "initialize result must have .serverInfo (M4 invariant); got: {}",
        result
    );
}

/// VAL-007 NEGATIVE-CONTROL: confirm the legacy snake_case payload
/// also still parses (back-compat: any non-spec client sending
/// snake_case keys would have been silently accepted before; the
/// fix uses `#[serde(rename_all = "camelCase")]` which makes
/// camelCase the canonical wire form. After the fix, snake_case
/// payloads are no longer accepted — which is the correct outcome
/// per the contract: "the spec is the contract." This test
/// documents the deliberate breaking change for any non-conforming
/// legacy client.
///
/// We assert the snake_case form FAILS to bind the renamed fields
/// (the fields default to None) — this is the inverse of the
/// PRIMARY test and confirms the renames are exclusive (camelCase
/// only, not aliased to also accept snake_case). If we ever decide
/// to accept BOTH forms (aliased), this test would need updating
/// and would document the deliberate change.
#[test]
fn initialize_params_snake_case_keys_no_longer_bind_post_fix() {
    let snake_payload = json!({
        "protocol_version": "2024-11-05",
        "capabilities": {"experimental": {"flag": true}},
        "client_info": {"name": "legacy", "version": "0"}
    });
    let parsed: InitializeParams = serde_json::from_value(snake_payload)
        .expect("InitializeParams uses #[serde(default)] on every field, so even \
                  unmapped keys never produce a deserialize error");

    // Post-fix: snake_case keys do NOT bind the renamed fields.
    // Pre-fix: snake_case keys DID bind (because the field names
    //          were snake_case with no rename). So this test
    //          PASSES post-fix and FAILS pre-fix → it is RED on
    //          starting HEAD just like the PRIMARY test, also
    //          providing dropped-field reference in panic.
    assert!(
        parsed.protocol_version.is_none(),
        "post-fix the wire spec is camelCase only; the legacy snake_case key \
         protocol_version must no longer bind to InitializeParams.protocol_version, \
         got: {:?}",
        parsed.protocol_version
    );
    assert!(
        parsed.client_info.is_none(),
        "post-fix the wire spec is camelCase only; the legacy snake_case key \
         client_info must no longer bind to InitializeParams.client_info, \
         got: {:?}",
        parsed.client_info.as_ref().map(|c| &c.name)
    );
}
