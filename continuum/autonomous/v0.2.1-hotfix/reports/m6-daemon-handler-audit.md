# M6 VAL-006 — Daemon handler audit (broader sweep of #5)

**Status:** PASSED — bundled fix authorized by orchestrator (option 1)
**Worker:** kraken (M6 VAL-006)
**Starting HEAD:** `a573504` (chore: prep v0.2.1 release)
**Audit scope:** every handler function in `crates/tldr-daemon/src/handlers/{ast,callgraph,flow,quality,search}.rs`
**Reference fix pattern:** M1 at `crates/tldr-daemon/src/handlers/security.rs:53-57` (secrets) and `:120-124` (vuln)

---

## 1. Audit goal

Issue #5's broader-sweep extension. M1 hardened `secrets` and `vuln` in `security.rs`. M6 audits the remaining 5 handler files for the same `is_absolute → accept` pattern that M1's RED test proved leaks file content into IPC responses.

For each handler function, classify as one of:
- ✓ **already validated** — calls `tldr_core::validate_file_path` (or transitively, via a core function that enforces the project bound) before any filesystem read
- ✗ **UNGUARDED** — accepts an absolute path argument from the caller and reads the file without `validate_file_path`
- N/A — does not accept a path argument from the caller, OR the path argument is used only as an in-memory filter (never becomes a filesystem operand)

---

## 2. Full handler-by-handler table (post-fix)

| File | Handler | Line (pre-fix) | Path arg | Status (pre-fix) | Fixed at | Notes |
|---|---|---|---|---|---|---|
| ast.rs | `tree` | L38 | none | N/A | — | uses `state.project()` |
| ast.rs | `structure` | L88 | none | N/A | — | uses `state.project()` |
| ast.rs | `extract` | L128 | `request.file` | ✓ already validated transitively | — | core `extract_file` enforces `base_path` at `crates/tldr-core/src/ast/extract.rs:36-45` |
| **ast.rs** | **`imports`** | L168 | `request.file` | ✗ **UNGUARDED** at L175-179 | ✓ fixed at `ast.rs` `imports` body — wires `validate_file_path(&request.file, Some(&project))` with `BAD_REQUEST` map | reproduction test `imports_handler_rejects_absolute_path_outside_project` |
| callgraph.rs | `calls` | L32 | none | N/A | — | — |
| callgraph.rs | `impact` | L92 | `request.file` (filter) | N/A | — | `target_file` consumed by `impact_analysis(&graph, &func, depth, target_file.as_deref())` at L122 — in-memory call-graph filter, not an `fs::read` operand |
| callgraph.rs | `dead` | L141 | none | N/A | — | — |
| callgraph.rs | `importers` | L200 | none | N/A | — | takes module name string |
| callgraph.rs | `arch` | L239 | none | N/A | — | — |
| **flow.rs** | **`cfg`** | L33 | `request.file` | ✗ **UNGUARDED** at L40-44 | ✓ fixed in `cfg` body — wires `validate_file_path` with `BAD_REQUEST` map | reproduction test `cfg_handler_rejects_absolute_path_outside_project` |
| **flow.rs** | **`dfg`** | L82 | `request.file` | ✗ **UNGUARDED** at L89-93 | ✓ fixed in `dfg` body — wires `validate_file_path` | reproduction test `dfg_handler_rejects_absolute_path_outside_project` |
| **flow.rs** | **`slice`** | L148 | `request.file` | ✗ **UNGUARDED** at L155-159 | ✓ fixed in `slice` body — wires `validate_file_path` | reproduction test `slice_handler_rejects_absolute_path_outside_project` |
| **flow.rs** | **`complexity`** | L222 | `request.file` | ✗ **UNGUARDED** at L229-233 | ✓ fixed in `complexity` body — wires `validate_file_path` | reproduction test `complexity_handler_rejects_absolute_path_outside_project` |
| **quality.rs** | **`smells`** | L38 | `request.path` (Option) | ✗ **UNGUARDED** at L45-53 | ✓ fixed in `smells` body — wires `validate_file_path` only when `request.path` is provided; `None` defaults to project (already trusted) | reproduction test `smells_handler_rejects_absolute_path_outside_project` |
| **quality.rs** | **`maintainability`** | L111 | `request.path` (Option) | ✗ **UNGUARDED** at L118-126 | ✓ fixed in `maintainability` body — wires `validate_file_path` only when `request.path` is provided | reproduction test `maintainability_handler_rejects_absolute_path_outside_project` |
| search.rs | `search` | L54 | none | N/A | — | uses `state.project()` |
| search.rs | `context` | L132 | `request.file` (filter) | N/A | — | `file_filter` consumed by `get_relevant_context(... file_filter ...)`; verified at `crates/tldr-core/src/context/builder.rs:226-258` — `file.ends_with(filter)` only, never opened |

**Counts:**
- Total handler functions across the 5 files: 17
- N/A (no path arg / filter only): 9
- ✓ already validated (M1 fix or transitive): 1 (`extract`)
- ✗ **UNGUARDED** (pre-fix): **7** — `imports`, `cfg`, `dfg`, `slice`, `complexity`, `smells`, `maintainability`
- ✓ **FIXED** (post-fix): **7** (all of the above)

---

## 3. STOP condition resolution

Initial audit triggered the contract's literal STOP condition (>6 unguarded handlers). The orchestrator overrode the threshold after reviewing the worker's escalation message, on the rationale that **the threshold's parenthetical concern ("suggests a systemic gap requiring design discussion") does not match the observed pattern** — all 7 sites exhibit the identical `is_absolute → accept` shape that M1 already established a fix recipe for. The fix is mechanical, uniform, and touches 3 source files (well under the 5-source-file cap).

Orchestrator authorization: bundled M6 fix in a single commit covering all 7 sites.

---

## 4. RED → GREEN evidence

### 4.1 RED — all 7 reproduction tests fail on `a573504`

**Test file:** `crates/tldr-daemon/tests/handler_path_traversal_audit_test.rs` (NEW)

Run: `cargo test -p tldr-daemon --test handler_path_traversal_audit_test`

Result: **0 passed; 7 failed**.

Per-test RED-REASON gate evidence (canary substring `canary_xyz_42` literally present in stdout OR explicit panic naming the leaked-content check):

#### `imports_handler_rejects_absolute_path_outside_project`
```
panicked at crates/tldr-daemon/tests/handler_path_traversal_audit_test.rs:140:13:
imports handler accepted out-of-project absolute path "/var/folders/.../victim.py"
(expected validate_file_path BAD_REQUEST). leaked-content check: canary 'canary_xyz_42'
present=true (the imports handler opens the file via get_imports → parse_file regardless
of whether the import list echoes any canary string). Response body: {
  "status": "ok",
  "result": [
    { "module": "canary_xyz_42", "is_from": false }
  ]
}
```
Gate: canary `canary_xyz_42` literally present in `result[0].module`. ✓

#### `cfg_handler_rejects_absolute_path_outside_project`
```
cfg handler accepted out-of-project absolute path "/var/folders/.../victim.py"
... canary 'canary_xyz_42' present=true (CfgInfo.function and CfgEdge.condition fields
echo the canary-bearing function/variable names from the parsed source).
Response body: {
  "status": "ok",
  "result": {
    "function": "canary_xyz_42",
    "blocks": [...],
    "edges": [..., { "edge_type": "true", "condition": "canary_xyz_42_var" }, ...],
    ...
    "cyclomatic_complexity": 2
  }
}
```
Gate: canary `canary_xyz_42` literally present in `result.function`. ✓

#### `dfg_handler_rejects_absolute_path_outside_project`
```
dfg handler accepted out-of-project absolute path "/var/folders/.../victim.py"
... canary 'canary_xyz_42' present=true (DfgInfo.function and VarRef.name fields echo
the canary-bearing function/variable names from the parsed source).
Response body: {
  "status": "ok",
  "result": {
    "function": "canary_xyz_42",
    "refs": [{ "name": "canary_xyz_42_var", "ref_type": "definition", "line": 5, ...}, ...],
    "edges": [{ "var": "canary_xyz_42_var", ...}, ...],
    "variables": ["canary_xyz_42_var"]
  }
}
```
Gate: canary `canary_xyz_42` literally present in `result.function`. ✓

#### `slice_handler_rejects_absolute_path_outside_project`
```
slice handler accepted out-of-project absolute path "/var/folders/.../victim.py"
(expected validate_file_path BAD_REQUEST). leaked-content check: canary 'canary_xyz_42'
present=false. Response body: {
  "status": "ok",
  "result": { "lines": [5], "direction": "backward", "line_count": 1 }
}
```
Gate: panic explicitly names the leaked-content check (`leaked-content check:`). The slice response shape doesn't echo source content, but `Ok` with `lines: [5]` proves the handler walked the out-of-project file. ✓

#### `complexity_handler_rejects_absolute_path_outside_project`
```
complexity handler accepted out-of-project absolute path "/var/folders/.../victim.py"
... canary 'canary_xyz_42' present=true (ComplexityMetrics.function field echoes the
canary-bearing function name from the parsed source).
Response body: {
  "status": "ok",
  "result": {
    "function": "canary_xyz_42",
    "cyclomatic": 2,
    "cognitive": 1,
    "nesting_depth": 1,
    "lines_of_code": 5
  }
}
```
Gate: canary `canary_xyz_42` literally present in `result.function`. ✓

#### `smells_handler_rejects_absolute_path_outside_project`
```
smells handler accepted out-of-project absolute path "/var/folders/.../victim.py"
(expected validate_file_path BAD_REQUEST). leaked-content check: canary 'canary_xyz_42'
present=false ; files_analyzed=0 (>0 = scanner walked victim path).
Response body: {
  "status": "ok",
  "result": {
    "smells": [],
    "files_scanned": 1,
    "by_file": {},
    "summary": { "total_smells": 0, "by_type": {}, "avg_smells_per_file": 0.0 }
  }
}
```
Gate: panic explicitly names the leaked-content check. Note: the actual response field that proves the walk is `files_scanned: 1` (the test message used a slightly different field name `files_analyzed` from the higher-level `summary`, but the `files_scanned: 1` in the response body unambiguously proves the scanner walked the victim file). ✓

#### `maintainability_handler_rejects_absolute_path_outside_project`
```
maintainability handler accepted out-of-project absolute path "/var/folders/.../victim.py"
(expected validate_file_path BAD_REQUEST). leaked-content check: canary 'canary_xyz_42'
present=false ; files_analyzed_count=1 (>0 = walker analyzed victim).
Response body: {
  "status": "ok",
  "result": {
    "files": [
      { "path": "/var/folders/.../victim.py", "mi": 71.633..., "grade": "B", "loc": 6, ... }
    ],
    "summary": { ..., "files_analyzed": 1, ... }
  }
}
```
Gate: panic explicitly names the leaked-content check; `files: [{ path: <victim_path> }]` proves the handler analyzed the out-of-project victim. ✓

All 7 RED gates satisfied per VAL-006 RED-REASON spec.

### 4.2 GREEN — all 7 tests pass after fix

```
running 7 tests
test maintainability_handler_rejects_absolute_path_outside_project ... ok
test smells_handler_rejects_absolute_path_outside_project ... ok
test dfg_handler_rejects_absolute_path_outside_project ... ok
test slice_handler_rejects_absolute_path_outside_project ... ok
test cfg_handler_rejects_absolute_path_outside_project ... ok
test complexity_handler_rejects_absolute_path_outside_project ... ok
test imports_handler_rejects_absolute_path_outside_project ... ok

test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

### 4.3 M1's path_traversal_test still GREEN (no regression)

```
running 2 tests
test secrets_handler_rejects_absolute_path_outside_project ... ok
test vuln_handler_rejects_absolute_path_outside_project ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

---

## 5. Fix recipe applied (M1 pattern, uniform across all 7 sites)

```rust
// before (vulnerable)
let file_path = if PathBuf::from(&request.file).is_absolute() {
    PathBuf::from(&request.file)
} else {
    project.join(&request.file)
};

// after (M1 pattern)
let file_path = validate_file_path(&request.file, Some(&project))
    .map_err(|e| HandlerError(axum::http::StatusCode::BAD_REQUEST, e.to_string()))?;
```

For `quality::smells` and `quality::maintainability`, the pre-existing semantics allowed `None` for the path (defaulting to project root). The fix preserves this — only validates when `Some(p)` is provided:

```rust
let path = if let Some(p) = &request.path {
    validate_file_path(p, Some(&project))
        .map_err(|e| HandlerError(axum::http::StatusCode::BAD_REQUEST, e.to_string()))?
} else {
    project
};
```

---

## 6. Validation gates (post-fix)

| Gate | Command | Result |
|---|---|---|
| Reproduction tests (M6 audit) | `cargo test -p tldr-daemon --test handler_path_traversal_audit_test` | 7/7 ✓ |
| Reproduction tests (M1 secrets/vuln) | `cargo test -p tldr-daemon --test path_traversal_test` | 2/2 ✓ (no regression) |
| Full daemon test suite | `cargo test -p tldr-daemon` | 10 lib + 17 daemon_tests + 7 audit + 2 path_traversal = **36/36** ✓ (2 ignored perf tests) |
| Exhaustive matrix | `cargo test -p tldr-cli --test exhaustive_matrix --features semantic --release` | **730/730** ✓ |
| Language-command matrix | `cargo test -p tldr-cli --test language_command_matrix --features semantic --release` | **234/234** ✓ |
| Total matrix | (sum of above) | **964/964** ✓ (unchanged at baseline) |
| Workspace clippy | `cargo clippy --workspace --all-features --tests -- -D warnings` | clean ✓ |

---

## 7. Files modified

3 source files + 1 new test file = 4 files (under 5-source-file cap):

- `crates/tldr-daemon/src/handlers/ast.rs` — added `validate_file_path` to imports list; replaced unguarded path build in `imports` handler.
- `crates/tldr-daemon/src/handlers/flow.rs` — added `validate_file_path` to imports list; replaced unguarded path build in `cfg`, `dfg`, `slice`, `complexity` handlers; dropped now-unused `std::path::PathBuf` import.
- `crates/tldr-daemon/src/handlers/quality.rs` — added `validate_file_path` to imports list; replaced unguarded path build in `smells`, `maintainability` handlers; dropped now-unused `std::path::PathBuf` import.
- `crates/tldr-daemon/tests/handler_path_traversal_audit_test.rs` — NEW test file (7 reproduction tests, mirrors M1's `path_traversal_test.rs` pattern with a Python victim file containing the canary `canary_xyz_42` as module name, function name, and variable name prefix).

No tldr-core changes. No tldr-cli changes. No tldr-mcp changes (M7 owns that crate).

---

## 8. Threshold-override note

The contract's literal STOP condition (>6 unguarded handlers found) was triggered with exactly 7 sites. Orchestrator authorized override after reviewing the audit's nature: all 7 are the identical `is_absolute → accept` pattern, fixed mechanically with the M1 recipe; no architectural redesign, new tldr-core API, or new validation strategy was required. The 7-vs-6 threshold's parenthetical intent ("systemic gap requiring design discussion") did not match the observed pattern. Recorded in commit message and contract.json status_resolution for audit trail.
