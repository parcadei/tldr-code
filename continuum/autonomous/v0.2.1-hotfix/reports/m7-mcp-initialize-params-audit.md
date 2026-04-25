# M7 VAL-007 — MCP Initialize Params Audit (Request-Side)

**Issue:** parcadei/tldr-code#19 (request-side audit — extends M4's response-side fix)
**Worker:** kraken (M7 VAL-007)
**Starting HEAD:** `a573504` (chore: prep v0.2.1 release)

## Scope

M4 (commit `2726358`) fixed the **response-side** wire-format compliance: `InitializeResult` now emits `protocolVersion` / `serverInfo` (camelCase). M7 audits the **request-side**: every struct in `crates/tldr-mcp/src/protocol.rs` deserialized from incoming client params, verifying it accepts spec-compliant camelCase keys per MCP 2024-11-05.

## Step 0 — Audit set

Read `crates/tldr-mcp/src/protocol.rs` end-to-end. Cross-referenced against `crates/tldr-mcp/src/server.rs` to identify which `Deserialize`-deriving structs are used as `params` types in dispatch handlers. The audit set:

| # | Struct | Defined at | Used as params for | Notes |
|---|--------|------------|---------------------|-------|
| 1 | `JsonRpcRequest` | protocol.rs:32 | wraps every request | M3 territory; only `jsonrpc`/`method`/`params`/`id` (no snake_case) |
| 2 | `InitializeParams` | protocol.rs:142 | `initialize` method (server.rs:121-124) | **AUDIT TARGET** |
| 3 | `ClientCapabilities` | protocol.rs:154 | nested in `InitializeParams.capabilities` | single-word field `experimental` |
| 4 | `ClientInfo` | protocol.rs:161 | nested in `InitializeParams.client_info` | single-word fields `name`, `version` |
| 5 | `ToolsCallParams` | protocol.rs:239 | `tools/call` method (server.rs:166-172) | single-word fields `name`, `arguments` |

`JsonRpcResponse`, `JsonRpcError`, `InitializeResult`, `ServerCapabilities`, `ToolsCapability`, `ServerInfo`, `ToolsListResult`, `ToolDefinition`, `ToolsCallResult`, `ContentItem` are `Serialize` only (response-side, M4 territory) — excluded from M7 scope.

## Field-level audit

| Struct | Field | snake_case? | Has `rename` attr? | Has `rename_all` on struct? | Status |
|--------|-------|-------------|---------------------|------------------------------|--------|
| `JsonRpcRequest` | `jsonrpc` | no | n/a | no | OK |
| `JsonRpcRequest` | `method` | no | n/a | no | OK |
| `JsonRpcRequest` | `params` | no | n/a | no | OK |
| `JsonRpcRequest` | `id` | no | n/a | no | OK |
| `InitializeParams` | `protocol_version` | **YES** | NO | NO (pre-fix) | **FAIL — silently drops `protocolVersion`** |
| `InitializeParams` | `capabilities` | no | n/a | NO (pre-fix) | OK |
| `InitializeParams` | `client_info` | **YES** | NO | NO (pre-fix) | **FAIL — silently drops `clientInfo`** |
| `ClientCapabilities` | `experimental` | no (single word) | n/a | n/a | OK |
| `ClientInfo` | `name` | no (single word) | n/a | n/a | OK |
| `ClientInfo` | `version` | no (single word) | n/a | n/a | OK |
| `ToolsCallParams` | `name` | no (single word) | n/a | n/a | OK |
| `ToolsCallParams` | `arguments` | no (single word) | n/a | n/a | OK |

**Issues found:** 1 struct (`InitializeParams`), 2 affected fields (`protocol_version`, `client_info`).

This matches the deferral note explicitly recorded in M4's status_resolution: *"`InitializeParams`/`ClientInfo` (Deserialize-only, incoming params) deliberately left unchanged — out of scope for VAL-004 wire-format-response gate; the silent `.and_then(|p| from_value(p).ok())` swallow at `server.rs:121-124` causes only loss of three eprintln client-info log lines, not handshake failure; flagged for v0.2.2 review."* M7 closes that flag.

## How the bug presents

`InitializeParams` carries `#[serde(default)]` on every field (see protocol.rs:144-152 pre-fix). When serde encounters an incoming JSON key like `protocolVersion` and looks for a field named `protocolVersion` on the struct, it finds none (the field is `protocol_version`), and because of `#[serde(default)]` it silently fills with `None` rather than erroring. The handler at `server.rs:120-145` then unconditionally proceeds:

```rust
let params: Option<InitializeParams> = request
    .params
    .as_ref()
    .and_then(|p| serde_json::from_value(p.clone()).ok());
//                                                ^^^^ swallows any error too,
//                                                     so even if we removed
//                                                     #[serde(default)] above
//                                                     the handler would still
//                                                     drop the params silently.

if let Some(ref p) = params {
    if let Some(ref info) = p.client_info {       // <- never enters: client_info is None
        eprintln!("MCP client: {} v{}", info.name, ...);
    }
    if let Some(ref ver) = p.protocol_version {   // <- never enters: protocol_version is None
        eprintln!("MCP protocol version: {}", ver);
    }
    if let Some(ref caps) = p.capabilities {
        if caps.experimental.is_some() {
            eprintln!("Client has experimental capabilities");
        }
    }
}
```

The `initialize` response is still returned (because `InitializeResult::default()` doesn't echo any client-supplied data), so Claude Code's day-1 handshake still **completes** — but every subsequent eprintln-driven diagnostic on the server side is dead code, and any future server logic that reads the announced `protocol_version` for behavior selection (e.g., toggling features per protocol-version negotiation) would silently default to the pre-handshake assumption. This is a quiet correctness bug masquerading as a logging-only issue.

## Reproduction test

File: `crates/tldr-mcp/tests/mcp_request_params_test.rs` — 3 tests, mirrors M3's lifecycle and M4's camelcase test conventions (uses `tldr_mcp::server::process_request` and `tldr_mcp::protocol::InitializeParams` directly, in-process, no shell).

| # | Test | Purpose |
|---|------|---------|
| 1 | `initialize_params_accepts_camelcase_keys` | PRIMARY — parses canonical camelCase JSON directly into `InitializeParams`; asserts every field is populated (NOT defaulted) |
| 2 | `initialize_request_via_process_request_accepts_camelcase_params` | SUPPLEMENTARY — drives the same payload through `process_request` end-to-end; asserts handshake succeeds |
| 3 | `initialize_params_snake_case_keys_no_longer_bind_post_fix` | NEGATIVE-CONTROL — confirms snake_case keys do NOT bind post-fix (camelCase is exclusive, not aliased) |

## RED — on starting HEAD `a573504` (pre-fix)

```
running 3 tests
test initialize_params_accepts_camelcase_keys ... FAILED
test initialize_params_snake_case_keys_no_longer_bind_post_fix ... FAILED
test initialize_request_via_process_request_accepts_camelcase_params ... ok

failures:

---- initialize_params_accepts_camelcase_keys stdout ----

thread 'initialize_params_accepts_camelcase_keys' (197018744) panicked at crates/tldr-mcp/tests/mcp_request_params_test.rs:92:5:
parsed.protocol_version was None despite protocolVersion="2024-11-05" being sent in JSON; payload: {"protocolVersion":"2024-11-05","capabilities":{"experimental":{"someFeatureFlag":true}},"clientInfo":{"name":"claude-code-test-client","version":"0.5.0"}}

---- initialize_params_snake_case_keys_no_longer_bind_post_fix stdout ----

thread 'initialize_params_snake_case_keys_no_longer_bind_post_fix' (197018745) panicked at crates/tldr-mcp/tests/mcp_request_params_test.rs:231:5:
post-fix the wire spec is camelCase only; the legacy snake_case key protocol_version must no longer bind to InitializeParams.protocol_version, got: Some("2024-11-05")

failures:
    initialize_params_accepts_camelcase_keys
    initialize_params_snake_case_keys_no_longer_bind_post_fix

test result: FAILED. 1 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

**RED-REASON gate satisfied:**
- PRIMARY test panic stdout contains the literal substring `parsed.protocol_version was None despite protocolVersion="2024-11-05" being sent in JSON` — names a specific dropped field (`protocol_version`), the camelCase key sent (`protocolVersion`), and the full JSON payload with the literal substring `clientInfo` reproducing the second dropped key.
- The third test (NEGATIVE-CONTROL) shows the inverse symptom (`got: Some("2024-11-05")` for the snake_case-keyed payload), confirming pre-fix the snake_case form binds — deliberate breaking change documented.

The SUPPLEMENTARY test passes RED because the handler tolerantly swallows the dropped fields via `.and_then(|p| from_value(p).ok())` and `if let Some(...)` — that is exactly the silent-loss bug in action: `process_request` returns a successful initialize response while internally never seeing the announced protocol version or client info. Post-fix, the supplementary test continues to pass while the PRIMARY test transitions from FAIL → PASS.

## Fix

Single-attribute change at `crates/tldr-mcp/src/protocol.rs`:

```diff
- /// MCP initialize request parameters
+ /// MCP initialize request parameters.
+ ///
+ /// Per MCP 2024-11-05 wire format, the `initialize` request params object
+ /// uses camelCase field names (`protocolVersion`, `clientInfo`). The
+ /// struct-level `#[serde(rename_all = "camelCase")]` is load-bearing —
+ /// without it, serde silently fails to bind `protocolVersion`/`clientInfo`
+ /// from a spec-compliant client (e.g. Claude Code) and every field
+ /// defaults to `None` (because each field carries `#[serde(default)]`),
+ /// so the server processes a degraded request without realizing — the
+ /// diagnostic `eprintln!` paths in `server::handle_initialize` become
+ /// dead code when the live client is spec-compliant. See parcadei/tldr-code#19
+ /// (M7 VAL-007 — request-side audit, symmetric to M4's response-side
+ /// fix on `InitializeResult`).
  #[derive(Debug, Clone, Deserialize)]
+ #[serde(rename_all = "camelCase")]
  pub struct InitializeParams {
      #[serde(default)]
      pub protocol_version: Option<String>,
      #[serde(default)]
      pub capabilities: Option<ClientCapabilities>,
      #[serde(default)]
      pub client_info: Option<ClientInfo>,
  }
```

**Why `rename_all = "camelCase"` over per-field `rename`:**
- Both affected fields (`protocol_version`, `client_info`) need camelCase. The struct-level form covers them in one attribute.
- Future-proof: any field added to `InitializeParams` will pick up the rename automatically — no risk of a future contributor forgetting an individual `#[serde(rename = ...)]` and reintroducing the same silent-drop bug.
- Symmetric with the M4 fix: M4 used per-field `rename` on `InitializeResult` because the struct only had two affected fields and they wanted explicit visibility on each rename. That is also defensible. M7 chose `rename_all` because the audit pattern itself is the lesson — "never let snake_case fields face the wire" — and a struct-level guard makes that explicit at the type definition. (No conflict with M4: different struct, different role; the mix is intentional.)

## GREEN — post-fix

```
running 3 tests
test initialize_params_snake_case_keys_no_longer_bind_post_fix ... ok
test initialize_params_accepts_camelcase_keys ... ok
test initialize_request_via_process_request_accepts_camelcase_params ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

## Acceptance results

| Criterion | Result |
|-----------|--------|
| (1) Audit complete: every params-side struct documented | YES — see field-level table above |
| (2) Reproduction test passes after fix | YES — 3/3 GREEN |
| (3) M3's `mcp_lifecycle_test` still passes | YES — 4/4 |
| (4) M4's `mcp_camelcase_test` still passes | YES — 5/5 |
| (5) `cargo test -p tldr-mcp` passes | YES — 45 lib + 5 mcp_camelcase + 4 mcp_lifecycle + 3 mcp_request_params = **57/57** |
| (6) Full matrix unchanged | YES — 730 exhaustive + 234 language = **964/964** at baseline |
| (7) Clippy `--workspace --all-features --tests -- -D warnings` clean | YES |

## Files touched

| Path | Type | Change |
|------|------|--------|
| `crates/tldr-mcp/src/protocol.rs` | source | added `#[serde(rename_all = "camelCase")]` + expanded docstring on `InitializeParams` |
| `crates/tldr-mcp/tests/mcp_request_params_test.rs` | new test | 3 tests covering primary deserialize, end-to-end dispatch, negative-control |
| `continuum/autonomous/v0.2.1-hotfix/reports/m7-mcp-initialize-params-audit.md` | report | this file |
| `continuum/autonomous/v0.2.1-hotfix/validation/m7-mcp-initialize-params-audit.json` | validation | machine-readable evidence |
| `continuum/autonomous/v0.2.1-hotfix/contract.json` | contract | VAL-007 status → `passed`, status_resolution populated |

**Source files: 1** (well within the implicit cap suggested by similar milestones; only `protocol.rs` is touched).
**STOP conditions checked:**
- Structs needing fixing: 1 (≤ 4 — within limit)
- M3 tests pass: yes
- M4 tests pass: yes
- Matrix unchanged: 964/964
- Clippy clean: yes
- Source files touched: 1 (≤ 3 — within limit)

No STOP condition triggered.

## Coordination notes

- Disjoint from M6: M6 touches `crates/tldr-daemon/src/handlers/*` only. M7 touches `crates/tldr-mcp/src/protocol.rs` only. Zero file overlap.
- Did not touch `JsonRpcRequest` (M3 territory), `InitializeResult` or any other Serialize-only response struct (M4 territory), or any other request-side struct (none other needed fixing).
- Local v0.2.1 tag deletion not in this milestone's scope (handled by orchestrator before M7; recreation deferred to M8).
- No changes to `Cargo.toml`, `Cargo.lock`, or `CHANGELOG.md` — version + changelog updates are M8's job.
