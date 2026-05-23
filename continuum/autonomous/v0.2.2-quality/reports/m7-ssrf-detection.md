# M7 — SSRF Detection Rule (VAL-007)

**Milestone:** ssrf-detection-rule
**Assertion:** VAL-007
**Issue:** follow-up from v0.2.1 M2 — saved in memory `tldr_code_v022_ssrf_rule.md`
**Worker:** kraken (M7 VAL-007, v0.2.2-quality wave 3)
**Starting HEAD:** c82e004 (M5 head; M6 lives on a sibling branch and was
absorbed cleanly via the contract — does not affect vuln.rs)

## Background

`VulnType::Ssrf` was added to the enum at
`crates/tldr-core/src/security/vuln.rs:48` and was correctly mapped at the CLI
boundary as of v0.2.1 hotfix M2 (commit at
`crates/tldr-cli/src/commands/remaining/vuln.rs:712`,
`CoreVulnType::Ssrf => VulnType::Ssrf`). However:

1. The detection rule itself at `vuln.rs:609-628`
   (`VulnType::Ssrf => match language`) returned `vec![]` for every
   supported language — the sink-pattern list was empty.
2. `VulnType::Ssrf` was NOT included in the default `vuln_types` list at
   `vuln.rs:838-845` that `scan_file_vulns` runs when the caller passes
   `vuln_filter = None` (the default CLI invocation path at
   `crates/tldr-cli/src/commands/remaining/vuln.rs:641`).

So `tldr vuln` never emitted an SSRF finding — even when scanning code
with obvious SSRF sinks AND even when the user explicitly passed
`--type ssrf` (which would have hit the empty sink list).

This was deferred from v0.2.1 to v0.2.2 — recorded in memory at
`~/.claude/projects/-Users-cosimo-Desktop-PatchWork/memory/tldr_code_v022_ssrf_rule.md`.

## Fix

Single-file change to `crates/tldr-core/src/security/vuln.rs`:

1. **Replaced the empty `VulnType::Ssrf => match language` block** (lines
   609-628) with per-language sink-pattern lists. The shape mirrors the
   existing `VulnType::Deserialization` arm at `vuln.rs:629-712` —
   `(pattern, description)` tuples matched as substrings against the
   source line. The taint engine's second pass at `scan_file_vulns:887-933`
   fires a finding when (a) `line.contains(sink_pattern)` AND (b)
   `line.contains(tainted_var)` on the same line. No taint-engine changes
   required; new sinks plug into the existing flow exactly like
   Deserialization sinks did.

2. **Added `VulnType::Ssrf` to the default `vuln_types` list** at
   `vuln.rs:838-845`, so the rule actually fires in the default
   `tldr vuln` call path.

## Languages shipped

The M7 minimum acceptance is 3 languages (Python + TS/JS + Go). The fix
ships all 7 languages enumerated in the contract — none deferred to v0.2.3:

| Language | Sink patterns |
|---|---|
| **Python** | `requests.{get,post,put,delete,head,patch,request}(`, `urllib.request.urlopen(`, `urlopen(`, `httpx.{get,post,request}(`, `aiohttp.ClientSession` |
| **TypeScript / JavaScript** | `fetch(`, `axios.{get,post,put,delete,request}(`, `axios(`, `http.{get,request}(`, `https.{get,request}(`, `got(`, `superagent.get(`, `node-fetch(` |
| **Go** | `http.{Get,Post,PostForm,Head,NewRequest,NewRequestWithContext}(` |
| **Java** | `URL(`, `.openConnection(`, `.openStream(`, `HttpClient.newHttpClient`, `.send(`, `URI.create(`, `HttpRequest.newBuilder(`, `RestTemplate`, `.{get,post}ForObject(` |
| **Rust** | `reqwest::get(`, `reqwest::Client`, `.get(`, `.post(`, `ureq::{get,post}(`, `hyper::Client`, `Url::parse(` |
| **Ruby** | `Net::HTTP.{get,post,start}(`, `URI.{open,parse}(`, `RestClient.{get,post}(`, `HTTParty.get(`, `open(` |
| **PHP** | `file_get_contents(`, `fopen(`, `curl_exec(`, `curl_setopt(`, `get_headers(`, `readfile(`, `Guzzle\\Client`, `->request(` |

## Languages deferred to v0.2.3

The following languages return `vec![]` from `get_sinks(VulnType::Ssrf, lang)`
— matching pre-M7 behavior. They are explicitly enumerated as the empty
arm so that adding sink patterns later is a localized one-arm change:

- **C** (no widely-standardized HTTP-client sink across libcurl /
  libfetch / mbedtls — needs library-specific patterns)
- **C++** (same)
- **Kotlin** (Ktor/OkHttp client patterns — straightforward extension)
- **Swift** (URLSession / Vapor `client.get(uri)` patterns)
- **C#** (HttpClient, WebClient, RestSharp patterns)
- **Scala** (sttp, Akka HTTP, Play WS patterns)
- **Lua / Luau** (`socket.http.request`, `lua-resty-http`)
- **Elixir** (HTTPoison, Tesla, Finch)
- **OCaml** (Cohttp_lwt_unix, Ezcurl)

These are pure extensions — adding any of them is a localized one-arm
change to the `VulnType::Ssrf => match language` block plus a
corresponding test. None require taint-engine changes.

## Tests added

All in one commit:

### Core unit tests (`crates/tldr-core/src/security/vuln.rs::tests`)

- `test_e2e_python_ssrf_requests_get`
- `test_e2e_python_ssrf_urllib_urlopen`
- `test_e2e_python_ssrf_httpx_get`
- `test_e2e_typescript_ssrf_fetch`
- `test_e2e_typescript_ssrf_axios_get`
- `test_e2e_javascript_ssrf_fetch`
- `test_e2e_go_ssrf_http_get`
- `test_e2e_go_ssrf_http_post`
- `test_e2e_go_ssrf_http_newrequest`
- `test_e2e_java_ssrf_url_openconnection`
- `test_e2e_rust_ssrf_reqwest_get`
- `test_e2e_ruby_ssrf_net_http_get`
- `test_e2e_php_ssrf_file_get_contents`
- `test_e2e_ssrf_in_default_vuln_types` (regression guard for #2 above)
- `test_get_sinks_ssrf_has_per_language_coverage` (sink-list inventory check)

15 tests total. Each uses the existing `assert_detects_vuln` helper
(line 1581) to write a temp fixture file, call `scan_file_vulns(path,
Some(VulnType::Ssrf))`, and assert at least one finding fires.

### CLI integration tests (`crates/tldr-cli/tests/vuln_ssrf_test.rs`)

- `vuln_typescript_emits_ssrf_finding`
- `vuln_go_emits_ssrf_finding`
- `vuln_ssrf_findings_carry_cwe_918`

3 tests total. Each invokes the actual `tldr` binary via `assert_cmd`
on a fixture file with `--lang <lang> --format json`, parses the JSON,
and asserts at least one finding has `vuln_type == "ssrf"` (the
canonical wire string — `VulnType` is `#[serde(rename_all = "snake_case")]`,
so `Ssrf` serializes as `"ssrf"`).

Python is NOT exercised through the CLI integration test because the
CLI dispatches `.py` files to a separate `analyze_python_file`
tree-sitter analyzer that already had its own SSRF sinks at
`crates/tldr-cli/src/commands/remaining/vuln.rs:305-326`. Python core
coverage is asserted via the unit tests above.

### Fixtures added

- `crates/tldr-cli/tests/fixtures/ssrf_python/Vuln.py` (4 sink patterns
  exercised: requests.get, requests.post, urllib.request.urlopen, httpx.get)
- `crates/tldr-cli/tests/fixtures/ssrf_typescript/Vuln.ts` (5 sinks:
  fetch, axios.get, axios.post, http.get, http.request)
- `crates/tldr-cli/tests/fixtures/ssrf_go/Vuln.go` (3 sinks: http.Get,
  http.Post, http.NewRequest)

## Wire format

`vuln_type` JSON wire string for SSRF findings: **`"ssrf"`**

Source: `tldr_core::security::vuln::VulnType` carries
`#[serde(rename_all = "snake_case")]` at `vuln.rs:37`. The variant
`Ssrf` therefore serializes as `"ssrf"` in JSON output. The
`Display` impl at `vuln.rs:60` returns the human-readable
`"Server-Side Request Forgery"` (used for terminal/text output, not
JSON). The CLI re-maps to its own `VulnType` enum at
`crates/tldr-cli/src/commands/remaining/vuln.rs:712`, where the same
snake_case wire form applies — confirmed end-to-end by the CLI
integration test `vuln_typescript_emits_ssrf_finding`.

CWE: `"CWE-918"` (already wired at `vuln.rs:724`; carried into JSON
output via `f.cwe_id` — confirmed by `vuln_ssrf_findings_carry_cwe_918`).

## Files modified

| File | Status | Lines |
|---|---|---|
| `crates/tldr-core/src/security/vuln.rs` | modified | +446 (108 in source / 338 in tests) |
| `crates/tldr-cli/tests/vuln_ssrf_test.rs` | new | 188 |
| `crates/tldr-cli/tests/fixtures/ssrf_python/Vuln.py` | new | 44 |
| `crates/tldr-cli/tests/fixtures/ssrf_typescript/Vuln.ts` | new | 51 |
| `crates/tldr-cli/tests/fixtures/ssrf_go/Vuln.go` | new | 39 |

Source files in `crates/tldr-core/src/security/`: **1** (well within the
STOP cap of 5).

## Validation

- Core security tests: **63/63 passing** (5013 filtered + 0 failed)
  - Pre-existing 48 vuln tests: still pass — zero regression
  - New 15 SSRF tests: all green
- CLI vuln-related tests: **12/12 passing** (vuln_autodetect 6 +
  vuln_sarif_deserialization 2 + vuln_ssrf 3 + walker_consolidation 1)
- Workspace clippy: **clean** (`cargo clippy --workspace --all-features
  --tests -- -D warnings`)
- Matrix:
  - `exhaustive_matrix --release --test-threads=1`: **730/730**
  - `language_command_matrix --release`: **234/234**
  - **Sum: 964/964** (matches M5 baseline c82e004)
- `--test-threads=1` confirmed needed for exhaustive_matrix per M5's
  filesystem race on the cold fastembed model cache discovery; multi-threaded would otherwise
  show 676/730 with 54 transient failures unrelated to vuln work.

## Constraints honored

- No `#[allow(...)]` added.
- No `#[ignore]` added.
- No `_`-prefix on used variables.
- No weakened assertions.
- No `cargo fmt --all` run.
- No `git add .` used.
- No push, no destructive git, no touching prior-orchestrator artifacts
  (the unrelated `continuum/autonomous/...` files modified in the
  working tree at session start were left untouched and not staged in
  the M7 commit).
- SqlInjection / Xss / CommandInjection / PathTraversal / Deserialization
  blocks in vuln.rs untouched (confirmed via `git diff` — only the
  Ssrf block at 609-628 + the default `vuln_types` list at 838-845 +
  the tests module changed).

## Red-reason gate confirmation

Every failing test on HEAD (pre-fix) emitted a stdout panic literally
containing `got 0` (or `got []` for sink-list checks, or `Got findings:
[SqlInjection]` for the default-types check). Zero generic test-fail /
fixture-not-found failures. Verbatim evidence in
`continuum/autonomous/v0.2.2-quality/reports/m7-red-capture.txt`.
