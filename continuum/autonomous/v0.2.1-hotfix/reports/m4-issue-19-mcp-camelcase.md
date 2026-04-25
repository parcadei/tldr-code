# M4 VAL-004 — MCP InitializeResult camelCase wire format

**Issue:** parcadei/tldr-code#19 (the only human-filed bug in this run — etal37, filed against v0.1.6, still present in v0.2.0)

**User-facing failure:** "Claude Code cannot connect to tldr-mcp" — the `initialize` response carries snake_case top-level fields (`protocol_version`, `server_info`) instead of MCP-2024-11-05-required camelCase (`protocolVersion`, `serverInfo`); spec-compliant clients reject the response and the lifecycle handshake never completes.

**Starting HEAD:** `1620b6d` (M3 commit — MCP lifecycle handshake fix).

**Status:** PASSED.

---

## Triage verification (Step 0)

1. `git log -1 --oneline` → `1620b6d fix(M3 VAL-003): close #12 — MCP lifecycle handshake`. Confirmed.
2. Read `crates/tldr-mcp/src/protocol.rs:158-162` — confirmed `InitializeResult` struct's `protocol_version` and `server_info` fields lack `#[serde(rename = ...)]` attributes:
   ```rust
   pub struct InitializeResult {
       pub protocol_version: String,
       pub capabilities: ServerCapabilities,
       pub server_info: ServerInfo,
   }
   ```
3. Audited every `Serialize`-derived struct in `protocol.rs` for snake_case fields lacking renames:
   - `InitializeResult.protocol_version` — **MISSING rename** (the bug)
   - `InitializeResult.server_info` — **MISSING rename** (the bug)
   - `ToolsCapability.list_changed` — already renamed to `listChanged` (L173)
   - `ToolDefinition.input_schema` — already renamed to `inputSchema` (L212)
   - `ToolsCallResult.is_error` — already renamed to `isError` (L229)
   - `ContentItem.content_type` — already renamed to `type` (L254)
   - All other fields are already camelCase or single-word
4. M3 lifecycle test panic-output dump (logged in `continuum/autonomous/v0.2.1-hotfix/reports/m3-issue-12-mcp-lifecycle.md`) confirmed the initialize response on HEAD `1620b6d` still emits `"protocol_version":"2024-11-05"` and `"server_info":{"name":"tldr-mcp",...}` — pre-condition intact.

---

## Reproduction test (Phase 1: write FAILING test FIRST)

**Path:** `crates/tldr-mcp/tests/mcp_camelcase_test.rs` (NEW; placed alongside M3's `mcp_lifecycle_test.rs` to keep test-harness style consistent).

**Test design:**
- Drives `tldr_mcp::server::process_request` in-process (same harness pattern as M3's lifecycle test — no shell, no subprocess, no binary spawn → portable across platforms).
- Sends two synthetic frames:
  - **Frame A:** `initialize` request (id=1) with valid camelCase `protocolVersion`/`clientInfo` params.
  - **Frame B:** `tools/list` request (id=2) — the second handshake frame Claude Code sends immediately after `initialize`.
- Captures both responses, parses each as JSON, walks the entire `.result` object recursively, and collects every object key at every depth into a sorted `BTreeSet<String>`.
- Asserts the set contains zero snake_case keys, where snake_case is defined as: contains at least one underscore, every byte is ASCII lowercase letter / digit / underscore, does NOT start with an underscore (matches contract regex `^[a-z]+_[a-z]+` while permitting digits within names; excludes leading-underscore identifiers like `_private` and SCREAMING_CASE like `ENV_VAR`).
- For the `initialize` response specifically: positively asserts `.result.protocolVersion` and `.result.serverInfo` exist (catches the case where someone deletes the fields rather than renames them).

**Recursive walk scope decision:**
The walk skips object keys appearing directly inside any `properties` object that itself sits inside an `inputSchema` value. Rationale: those keys are USER-DEFINED parameter names that handlers extract via `get_optional_string(&args, "exclude_hidden")` etc. (verified at `crates/tldr-mcp/src/tools/ast.rs:21,62,90`). Renaming them would silently break every `tools/call` invocation. The MCP 2024-11-05 wire-format requirement applies to MCP-defined message field names, not to JSON Schema property declarations contained within an `inputSchema` value. **Two helper unit tests in the same file (`snake_case_detector_recognizes_target_pattern` and `collect_keys_walks_recursively_and_skips_schema_properties`) lock down both the detector and the walker so a failure in `initialize_response_uses_camel_case` is unambiguously a real server bug, not a helper bug.**

**5 tests total:**
1. `snake_case_detector_recognizes_target_pattern` — sanity unit test for the snake_case detector.
2. `collect_keys_walks_recursively_and_skips_schema_properties` — sanity unit test for the recursive walker.
3. `initialize_response_uses_camel_case` — Issue #19 primary assertion.
4. `tools_list_response_uses_camel_case` — broader-audit assertion (confirms day-2 handshake step is also clean).
5. `day_one_handshake_responses_have_zero_snake_case_keys` — combined shipping criterion.

---

## RED on `1620b6d` (Phase 2: confirm test fails before fix)

```
$ cargo test -p tldr-mcp --test mcp_camelcase_test --release

running 5 tests
test snake_case_detector_recognizes_target_pattern ... ok
test collect_keys_walks_recursively_and_skips_schema_properties ... ok
test initialize_response_uses_camel_case ... FAILED
test tools_list_response_uses_camel_case ... ok
test day_one_handshake_responses_have_zero_snake_case_keys ... FAILED

failures:

---- initialize_response_uses_camel_case stdout ----

thread 'initialize_response_uses_camel_case' (196851604) panicked at crates/tldr-mcp/tests/mcp_camelcase_test.rs:230:5:
initialize response contains snake_case keys (MCP 2024-11-05 requires camelCase): ["protocol_version", "server_info"]
full collected keys: {"capabilities", "listChanged", "name", "protocol_version", "server_info", "tools", "version"}
full response: {"jsonrpc":"2.0","result":{"protocol_version":"2024-11-05","capabilities":{"tools":{"listChanged":false}},"server_info":{"name":"tldr-mcp","version":"0.2.0"}},"id":1}

---- day_one_handshake_responses_have_zero_snake_case_keys stdout ----

thread 'day_one_handshake_responses_have_zero_snake_case_keys' (196851603) panicked at crates/tldr-mcp/tests/mcp_camelcase_test.rs:311:5:
MCP 2024-11-05 wire-format violation: snake_case keys found in day-1 handshake responses: [("initialize", ["protocol_version", "server_info"])]


failures:
    day_one_handshake_responses_have_zero_snake_case_keys
    initialize_response_uses_camel_case

test result: FAILED. 3 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out
```

**RED-REASON GATE satisfied.** The literal RED stdout contains BOTH `protocol_version` AND `server_info` — proof the recursive walk found the specific snake_case keys named in the contract. The full response body is also dumped (`"protocol_version":"2024-11-05"`, `"server_info":{...}`) confirming the wire output matches the bug description.

**BROADER-AUDIT findings (key observation):** `tools_list_response_uses_camel_case` PASSED on the unfixed HEAD — confirming the recursive scan found ZERO snake_case violations in the `tools/list` response (after the JSON-Schema-properties exclusion). The previously-renamed fields (`listChanged`, `isError`, `inputSchema`, `type`) are already wire-correct. **The literal triage line ("rename protocol_version + server_info on InitializeResult") was sufficient — no broader source-file fixes needed.**

---

## Fix (Phase 3: minimum code to pass)

**File:** `crates/tldr-mcp/src/protocol.rs`

```diff
-/// MCP initialize response result
+/// MCP initialize response result.
+///
+/// Per MCP 2024-11-05 wire format, the response result object uses
+/// camelCase field names. The `#[serde(rename = ...)]` attributes on
+/// `protocol_version` and `server_info` are load-bearing — without them
+/// the server emits snake_case (`protocol_version`, `server_info`) and
+/// spec-compliant clients (e.g. Claude Code) reject the response,
+/// breaking the lifecycle handshake. See parcadei/tldr-code#19.
 #[derive(Debug, Clone, Serialize)]
 pub struct InitializeResult {
+    #[serde(rename = "protocolVersion")]
     pub protocol_version: String,
     pub capabilities: ServerCapabilities,
+    #[serde(rename = "serverInfo")]
     pub server_info: ServerInfo,
 }
```

**Complete list of `#[serde(rename = ...)]` attributes added in this milestone:**

| Struct | Field | rename_to |
|---|---|---|
| `InitializeResult` | `protocol_version` | `protocolVersion` |
| `InitializeResult` | `server_info` | `serverInfo` |

**Total snake_case keys fixed: 2.**

**No other source files modified.** Hard cap of 5 files respected (well under).

---

## GREEN after fix (Phase 4: re-run reproduction test)

```
$ cargo test -p tldr-mcp --test mcp_camelcase_test --release

running 5 tests
test collect_keys_walks_recursively_and_skips_schema_properties ... ok
test snake_case_detector_recognizes_target_pattern ... ok
test initialize_response_uses_camel_case ... ok
test tools_list_response_uses_camel_case ... ok
test day_one_handshake_responses_have_zero_snake_case_keys ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

Recursive snake_case walk on initialize + tools/list responses → **0 matches**.

---

## Full validation matrix (Phase 5)

### Full tldr-mcp suite

```
$ cargo test -p tldr-mcp --release
...
test result: ok. 45 passed; 0 failed; 0 ignored        (lib tests)
test result: ok. 5 passed; 0 failed; 0 ignored         (mcp_camelcase_test — NEW)
test result: ok. 4 passed; 0 failed; 0 ignored         (mcp_lifecycle_test — M3, NO REGRESSION)
test result: ok. 0 passed; 0 failed; 0 ignored         (main, doc-tests)
```

**M3's lifecycle test still passes — confirms the camelCase rename did not break the lifecycle handshake.**

### Exhaustive matrix (730 baseline)

```
$ cargo test -p tldr-cli --test exhaustive_matrix --features semantic --release
test result: ok. 730 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 33.19s
```

### Language-command matrix (234 baseline)

```
$ cargo test -p tldr-cli --test language_command_matrix --features semantic --release
test result: ok. 234 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.22s
```

**Total matrix: 964/964 — unchanged at baseline.**

### Clippy (Phase 6)

```
$ cargo clippy --workspace --all-features --tests -- -D warnings
    Checking tldr-mcp v0.2.0 (.../crates/tldr-mcp)
    Checking tldr-cli v0.2.0 (.../crates/tldr-cli)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 9.68s
```

**Clean. Two clippy lints fired in the new test file during initial implementation (`clippy::search_is_some`, `clippy::manual_contains`) — both refactored to use idiomatic constructs (`Iterator::any` then `slice::contains`). No `#[allow]` suppressions used. No assertions weakened. No `#[ignore]` annotations.**

---

## Scope decisions and audit notes

### What I did NOT change, and why

**`InitializeParams.protocol_version` and `InitializeParams.client_info` (and `ClientInfo.*`) — Deserialize-only structs (incoming params), left unchanged.**

These are on the deserialize side: the server READS what the client sent. The contract VAL-004 scope is response wire format (Serialize side — what the server SENDS back). The current code at `server.rs:121-124` already uses `.and_then(|p| serde_json::from_value(p.clone()).ok())` which silently ignores deser failures and falls back to `None` — so the missing renames here cause only the loss of three `eprintln!` log lines (`MCP client: <name> v<version>`, `MCP protocol version: <ver>`, `Client has experimental capabilities`). **No wire-format failure.** The handshake still completes correctly because the `handle_initialize` function does not depend on the parsed params (only logs them). Fixing this to actually parse client info correctly is a separate quality-of-life issue — flagged for v0.2.2 review.

**JSON Schema property declarations under `inputSchema.properties` — left unchanged across `crates/tldr-mcp/src/tools/{ast,callgraph,flow,quality,search,security}.rs` and `mod.rs`.**

Under the `tools/list` response, every snake_case key found by a naive recursive scan (`exclude_hidden`, `max_results`, `base_path`, `entry_points`, `context_lines`, `top_k`, `entry_point`, `include_docstrings`, `changed_files`, `smell_type`, `include_halstead`, `entropy_threshold`, `include_test`, `severity_filter`, `vuln_type`) appears inside an `inputSchema.properties` JSON Schema object. These are user-defined argument names that handlers extract verbatim via `get_optional_*(&args, "<exact_key>")` — verified at:
- `crates/tldr-mcp/src/tools/ast.rs:21` → `get_optional_bool(&args, "exclude_hidden")`
- `crates/tldr-mcp/src/tools/ast.rs:62` → `get_optional_int(&args, "max_results")`
- `crates/tldr-mcp/src/tools/ast.rs:90` → `get_optional_string(&args, "base_path")`
- (and so on across all tool handler files)

Renaming the schema property keys without simultaneously renaming every handler's argument-extraction call would silently break every `tools/call` invocation — for both Claude Code and any other MCP client. Renaming both sides would touch 7+ source files (mod.rs + 6 handler files + every existing handler test) and constitute a breaking API change to all `tools/call` callers — far beyond the 5-file cap and well beyond Issue #19's scope.

The MCP 2024-11-05 wire-format requirement applies to **MCP-defined message field names** (e.g., `JsonRpcResponse.result`, `InitializeResult.protocolVersion`, `Tool.inputSchema`), not to JSON Schema property declarations contained within an `inputSchema` value (which is itself a value, not a struct field). JSON Schema (a separate spec) does not mandate camelCase for property names, and Claude Code uses these declarations to construct argument objects for `tools/call` — the handshake completes because the schema property names match the keys Claude Code passes back.

The reproduction test explicitly encodes this exclusion in `collect_keys` (skips object keys directly inside `inputSchema.properties`), and a sanity unit test (`collect_keys_walks_recursively_and_skips_schema_properties`) locks the behavior down. **If a future audit decides to also rename these, that is a separate API-design milestone, not a bug-fix.**

### What I would have changed if needed

If the `tools/list` recursive scan had found snake_case keys OUTSIDE the `inputSchema.properties` exclusion zone (i.e., on the `ToolsListResult`, `ToolDefinition`, `ToolsCallResult`, `ContentItem`, `ServerCapabilities`, or `ServerInfo` structs themselves — wire-spec-defined response struct fields), I would have added `#[serde(rename = ...)]` to each in this same commit. The audit confirmed all such struct fields had their renames added in v0.1.6 work and are already wire-correct. Only `InitializeResult` was missed.

---

## M3 coordination

M3 (commit `1620b6d`) deliberately left `InitializeResult` at original snake_case shape — that was M4 VAL-004's territory. M3's lifecycle test panic-output dump confirmed the initialize response on HEAD `1620b6d` still contained `"protocol_version"` and `"server_info"` snake_case keys (M4 pre-condition intact). After M4's fix:
- M3's `mcp_lifecycle_test::lifecycle_handshake_three_frames` still passes — the lifecycle handshake structure (3 frames sent → 2 responses, no notification reply) is unchanged.
- M3's `mcp_lifecycle_test::notification_frame_emits_no_response`, `unknown_notification_method_emits_no_response`, `unknown_request_method_emits_method_not_found` all still pass.
- The only behavioral change is the wire-format key names of `InitializeResult` (snake_case → camelCase), exactly as required.

---

## Files modified

1. `crates/tldr-mcp/src/protocol.rs` — added two `#[serde(rename = ...)]` attributes + module-level docstring expansion explaining why they are load-bearing.
2. `crates/tldr-mcp/tests/mcp_camelcase_test.rs` — NEW test file (5 tests).

**Two files. Hard cap (5) respected.**

---

## Acceptance summary

| Gate | Result |
|---|---|
| (1) Reproduce test passes after fix | YES (5/5 GREEN) |
| (2) M3 VAL-003 lifecycle test still passes (no regression) | YES (4/4) |
| (3) `cargo test -p tldr-mcp` passes | YES (45 lib + 5 + 4 = 54 total) |
| (4) Full matrix unchanged (964/964) | YES (730 + 234) |
| (5) Clippy clean | YES (no `#[allow]`, no suppressions) |
| (6) Recursive snake_case walk on initialize + tools/list returns zero matches | YES (excluding JSON Schema property names; rationale documented above) |
