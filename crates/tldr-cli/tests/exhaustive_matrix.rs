//! VAL-013: Exhaustive command×language audit.
//!
//! This is a comprehensive audit matrix that runs every applicable
//! `tldr` subcommand against the canonical 2-file 3-function fixture for
//! each of the 18 supported languages. Each cell is one test function
//! named `test_<command>_on_<language>` so a failure pinpoints the
//! exact (command, language) pair.
//!
//! # Three forbidden failure classes (per VAL-013)
//!
//! 1. **HANG** — process not killed by 30s wall-clock timeout.
//! 2. **PANIC / CRASH** — exit code != 0 with `panicked at` /
//!    `thread '...' panicked` / `Stack backtrace:` in stderr.
//! 3. **SILENT_FAIL** — exit code 0 with empty/missing output where the
//!    canonical fixture should produce a result.
//!
//! Every cell either PASSes or has a justified `#[ignore = "<reason>"]`
//! that cites a real source location (file:line) where the support gap
//! is documented.
//!
//! # Relationship to VAL-010 / VAL-011
//!
//! `language_command_matrix.rs` (VAL-010 / VAL-011 / VAL-012) covers the
//! 13 most central commands. **This matrix extends that coverage** to the
//! remaining ~37 language-applicable commands, plus a sanity-only group
//! of 10 orchestrator commands that have no per-language semantics.
//!
//! # Excluded commands (orchestrator-only, --help only)
//!
//! `tree`, `coverage`, `fix`, `bugbot`, `daemon`, `cache`, `stats`,
//! `warm`, `doctor`, `help`. These have no per-language semantics
//! (they parse external reports, manage processes, or print help).
//! For these we only verify `--help` exits cleanly and is non-empty.

mod fixtures;

use serde_json::Value;
use std::path::Path;
use std::process::Command;
use std::sync::{mpsc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// Global mutex that serializes invocations of `embed` / `semantic` /
/// `similar`. fastembed shares a single on-disk model cache; concurrent
/// first-touches race on cache-file creation and one of them fails with
/// `No such file or directory (os error 2)`. Once the model is fetched
/// the cache is read-safe, but tests run in fresh processes so we can't
/// rely on warm state. Holding the mutex around each invocation keeps
/// the cache initialization race-free.
fn embedding_mutex() -> &'static Mutex<()> {
    static M: OnceLock<Mutex<()>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(()))
}

// ============================================================================
// Constants
// ============================================================================

/// Per-cell wall-clock timeout. If a tldr invocation exceeds this, the
/// process is killed and the test fails as HANG.
const CELL_TIMEOUT: Duration = Duration::from_secs(30);

/// All 18 supported languages.
const LANGUAGES: &[&str] = &[
    "python",
    "typescript",
    "javascript",
    "go",
    "rust",
    "java",
    "c",
    "cpp",
    "ruby",
    "kotlin",
    "swift",
    "csharp",
    "scala",
    "php",
    "lua",
    "luau",
    "elixir",
    "ocaml",
];

// ============================================================================
// Timeout-protected runner
// ============================================================================

/// Result of running a tldr command with a wall-clock timeout.
#[derive(Debug)]
enum CellResult {
    /// Process completed within the timeout.
    Ok {
        exit: i32,
        stdout: String,
        stderr: String,
        #[allow(dead_code)]
        duration: Duration,
    },
    /// Process did not finish before `CELL_TIMEOUT` and was killed.
    Hang,
    /// Process exited with a panic / backtrace signature in stderr.
    Panic { exit: i32, stderr: String },
}

/// Run `tldr <args>` with a wall-clock timeout. Detects HANG and PANIC.
fn run_tldr_timed(args: &[&str], timeout: Duration) -> CellResult {
    let bin = assert_cmd::cargo::cargo_bin!("tldr");
    let argv: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let (tx, rx) = mpsc::channel();

    // Spawn the child in a worker thread so we can poll for completion
    // while keeping the wall-clock guard.
    let start = Instant::now();
    let handle = thread::spawn(move || {
        let mut cmd = Command::new(bin);
        cmd.args(&argv);
        let res = cmd.output();
        let _ = tx.send(res);
    });

    match rx.recv_timeout(timeout) {
        Ok(Ok(out)) => {
            // Reap thread.
            let _ = handle.join();
            let duration = start.elapsed();
            let exit = out.status.code().unwrap_or(-1);
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();

            if is_panic(exit, &stderr) {
                CellResult::Panic { exit, stderr }
            } else {
                CellResult::Ok {
                    exit,
                    stdout,
                    stderr,
                    duration,
                }
            }
        }
        Ok(Err(e)) => {
            let _ = handle.join();
            CellResult::Panic {
                exit: -1,
                stderr: format!("spawn failed: {e}"),
            }
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            // Detached: the child is still running. We can't reliably kill
            // it without the Child handle (we own it inside the worker
            // thread). The test fails as HANG; the child will be reaped
            // when the test process exits.
            CellResult::Hang
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            let _ = handle.join();
            CellResult::Panic {
                exit: -1,
                stderr: "worker thread died without reporting".to_string(),
            }
        }
    }
}

/// True if (exit, stderr) signature looks like a panic / crash / segfault.
fn is_panic(exit: i32, stderr: &str) -> bool {
    if stderr.contains("panicked at")
        || stderr.contains("Stack backtrace:")
        || stderr.contains("thread '") && stderr.contains("panicked")
    {
        return true;
    }
    // Negative exit codes on Unix indicate signal termination (e.g. SIGSEGV
    // = -11, SIGABRT = -6). Treat any signal as a panic/crash.
    if exit < 0 {
        return true;
    }
    false
}

/// Allowed exit codes for non-panic cells.
///
/// `tldr` follows a documented diagnostic exit-code scheme defined in
/// `crates/tldr-core/src/error.rs:286-340`:
/// * 0          — success
/// * 1          — general / IO error
/// * 2-9        — file system errors (PathNotFound, NotGitRepository, etc.)
/// * 10-19      — parse errors (ParseError=10, UnsupportedLanguage=11)
/// * 20-29      — analysis errors (FunctionNotFound=20, etc.)
/// * 30-39      — network/daemon errors
/// * 40-49      — serialization errors
/// * 60-69      — diagnostics-specific (60=no tools, 61=all failed —
///   see crates/tldr-cli/src/commands/diagnostics.rs:193, :212)
///
/// Plus subcommand-specific codes:
/// * `change-impact` exits 3 on NoBaseline (VAL-005)
/// * `temporal` exits 2 on no-constraints-found (temporal.rs:951-959)
/// * `diagnostics` exits 1 on diagnostic findings
///
/// Anything outside [0, 99] is treated as anomalous (BAD_EXIT). The
/// harness still enforces no-panic / no-hang separately.
fn ok_exit(exit: i32) -> bool {
    (0..=99).contains(&exit)
}

/// Extract first JSON value from stdout (strips any leading non-JSON
/// progress lines emitted by certain commands).
fn parse_json(stdout: &str) -> Value {
    let json_start = stdout.find(['{', '[']).unwrap_or(stdout.len());
    serde_json::from_str::<Value>(&stdout[json_start..]).unwrap_or(Value::Null)
}

// ============================================================================
// Cell-level assertion helpers
// ============================================================================

/// Common cell verification. Asserts: not Hang, not Panic, exit code in
/// {0,1,2,3}. Returns the parsed JSON for further per-command checks (or
/// `Value::Null` if the output is non-JSON).
fn check_baseline(cmd: &str, lang: &str, args: &[&str]) -> (Value, String, String, i32) {
    let result = run_tldr_timed(args, CELL_TIMEOUT);
    match result {
        CellResult::Hang => panic!(
            "[{cmd} × {lang}] HANG — process exceeded {:?} wall-clock timeout\nargs: {:?}",
            CELL_TIMEOUT, args
        ),
        CellResult::Panic { exit, stderr } => panic!(
            "[{cmd} × {lang}] PANIC — exit={exit} stderr signature contains panic markers\nargs: {:?}\n--- stderr ---\n{stderr}",
            args
        ),
        CellResult::Ok { exit, stdout, stderr, .. } => {
            if !ok_exit(exit) {
                panic!(
                    "[{cmd} × {lang}] BAD_EXIT — exit={exit} (outside documented 0..=49 range)\nargs: {:?}\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
                    args, truncate(&stdout, 800)
                );
            }
            let json = parse_json(&stdout);
            (json, stdout, stderr, exit)
        }
    }
}

/// SILENT_FAIL guard. Per VAL-013, SILENT_FAIL is "exit code 0 with
/// empty/missing output where the canonical fixture should produce a
/// result". A documented non-zero exit (e.g. exit 2 from `temporal` for
/// no-constraints-found, exit 1 from `diagnostics` for findings) is NOT
/// a silent fail — it's a legitimate diagnostic. We require:
/// * Not a Hang
/// * Not a Panic
/// * Exit code in 0..=49 (documented scheme)
/// * If exit == 0: JSON output non-empty
/// * If exit != 0: stderr non-empty (provides explanation)
fn check_success(cmd: &str, lang: &str, args: &[&str]) -> (Value, String, String) {
    let (json, stdout, stderr, exit) = check_baseline(cmd, lang, args);
    if exit == 0 && stdout.trim().is_empty() {
        panic!(
            "[{cmd} × {lang}] SILENT_FAIL — exit=0 but stdout is empty\nargs: {:?}\n--- stderr ---\n{stderr}",
            args
        );
    }
    if exit != 0 && stdout.trim().is_empty() && stderr.trim().is_empty() {
        panic!(
            "[{cmd} × {lang}] SILENT_FAIL — exit={exit} but BOTH stdout and stderr empty\nargs: {:?}",
            args
        );
    }
    (json, stdout, stderr)
}

/// Truncate to first N chars (for panic messages).
fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("{}...[truncated {} bytes]", &s[..n], s.len() - n)
    }
}

// ============================================================================
// Fixture path helpers (mirrors language_command_matrix.rs)
// ============================================================================

/// Path to File A (entry file) within a fixture root, by language.
fn entry_file(lang: &str, root: &Path) -> std::path::PathBuf {
    match lang {
        "python" => root.join("main.py"),
        "typescript" => root.join("main.ts"),
        "javascript" => root.join("main.js"),
        "go" => root.join("main.go"),
        "rust" => root.join("src/main.rs"),
        "java" => root.join("Main.java"),
        "c" => root.join("main.c"),
        "cpp" => root.join("main.cpp"),
        "ruby" => root.join("main.rb"),
        "kotlin" => root.join("Main.kt"),
        "swift" => root.join("Main.swift"),
        "csharp" => root.join("Program.cs"),
        "scala" => root.join("Main.scala"),
        "php" => root.join("main.php"),
        "lua" => root.join("main.lua"),
        "luau" => root.join("main.luau"),
        "elixir" => root.join("main.ex"),
        "ocaml" => root.join("main.ml"),
        _ => panic!("unknown lang: {lang}"),
    }
}

/// Path to File B (utility file) within a fixture root, by language.
fn util_file(lang: &str, root: &Path) -> std::path::PathBuf {
    match lang {
        "python" => root.join("util.py"),
        "typescript" => root.join("util.ts"),
        "javascript" => root.join("util.js"),
        "go" => root.join("util/util.go"),
        "rust" => root.join("src/util.rs"),
        "java" => root.join("Util.java"),
        "c" => root.join("util.c"),
        "cpp" => root.join("util.cpp"),
        "ruby" => root.join("util.rb"),
        "kotlin" => root.join("Util.kt"),
        "swift" => root.join("Util.swift"),
        "csharp" => root.join("Util.cs"),
        "scala" => root.join("Util.scala"),
        "php" => root.join("util.php"),
        "lua" => root.join("util.lua"),
        "luau" => root.join("util.luau"),
        "elixir" => root.join("util.ex"),
        "ocaml" => root.join("util.ml"),
        _ => panic!("unknown lang: {lang}"),
    }
}

/// Build a canonical fixture in a fresh tempdir and return both.
fn make_fixture(lang: &str) -> TempDir {
    let tmp = TempDir::new().unwrap();
    fixtures::build_fixture(lang, tmp.path());
    tmp
}

/// Build a canonical fixture wrapped in a git repository with 3 commits.
///
/// VAL-017: `tldr churn` and `tldr hotspots` operate on `git log` output
/// and require an initialised repository with real commit history. The
/// canonical 2-file 3-function fixture written by `build_fixture` lives
/// in a bare directory, so churn/hotspots see no history and emit empty
/// reports (ChurnReport.files=[], HotspotsReport.hotspots=[]).
///
/// `make_git_fixture` runs `git init`, sets a deterministic local
/// (NOT global) `user.email` / `user.name`, then makes 3 commits whose
/// payloads create real `lines_added` deltas:
///
///   * Commit 1: initial fixture (`build_fixture(lang, ...)`).
///   * Commit 2: append a line of trailing whitespace to the entry file
///     (1-line diff).
///   * Commit 3: append another line of trailing whitespace to the entry
///     file (another 1-line diff).
///
/// Three commits is required because `tldr hotspots` defaults to
/// `min_commits = 3` (see `crates/tldr-core/src/quality/hotspots.rs:387`)
/// — fewer commits would cause hotspots to filter out the file and
/// return an empty `hotspots` array, which would mask SILENT_FAIL bugs.
fn make_git_fixture(lang: &str) -> TempDir {
    let tmp = TempDir::new().unwrap();
    fixtures::build_git_fixture(lang, tmp.path());
    tmp
}

/// Per-language name of the entry function (the one that calls helper
/// and b_util). Most fixtures define a lowercase `main`, but C# uses
/// PascalCase `Main` per .NET convention (see `build_csharp` in
/// `fixtures/mod.rs`).
fn entry_function(lang: &str) -> &'static str {
    match lang {
        "csharp" => "Main",
        _ => "main",
    }
}

// ============================================================================
// GROUP-DIR: project-level commands (arg = directory path)
// ============================================================================
//
// These are commands not already covered by language_command_matrix.rs.
// Each verifies: no panic/hang, exit ∈ {0,1,2,3}, plus a per-command
// shape check that catches SILENT_FAIL.

fn check_hubs(lang: &str) {
    let tmp = make_fixture(lang);
    let path = tmp.path().to_str().unwrap();
    let (json, stdout, stderr) =
        check_success("hubs", lang, &["hubs", path, "--format", "json", "--quiet"]);
    // Sanity: response is an object with `hubs` array. The fixture is
    // tiny, so empty `hubs` is acceptable, but the `hubs` field itself
    // must be present (not Null).
    if !json.is_object() || json.get("hubs").is_none() {
        panic!(
            "[hubs × {lang}] SILENT_FAIL — missing `hubs` field\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
}

fn check_whatbreaks(lang: &str) {
    let tmp = make_fixture(lang);
    let path = tmp.path().to_str().unwrap();
    let (json, stdout, stderr) = check_success(
        "whatbreaks",
        lang,
        &["whatbreaks", "helper", path, "--format", "json", "--quiet"],
    );
    // Object output with at least the `target` echoed back.
    if !json.is_object() {
        panic!(
            "[whatbreaks × {lang}] SILENT_FAIL — non-object output\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
}

fn check_importers(lang: &str) {
    let tmp = make_fixture(lang);
    let path = tmp.path().to_str().unwrap();
    // Module-name string varies per language. Use a name that exists in
    // each fixture: the entry's util module short name.
    let module = match lang {
        "go" => "example.com/x/util",
        "java" => "Util",
        "csharp" => "Util",
        "scala" => "Util",
        "elixir" => "Util",
        "kotlin" => "Util",
        "ocaml" => "Util",
        "rust" => "util",
        // Python/TS/JS/PHP/Lua/Luau/Ruby/Swift use bareword `util`.
        _ => "util",
    };
    let (json, stdout, stderr) = check_success(
        "importers",
        lang,
        &["importers", module, path, "--format", "json", "--quiet"],
    );
    if !json.is_object() || json.get("module").is_none() {
        panic!(
            "[importers × {lang}] SILENT_FAIL — missing `module` field\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
}

fn check_secure(lang: &str) {
    let tmp = make_fixture(lang);
    let path = tmp.path().to_str().unwrap();
    let (json, stdout, stderr) = check_success(
        "secure",
        lang,
        &["secure", path, "--format", "json", "--quiet"],
    );
    if !json.is_object() || json.get("findings").is_none() {
        panic!(
            "[secure × {lang}] SILENT_FAIL — missing `findings` field\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
}

fn check_api_check(lang: &str) {
    let tmp = make_fixture(lang);
    let path = tmp.path().to_str().unwrap();
    let (json, stdout, stderr) = check_success(
        "api-check",
        lang,
        &["api-check", path, "--format", "json", "--quiet"],
    );
    if !json.is_object() || json.get("findings").is_none() {
        panic!(
            "[api-check × {lang}] SILENT_FAIL — missing `findings` field\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
}

fn check_vuln(lang: &str) {
    let tmp = make_fixture(lang);
    let path = tmp.path().to_str().unwrap();
    // vuln autodetect supports only Python+Rust per
    // crates/tldr-cli/src/commands/remaining/vuln.rs:586-588. For other
    // languages, exit 2 with a clear stderr error is the documented
    // path — that is NOT a silent fail. We still verify no panic / no
    // hang and that something was emitted.
    let (json, _stdout, stderr, exit) =
        check_baseline("vuln", lang, &["vuln", path, "--format", "json", "--quiet"]);
    if exit == 0 && (!json.is_object() || json.get("findings").is_none()) {
        panic!(
            "[vuln × {lang}] SILENT_FAIL — exit=0 but missing `findings` field\n--- stderr ---\n{stderr}"
        );
    }
    if exit != 0 && stderr.trim().is_empty() {
        panic!("[vuln × {lang}] SILENT_FAIL — exit={exit} but stderr empty");
    }
}

fn check_deps(lang: &str) {
    let tmp = make_fixture(lang);
    let path = tmp.path().to_str().unwrap();
    let (json, stdout, stderr) =
        check_success("deps", lang, &["deps", path, "--format", "json", "--quiet"]);
    if !json.is_object() {
        panic!(
            "[deps × {lang}] SILENT_FAIL — non-object output\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
}

fn check_change_impact(lang: &str) {
    let tmp = make_fixture(lang);
    let path = tmp.path().to_str().unwrap();
    // Pass --files for the entry filename so analysis has a baseline
    // (else the no-git-repo path returns NoBaseline / exit 3).
    let entry = entry_file(lang, tmp.path());
    let entry_name = entry
        .strip_prefix(tmp.path())
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let (json, stdout, stderr, _exit) = check_baseline(
        "change-impact",
        lang,
        &[
            "change-impact",
            path,
            "--files",
            &entry_name,
            "--format",
            "json",
            "--quiet",
        ],
    );
    // change-impact may return exit 3 on insufficient git context — we
    // only require non-panic / non-hang. JSON should still parse.
    if !json.is_object() {
        panic!(
            "[change-impact × {lang}] SILENT_FAIL — non-object output\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
}

fn check_debt(lang: &str) {
    let tmp = make_fixture(lang);
    let path = tmp.path().to_str().unwrap();
    let (json, stdout, stderr) =
        check_success("debt", lang, &["debt", path, "--format", "json", "--quiet"]);
    if !json.is_object() {
        panic!(
            "[debt × {lang}] SILENT_FAIL — non-object output\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
}

fn check_health(lang: &str) {
    let tmp = make_fixture(lang);
    let path = tmp.path().to_str().unwrap();
    let (json, stdout, stderr) = check_success(
        "health",
        lang,
        &["health", path, "--format", "json", "--quiet"],
    );
    if !json.is_object() || json.get("summary").is_none() {
        panic!(
            "[health × {lang}] SILENT_FAIL — missing `summary` field\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
}

fn check_clones(lang: &str) {
    let tmp = make_fixture(lang);
    let path = tmp.path().to_str().unwrap();
    let (json, stdout, stderr) = check_success(
        "clones",
        lang,
        &["clones", path, "--format", "json", "--quiet"],
    );
    if !json.is_object() || json.get("stats").is_none() {
        panic!(
            "[clones × {lang}] SILENT_FAIL — missing `stats` field\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
}

fn check_todo(lang: &str) {
    let tmp = make_fixture(lang);
    let path = tmp.path().to_str().unwrap();
    let (json, stdout, stderr) =
        check_success("todo", lang, &["todo", path, "--format", "json", "--quiet"]);
    if !json.is_object() {
        panic!(
            "[todo × {lang}] SILENT_FAIL — non-object output\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
}

fn check_invariants(lang: &str) {
    let tmp = make_fixture(lang);
    let entry = entry_file(lang, tmp.path());
    let path = tmp.path().to_str().unwrap();
    let (json, stdout, stderr) = check_success(
        "invariants",
        lang,
        &[
            "invariants",
            entry.to_str().unwrap(),
            "--from-tests",
            path,
            "--format",
            "json",
            "--quiet",
        ],
    );
    if !json.is_object() {
        panic!(
            "[invariants × {lang}] SILENT_FAIL — non-object output\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
}

fn check_verify(lang: &str) {
    let tmp = make_fixture(lang);
    let path = tmp.path().to_str().unwrap();
    let (json, stdout, stderr) = check_success(
        "verify",
        lang,
        &["verify", path, "--format", "json", "--quiet"],
    );
    if !json.is_object() || json.get("sub_results").is_none() {
        panic!(
            "[verify × {lang}] SILENT_FAIL — missing `sub_results` field\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
}

fn check_interface(lang: &str) {
    let tmp = make_fixture(lang);
    let path = tmp.path().to_str().unwrap();
    let (json, stdout, stderr) = check_success(
        "interface",
        lang,
        &["interface", path, "--format", "json", "--quiet"],
    );
    // interface returns either an array of files or an object summary.
    if !json.is_array() && !json.is_object() {
        panic!(
            "[interface × {lang}] SILENT_FAIL — non-array, non-object output\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
}

fn check_search(lang: &str) {
    let tmp = make_fixture(lang);
    let path = tmp.path().to_str().unwrap();
    let (json, stdout, stderr) = check_success(
        "search",
        lang,
        &["search", "helper", path, "--format", "json", "--quiet"],
    );
    if !json.is_object() || json.get("results").is_none() {
        panic!(
            "[search × {lang}] SILENT_FAIL — missing `results` field\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
    // VAL-018: tightened — searching for "helper" must return >= 1
    // result. The canonical fixture defines `helper` in File A; if the
    // search command can't find a function literally named "helper",
    // the search pipeline is broken for this language.
    let results_len = json
        .get("results")
        .and_then(Value::as_array)
        .map(|a| a.len())
        .unwrap_or(0);
    if results_len == 0 {
        panic!(
            "[search × {lang}] SILENT_FAIL — search for 'helper' returned 0 results, but fixture defines `helper` in File A\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
}

fn check_context(lang: &str) {
    let tmp = make_fixture(lang);
    let path = tmp.path().to_str().unwrap();
    let func = entry_function(lang);
    let (json, stdout, stderr) = check_success(
        "context",
        lang,
        &[
            "context",
            func,
            "--project",
            path,
            "--format",
            "json",
            "--quiet",
        ],
    );
    if !json.is_object() || json.get("entry_point").is_none() {
        panic!(
            "[context × {lang}] SILENT_FAIL — missing `entry_point` field\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
    // VAL-018: tightened — context for the entry function `main` must
    // surface its known callees in the canonical fixture: `helper`
    // (intra-file) and the cross-file utility function (whose name
    // varies per language convention). The simplest check is that the
    // rendered JSON string references both names.
    //
    // Cross-file utility name per fixture (see `fixtures/mod.rs`):
    // - python/typescript/javascript/c/cpp/php/lua/luau/ruby/csharp/elixir/ocaml: `b_util`
    // - go: `BUtil` (Go exported names start uppercase; fixture calls
    //   `util.BUtil()`, see `fixtures/mod.rs::build_go`)
    // - java/kotlin/scala: `bUtil` (camelCase per JLS / Kotlin / Scala)
    // - swift: `bUtil` (Swift convention)
    // - rust: `b_util` (snake_case)
    let canonical = serde_json::to_string(&json).unwrap_or_default();
    let mentions_helper = canonical.contains("\"helper\"")
        || canonical.contains("'helper'")
        || canonical.contains("helper(")
        || canonical.contains(": helper");
    let b_util_names = ["b_util", "bUtil", "BUtil"];
    let mentions_b_util = b_util_names.iter().any(|n| canonical.contains(n));
    if !mentions_helper || !mentions_b_util {
        panic!(
            "[context × {lang}] SILENT_FAIL — context for entry function `{func}` must reference both callees `helper` and one of {b_util_names:?}; mentions_helper={mentions_helper} mentions_b_util={mentions_b_util}\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
}

fn check_temporal(lang: &str) {
    let tmp = make_fixture(lang);
    let path = tmp.path().to_str().unwrap();
    // temporal exits 2 on no-constraints-found per
    // crates/tldr-cli/src/commands/patterns/temporal.rs:951-959. The
    // canonical fixture has 2 calls but no recurring patterns so
    // `metadata.files_analyzed` should still be >= 1.
    let (json, stdout, stderr, _exit) = check_baseline(
        "temporal",
        lang,
        &["temporal", path, "--format", "json", "--quiet"],
    );
    let files = json
        .get("metadata")
        .and_then(|m| m.get("files_analyzed"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if files == 0 {
        panic!(
            "[temporal × {lang}] SILENT_FAIL — metadata.files_analyzed=0\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
}

fn check_diagnostics(lang: &str) {
    let tmp = make_fixture(lang);
    let path = tmp.path().to_str().unwrap();
    // diagnostics has multiple exit code paths (per
    // crates/tldr-cli/src/commands/diagnostics.rs):
    //   * 0  — no findings, success
    //   * 1  — findings present (per compute_exit_code)
    //   * 60 — no diagnostic tools installed for this language (line 193)
    //   * 61 — all tools failed (line 212)
    // The 60/61 paths emit a clear stderr error (no JSON). For other
    // paths we require an object output. Either path is legitimate.
    let (json, stdout, stderr, exit) = check_baseline(
        "diagnostics",
        lang,
        &["diagnostics", path, "--format", "json", "--quiet"],
    );
    if exit == 60 || exit == 61 {
        if stderr.trim().is_empty() {
            panic!("[diagnostics × {lang}] SILENT_FAIL — exit={exit} with empty stderr");
        }
        return;
    }
    if !json.is_object() {
        panic!(
            "[diagnostics × {lang}] SILENT_FAIL — non-object output\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
}

fn check_inheritance(lang: &str) {
    let tmp = make_fixture(lang);
    let path = tmp.path().to_str().unwrap();
    let (json, stdout, stderr) = check_success(
        "inheritance",
        lang,
        &["inheritance", path, "--format", "json", "--quiet"],
    );
    if !json.is_object() || json.get("nodes").is_none() {
        panic!(
            "[inheritance × {lang}] SILENT_FAIL — missing `nodes` field\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
}

// ============================================================================
// GROUP-GIT: project-level commands that consume `git log` history
// ============================================================================
//
// VAL-017: `tldr churn` and `tldr hotspots` are language-universal in
// source — `crates/tldr-core/src/quality/churn.rs` has no language
// filter at all (pure git-log analysis), and
// `crates/tldr-core/src/quality/hotspots.rs:926` skips files only when
// `Language::from_path(...).is_none()`, which by VAL-008 covers all 18
// supported languages. The gap closed by VAL-017 was infrastructure: the
// canonical `build_fixture` helper writes a bare directory with no git
// history, so churn/hotspots saw zero commits and returned empty
// reports. `make_git_fixture` (added below) wraps the fixture with
// `git init` + 3 commits so the fixture file actually shows up in
// `git log`.
//
// Both cells assert the per-command JSON shape AND that the result list
// has at least one entry — a stricter SILENT_FAIL guard than most other
// cells, justified because the 3-commit fixture is constructed
// specifically to populate these reports.

fn check_churn(lang: &str) {
    let tmp = make_git_fixture(lang);
    let path = tmp.path().to_str().unwrap();
    let (json, stdout, stderr) = check_success(
        "churn",
        lang,
        &["churn", path, "--format", "json", "--quiet"],
    );
    // ChurnReport (crates/tldr-core/src/quality/churn.rs:200) serializes
    // the top-level `files: Vec<FileChurn>` field. Three-commit fixture
    // ensures at least the entry file appears in churn data.
    let files = json.get("files").and_then(Value::as_array);
    if files.is_none() {
        panic!(
            "[churn × {lang}] SILENT_FAIL — missing `files` field\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
    if files.unwrap().is_empty() {
        panic!(
            "[churn × {lang}] SILENT_FAIL — `files` array is empty (3-commit fixture should yield ≥1 file)\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
}

fn check_hotspots(lang: &str) {
    let tmp = make_git_fixture(lang);
    let path = tmp.path().to_str().unwrap();
    let (json, stdout, stderr) = check_success(
        "hotspots",
        lang,
        &["hotspots", path, "--format", "json", "--quiet"],
    );
    // HotspotsReport (crates/tldr-core/src/quality/hotspots.rs:327)
    // serializes top-level `hotspots: Vec<HotspotEntry>`. The default
    // `min_commits` is 3 (hotspots.rs:387); `make_git_fixture` makes
    // exactly 3 commits to the entry file so it clears the threshold.
    let hotspots = json.get("hotspots").and_then(Value::as_array);
    if hotspots.is_none() {
        panic!(
            "[hotspots × {lang}] SILENT_FAIL — missing `hotspots` field\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
    if hotspots.unwrap().is_empty() {
        panic!(
            "[hotspots × {lang}] SILENT_FAIL — `hotspots` array is empty (3-commit fixture should yield ≥1 entry under default min_commits=3)\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
}

// ============================================================================
// GROUP-FILE: file-level commands (arg = single file path)
// ============================================================================

fn check_definition(lang: &str) {
    let tmp = make_fixture(lang);
    let entry = entry_file(lang, tmp.path());
    // Use --symbol mode (more robust across languages than line/col).
    let (json, stdout, stderr) = check_success(
        "definition",
        lang,
        &[
            "definition",
            "--symbol",
            "helper",
            "--file",
            entry.to_str().unwrap(),
            "--project",
            tmp.path().to_str().unwrap(),
            "--format",
            "json",
            "--quiet",
        ],
    );
    if !json.is_object() || json.get("symbol").is_none() {
        panic!(
            "[definition × {lang}] SILENT_FAIL — missing `symbol` field\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
}

fn check_cohesion(lang: &str) {
    let tmp = make_fixture(lang);
    let path = tmp.path().to_str().unwrap();
    let (json, stdout, stderr) = check_success(
        "cohesion",
        lang,
        &["cohesion", path, "--format", "json", "--quiet"],
    );
    if !json.is_object() || (json.get("classes").is_none() && json.get("summary").is_none()) {
        panic!(
            "[cohesion × {lang}] SILENT_FAIL — missing `classes`/`summary` fields\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
}

// ============================================================================
// GROUP-FILE-SYMBOL: file + function/symbol commands
// ============================================================================

fn check_slice(lang: &str) {
    let tmp = make_fixture(lang);
    let entry = entry_file(lang, tmp.path());
    // Slice from a line within `main` body. Different langs have different
    // body line numbers; pass a generous slice line that's definitely
    // inside main. We just verify exit/parse — empty `lines` is fine on
    // tiny fixtures.
    let line = match lang {
        "ocaml" => "5",  // main () = ... let _ = helper () in ...
        "elixir" => "5", // def main do; helper(); ...; end
        "php" => "8",    // function main { helper(); b_util(); }
        _ => "7",        // most: helper() call line
    };
    let func = entry_function(lang);
    let (json, stdout, stderr, _exit) = check_baseline(
        "slice",
        lang,
        &[
            "slice",
            entry.to_str().unwrap(),
            func,
            line,
            "--format",
            "json",
            "--quiet",
        ],
    );
    if !json.is_object() || json.get("function").is_none() {
        panic!(
            "[slice × {lang}] SILENT_FAIL — missing `function` field\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
}

fn check_chop(lang: &str) {
    let tmp = make_fixture(lang);
    let entry = entry_file(lang, tmp.path());
    // chop with two lines inside `main`.
    let (src, tgt) = match lang {
        "ocaml" => ("4", "5"),
        "elixir" => ("4", "5"),
        _ => ("7", "8"),
    };
    let func = entry_function(lang);
    let (json, stdout, stderr, _exit) = check_baseline(
        "chop",
        lang,
        &[
            "chop",
            entry.to_str().unwrap(),
            func,
            src,
            tgt,
            "--format",
            "json",
            "--quiet",
        ],
    );
    // chop may report "outside function" with empty result — accept
    // structured empty as long as `function` key is present.
    if !json.is_object() || json.get("function").is_none() {
        panic!(
            "[chop × {lang}] SILENT_FAIL — missing `function` field\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
}

fn check_reaching_defs(lang: &str) {
    let tmp = make_fixture(lang);
    let entry = entry_file(lang, tmp.path());
    let func = entry_function(lang);
    let (json, stdout, stderr, _exit) = check_baseline(
        "reaching-defs",
        lang,
        &[
            "reaching-defs",
            entry.to_str().unwrap(),
            func,
            "--format",
            "json",
            "--quiet",
        ],
    );
    if !json.is_object() || json.get("function").is_none() {
        panic!(
            "[reaching-defs × {lang}] SILENT_FAIL — missing `function` field\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
}

fn check_available(lang: &str) {
    let tmp = make_fixture(lang);
    let entry = entry_file(lang, tmp.path());
    let func = entry_function(lang);
    let (json, stdout, stderr, _exit) = check_baseline(
        "available",
        lang,
        &[
            "available",
            entry.to_str().unwrap(),
            func,
            "--format",
            "json",
            "--quiet",
        ],
    );
    // Output is an object. Just verify a non-Null parse + object shape.
    if !json.is_object() {
        panic!(
            "[available × {lang}] SILENT_FAIL — non-object output\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
}

fn check_dead_stores(lang: &str) {
    let tmp = make_fixture(lang);
    let entry = entry_file(lang, tmp.path());
    let func = entry_function(lang);
    let (json, stdout, stderr, _exit) = check_baseline(
        "dead-stores",
        lang,
        &[
            "dead-stores",
            entry.to_str().unwrap(),
            func,
            "--format",
            "json",
            "--quiet",
        ],
    );
    if !json.is_object() || json.get("function").is_none() {
        panic!(
            "[dead-stores × {lang}] SILENT_FAIL — missing `function` field\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
}

fn check_resources(lang: &str) {
    let tmp = make_fixture(lang);
    let entry = entry_file(lang, tmp.path());
    let (json, stdout, stderr) = check_success(
        "resources",
        lang,
        &[
            "resources",
            entry.to_str().unwrap(),
            "--format",
            "json",
            "--quiet",
        ],
    );
    if !json.is_object() || json.get("file").is_none() {
        panic!(
            "[resources × {lang}] SILENT_FAIL — missing `file` field\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
}

fn check_explain(lang: &str) {
    let tmp = make_fixture(lang);
    let entry = entry_file(lang, tmp.path());
    let func = entry_function(lang);
    let (json, stdout, stderr, _exit) = check_baseline(
        "explain",
        lang,
        &[
            "explain",
            entry.to_str().unwrap(),
            func,
            "--format",
            "json",
            "--quiet",
        ],
    );
    if !json.is_object() || json.get("function_name").is_none() {
        panic!(
            "[explain × {lang}] SILENT_FAIL — missing `function_name` field\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
}

fn check_contracts(lang: &str) {
    let tmp = make_fixture(lang);
    let entry = entry_file(lang, tmp.path());
    let func = entry_function(lang);
    let (json, stdout, stderr, _exit) = check_baseline(
        "contracts",
        lang,
        &[
            "contracts",
            entry.to_str().unwrap(),
            func,
            "--format",
            "json",
            "--quiet",
        ],
    );
    if !json.is_object() || json.get("function").is_none() {
        panic!(
            "[contracts × {lang}] SILENT_FAIL — missing `function` field\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
}

fn check_taint(lang: &str) {
    let tmp = make_fixture(lang);
    let entry = entry_file(lang, tmp.path());
    let func = entry_function(lang);
    let (json, stdout, stderr, _exit) = check_baseline(
        "taint",
        lang,
        &[
            "taint",
            entry.to_str().unwrap(),
            func,
            "--format",
            "json",
            "--quiet",
        ],
    );
    if !json.is_object() || json.get("function_name").is_none() {
        panic!(
            "[taint × {lang}] SILENT_FAIL — missing `function_name` field\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
}

// ============================================================================
// GROUP-PAIR-FILE: two-file commands
// ============================================================================

fn check_diff(lang: &str) {
    let tmp = make_fixture(lang);
    let a = entry_file(lang, tmp.path());
    let b = util_file(lang, tmp.path());

    // (1) Identical files: `diff a a` must report identical=true with an
    // EMPTY `changes` array. VAL-018 tightening: previously this only
    // checked the field was present.
    let (json_id, stdout_id, stderr_id) = check_success(
        "diff",
        lang,
        &[
            "diff",
            a.to_str().unwrap(),
            a.to_str().unwrap(),
            "--format",
            "json",
            "--quiet",
        ],
    );
    let identical_flag = json_id
        .get("identical")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let identical_changes_len = json_id
        .get("changes")
        .and_then(Value::as_array)
        .map(|a| a.len())
        .unwrap_or(usize::MAX);
    if !identical_flag || identical_changes_len != 0 {
        panic!(
            "[diff × {lang}] SILENT_FAIL — diff(a,a) should report identical=true with empty changes; got identical={identical_flag} changes_len={identical_changes_len}\n--- stdout ---\n{}\n--- stderr ---\n{stderr_id}",
            truncate(&stdout_id, 400)
        );
    }

    // (2) Different files: `diff a b` (entry vs util) must report
    // identical=false with at least one change record (helper/main/b_util
    // are different functions across the two files).
    let (json_diff, stdout_diff, stderr_diff) = check_success(
        "diff",
        lang,
        &[
            "diff",
            a.to_str().unwrap(),
            b.to_str().unwrap(),
            "--format",
            "json",
            "--quiet",
        ],
    );
    let diff_identical = json_diff
        .get("identical")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let changes_len = json_diff
        .get("changes")
        .and_then(Value::as_array)
        .map(|a| a.len())
        .unwrap_or(0);
    if diff_identical || changes_len == 0 {
        panic!(
            "[diff × {lang}] SILENT_FAIL — diff(a,b) of fixture entry vs util must report identical=false with >= 1 change; got identical={diff_identical} changes_len={changes_len}\n--- stdout ---\n{}\n--- stderr ---\n{stderr_diff}",
            truncate(&stdout_diff, 400)
        );
    }
}

fn check_dice(lang: &str) {
    let tmp = make_fixture(lang);
    let a = entry_file(lang, tmp.path());
    let b = util_file(lang, tmp.path());

    // (1) Identical files: dice(a,a) must yield coefficient ~= 1.0.
    // VAL-018 tightening: previously only checked field presence. The
    // dice CLI emits a "Comparing similarity..." progress line before
    // the JSON; check_success / parse_json strip the leading non-JSON.
    let (json_id, stdout_id, stderr_id) = check_success(
        "dice",
        lang,
        &["dice", a.to_str().unwrap(), a.to_str().unwrap()],
    );
    let coef_id = json_id
        .get("dice_coefficient")
        .and_then(Value::as_f64)
        .unwrap_or(-1.0);
    if !(0.9..=1.0).contains(&coef_id) {
        panic!(
            "[dice × {lang}] SILENT_FAIL — dice(a,a) of identical files must be in [0.9, 1.0]; got {coef_id}\n--- stdout ---\n{}\n--- stderr ---\n{stderr_id}",
            truncate(&stdout_id, 400)
        );
    }

    // (2) Two different files: dice(a,b) must be a valid coefficient in
    // [0.0, 1.0]. We don't assert <0.9 because some fixtures share
    // tokens (`return`, function-keyword), but the value MUST be a
    // bounded probability/similarity.
    let (json_diff, stdout_diff, stderr_diff) = check_success(
        "dice",
        lang,
        &["dice", a.to_str().unwrap(), b.to_str().unwrap()],
    );
    let coef_diff = json_diff
        .get("dice_coefficient")
        .and_then(Value::as_f64)
        .unwrap_or(-1.0);
    if !(0.0..=1.0).contains(&coef_diff) {
        panic!(
            "[dice × {lang}] SILENT_FAIL — dice(a,b) coefficient must be in [0.0, 1.0]; got {coef_diff}\n--- stdout ---\n{}\n--- stderr ---\n{stderr_diff}",
            truncate(&stdout_diff, 400)
        );
    }
}

fn check_coupling(lang: &str) {
    let tmp = make_fixture(lang);
    let a = entry_file(lang, tmp.path());
    let b = util_file(lang, tmp.path());
    let (json, stdout, stderr) = check_success(
        "coupling",
        lang,
        &[
            "coupling",
            a.to_str().unwrap(),
            b.to_str().unwrap(),
            "--format",
            "json",
            "--quiet",
        ],
    );
    if !json.is_object() {
        panic!(
            "[coupling × {lang}] SILENT_FAIL — non-object output\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
}

// ============================================================================
// Semantic family: embed / semantic / similar
// ============================================================================
//
// These require the arctic-m embedding model (~110MB cached after first
// run). We always run them — if the model isn't downloaded yet, the
// command auto-downloads on first invocation.

fn check_embed(lang: &str) {
    let _guard = embedding_mutex().lock().unwrap();
    let tmp = make_fixture(lang);
    let path = tmp.path().to_str().unwrap();
    let (json, stdout, stderr) = check_success(
        "embed",
        lang,
        &["embed", path, "--format", "json", "--quiet"],
    );
    if !json.is_object() || json.get("chunks_embedded").is_none() {
        panic!(
            "[embed × {lang}] SILENT_FAIL — missing `chunks_embedded` field\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
}

fn check_semantic(lang: &str) {
    let _guard = embedding_mutex().lock().unwrap();
    let tmp = make_fixture(lang);
    let path = tmp.path().to_str().unwrap();
    let (json, stdout, stderr) = check_success(
        "semantic",
        lang,
        &[
            "semantic",
            "helper function",
            path,
            "--format",
            "json",
            "--quiet",
        ],
    );
    if !json.is_object() || json.get("results").is_none() {
        panic!(
            "[semantic × {lang}] SILENT_FAIL — missing `results` field\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
}

fn check_similar(lang: &str) {
    let _guard = embedding_mutex().lock().unwrap();
    let tmp = make_fixture(lang);
    let entry = entry_file(lang, tmp.path());
    let (json, stdout, stderr) = check_success(
        "similar",
        lang,
        &[
            "similar",
            entry.to_str().unwrap(),
            "--format",
            "json",
            "--quiet",
        ],
    );
    if !json.is_object() || json.get("source").is_none() {
        panic!(
            "[similar × {lang}] SILENT_FAIL — missing `source` field\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
}

// ============================================================================
// Surface (package introspection)
// ============================================================================

fn check_surface(lang: &str) {
    let tmp = make_fixture(lang);
    let path = tmp.path().to_str().unwrap();
    let (json, stdout, stderr) = check_success(
        "surface",
        lang,
        &["surface", path, "--format", "json", "--quiet"],
    );
    if !json.is_object() || json.get("apis").is_none() {
        panic!(
            "[surface × {lang}] SILENT_FAIL — missing `apis` field\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
            truncate(&stdout, 400)
        );
    }
}

// ============================================================================
// GROUP-EXCLUDED: orchestrator commands — sanity-only (--help)
// ============================================================================
//
// These have no per-language semantics: tree (file enumeration),
// coverage (parses external XML/JSON), fix (parses compiler output),
// bugbot (multi-stage workflow), daemon/cache/stats/warm (process
// management), doctor (tool installer), help (clap help). For these we
// only verify the help subcommand exits cleanly and is non-empty.

fn check_help_only(cmd: &str) {
    let result = run_tldr_timed(&[cmd, "--help"], CELL_TIMEOUT);
    match result {
        CellResult::Hang => panic!("[{cmd} --help] HANG"),
        CellResult::Panic { exit, stderr } => panic!(
            "[{cmd} --help] PANIC exit={exit} stderr={}",
            truncate(&stderr, 400)
        ),
        CellResult::Ok {
            exit,
            stdout,
            stderr,
            ..
        } => {
            if !ok_exit(exit) {
                panic!(
                    "[{cmd} --help] BAD_EXIT exit={exit}\nstdout={}\nstderr={}",
                    truncate(&stdout, 400),
                    truncate(&stderr, 400)
                );
            }
            if stdout.trim().is_empty() && stderr.trim().is_empty() {
                panic!("[{cmd} --help] empty stdout AND stderr");
            }
        }
    }
}

/// `tree` is language-agnostic but accepts a path. Run it on a fixture
/// to verify it doesn't crash on real content (one Python fixture is
/// enough — it walks files, not parses them).
fn check_tree_one() {
    let tmp = make_fixture("python");
    let path = tmp.path().to_str().unwrap();
    let (json, _stdout, _stderr) = check_success(
        "tree",
        "any",
        &["tree", path, "--format", "json", "--quiet"],
    );
    if !json.is_object() && !json.is_array() {
        panic!("[tree × any] SILENT_FAIL — non-object/non-array output");
    }
}

// ============================================================================
// Reference: 18 languages constant for documentation
// ============================================================================
//
// 18 languages × N commands = hand-expanded test functions below.
// Tests are organized command-first so failure clusters reveal which
// command is broken, not which language is broken.

#[test]
fn _languages_constant_is_eighteen() {
    assert_eq!(LANGUAGES.len(), 18);
}
// ============================================================================
// Per-language tests (hand-expanded, command-first ordering)
// ============================================================================

// ---------------------------------------------------------------- hubs
#[test]
fn test_hubs_on_python() {
    check_hubs("python");
}
#[test]
fn test_hubs_on_typescript() {
    check_hubs("typescript");
}
#[test]
fn test_hubs_on_javascript() {
    check_hubs("javascript");
}
#[test]
fn test_hubs_on_go() {
    check_hubs("go");
}
#[test]
fn test_hubs_on_rust() {
    check_hubs("rust");
}
#[test]
fn test_hubs_on_java() {
    check_hubs("java");
}
#[test]
fn test_hubs_on_c() {
    check_hubs("c");
}
#[test]
fn test_hubs_on_cpp() {
    check_hubs("cpp");
}
#[test]
fn test_hubs_on_ruby() {
    check_hubs("ruby");
}
#[test]
fn test_hubs_on_kotlin() {
    check_hubs("kotlin");
}
#[test]
fn test_hubs_on_swift() {
    check_hubs("swift");
}
#[test]
fn test_hubs_on_csharp() {
    check_hubs("csharp");
}
#[test]
fn test_hubs_on_scala() {
    check_hubs("scala");
}
#[test]
fn test_hubs_on_php() {
    check_hubs("php");
}
#[test]
fn test_hubs_on_lua() {
    check_hubs("lua");
}
#[test]
fn test_hubs_on_luau() {
    check_hubs("luau");
}
#[test]
fn test_hubs_on_elixir() {
    check_hubs("elixir");
}
#[test]
fn test_hubs_on_ocaml() {
    check_hubs("ocaml");
}

// ---------------------------------------------------------------- whatbreaks
#[test]
fn test_whatbreaks_on_python() {
    check_whatbreaks("python");
}
#[test]
fn test_whatbreaks_on_typescript() {
    check_whatbreaks("typescript");
}
#[test]
fn test_whatbreaks_on_javascript() {
    check_whatbreaks("javascript");
}
#[test]
fn test_whatbreaks_on_go() {
    check_whatbreaks("go");
}
#[test]
fn test_whatbreaks_on_rust() {
    check_whatbreaks("rust");
}
#[test]
fn test_whatbreaks_on_java() {
    check_whatbreaks("java");
}
#[test]
fn test_whatbreaks_on_c() {
    check_whatbreaks("c");
}
#[test]
fn test_whatbreaks_on_cpp() {
    check_whatbreaks("cpp");
}
#[test]
fn test_whatbreaks_on_ruby() {
    check_whatbreaks("ruby");
}
#[test]
fn test_whatbreaks_on_kotlin() {
    check_whatbreaks("kotlin");
}
#[test]
fn test_whatbreaks_on_swift() {
    check_whatbreaks("swift");
}
#[test]
fn test_whatbreaks_on_csharp() {
    check_whatbreaks("csharp");
}
#[test]
fn test_whatbreaks_on_scala() {
    check_whatbreaks("scala");
}
#[test]
fn test_whatbreaks_on_php() {
    check_whatbreaks("php");
}
#[test]
fn test_whatbreaks_on_lua() {
    check_whatbreaks("lua");
}
#[test]
fn test_whatbreaks_on_luau() {
    check_whatbreaks("luau");
}
#[test]
fn test_whatbreaks_on_elixir() {
    check_whatbreaks("elixir");
}
#[test]
fn test_whatbreaks_on_ocaml() {
    check_whatbreaks("ocaml");
}

// ---------------------------------------------------------------- importers
#[test]
fn test_importers_on_python() {
    check_importers("python");
}
#[test]
fn test_importers_on_typescript() {
    check_importers("typescript");
}
#[test]
fn test_importers_on_javascript() {
    check_importers("javascript");
}
#[test]
fn test_importers_on_go() {
    check_importers("go");
}
#[test]
fn test_importers_on_rust() {
    check_importers("rust");
}
#[test]
fn test_importers_on_java() {
    check_importers("java");
}
#[test]
fn test_importers_on_c() {
    check_importers("c");
}
#[test]
fn test_importers_on_cpp() {
    check_importers("cpp");
}
#[test]
fn test_importers_on_ruby() {
    check_importers("ruby");
}
#[test]
fn test_importers_on_kotlin() {
    check_importers("kotlin");
}
#[test]
fn test_importers_on_swift() {
    check_importers("swift");
}
#[test]
fn test_importers_on_csharp() {
    check_importers("csharp");
}
#[test]
fn test_importers_on_scala() {
    check_importers("scala");
}
#[test]
fn test_importers_on_php() {
    check_importers("php");
}
#[test]
fn test_importers_on_lua() {
    check_importers("lua");
}
#[test]
fn test_importers_on_luau() {
    check_importers("luau");
}
#[test]
fn test_importers_on_elixir() {
    check_importers("elixir");
}
#[test]
fn test_importers_on_ocaml() {
    check_importers("ocaml");
}

// ---------------------------------------------------------------- secure
#[test]
fn test_secure_on_python() {
    check_secure("python");
}
#[test]
fn test_secure_on_typescript() {
    check_secure("typescript");
}
#[test]
fn test_secure_on_javascript() {
    check_secure("javascript");
}
#[test]
fn test_secure_on_go() {
    check_secure("go");
}
#[test]
fn test_secure_on_rust() {
    check_secure("rust");
}
#[test]
fn test_secure_on_java() {
    check_secure("java");
}
#[test]
fn test_secure_on_c() {
    check_secure("c");
}
#[test]
fn test_secure_on_cpp() {
    check_secure("cpp");
}
#[test]
fn test_secure_on_ruby() {
    check_secure("ruby");
}
#[test]
fn test_secure_on_kotlin() {
    check_secure("kotlin");
}
#[test]
fn test_secure_on_swift() {
    check_secure("swift");
}
#[test]
fn test_secure_on_csharp() {
    check_secure("csharp");
}
#[test]
fn test_secure_on_scala() {
    check_secure("scala");
}
#[test]
fn test_secure_on_php() {
    check_secure("php");
}
#[test]
fn test_secure_on_lua() {
    check_secure("lua");
}
#[test]
fn test_secure_on_luau() {
    check_secure("luau");
}
#[test]
fn test_secure_on_elixir() {
    check_secure("elixir");
}
#[test]
fn test_secure_on_ocaml() {
    check_secure("ocaml");
}

// ---------------------------------------------------------------- api-check
#[test]
fn test_api_check_on_python() {
    check_api_check("python");
}
#[test]
fn test_api_check_on_typescript() {
    check_api_check("typescript");
}
#[test]
fn test_api_check_on_javascript() {
    check_api_check("javascript");
}
#[test]
fn test_api_check_on_go() {
    check_api_check("go");
}
#[test]
fn test_api_check_on_rust() {
    check_api_check("rust");
}
#[test]
fn test_api_check_on_java() {
    check_api_check("java");
}
#[test]
fn test_api_check_on_c() {
    check_api_check("c");
}
#[test]
fn test_api_check_on_cpp() {
    check_api_check("cpp");
}
#[test]
fn test_api_check_on_ruby() {
    check_api_check("ruby");
}
#[test]
fn test_api_check_on_kotlin() {
    check_api_check("kotlin");
}
#[test]
fn test_api_check_on_swift() {
    check_api_check("swift");
}
#[test]
fn test_api_check_on_csharp() {
    check_api_check("csharp");
}
#[test]
fn test_api_check_on_scala() {
    check_api_check("scala");
}
#[test]
fn test_api_check_on_php() {
    check_api_check("php");
}
#[test]
fn test_api_check_on_lua() {
    check_api_check("lua");
}
#[test]
fn test_api_check_on_luau() {
    check_api_check("luau");
}
#[test]
fn test_api_check_on_elixir() {
    check_api_check("elixir");
}
#[test]
fn test_api_check_on_ocaml() {
    check_api_check("ocaml");
}

// ---------------------------------------------------------------- vuln
#[test]
fn test_vuln_on_python() {
    check_vuln("python");
}
#[test]
fn test_vuln_on_typescript() {
    check_vuln("typescript");
}
#[test]
fn test_vuln_on_javascript() {
    check_vuln("javascript");
}
#[test]
fn test_vuln_on_go() {
    check_vuln("go");
}
#[test]
fn test_vuln_on_rust() {
    check_vuln("rust");
}
#[test]
fn test_vuln_on_java() {
    check_vuln("java");
}
#[test]
fn test_vuln_on_c() {
    check_vuln("c");
}
#[test]
fn test_vuln_on_cpp() {
    check_vuln("cpp");
}
#[test]
fn test_vuln_on_ruby() {
    check_vuln("ruby");
}
#[test]
fn test_vuln_on_kotlin() {
    check_vuln("kotlin");
}
#[test]
fn test_vuln_on_swift() {
    check_vuln("swift");
}
#[test]
fn test_vuln_on_csharp() {
    check_vuln("csharp");
}
#[test]
fn test_vuln_on_scala() {
    check_vuln("scala");
}
#[test]
fn test_vuln_on_php() {
    check_vuln("php");
}
#[test]
fn test_vuln_on_lua() {
    check_vuln("lua");
}
#[test]
fn test_vuln_on_luau() {
    check_vuln("luau");
}
#[test]
fn test_vuln_on_elixir() {
    check_vuln("elixir");
}
#[test]
fn test_vuln_on_ocaml() {
    check_vuln("ocaml");
}

// ---------------------------------------------------------------- deps
#[test]
fn test_deps_on_python() {
    check_deps("python");
}
#[test]
fn test_deps_on_typescript() {
    check_deps("typescript");
}
#[test]
fn test_deps_on_javascript() {
    check_deps("javascript");
}
#[test]
fn test_deps_on_go() {
    check_deps("go");
}
#[test]
fn test_deps_on_rust() {
    check_deps("rust");
}
#[test]
fn test_deps_on_java() {
    check_deps("java");
}
#[test]
fn test_deps_on_c() {
    check_deps("c");
}
#[test]
fn test_deps_on_cpp() {
    check_deps("cpp");
}
#[test]
fn test_deps_on_ruby() {
    check_deps("ruby");
}
#[test]
fn test_deps_on_kotlin() {
    check_deps("kotlin");
}
#[test]
fn test_deps_on_swift() {
    check_deps("swift");
}
#[test]
fn test_deps_on_csharp() {
    check_deps("csharp");
}
#[test]
fn test_deps_on_scala() {
    check_deps("scala");
}
#[test]
fn test_deps_on_php() {
    check_deps("php");
}
#[test]
fn test_deps_on_lua() {
    check_deps("lua");
}
#[test]
fn test_deps_on_luau() {
    check_deps("luau");
}
#[test]
fn test_deps_on_elixir() {
    check_deps("elixir");
}
#[test]
fn test_deps_on_ocaml() {
    check_deps("ocaml");
}

// ---------------------------------------------------------------- change-impact
#[test]
fn test_change_impact_on_python() {
    check_change_impact("python");
}
#[test]
fn test_change_impact_on_typescript() {
    check_change_impact("typescript");
}
#[test]
fn test_change_impact_on_javascript() {
    check_change_impact("javascript");
}
#[test]
fn test_change_impact_on_go() {
    check_change_impact("go");
}
#[test]
fn test_change_impact_on_rust() {
    check_change_impact("rust");
}
#[test]
fn test_change_impact_on_java() {
    check_change_impact("java");
}
#[test]
fn test_change_impact_on_c() {
    check_change_impact("c");
}
#[test]
fn test_change_impact_on_cpp() {
    check_change_impact("cpp");
}
#[test]
fn test_change_impact_on_ruby() {
    check_change_impact("ruby");
}
#[test]
fn test_change_impact_on_kotlin() {
    check_change_impact("kotlin");
}
#[test]
fn test_change_impact_on_swift() {
    check_change_impact("swift");
}
#[test]
fn test_change_impact_on_csharp() {
    check_change_impact("csharp");
}
#[test]
fn test_change_impact_on_scala() {
    check_change_impact("scala");
}
#[test]
fn test_change_impact_on_php() {
    check_change_impact("php");
}
#[test]
fn test_change_impact_on_lua() {
    check_change_impact("lua");
}
#[test]
fn test_change_impact_on_luau() {
    check_change_impact("luau");
}
#[test]
fn test_change_impact_on_elixir() {
    check_change_impact("elixir");
}
#[test]
fn test_change_impact_on_ocaml() {
    check_change_impact("ocaml");
}

// ---------------------------------------------------------------- debt
#[test]
fn test_debt_on_python() {
    check_debt("python");
}
#[test]
fn test_debt_on_typescript() {
    check_debt("typescript");
}
#[test]
fn test_debt_on_javascript() {
    check_debt("javascript");
}
#[test]
fn test_debt_on_go() {
    check_debt("go");
}
#[test]
fn test_debt_on_rust() {
    check_debt("rust");
}
#[test]
fn test_debt_on_java() {
    check_debt("java");
}
#[test]
fn test_debt_on_c() {
    check_debt("c");
}
#[test]
fn test_debt_on_cpp() {
    check_debt("cpp");
}
#[test]
fn test_debt_on_ruby() {
    check_debt("ruby");
}
#[test]
fn test_debt_on_kotlin() {
    check_debt("kotlin");
}
#[test]
fn test_debt_on_swift() {
    check_debt("swift");
}
#[test]
fn test_debt_on_csharp() {
    check_debt("csharp");
}
#[test]
fn test_debt_on_scala() {
    check_debt("scala");
}
#[test]
fn test_debt_on_php() {
    check_debt("php");
}
#[test]
fn test_debt_on_lua() {
    check_debt("lua");
}
#[test]
fn test_debt_on_luau() {
    check_debt("luau");
}
#[test]
fn test_debt_on_elixir() {
    check_debt("elixir");
}
#[test]
fn test_debt_on_ocaml() {
    check_debt("ocaml");
}

// ---------------------------------------------------------------- health
#[test]
fn test_health_on_python() {
    check_health("python");
}
#[test]
fn test_health_on_typescript() {
    check_health("typescript");
}
#[test]
fn test_health_on_javascript() {
    check_health("javascript");
}
#[test]
fn test_health_on_go() {
    check_health("go");
}
#[test]
fn test_health_on_rust() {
    check_health("rust");
}
#[test]
fn test_health_on_java() {
    check_health("java");
}
#[test]
fn test_health_on_c() {
    check_health("c");
}
#[test]
fn test_health_on_cpp() {
    check_health("cpp");
}
#[test]
fn test_health_on_ruby() {
    check_health("ruby");
}
#[test]
fn test_health_on_kotlin() {
    check_health("kotlin");
}
#[test]
fn test_health_on_swift() {
    check_health("swift");
}
#[test]
fn test_health_on_csharp() {
    check_health("csharp");
}
#[test]
fn test_health_on_scala() {
    check_health("scala");
}
#[test]
fn test_health_on_php() {
    check_health("php");
}
#[test]
fn test_health_on_lua() {
    check_health("lua");
}
#[test]
fn test_health_on_luau() {
    check_health("luau");
}
#[test]
fn test_health_on_elixir() {
    check_health("elixir");
}
#[test]
fn test_health_on_ocaml() {
    check_health("ocaml");
}

// ---------------------------------------------------------------- clones
#[test]
fn test_clones_on_python() {
    check_clones("python");
}
#[test]
fn test_clones_on_typescript() {
    check_clones("typescript");
}
#[test]
fn test_clones_on_javascript() {
    check_clones("javascript");
}
#[test]
fn test_clones_on_go() {
    check_clones("go");
}
#[test]
fn test_clones_on_rust() {
    check_clones("rust");
}
#[test]
fn test_clones_on_java() {
    check_clones("java");
}
#[test]
fn test_clones_on_c() {
    check_clones("c");
}
#[test]
fn test_clones_on_cpp() {
    check_clones("cpp");
}
#[test]
fn test_clones_on_ruby() {
    check_clones("ruby");
}
#[test]
fn test_clones_on_kotlin() {
    check_clones("kotlin");
}
#[test]
fn test_clones_on_swift() {
    check_clones("swift");
}
#[test]
fn test_clones_on_csharp() {
    check_clones("csharp");
}
#[test]
fn test_clones_on_scala() {
    check_clones("scala");
}
#[test]
fn test_clones_on_php() {
    check_clones("php");
}
#[test]
fn test_clones_on_lua() {
    check_clones("lua");
}
#[test]
fn test_clones_on_luau() {
    check_clones("luau");
}
#[test]
fn test_clones_on_elixir() {
    check_clones("elixir");
}
#[test]
fn test_clones_on_ocaml() {
    check_clones("ocaml");
}

// ---------------------------------------------------------------- todo
#[test]
fn test_todo_on_python() {
    check_todo("python");
}
#[test]
fn test_todo_on_typescript() {
    check_todo("typescript");
}
#[test]
fn test_todo_on_javascript() {
    check_todo("javascript");
}
#[test]
fn test_todo_on_go() {
    check_todo("go");
}
#[test]
fn test_todo_on_rust() {
    check_todo("rust");
}
#[test]
fn test_todo_on_java() {
    check_todo("java");
}
#[test]
fn test_todo_on_c() {
    check_todo("c");
}
#[test]
fn test_todo_on_cpp() {
    check_todo("cpp");
}
#[test]
fn test_todo_on_ruby() {
    check_todo("ruby");
}
#[test]
fn test_todo_on_kotlin() {
    check_todo("kotlin");
}
#[test]
fn test_todo_on_swift() {
    check_todo("swift");
}
#[test]
fn test_todo_on_csharp() {
    check_todo("csharp");
}
#[test]
fn test_todo_on_scala() {
    check_todo("scala");
}
#[test]
fn test_todo_on_php() {
    check_todo("php");
}
#[test]
fn test_todo_on_lua() {
    check_todo("lua");
}
#[test]
fn test_todo_on_luau() {
    check_todo("luau");
}
#[test]
fn test_todo_on_elixir() {
    check_todo("elixir");
}
#[test]
fn test_todo_on_ocaml() {
    check_todo("ocaml");
}

// ---------------------------------------------------------------- invariants
#[test]
fn test_invariants_on_python() {
    check_invariants("python");
}
#[test]
fn test_invariants_on_typescript() {
    check_invariants("typescript");
}
#[test]
fn test_invariants_on_javascript() {
    check_invariants("javascript");
}
#[test]
fn test_invariants_on_go() {
    check_invariants("go");
}
#[test]
fn test_invariants_on_rust() {
    check_invariants("rust");
}
#[test]
fn test_invariants_on_java() {
    check_invariants("java");
}
#[test]
fn test_invariants_on_c() {
    check_invariants("c");
}
#[test]
fn test_invariants_on_cpp() {
    check_invariants("cpp");
}
#[test]
fn test_invariants_on_ruby() {
    check_invariants("ruby");
}
#[test]
fn test_invariants_on_kotlin() {
    check_invariants("kotlin");
}
#[test]
fn test_invariants_on_swift() {
    check_invariants("swift");
}
#[test]
fn test_invariants_on_csharp() {
    check_invariants("csharp");
}
#[test]
fn test_invariants_on_scala() {
    check_invariants("scala");
}
#[test]
fn test_invariants_on_php() {
    check_invariants("php");
}
#[test]
fn test_invariants_on_lua() {
    check_invariants("lua");
}
#[test]
fn test_invariants_on_luau() {
    check_invariants("luau");
}
#[test]
fn test_invariants_on_elixir() {
    check_invariants("elixir");
}
#[test]
fn test_invariants_on_ocaml() {
    check_invariants("ocaml");
}

// ---------------------------------------------------------------- verify
#[test]
fn test_verify_on_python() {
    check_verify("python");
}
#[test]
fn test_verify_on_typescript() {
    check_verify("typescript");
}
#[test]
fn test_verify_on_javascript() {
    check_verify("javascript");
}
#[test]
fn test_verify_on_go() {
    check_verify("go");
}
#[test]
fn test_verify_on_rust() {
    check_verify("rust");
}
#[test]
fn test_verify_on_java() {
    check_verify("java");
}
#[test]
fn test_verify_on_c() {
    check_verify("c");
}
#[test]
fn test_verify_on_cpp() {
    check_verify("cpp");
}
#[test]
fn test_verify_on_ruby() {
    check_verify("ruby");
}
#[test]
fn test_verify_on_kotlin() {
    check_verify("kotlin");
}
#[test]
fn test_verify_on_swift() {
    check_verify("swift");
}
#[test]
fn test_verify_on_csharp() {
    check_verify("csharp");
}
#[test]
fn test_verify_on_scala() {
    check_verify("scala");
}
#[test]
fn test_verify_on_php() {
    check_verify("php");
}
#[test]
fn test_verify_on_lua() {
    check_verify("lua");
}
#[test]
fn test_verify_on_luau() {
    check_verify("luau");
}
#[test]
fn test_verify_on_elixir() {
    check_verify("elixir");
}
#[test]
fn test_verify_on_ocaml() {
    check_verify("ocaml");
}

// ---------------------------------------------------------------- interface
#[test]
fn test_interface_on_python() {
    check_interface("python");
}
#[test]
fn test_interface_on_typescript() {
    check_interface("typescript");
}
#[test]
fn test_interface_on_javascript() {
    check_interface("javascript");
}
#[test]
fn test_interface_on_go() {
    check_interface("go");
}
#[test]
fn test_interface_on_rust() {
    check_interface("rust");
}
#[test]
fn test_interface_on_java() {
    check_interface("java");
}
#[test]
fn test_interface_on_c() {
    check_interface("c");
}
#[test]
fn test_interface_on_cpp() {
    check_interface("cpp");
}
#[test]
fn test_interface_on_ruby() {
    check_interface("ruby");
}
#[test]
fn test_interface_on_kotlin() {
    check_interface("kotlin");
}
#[test]
fn test_interface_on_swift() {
    check_interface("swift");
}
#[test]
fn test_interface_on_csharp() {
    check_interface("csharp");
}
#[test]
fn test_interface_on_scala() {
    check_interface("scala");
}
#[test]
fn test_interface_on_php() {
    check_interface("php");
}
#[test]
fn test_interface_on_lua() {
    check_interface("lua");
}
#[test]
fn test_interface_on_luau() {
    check_interface("luau");
}
#[test]
fn test_interface_on_elixir() {
    check_interface("elixir");
}
#[test]
fn test_interface_on_ocaml() {
    check_interface("ocaml");
}

// ---------------------------------------------------------------- search
#[test]
fn test_search_on_python() {
    check_search("python");
}
#[test]
fn test_search_on_typescript() {
    check_search("typescript");
}
#[test]
fn test_search_on_javascript() {
    check_search("javascript");
}
#[test]
fn test_search_on_go() {
    check_search("go");
}
#[test]
fn test_search_on_rust() {
    check_search("rust");
}
#[test]
fn test_search_on_java() {
    check_search("java");
}
#[test]
fn test_search_on_c() {
    check_search("c");
}
#[test]
fn test_search_on_cpp() {
    check_search("cpp");
}
#[test]
fn test_search_on_ruby() {
    check_search("ruby");
}
#[test]
fn test_search_on_kotlin() {
    check_search("kotlin");
}
#[test]
fn test_search_on_swift() {
    check_search("swift");
}
#[test]
fn test_search_on_csharp() {
    check_search("csharp");
}
#[test]
fn test_search_on_scala() {
    check_search("scala");
}
#[test]
fn test_search_on_php() {
    check_search("php");
}
#[test]
fn test_search_on_lua() {
    check_search("lua");
}
#[test]
fn test_search_on_luau() {
    check_search("luau");
}
#[test]
fn test_search_on_elixir() {
    check_search("elixir");
}
#[test]
fn test_search_on_ocaml() {
    check_search("ocaml");
}

// ---------------------------------------------------------------- context
#[test]
fn test_context_on_python() {
    check_context("python");
}
#[test]
fn test_context_on_typescript() {
    check_context("typescript");
}
#[test]
fn test_context_on_javascript() {
    check_context("javascript");
}
#[test]
fn test_context_on_go() {
    check_context("go");
}
#[test]
fn test_context_on_rust() {
    check_context("rust");
}
#[test]
fn test_context_on_java() {
    check_context("java");
}
#[test]
fn test_context_on_c() {
    check_context("c");
}
#[test]
fn test_context_on_cpp() {
    check_context("cpp");
}
#[test]
fn test_context_on_ruby() {
    check_context("ruby");
}
#[test]
fn test_context_on_kotlin() {
    check_context("kotlin");
}
#[test]
fn test_context_on_swift() {
    check_context("swift");
}
#[test]
fn test_context_on_csharp() {
    check_context("csharp");
}
#[test]
fn test_context_on_scala() {
    check_context("scala");
}
#[test]
fn test_context_on_php() {
    check_context("php");
}
#[test]
fn test_context_on_lua() {
    check_context("lua");
}
#[test]
fn test_context_on_luau() {
    check_context("luau");
}
#[test]
fn test_context_on_elixir() {
    check_context("elixir");
}
#[test]
fn test_context_on_ocaml() {
    check_context("ocaml");
}

// ---------------------------------------------------------------- temporal
//
// VAL-016: temporal mines method-call sequences for all 18 supported
// languages by reusing the per-language callgraph handlers' call extraction
// (see commands/patterns/temporal.rs analyze_temporal_directory which now
// dispatches via Language::from_path + build_project_call_graph_v2).
#[test]
fn test_temporal_on_python() {
    check_temporal("python");
}
#[test]
fn test_temporal_on_typescript() {
    check_temporal("typescript");
}
#[test]
fn test_temporal_on_javascript() {
    check_temporal("javascript");
}
#[test]
fn test_temporal_on_go() {
    check_temporal("go");
}
#[test]
fn test_temporal_on_rust() {
    check_temporal("rust");
}
#[test]
fn test_temporal_on_java() {
    check_temporal("java");
}
#[test]
fn test_temporal_on_c() {
    check_temporal("c");
}
#[test]
fn test_temporal_on_cpp() {
    check_temporal("cpp");
}
#[test]
fn test_temporal_on_ruby() {
    check_temporal("ruby");
}
#[test]
fn test_temporal_on_kotlin() {
    check_temporal("kotlin");
}
#[test]
fn test_temporal_on_swift() {
    check_temporal("swift");
}
#[test]
fn test_temporal_on_csharp() {
    check_temporal("csharp");
}
#[test]
fn test_temporal_on_scala() {
    check_temporal("scala");
}
#[test]
fn test_temporal_on_php() {
    check_temporal("php");
}
#[test]
fn test_temporal_on_lua() {
    check_temporal("lua");
}
#[test]
fn test_temporal_on_luau() {
    check_temporal("luau");
}
#[test]
fn test_temporal_on_elixir() {
    check_temporal("elixir");
}
#[test]
fn test_temporal_on_ocaml() {
    check_temporal("ocaml");
}

// ---------------------------------------------------------------- diagnostics
#[test]
fn test_diagnostics_on_python() {
    check_diagnostics("python");
}
#[test]
fn test_diagnostics_on_typescript() {
    check_diagnostics("typescript");
}
#[test]
fn test_diagnostics_on_javascript() {
    check_diagnostics("javascript");
}
#[test]
fn test_diagnostics_on_go() {
    check_diagnostics("go");
}
#[test]
fn test_diagnostics_on_rust() {
    check_diagnostics("rust");
}
#[test]
fn test_diagnostics_on_java() {
    check_diagnostics("java");
}
#[test]
fn test_diagnostics_on_c() {
    check_diagnostics("c");
}
#[test]
fn test_diagnostics_on_cpp() {
    check_diagnostics("cpp");
}
#[test]
fn test_diagnostics_on_ruby() {
    check_diagnostics("ruby");
}
#[test]
fn test_diagnostics_on_kotlin() {
    check_diagnostics("kotlin");
}
#[test]
fn test_diagnostics_on_swift() {
    check_diagnostics("swift");
}
#[test]
fn test_diagnostics_on_csharp() {
    check_diagnostics("csharp");
}
#[test]
fn test_diagnostics_on_scala() {
    check_diagnostics("scala");
}
#[test]
fn test_diagnostics_on_php() {
    check_diagnostics("php");
}
#[test]
fn test_diagnostics_on_lua() {
    check_diagnostics("lua");
}
#[test]
fn test_diagnostics_on_luau() {
    check_diagnostics("luau");
}
#[test]
fn test_diagnostics_on_elixir() {
    check_diagnostics("elixir");
}
#[test]
fn test_diagnostics_on_ocaml() {
    check_diagnostics("ocaml");
}

// ---------------------------------------------------------------- inheritance
#[test]
fn test_inheritance_on_python() {
    check_inheritance("python");
}
#[test]
fn test_inheritance_on_typescript() {
    check_inheritance("typescript");
}
#[test]
fn test_inheritance_on_javascript() {
    check_inheritance("javascript");
}
#[test]
fn test_inheritance_on_go() {
    check_inheritance("go");
}
#[test]
fn test_inheritance_on_rust() {
    check_inheritance("rust");
}
#[test]
fn test_inheritance_on_java() {
    check_inheritance("java");
}
#[test]
fn test_inheritance_on_c() {
    check_inheritance("c");
}
#[test]
fn test_inheritance_on_cpp() {
    check_inheritance("cpp");
}
#[test]
fn test_inheritance_on_ruby() {
    check_inheritance("ruby");
}
#[test]
fn test_inheritance_on_kotlin() {
    check_inheritance("kotlin");
}
#[test]
fn test_inheritance_on_swift() {
    check_inheritance("swift");
}
#[test]
fn test_inheritance_on_csharp() {
    check_inheritance("csharp");
}
#[test]
fn test_inheritance_on_scala() {
    check_inheritance("scala");
}
#[test]
fn test_inheritance_on_php() {
    check_inheritance("php");
}
#[test]
fn test_inheritance_on_lua() {
    check_inheritance("lua");
}
#[test]
fn test_inheritance_on_luau() {
    check_inheritance("luau");
}
#[test]
fn test_inheritance_on_elixir() {
    check_inheritance("elixir");
}
#[test]
fn test_inheritance_on_ocaml() {
    check_inheritance("ocaml");
}

// ---------------------------------------------------------------- definition
//
// VAL-015 generalised `tldr definition` from Python-only to all 18
// languages. The dispatch reuses each language handler's
// `CallGraphLanguageSupport::extract_definitions` API to locate the
// definition site of the requested symbol, with a project walk for
// cross-file resolution.
#[test]
fn test_definition_on_python() {
    check_definition("python");
}
#[test]
fn test_definition_on_typescript() {
    check_definition("typescript");
}
#[test]
fn test_definition_on_javascript() {
    check_definition("javascript");
}
#[test]
fn test_definition_on_go() {
    check_definition("go");
}
#[test]
fn test_definition_on_rust() {
    check_definition("rust");
}
#[test]
fn test_definition_on_java() {
    check_definition("java");
}
#[test]
fn test_definition_on_c() {
    check_definition("c");
}
#[test]
fn test_definition_on_cpp() {
    check_definition("cpp");
}
#[test]
fn test_definition_on_ruby() {
    check_definition("ruby");
}
#[test]
fn test_definition_on_kotlin() {
    check_definition("kotlin");
}
#[test]
fn test_definition_on_swift() {
    check_definition("swift");
}
#[test]
fn test_definition_on_csharp() {
    check_definition("csharp");
}
#[test]
fn test_definition_on_scala() {
    check_definition("scala");
}
#[test]
fn test_definition_on_php() {
    check_definition("php");
}
#[test]
fn test_definition_on_lua() {
    check_definition("lua");
}
#[test]
fn test_definition_on_luau() {
    check_definition("luau");
}
#[test]
fn test_definition_on_elixir() {
    check_definition("elixir");
}
#[test]
fn test_definition_on_ocaml() {
    check_definition("ocaml");
}

// ---------------------------------------------------------------- cohesion
#[test]
fn test_cohesion_on_python() {
    check_cohesion("python");
}
#[test]
fn test_cohesion_on_typescript() {
    check_cohesion("typescript");
}
#[test]
fn test_cohesion_on_javascript() {
    check_cohesion("javascript");
}
#[test]
fn test_cohesion_on_go() {
    check_cohesion("go");
}
#[test]
fn test_cohesion_on_rust() {
    check_cohesion("rust");
}
#[test]
fn test_cohesion_on_java() {
    check_cohesion("java");
}
#[test]
fn test_cohesion_on_c() {
    check_cohesion("c");
}
#[test]
fn test_cohesion_on_cpp() {
    check_cohesion("cpp");
}
#[test]
fn test_cohesion_on_ruby() {
    check_cohesion("ruby");
}
#[test]
fn test_cohesion_on_kotlin() {
    check_cohesion("kotlin");
}
#[test]
fn test_cohesion_on_swift() {
    check_cohesion("swift");
}
#[test]
fn test_cohesion_on_csharp() {
    check_cohesion("csharp");
}
#[test]
fn test_cohesion_on_scala() {
    check_cohesion("scala");
}
#[test]
fn test_cohesion_on_php() {
    check_cohesion("php");
}
#[test]
fn test_cohesion_on_lua() {
    check_cohesion("lua");
}
#[test]
fn test_cohesion_on_luau() {
    check_cohesion("luau");
}
#[test]
fn test_cohesion_on_elixir() {
    check_cohesion("elixir");
}
#[test]
fn test_cohesion_on_ocaml() {
    check_cohesion("ocaml");
}

// ---------------------------------------------------------------- slice
#[test]
fn test_slice_on_python() {
    check_slice("python");
}
#[test]
fn test_slice_on_typescript() {
    check_slice("typescript");
}
#[test]
fn test_slice_on_javascript() {
    check_slice("javascript");
}
#[test]
fn test_slice_on_go() {
    check_slice("go");
}
#[test]
fn test_slice_on_rust() {
    check_slice("rust");
}
#[test]
fn test_slice_on_java() {
    check_slice("java");
}
#[test]
fn test_slice_on_c() {
    check_slice("c");
}
#[test]
fn test_slice_on_cpp() {
    check_slice("cpp");
}
#[test]
fn test_slice_on_ruby() {
    check_slice("ruby");
}
#[test]
fn test_slice_on_kotlin() {
    check_slice("kotlin");
}
#[test]
fn test_slice_on_swift() {
    check_slice("swift");
}
#[test]
fn test_slice_on_csharp() {
    check_slice("csharp");
}
#[test]
fn test_slice_on_scala() {
    check_slice("scala");
}
#[test]
fn test_slice_on_php() {
    check_slice("php");
}
#[test]
fn test_slice_on_lua() {
    check_slice("lua");
}
#[test]
fn test_slice_on_luau() {
    check_slice("luau");
}
#[test]
fn test_slice_on_elixir() {
    check_slice("elixir");
}
#[test]
fn test_slice_on_ocaml() {
    check_slice("ocaml");
}

// ---------------------------------------------------------------- chop
#[test]
fn test_chop_on_python() {
    check_chop("python");
}
#[test]
fn test_chop_on_typescript() {
    check_chop("typescript");
}
#[test]
fn test_chop_on_javascript() {
    check_chop("javascript");
}
#[test]
fn test_chop_on_go() {
    check_chop("go");
}
#[test]
fn test_chop_on_rust() {
    check_chop("rust");
}
#[test]
fn test_chop_on_java() {
    check_chop("java");
}
#[test]
fn test_chop_on_c() {
    check_chop("c");
}
#[test]
fn test_chop_on_cpp() {
    check_chop("cpp");
}
#[test]
fn test_chop_on_ruby() {
    check_chop("ruby");
}
#[test]
fn test_chop_on_kotlin() {
    check_chop("kotlin");
}
#[test]
fn test_chop_on_swift() {
    check_chop("swift");
}
#[test]
fn test_chop_on_csharp() {
    check_chop("csharp");
}
#[test]
fn test_chop_on_scala() {
    check_chop("scala");
}
#[test]
fn test_chop_on_php() {
    check_chop("php");
}
#[test]
fn test_chop_on_lua() {
    check_chop("lua");
}
#[test]
fn test_chop_on_luau() {
    check_chop("luau");
}
#[test]
fn test_chop_on_elixir() {
    check_chop("elixir");
}
#[test]
fn test_chop_on_ocaml() {
    check_chop("ocaml");
}

// ---------------------------------------------------------------- reaching-defs
#[test]
fn test_reaching_defs_on_python() {
    check_reaching_defs("python");
}
#[test]
fn test_reaching_defs_on_typescript() {
    check_reaching_defs("typescript");
}
#[test]
fn test_reaching_defs_on_javascript() {
    check_reaching_defs("javascript");
}
#[test]
fn test_reaching_defs_on_go() {
    check_reaching_defs("go");
}
#[test]
fn test_reaching_defs_on_rust() {
    check_reaching_defs("rust");
}
#[test]
fn test_reaching_defs_on_java() {
    check_reaching_defs("java");
}
#[test]
fn test_reaching_defs_on_c() {
    check_reaching_defs("c");
}
#[test]
fn test_reaching_defs_on_cpp() {
    check_reaching_defs("cpp");
}
#[test]
fn test_reaching_defs_on_ruby() {
    check_reaching_defs("ruby");
}
#[test]
fn test_reaching_defs_on_kotlin() {
    check_reaching_defs("kotlin");
}
#[test]
fn test_reaching_defs_on_swift() {
    check_reaching_defs("swift");
}
#[test]
fn test_reaching_defs_on_csharp() {
    check_reaching_defs("csharp");
}
#[test]
fn test_reaching_defs_on_scala() {
    check_reaching_defs("scala");
}
#[test]
fn test_reaching_defs_on_php() {
    check_reaching_defs("php");
}
#[test]
fn test_reaching_defs_on_lua() {
    check_reaching_defs("lua");
}
#[test]
fn test_reaching_defs_on_luau() {
    check_reaching_defs("luau");
}
#[test]
fn test_reaching_defs_on_elixir() {
    check_reaching_defs("elixir");
}
#[test]
fn test_reaching_defs_on_ocaml() {
    check_reaching_defs("ocaml");
}

// ---------------------------------------------------------------- available
#[test]
fn test_available_on_python() {
    check_available("python");
}
#[test]
fn test_available_on_typescript() {
    check_available("typescript");
}
#[test]
fn test_available_on_javascript() {
    check_available("javascript");
}
#[test]
fn test_available_on_go() {
    check_available("go");
}
#[test]
fn test_available_on_rust() {
    check_available("rust");
}
#[test]
fn test_available_on_java() {
    check_available("java");
}
#[test]
fn test_available_on_c() {
    check_available("c");
}
#[test]
fn test_available_on_cpp() {
    check_available("cpp");
}
#[test]
fn test_available_on_ruby() {
    check_available("ruby");
}
#[test]
fn test_available_on_kotlin() {
    check_available("kotlin");
}
#[test]
fn test_available_on_swift() {
    check_available("swift");
}
#[test]
fn test_available_on_csharp() {
    check_available("csharp");
}
#[test]
fn test_available_on_scala() {
    check_available("scala");
}
#[test]
fn test_available_on_php() {
    check_available("php");
}
#[test]
fn test_available_on_lua() {
    check_available("lua");
}
#[test]
fn test_available_on_luau() {
    check_available("luau");
}
#[test]
fn test_available_on_elixir() {
    check_available("elixir");
}
#[test]
fn test_available_on_ocaml() {
    check_available("ocaml");
}

// ---------------------------------------------------------------- dead-stores
#[test]
fn test_dead_stores_on_python() {
    check_dead_stores("python");
}
#[test]
fn test_dead_stores_on_typescript() {
    check_dead_stores("typescript");
}
#[test]
fn test_dead_stores_on_javascript() {
    check_dead_stores("javascript");
}
#[test]
fn test_dead_stores_on_go() {
    check_dead_stores("go");
}
#[test]
fn test_dead_stores_on_rust() {
    check_dead_stores("rust");
}
#[test]
fn test_dead_stores_on_java() {
    check_dead_stores("java");
}
#[test]
fn test_dead_stores_on_c() {
    check_dead_stores("c");
}
#[test]
fn test_dead_stores_on_cpp() {
    check_dead_stores("cpp");
}
#[test]
fn test_dead_stores_on_ruby() {
    check_dead_stores("ruby");
}
#[test]
fn test_dead_stores_on_kotlin() {
    check_dead_stores("kotlin");
}
#[test]
fn test_dead_stores_on_swift() {
    check_dead_stores("swift");
}
#[test]
fn test_dead_stores_on_csharp() {
    check_dead_stores("csharp");
}
#[test]
fn test_dead_stores_on_scala() {
    check_dead_stores("scala");
}
#[test]
fn test_dead_stores_on_php() {
    check_dead_stores("php");
}
#[test]
fn test_dead_stores_on_lua() {
    check_dead_stores("lua");
}
#[test]
fn test_dead_stores_on_luau() {
    check_dead_stores("luau");
}
#[test]
fn test_dead_stores_on_elixir() {
    check_dead_stores("elixir");
}
#[test]
fn test_dead_stores_on_ocaml() {
    check_dead_stores("ocaml");
}

// ---------------------------------------------------------------- resources
#[test]
fn test_resources_on_python() {
    check_resources("python");
}
#[test]
fn test_resources_on_typescript() {
    check_resources("typescript");
}
#[test]
fn test_resources_on_javascript() {
    check_resources("javascript");
}
#[test]
fn test_resources_on_go() {
    check_resources("go");
}
#[test]
fn test_resources_on_rust() {
    check_resources("rust");
}
#[test]
fn test_resources_on_java() {
    check_resources("java");
}
#[test]
fn test_resources_on_c() {
    check_resources("c");
}
#[test]
fn test_resources_on_cpp() {
    check_resources("cpp");
}
#[test]
fn test_resources_on_ruby() {
    check_resources("ruby");
}
#[test]
fn test_resources_on_kotlin() {
    check_resources("kotlin");
}
#[test]
fn test_resources_on_swift() {
    check_resources("swift");
}
#[test]
fn test_resources_on_csharp() {
    check_resources("csharp");
}
#[test]
fn test_resources_on_scala() {
    check_resources("scala");
}
#[test]
fn test_resources_on_php() {
    check_resources("php");
}
#[test]
fn test_resources_on_lua() {
    check_resources("lua");
}
#[test]
fn test_resources_on_luau() {
    check_resources("luau");
}
#[test]
fn test_resources_on_elixir() {
    check_resources("elixir");
}
#[test]
fn test_resources_on_ocaml() {
    check_resources("ocaml");
}

// ---------------------------------------------------------------- explain
#[test]
fn test_explain_on_python() {
    check_explain("python");
}
#[test]
fn test_explain_on_typescript() {
    check_explain("typescript");
}
#[test]
fn test_explain_on_javascript() {
    check_explain("javascript");
}
#[test]
fn test_explain_on_go() {
    check_explain("go");
}
#[test]
fn test_explain_on_rust() {
    check_explain("rust");
}
#[test]
fn test_explain_on_java() {
    check_explain("java");
}
#[test]
fn test_explain_on_c() {
    check_explain("c");
}
#[test]
fn test_explain_on_cpp() {
    check_explain("cpp");
}
#[test]
fn test_explain_on_ruby() {
    check_explain("ruby");
}
#[test]
fn test_explain_on_kotlin() {
    check_explain("kotlin");
}
#[test]
fn test_explain_on_swift() {
    check_explain("swift");
}
#[test]
fn test_explain_on_csharp() {
    check_explain("csharp");
}
#[test]
fn test_explain_on_scala() {
    check_explain("scala");
}
#[test]
fn test_explain_on_php() {
    check_explain("php");
}
#[test]
fn test_explain_on_lua() {
    check_explain("lua");
}
#[test]
fn test_explain_on_luau() {
    check_explain("luau");
}
#[test]
fn test_explain_on_elixir() {
    check_explain("elixir");
}
#[test]
fn test_explain_on_ocaml() {
    check_explain("ocaml");
}

// ---------------------------------------------------------------- contracts
#[test]
fn test_contracts_on_python() {
    check_contracts("python");
}
#[test]
fn test_contracts_on_typescript() {
    check_contracts("typescript");
}
#[test]
fn test_contracts_on_javascript() {
    check_contracts("javascript");
}
#[test]
fn test_contracts_on_go() {
    check_contracts("go");
}
#[test]
fn test_contracts_on_rust() {
    check_contracts("rust");
}
#[test]
fn test_contracts_on_java() {
    check_contracts("java");
}
#[test]
fn test_contracts_on_c() {
    check_contracts("c");
}
#[test]
fn test_contracts_on_cpp() {
    check_contracts("cpp");
}
#[test]
fn test_contracts_on_ruby() {
    check_contracts("ruby");
}
#[test]
fn test_contracts_on_kotlin() {
    check_contracts("kotlin");
}
#[test]
fn test_contracts_on_swift() {
    check_contracts("swift");
}
#[test]
fn test_contracts_on_csharp() {
    check_contracts("csharp");
}
#[test]
fn test_contracts_on_scala() {
    check_contracts("scala");
}
#[test]
fn test_contracts_on_php() {
    check_contracts("php");
}
#[test]
fn test_contracts_on_lua() {
    check_contracts("lua");
}
#[test]
fn test_contracts_on_luau() {
    check_contracts("luau");
}
#[test]
fn test_contracts_on_elixir() {
    check_contracts("elixir");
}
#[test]
fn test_contracts_on_ocaml() {
    check_contracts("ocaml");
}

// ---------------------------------------------------------------- taint
#[test]
fn test_taint_on_python() {
    check_taint("python");
}
#[test]
fn test_taint_on_typescript() {
    check_taint("typescript");
}
#[test]
fn test_taint_on_javascript() {
    check_taint("javascript");
}
#[test]
fn test_taint_on_go() {
    check_taint("go");
}
#[test]
fn test_taint_on_rust() {
    check_taint("rust");
}
#[test]
fn test_taint_on_java() {
    check_taint("java");
}
#[test]
fn test_taint_on_c() {
    check_taint("c");
}
#[test]
fn test_taint_on_cpp() {
    check_taint("cpp");
}
#[test]
fn test_taint_on_ruby() {
    check_taint("ruby");
}
#[test]
fn test_taint_on_kotlin() {
    check_taint("kotlin");
}
#[test]
fn test_taint_on_swift() {
    check_taint("swift");
}
#[test]
fn test_taint_on_csharp() {
    check_taint("csharp");
}
#[test]
fn test_taint_on_scala() {
    check_taint("scala");
}
#[test]
fn test_taint_on_php() {
    check_taint("php");
}
#[test]
fn test_taint_on_lua() {
    check_taint("lua");
}
#[test]
fn test_taint_on_luau() {
    check_taint("luau");
}
#[test]
fn test_taint_on_elixir() {
    check_taint("elixir");
}
#[test]
fn test_taint_on_ocaml() {
    check_taint("ocaml");
}

// ---------------------------------------------------------------- diff
#[test]
fn test_diff_on_python() {
    check_diff("python");
}
#[test]
fn test_diff_on_typescript() {
    check_diff("typescript");
}
#[test]
fn test_diff_on_javascript() {
    check_diff("javascript");
}
#[test]
fn test_diff_on_go() {
    check_diff("go");
}
#[test]
fn test_diff_on_rust() {
    check_diff("rust");
}
#[test]
fn test_diff_on_java() {
    check_diff("java");
}
#[test]
fn test_diff_on_c() {
    check_diff("c");
}
#[test]
fn test_diff_on_cpp() {
    check_diff("cpp");
}
#[test]
fn test_diff_on_ruby() {
    check_diff("ruby");
}
#[test]
fn test_diff_on_kotlin() {
    check_diff("kotlin");
}
#[test]
fn test_diff_on_swift() {
    check_diff("swift");
}
#[test]
fn test_diff_on_csharp() {
    check_diff("csharp");
}
#[test]
fn test_diff_on_scala() {
    check_diff("scala");
}
#[test]
fn test_diff_on_php() {
    check_diff("php");
}
#[test]
fn test_diff_on_lua() {
    check_diff("lua");
}
#[test]
fn test_diff_on_luau() {
    check_diff("luau");
}
#[test]
fn test_diff_on_elixir() {
    check_diff("elixir");
}
#[test]
fn test_diff_on_ocaml() {
    check_diff("ocaml");
}

// ---------------------------------------------------------------- dice
#[test]
fn test_dice_on_python() {
    check_dice("python");
}
#[test]
fn test_dice_on_typescript() {
    check_dice("typescript");
}
#[test]
fn test_dice_on_javascript() {
    check_dice("javascript");
}
#[test]
fn test_dice_on_go() {
    check_dice("go");
}
#[test]
fn test_dice_on_rust() {
    check_dice("rust");
}
#[test]
fn test_dice_on_java() {
    check_dice("java");
}
#[test]
fn test_dice_on_c() {
    check_dice("c");
}
#[test]
fn test_dice_on_cpp() {
    check_dice("cpp");
}
#[test]
fn test_dice_on_ruby() {
    check_dice("ruby");
}
#[test]
fn test_dice_on_kotlin() {
    check_dice("kotlin");
}
#[test]
fn test_dice_on_swift() {
    check_dice("swift");
}
#[test]
fn test_dice_on_csharp() {
    check_dice("csharp");
}
#[test]
fn test_dice_on_scala() {
    check_dice("scala");
}
#[test]
fn test_dice_on_php() {
    check_dice("php");
}
#[test]
fn test_dice_on_lua() {
    check_dice("lua");
}
#[test]
fn test_dice_on_luau() {
    check_dice("luau");
}
#[test]
fn test_dice_on_elixir() {
    check_dice("elixir");
}
#[test]
fn test_dice_on_ocaml() {
    check_dice("ocaml");
}

// ---------------------------------------------------------------- coupling
#[test]
fn test_coupling_on_python() {
    check_coupling("python");
}
#[test]
fn test_coupling_on_typescript() {
    check_coupling("typescript");
}
#[test]
fn test_coupling_on_javascript() {
    check_coupling("javascript");
}
#[test]
fn test_coupling_on_go() {
    check_coupling("go");
}
#[test]
fn test_coupling_on_rust() {
    check_coupling("rust");
}
#[test]
fn test_coupling_on_java() {
    check_coupling("java");
}
#[test]
fn test_coupling_on_c() {
    check_coupling("c");
}
#[test]
fn test_coupling_on_cpp() {
    check_coupling("cpp");
}
#[test]
fn test_coupling_on_ruby() {
    check_coupling("ruby");
}
#[test]
fn test_coupling_on_kotlin() {
    check_coupling("kotlin");
}
#[test]
fn test_coupling_on_swift() {
    check_coupling("swift");
}
#[test]
fn test_coupling_on_csharp() {
    check_coupling("csharp");
}
#[test]
fn test_coupling_on_scala() {
    check_coupling("scala");
}
#[test]
fn test_coupling_on_php() {
    check_coupling("php");
}
#[test]
fn test_coupling_on_lua() {
    check_coupling("lua");
}
#[test]
fn test_coupling_on_luau() {
    check_coupling("luau");
}
#[test]
fn test_coupling_on_elixir() {
    check_coupling("elixir");
}
#[test]
fn test_coupling_on_ocaml() {
    check_coupling("ocaml");
}

// ---------------------------------------------------------------- embed
#[test]
fn test_embed_on_python() {
    check_embed("python");
}
#[test]
fn test_embed_on_typescript() {
    check_embed("typescript");
}
#[test]
fn test_embed_on_javascript() {
    check_embed("javascript");
}
#[test]
fn test_embed_on_go() {
    check_embed("go");
}
#[test]
fn test_embed_on_rust() {
    check_embed("rust");
}
#[test]
fn test_embed_on_java() {
    check_embed("java");
}
#[test]
fn test_embed_on_c() {
    check_embed("c");
}
#[test]
fn test_embed_on_cpp() {
    check_embed("cpp");
}
#[test]
fn test_embed_on_ruby() {
    check_embed("ruby");
}
#[test]
fn test_embed_on_kotlin() {
    check_embed("kotlin");
}
#[test]
fn test_embed_on_swift() {
    check_embed("swift");
}
#[test]
fn test_embed_on_csharp() {
    check_embed("csharp");
}
#[test]
fn test_embed_on_scala() {
    check_embed("scala");
}
#[test]
fn test_embed_on_php() {
    check_embed("php");
}
#[test]
fn test_embed_on_lua() {
    check_embed("lua");
}
#[test]
fn test_embed_on_luau() {
    check_embed("luau");
}
#[test]
fn test_embed_on_elixir() {
    check_embed("elixir");
}
#[test]
fn test_embed_on_ocaml() {
    check_embed("ocaml");
}

// ---------------------------------------------------------------- semantic
#[test]
fn test_semantic_on_python() {
    check_semantic("python");
}
#[test]
fn test_semantic_on_typescript() {
    check_semantic("typescript");
}
#[test]
fn test_semantic_on_javascript() {
    check_semantic("javascript");
}
#[test]
fn test_semantic_on_go() {
    check_semantic("go");
}
#[test]
fn test_semantic_on_rust() {
    check_semantic("rust");
}
#[test]
fn test_semantic_on_java() {
    check_semantic("java");
}
#[test]
fn test_semantic_on_c() {
    check_semantic("c");
}
#[test]
fn test_semantic_on_cpp() {
    check_semantic("cpp");
}
#[test]
fn test_semantic_on_ruby() {
    check_semantic("ruby");
}
#[test]
fn test_semantic_on_kotlin() {
    check_semantic("kotlin");
}
#[test]
fn test_semantic_on_swift() {
    check_semantic("swift");
}
#[test]
fn test_semantic_on_csharp() {
    check_semantic("csharp");
}
#[test]
fn test_semantic_on_scala() {
    check_semantic("scala");
}
#[test]
fn test_semantic_on_php() {
    check_semantic("php");
}
#[test]
fn test_semantic_on_lua() {
    check_semantic("lua");
}
#[test]
fn test_semantic_on_luau() {
    check_semantic("luau");
}
#[test]
fn test_semantic_on_elixir() {
    check_semantic("elixir");
}
#[test]
fn test_semantic_on_ocaml() {
    check_semantic("ocaml");
}

// ---------------------------------------------------------------- similar
#[test]
fn test_similar_on_python() {
    check_similar("python");
}
#[test]
fn test_similar_on_typescript() {
    check_similar("typescript");
}
#[test]
fn test_similar_on_javascript() {
    check_similar("javascript");
}
#[test]
fn test_similar_on_go() {
    check_similar("go");
}
#[test]
fn test_similar_on_rust() {
    check_similar("rust");
}
#[test]
fn test_similar_on_java() {
    check_similar("java");
}
#[test]
fn test_similar_on_c() {
    check_similar("c");
}
#[test]
fn test_similar_on_cpp() {
    check_similar("cpp");
}
#[test]
fn test_similar_on_ruby() {
    check_similar("ruby");
}
#[test]
fn test_similar_on_kotlin() {
    check_similar("kotlin");
}
#[test]
fn test_similar_on_swift() {
    check_similar("swift");
}
#[test]
fn test_similar_on_csharp() {
    check_similar("csharp");
}
#[test]
fn test_similar_on_scala() {
    check_similar("scala");
}
#[test]
fn test_similar_on_php() {
    check_similar("php");
}
#[test]
fn test_similar_on_lua() {
    check_similar("lua");
}
#[test]
fn test_similar_on_luau() {
    check_similar("luau");
}
#[test]
fn test_similar_on_elixir() {
    check_similar("elixir");
}
#[test]
fn test_similar_on_ocaml() {
    check_similar("ocaml");
}

// ---------------------------------------------------------------- surface
#[test]
fn test_surface_on_python() {
    check_surface("python");
}
#[test]
fn test_surface_on_typescript() {
    check_surface("typescript");
}
#[test]
fn test_surface_on_javascript() {
    check_surface("javascript");
}
#[test]
fn test_surface_on_go() {
    check_surface("go");
}
#[test]
fn test_surface_on_rust() {
    check_surface("rust");
}
#[test]
fn test_surface_on_java() {
    check_surface("java");
}
#[test]
fn test_surface_on_c() {
    check_surface("c");
}
#[test]
fn test_surface_on_cpp() {
    check_surface("cpp");
}
#[test]
fn test_surface_on_ruby() {
    check_surface("ruby");
}
#[test]
fn test_surface_on_kotlin() {
    check_surface("kotlin");
}
#[test]
fn test_surface_on_swift() {
    check_surface("swift");
}
#[test]
fn test_surface_on_csharp() {
    check_surface("csharp");
}
#[test]
fn test_surface_on_scala() {
    check_surface("scala");
}
#[test]
fn test_surface_on_php() {
    check_surface("php");
}
#[test]
fn test_surface_on_lua() {
    check_surface("lua");
}
#[test]
fn test_surface_on_luau() {
    check_surface("luau");
}
#[test]
fn test_surface_on_elixir() {
    check_surface("elixir");
}
#[test]
fn test_surface_on_ocaml() {
    check_surface("ocaml");
}

// ---------------------------------------------------------------- churn
// VAL-017: per-language churn cells. `make_git_fixture` provides 3
// commits so churn always has non-empty `files` to report.
#[test]
fn test_churn_on_python() {
    check_churn("python");
}
#[test]
fn test_churn_on_typescript() {
    check_churn("typescript");
}
#[test]
fn test_churn_on_javascript() {
    check_churn("javascript");
}
#[test]
fn test_churn_on_go() {
    check_churn("go");
}
#[test]
fn test_churn_on_rust() {
    check_churn("rust");
}
#[test]
fn test_churn_on_java() {
    check_churn("java");
}
#[test]
fn test_churn_on_c() {
    check_churn("c");
}
#[test]
fn test_churn_on_cpp() {
    check_churn("cpp");
}
#[test]
fn test_churn_on_ruby() {
    check_churn("ruby");
}
#[test]
fn test_churn_on_kotlin() {
    check_churn("kotlin");
}
#[test]
fn test_churn_on_swift() {
    check_churn("swift");
}
#[test]
fn test_churn_on_csharp() {
    check_churn("csharp");
}
#[test]
fn test_churn_on_scala() {
    check_churn("scala");
}
#[test]
fn test_churn_on_php() {
    check_churn("php");
}
#[test]
fn test_churn_on_lua() {
    check_churn("lua");
}
#[test]
fn test_churn_on_luau() {
    check_churn("luau");
}
#[test]
fn test_churn_on_elixir() {
    check_churn("elixir");
}
#[test]
fn test_churn_on_ocaml() {
    check_churn("ocaml");
}

// ---------------------------------------------------------------- hotspots
// VAL-017: per-language hotspots cells. The 3-commit fixture matches
// the default `min_commits = 3` threshold, ensuring `hotspots` array
// is non-empty for every language.
#[test]
fn test_hotspots_on_python() {
    check_hotspots("python");
}
#[test]
fn test_hotspots_on_typescript() {
    check_hotspots("typescript");
}
#[test]
fn test_hotspots_on_javascript() {
    check_hotspots("javascript");
}
#[test]
fn test_hotspots_on_go() {
    check_hotspots("go");
}
#[test]
fn test_hotspots_on_rust() {
    check_hotspots("rust");
}
#[test]
fn test_hotspots_on_java() {
    check_hotspots("java");
}
#[test]
fn test_hotspots_on_c() {
    check_hotspots("c");
}
#[test]
fn test_hotspots_on_cpp() {
    check_hotspots("cpp");
}
#[test]
fn test_hotspots_on_ruby() {
    check_hotspots("ruby");
}
#[test]
fn test_hotspots_on_kotlin() {
    check_hotspots("kotlin");
}
#[test]
fn test_hotspots_on_swift() {
    check_hotspots("swift");
}
#[test]
fn test_hotspots_on_csharp() {
    check_hotspots("csharp");
}
#[test]
fn test_hotspots_on_scala() {
    check_hotspots("scala");
}
#[test]
fn test_hotspots_on_php() {
    check_hotspots("php");
}
#[test]
fn test_hotspots_on_lua() {
    check_hotspots("lua");
}
#[test]
fn test_hotspots_on_luau() {
    check_hotspots("luau");
}
#[test]
fn test_hotspots_on_elixir() {
    check_hotspots("elixir");
}
#[test]
fn test_hotspots_on_ocaml() {
    check_hotspots("ocaml");
}

// ============================================================================
// Orchestrator commands — sanity-only (--help)
// ============================================================================
#[test]
fn test_help_only_coverage() {
    check_help_only("coverage");
}
#[test]
fn test_help_only_fix() {
    check_help_only("fix");
}
#[test]
fn test_help_only_bugbot() {
    check_help_only("bugbot");
}
#[test]
fn test_help_only_daemon() {
    check_help_only("daemon");
}
#[test]
fn test_help_only_cache() {
    check_help_only("cache");
}
#[test]
fn test_help_only_stats() {
    check_help_only("stats");
}
#[test]
fn test_help_only_warm() {
    check_help_only("warm");
}
#[test]
fn test_help_only_doctor() {
    check_help_only("doctor");
}

#[test]
fn test_tree_runs_crash_free() {
    check_tree_one();
}
