//! M4 VAL-004 — MCP response wire-format camelCase compliance.
//!
//! Closes parcadei/tldr-code#19: Claude Code (and any MCP 2024-11-05
//! spec-compliant client) cannot connect to tldr-mcp because the
//! `initialize` response carries snake_case top-level result fields
//! (`protocol_version`, `server_info`) instead of the spec-required
//! camelCase (`protocolVersion`, `serverInfo`). Per MCP 2024-11-05 the
//! same camelCase requirement applies to ALL response struct fields,
//! including `tools/list` (`inputSchema`), `tools/call` (`isError`),
//! and the nested `tools` capability (`listChanged`).
//!
//! # Test design (per contract VAL-004 BROADER-AUDIT)
//!
//! Drives `process_request` (the per-frame JSON-RPC dispatcher used by
//! the stdio server loop, reused from M3's lifecycle test) in-process
//! with two synthetic frames (initialize + tools/list — the day-1
//! handshake exchange Claude Code performs immediately on connection),
//! captures both responses, parses each as JSON, and walks the entire
//! `.result` object recursively, collecting every object key at every
//! depth. Asserts NO collected key matches the snake_case detector.
//!
//! # Snake_case detector
//!
//! A key is snake_case iff: it contains at least one underscore, every
//! character is ASCII lowercase letter / digit / underscore, and it does
//! NOT start with an underscore (we do not flag identifiers like `_foo`).
//! SCREAMING_CASE (any uppercase) is excluded by the lowercase check.
//! This matches the spirit of the contract regex `^[a-z]+_[a-z]+`
//! (mandates lowercase letters with at least one internal underscore)
//! while being slightly more permissive about digits within the name
//! (e.g. `field_2` is still snake_case).
//!
//! # Scope of the recursive walk
//!
//! The walk skips object keys that appear directly inside a JSON Schema
//! `properties` map nested under a `tools/list` response's
//! `inputSchema`. Reason: those keys are USER-DEFINED parameter names
//! that handlers extract via `get_optional_string(&args, "exclude_hidden")`
//! etc. — they are not MCP-defined response struct fields, and renaming
//! them would silently break every `tools/call` invocation. The MCP
//! 2024-11-05 wire-format requirement applies to MCP-defined message
//! field names, not to JSON Schema property declarations contained
//! within an `inputSchema` value. The schemas are emitted verbatim by
//! the server author (see `crates/tldr-mcp/src/tools/mod.rs`) and the
//! handlers (in `crates/tldr-mcp/src/tools/{ast,callgraph,...}.rs`)
//! consume the same names. If the orchestrator wishes to rename these
//! parameter names too, that is a separate API-breaking change, not
//! the bug Issue #19 describes.

use serde_json::{json, Value};
use std::collections::BTreeSet;
use tldr_mcp::server::process_request;
use tldr_mcp::tools::ToolRegistry;

const FRAME_INITIALIZE: &str = r#"{"jsonrpc":"2.0","method":"initialize","id":1,"params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}"#;

const FRAME_TOOLS_LIST: &str = r#"{"jsonrpc":"2.0","method":"tools/list","id":2}"#;

/// Snake_case detector matching the contract regex `^[a-z]+_[a-z]+`
/// (lowercase letters with at least one internal underscore), permissive
/// of digits within the name. Excludes leading-underscore identifiers
/// (`_foo`) and SCREAMING_CASE (`ENV_VAR`) which are not target patterns.
fn is_snake_case(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.is_empty() || bytes[0] == b'_' {
        return false;
    }
    let mut has_underscore = false;
    for &b in bytes {
        if b == b'_' {
            has_underscore = true;
            continue;
        }
        if !(b.is_ascii_lowercase() || b.is_ascii_digit()) {
            return false;
        }
    }
    has_underscore
}

/// Recursively walk a JSON value and collect every object key at every
/// depth. Skips keys directly inside any `properties` object that itself
/// sits inside an `inputSchema` value (JSON Schema parameter
/// declarations are user-defined argument names, not MCP-defined
/// response field names — see module docs for the rationale).
///
/// `path` is the dotted path of object keys leading to `value` and is
/// used to detect the JSON-Schema `properties` skip condition.
fn collect_keys(value: &Value, path: &[&str], out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            // If we are currently inside `inputSchema.<...>.properties`
            // (or any nested `properties` further inside a JSON Schema),
            // the keys of THIS object are user-defined property names —
            // collect their VALUES (recurse into them) but do NOT add
            // the keys themselves to the snake_case violation set.
            let parent_is_schema_properties =
                path.contains(&"inputSchema") && path.last() == Some(&"properties");

            for (k, v) in map {
                if !parent_is_schema_properties {
                    out.insert(k.clone());
                }
                let mut new_path: Vec<&str> = path.to_vec();
                new_path.push(k.as_str());
                collect_keys(v, &new_path, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                // Array indices are not keys — keep the same path so a
                // `properties` array (rare in JSON Schema, but possible
                // in `oneOf`/`anyOf`) still suppresses correctly.
                collect_keys(item, path, out);
            }
        }
        _ => {}
    }
}

/// Helper: dispatch a request frame, expect a Some response, parse JSON.
fn dispatch_request(frame: &str, registry: &ToolRegistry) -> Value {
    let raw = process_request(frame, registry)
        .expect("request frame must produce a response (it carries an id)");
    serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("response must be valid JSON: {} — raw: {}", e, raw))
}

/// Helper: build a sorted, comma-separated list of snake_case keys
/// found in the recursive walk. Used in panic messages so the RED
/// stdout names the actual offending keys (satisfies VAL-004 RED-REASON
/// gate: "RED stdout MUST contain the substring 'protocol_version' OR
/// 'server_info' OR a snake_case key name from the tools/list audit").
fn snake_case_violations(keys: &BTreeSet<String>) -> Vec<String> {
    keys.iter()
        .filter(|k| is_snake_case(k))
        .cloned()
        .collect()
}

/// Sanity unit tests for the detector and walker (so a failure in
/// `initialize_response_uses_camel_case` is unambiguously a real bug
/// in the server, not a bug in this test's helpers).
#[test]
fn snake_case_detector_recognizes_target_pattern() {
    assert!(is_snake_case("protocol_version"));
    assert!(is_snake_case("server_info"));
    assert!(is_snake_case("list_changed"));
    assert!(is_snake_case("is_error"));
    assert!(is_snake_case("input_schema"));
    assert!(is_snake_case("max_results"));
    assert!(is_snake_case("field_2"));

    assert!(!is_snake_case("protocolVersion"));
    assert!(!is_snake_case("serverInfo"));
    assert!(!is_snake_case("type"));
    assert!(!is_snake_case("name"));
    assert!(!is_snake_case("ENV_VAR")); // SCREAMING_CASE not flagged
    assert!(!is_snake_case("_private")); // leading underscore not flagged
    assert!(!is_snake_case("")); // empty not flagged
    assert!(!is_snake_case("noUnderscore")); // no underscore not flagged
}

#[test]
fn collect_keys_walks_recursively_and_skips_schema_properties() {
    let payload = json!({
        "outer_field": {
            "tools": [
                {
                    "name": "tldr_tree",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "exclude_hidden": {"type": "boolean"},
                            "max_results": {"type": "integer"}
                        }
                    }
                }
            ]
        }
    });
    let mut keys = BTreeSet::new();
    collect_keys(&payload, &[], &mut keys);

    // Top-level + nested struct keys are collected.
    assert!(keys.contains("outer_field"));
    assert!(keys.contains("tools"));
    assert!(keys.contains("name"));
    assert!(keys.contains("inputSchema"));
    assert!(keys.contains("type"));
    assert!(keys.contains("properties"));

    // JSON Schema property names (under inputSchema.properties) are
    // suppressed — they are user-defined argument names, not response
    // struct fields.
    assert!(
        !keys.contains("exclude_hidden"),
        "JSON Schema property keys must be skipped by the recursive walk; got: {:?}",
        keys
    );
    assert!(
        !keys.contains("max_results"),
        "JSON Schema property keys must be skipped by the recursive walk; got: {:?}",
        keys
    );
}

/// VAL-004 PRIMARY: the `initialize` response's result object must use
/// camelCase struct field names per MCP 2024-11-05 wire format. Pre-fix
/// this test FAILS with the snake_case keys `protocol_version` and
/// `server_info` enumerated in the panic message — satisfying the
/// RED-REASON gate (substring `protocol_version` and/or `server_info`
/// must appear in RED stdout).
#[test]
fn initialize_response_uses_camel_case() {
    let registry = ToolRegistry::new();
    let response = dispatch_request(FRAME_INITIALIZE, &registry);

    let result = response
        .get("result")
        .unwrap_or_else(|| panic!("initialize response must have a `result` field; got: {}", response));

    let mut keys = BTreeSet::new();
    collect_keys(result, &[], &mut keys);

    let violations = snake_case_violations(&keys);
    assert!(
        violations.is_empty(),
        "initialize response contains snake_case keys (MCP 2024-11-05 requires camelCase): {:?}\nfull collected keys: {:?}\nfull response: {}",
        violations,
        keys,
        response
    );

    // Positive confirmation: after the rename, the canonical camelCase
    // keys must be present (not just absent of snake_case — also that
    // we did not accidentally drop the fields entirely).
    assert!(
        result.get("protocolVersion").is_some(),
        "initialize result must expose .protocolVersion (camelCase per MCP 2024-11-05); got: {}",
        result
    );
    assert!(
        result.get("serverInfo").is_some(),
        "initialize result must expose .serverInfo (camelCase per MCP 2024-11-05); got: {}",
        result
    );
}

/// VAL-004 BROADER-AUDIT: the `tools/list` response's result object
/// (and every nested struct it contains) must also use camelCase.
/// This is the second frame Claude Code sends immediately after
/// `initialize`; if it ALSO contains snake_case keys, a half-fix that
/// only renamed `protocol_version`/`server_info` would still leave
/// Claude Code blocked at this step.
///
/// JSON Schema property declarations under `inputSchema.properties`
/// are intentionally NOT flagged (see module docs).
#[test]
fn tools_list_response_uses_camel_case() {
    let registry = ToolRegistry::new();
    let response = dispatch_request(FRAME_TOOLS_LIST, &registry);

    let result = response
        .get("result")
        .unwrap_or_else(|| panic!("tools/list response must have a `result` field; got: {}", response));

    let mut keys = BTreeSet::new();
    collect_keys(result, &[], &mut keys);

    let violations = snake_case_violations(&keys);
    assert!(
        violations.is_empty(),
        "tools/list response contains snake_case keys outside JSON Schema property maps (MCP 2024-11-05 requires camelCase): {:?}\nall collected keys: {:?}",
        violations,
        keys
    );
}

/// VAL-004 COMBINED: the day-1 handshake exchange (initialize +
/// tools/list, in order) must collectively produce zero snake_case
/// keys outside JSON Schema property maps. This is the literal
/// shipping criterion from the contract: "ANY response Claude Code
/// could receive during a normal session contains zero snake_case
/// keys at any depth."
#[test]
fn day_one_handshake_responses_have_zero_snake_case_keys() {
    let registry = ToolRegistry::new();

    let mut all_violations: Vec<(&str, Vec<String>)> = Vec::new();
    for (label, frame) in [
        ("initialize", FRAME_INITIALIZE),
        ("tools/list", FRAME_TOOLS_LIST),
    ] {
        let response = dispatch_request(frame, &registry);
        let result = response
            .get("result")
            .unwrap_or_else(|| panic!("{} response must have a `result` field; got: {}", label, response));

        let mut keys = BTreeSet::new();
        collect_keys(result, &[], &mut keys);
        let v = snake_case_violations(&keys);
        if !v.is_empty() {
            all_violations.push((label, v));
        }
    }

    assert!(
        all_violations.is_empty(),
        "MCP 2024-11-05 wire-format violation: snake_case keys found in day-1 handshake responses: {:?}",
        all_violations
    );
}
