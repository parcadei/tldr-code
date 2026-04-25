# M3 / VAL-003: Close issue #12 — MCP lifecycle handshake

**Status:** PASSED
**Issue:** parcadei/tldr-code#12 ("MCP lifecycle handshake fails: server rejects notifications/initialized (no id) and responds to notifications")
**Worker:** kraken (M3 VAL-003)
**Starting HEAD:** 00ee2dc (M1 + M2 already merged on top of f7cccff v0.2.0)
**Reproduction test (integration):** `crates/tldr-mcp/tests/mcp_lifecycle_test.rs`
**Reproduction test (unit, deserializer-only):** `crates/tldr-mcp/src/protocol.rs::tests::test_parse_notification_has_no_id`

## Triage verification

The contract cited three pre-fix line numbers. All verified at HEAD `00ee2dc` against the live source:

| Triage citation | Verified line/range | Pre-fix code (excerpt) |
|---|---|---|
| `protocol.rs:31` — `pub id: Value` | exactly line 31 of `crates/tldr-mcp/src/protocol.rs` | `pub id: Value,` (no `Option`, no `#[serde(default)]`) |
| `server.rs:34-41` — handler always emits a response frame | exactly lines 34-41 of `crates/tldr-mcp/src/server.rs` | `let response = process_request(&line, &registry); writeln!(stdout_handle, "{}", response)` (unconditional write) |
| `server.rs:65` — handler routes literal `"initialized"` | exactly line 65 of `crates/tldr-mcp/src/server.rs` | `"initialized" => handle_initialized(&request),` |

All three verified — no line drift since triage.

## The three sub-bugs

Per the spec (JSON-RPC 2.0 §4.1 + MCP 2024-11-05 lifecycle), the bugs interlock as follows:

### Sub-bug (a) — `id` is mandatory at deserialization

`JsonRpcRequest.id: Value` (no serde default) means a frame missing the `id` field is rejected by `parse_request` with `serde_json::Error: missing field \`id\` at line 1 column N`, surfacing as a JSON-RPC parse error (-32700). This makes any spec-compliant notification frame (including the canonical `notifications/initialized`) fail at the first hurdle.

### Sub-bug (b) — handler always emits a response frame

`process_request` returns `String` and the dispatch loop unconditionally writes that string to stdout. Even in the (hypothetical, post-(a)-fix) case where a notification *were* dispatched correctly, the handler would still synthesize a response frame and emit it — violating JSON-RPC 2.0 §4.1 ("a server MUST NOT reply to a notification").

### Sub-bug (c) — wrong route for `notifications/initialized`

The dispatch table at `server.rs:65` matches the literal method name `"initialized"`. MCP 2024-11-05 specifies the canonical post-handshake notification method as `"notifications/initialized"` (namespaced). Bare `"initialized"` was a v0.1.x typo — it has never been a spec form in any MCP draft. So even if (a) and (b) were resolved, a spec-compliant client sending `notifications/initialized` would receive a `method_not_found` (-32601) error.

The bugs short-circuit each other: in practice, a real client trying the spec-correct frame `{"jsonrpc":"2.0","method":"notifications/initialized"}` is blocked at (a) before ever reaching (b) or (c). Fixing only (a) without (b)+(c) would surface (c) as a `method_not_found` reply (which itself violates (b)).

## Reproduction test design

The contract requires the test to **drive the dispatcher in-process** (no `Command::new` spawning of the binary — fragile across debug/release and CI portability). The test calls `process_request(frame, &registry)` directly, bypassing stdin/stdout and the byte-frame loop.

To enable this from an integration test (under `crates/tldr-mcp/tests/`), one scaffold-only change is needed: `crates/tldr-mcp/src/lib.rs` exposes `mod server` as `pub mod server` (was private; only `run` was re-exported). This is purely a visibility change — no behavioral logic moves and no API consumer is affected because the existing `pub use server::run` continues to work unchanged. After the change, integration tests can `use tldr_mcp::server::process_request;`.

The integration test file `crates/tldr-mcp/tests/mcp_lifecycle_test.rs` contains four tests:

1. **`notification_frame_emits_no_response`** — isolated assertion that Frame B (no `id`) yields `None` from `process_request`.
2. **`lifecycle_handshake_three_frames`** — the full VAL-003 contract sequence: send Frame A (`initialize`, `id=1`), Frame B (`notifications/initialized`, no `id`), Frame C (`tools/list`, `id=2`); collect emitted frames; assert exactly 2 frames (the `initialize` and `tools/list` responses, NOT a frame for B); assert their ids are 1 and 2 respectively; assert both are `result` shape (not `error`).
3. **`unknown_notification_method_emits_no_response`** — locks down behavior for the legacy bare `"initialized"` (and any unknown notification name): notifications never receive responses even when the method is unknown.
4. **`unknown_request_method_emits_method_not_found`** — contrast case: a *request* (with `id`) using an unknown method MUST receive a `-32601` response. Ensures the fix did not over-suppress legitimate error responses.

Plus a unit test in `protocol.rs`: **`test_parse_notification_has_no_id`** — locks down that `JsonRpcRequest` deserializes a notification frame with `request.id == None`.

## RED on `00ee2dc` (literal stdout)

```
$ cargo test -p tldr-mcp --test mcp_lifecycle_test --release

running 4 tests
test unknown_request_method_emits_method_not_found ... ok
test notification_frame_emits_no_response ... FAILED
test unknown_notification_method_emits_no_response ... FAILED
test lifecycle_handshake_three_frames ... FAILED

failures:

---- notification_frame_emits_no_response stdout ----
thread 'notification_frame_emits_no_response' panicked at crates/tldr-mcp/tests/mcp_lifecycle_test.rs:57:5:
notification frame produced response (must be None per JSON-RPC 2.0 §4.1):
Some("{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32700,\"message\":\"missing field `id` at line 1 column 54\"},\"id\":null}")

---- lifecycle_handshake_three_frames stdout ----
thread 'lifecycle_handshake_three_frames' panicked at crates/tldr-mcp/tests/mcp_lifecycle_test.rs:79:5:
assertion `left == right` failed: expected exactly 2 emitted frames (initialize + tools/list, no response for notification), got 3:
["{...initialize success, id=1, with snake_case protocol_version...}",
 "{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32700,\"message\":\"missing field `id` at line 1 column 54\"},\"id\":null}",
 "{...tools/list success, id=2...}"]
  left: 3
 right: 2

---- unknown_notification_method_emits_no_response stdout ----
thread 'unknown_notification_method_emits_no_response' panicked at crates/tldr-mcp/tests/mcp_lifecycle_test.rs:128:5:
unknown-method notification must not receive a response, got:
Some("{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32700,\"message\":\"missing field `id` at line 1 column 54\"},\"id\":null}")

test result: FAILED. 1 passed; 3 failed; 0 ignored; 0 measured; 0 filtered out
```

The `unknown_request_method_emits_method_not_found` contrast test passes RED because it sends a frame *with* `id`, so it hits the bug-free request path of the dispatcher.

### RED-REASON gate analysis

The contract requires at least one of three substrings in RED stdout:

- (a) `missing field` / `missing \`id\`` — present (literal substring `missing field \`id\` at line 1 column 54` in all three failures)
- (b) more than zero frames between B and C — present (`got 3` in `lifecycle_handshake_three_frames`; the middle frame is the unwanted parse-error response to Frame B)
- (c) `unknown method` near `notifications/initialized` — not exercised, because (a) short-circuits before routing. Reaching (c) would require (a) to be fixed without (c) being fixed.

Gate (a) AND (b) both fire. Gate satisfied.

## The fix

### Patch 1 — `crates/tldr-mcp/src/protocol.rs:24-39`

```rust
/// JSON-RPC request structure.
///
/// Per JSON-RPC 2.0 + MCP 2024-11-05, requests with an `id` expect a paired
/// response, while *notifications* (e.g. `notifications/initialized`) omit
/// `id` entirely and MUST NOT receive a response. `id` is therefore optional
/// at deserialization time; the dispatcher (`server::process_request`) treats
/// `id == None` as the notification path and emits no response frame.
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
}
```

### Patch 2 — `crates/tldr-mcp/src/server.rs::process_request`

- Signature: `fn process_request(...) -> String` → `pub fn process_request(...) -> Option<String>`
- Returns `None` for notifications (`request.id.is_none()`), `Some(json)` for requests
- Routes `"notifications/initialized"` (legacy bare `"initialized"` removed)
- Notification with wrong jsonrpc version is silently dropped (cannot respond by definition)
- Notification dispatched through any handler (including unknown method) drops the result without emitting a response

The dispatch loop in `run()` is updated to skip the `writeln!` when `process_request` returns `None`. The `Option<String>` return type makes "did we emit a response?" explicit at the type level — preventing future regressions of the same shape.

### Patch 3 — `crates/tldr-mcp/src/lib.rs`

One-line scaffold change: `mod server` → `pub mod server` (and a docstring entry mentioning the module). This exposes `process_request` for integration tests in `tests/mcp_lifecycle_test.rs`. The existing `pub use server::run` re-export is preserved, so binary-side consumers see no change.

### Patch 4 — `crates/tldr-mcp/src/protocol.rs::tests::test_parse_valid_request`

The pre-existing test asserted `request.id == json!(1)`. Updated to `request.id == Some(json!(1))` to match the new `Option<Value>` shape. This test was correct in shape but tied to the old type — it is fixed in this commit (NOT weakened, NOT removed). Also added `test_parse_notification_has_no_id` as a positive deserializer-side test.

## Spec decision: removed legacy `"initialized"` route

MCP 2024-11-05 lifecycle specifies the canonical client→server post-handshake notification method as `"notifications/initialized"` (namespaced under the `notifications/` prefix used for all server-bound MCP notifications). The bare `"initialized"` route in `server.rs:65` was never a spec form in any published MCP draft — it was a v0.1.x typo from when the dispatcher was first scaffolded. Spec-compliant clients (Claude Code, the Anthropic mcp-python-sdk, the official TypeScript SDK) only ever emit the namespaced form.

Two options were considered:

1. **Replace** bare `"initialized"` with `"notifications/initialized"` (no legacy alias) — chosen.
2. Keep both routes as a compatibility shim.

Option 1 was chosen because keeping a non-spec route would mask client bugs (any client sending bare `"initialized"` would silently work locally but break against any other MCP server in the ecosystem). No real-world client emits the bare form.

## GREEN after fix

```
$ cargo test -p tldr-mcp --test mcp_lifecycle_test --release

running 4 tests
test unknown_notification_method_emits_no_response ... ok
test unknown_request_method_emits_method_not_found ... ok
test notification_frame_emits_no_response ... ok
test lifecycle_handshake_three_frames ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

Full tldr-mcp suite (45 lib + 0 main + 4 integration + 0 doc = 49 tests):

```
$ cargo test -p tldr-mcp --release
test result: ok. 45 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s   (lib)
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s    (integration)
```

## Regression checks (all PASS)

| Gate | Result |
|---|---|
| Reproduction tests (4) RED on HEAD | PASS — 3 fail with documented symptoms; 1 contrast test passes both red & green |
| Reproduction tests (4) GREEN after fix | PASS — 4/4 |
| All existing `cargo test -p tldr-mcp` lib tests | PASS — 45/45 (44 baseline + 1 new `test_parse_notification_has_no_id`); 0 broken |
| `cargo test -p tldr-cli --test exhaustive_matrix --features semantic --release` | PASS — 730/730 (unchanged from baseline) |
| `cargo test -p tldr-cli --test language_command_matrix --features semantic --release` | PASS — 234/234 (unchanged from baseline) |
| Matrix total | PASS — 964/964 (730+234) |
| `cargo clippy --workspace --all-features --tests -- -D warnings` | PASS — clean (no warnings) |
| Files touched ≤ 5 | PASS — 4 (protocol.rs, server.rs, lib.rs, tests/mcp_lifecycle_test.rs) |
| Source files touched ≤ 2 + scaffold | PASS — 2 source (protocol.rs, server.rs) + 1 scaffold (lib.rs visibility) + 1 new test |
| `#[allow]`, `#[ignore]`, `_`-prefix shenanigans, weakened assertions | NONE introduced |
| M4 coordination — `InitializeResult` left untouched | PASS — only `JsonRpcRequest` modified in protocol.rs; `InitializeResult` still has snake_case fields (M4's bug) |

## Coordination note for M4 (VAL-004)

Observed during the lifecycle_handshake_three_frames RED panic (literal stdout):

```
"{\"jsonrpc\":\"2.0\",\"result\":{\"protocol_version\":\"2024-11-05\",\"capabilities\":{\"tools\":{\"listChanged\":false}},\"server_info\":{\"name\":\"tldr-mcp\",\"version\":\"0.2.0\"}},\"id\":1}"
```

The initialize response contains `protocol_version` and `server_info` (snake_case). This is the M4 bug pre-condition — left intentionally unchanged. M4 will modify `InitializeResult` (a different struct in the same `protocol.rs` file) and the relevant call site in `handle_initialize`. M4 must base off this commit (M3's commit SHA, recorded below). The M3 changes to `protocol.rs` are scoped strictly to the `JsonRpcRequest` struct and its associated docs/tests — no overlap with `InitializeResult`.

Spec-name gotcha for M4's broader audit: any other dispatch route or struct field name lacking a `notifications/` namespace prefix or lacking a `serde(rename = ...)` should be inspected. M3 found and fixed only the JsonRpcRequest deserialization issue and the bare `"initialized"` route — M4 owns the remaining snake_case audit on the response surface.

## Files modified

- `crates/tldr-mcp/src/protocol.rs` — JsonRpcRequest.id → Option<Value>; new docstring; updated test_parse_valid_request; new test_parse_notification_has_no_id.
- `crates/tldr-mcp/src/server.rs` — process_request signature String → Option<String>; route "notifications/initialized" (replacing "initialized"); notification suppression at three exit points; run() loop adapted.
- `crates/tldr-mcp/src/lib.rs` — `mod server` → `pub mod server` (one-line scaffold for integration test access); module docstring update.
- `crates/tldr-mcp/tests/mcp_lifecycle_test.rs` — NEW. 4 integration tests + module docstring.
