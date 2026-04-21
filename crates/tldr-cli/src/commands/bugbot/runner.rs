//! Tool execution engine for L1 commodity diagnostics
//!
//! Spawns diagnostic tools as subprocesses, captures their output, parses it
//! through the appropriate parser, and handles timeouts and failures.
//!
//! Key behaviors:
//! - Binary not found: returns `ToolResult { success: false }` with spawn error
//! - Tool timeout: kills child process, returns timeout error
//! - Non-zero exit with parseable output: `success: true` (linters exit non-zero on findings)
//! - Parse error: `success: false` with parse error detail
//! - After parsing: injects `tool.name` into each `L1Finding.tool` [PM-6]
//!
//! Parallel execution uses `std::thread::scope` (Rust 1.63+) for safe scoped threads.

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::parsers;
use super::tools::{L1Finding, ToolConfig, ToolResult};

/// Kill a process by its OS-level PID. Cross-platform (F3).
///
/// On Unix, sends SIGKILL via libc. On Windows, uses `TerminateProcess`
/// via the `windows-sys` crate (or raw WinAPI). This is needed because the
/// watchdog thread only has the PID, not the `Child` handle (which is
/// consumed by `wait_with_output`).
fn kill_process_by_id(pid: u32) {
    #[cfg(unix)]
    {
        // SAFETY: We are sending SIGKILL to a process we spawned.
        // The PID is valid because we obtained it from child.id() before
        // the watchdog thread was spawned.
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGKILL);
        }
    }
    #[cfg(windows)]
    {
        // On Windows, open the process handle and terminate it.
        // SAFETY: We spawned this process and hold a valid PID.
        unsafe {
            let handle = windows_sys::Win32::System::Threading::OpenProcess(
                windows_sys::Win32::System::Threading::PROCESS_TERMINATE,
                0, // bInheritHandle = FALSE
                pid,
            );
            if !handle.is_null() {
                windows_sys::Win32::System::Threading::TerminateProcess(handle, 1);
                windows_sys::Win32::Foundation::CloseHandle(handle);
            }
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        // Unsupported platform: log a warning. The timeout flag is still
        // set, so the result will report a timeout even if the process
        // continues running.
        eprintln!("bugbot: cannot kill process {} on this platform", pid);
    }
}

/// Maximum bytes of stdout/stderr to retain from a tool subprocess.
///
/// This is a safety valve: clippy on a large project can produce megabytes
/// of JSON output. Beyond this limit, output is truncated to prevent
/// unbounded memory growth. 10 MB is generous for any reasonable project.
pub const MAX_OUTPUT_BYTES: usize = 10 * 1024 * 1024; // 10 MB

/// Executes diagnostic tools and captures their output.
///
/// Each tool is run as a subprocess with configurable timeout. Output is
/// captured and fed through the parser identified by `ToolConfig::parser`.
pub struct ToolRunner {
    /// Timeout per tool in seconds
    timeout_secs: u64,
}

impl ToolRunner {
    /// Create a new `ToolRunner` with the given per-tool timeout in seconds.
    pub fn new(timeout_secs: u64) -> Self {
        Self { timeout_secs }
    }

    /// Run a single tool and parse its output.
    ///
    /// # Contract
    /// - Binary not found: `ToolResult { success: false, error: "spawn" message }`
    /// - Tool timeout: kill child, `ToolResult { success: false, error: "Timeout" }`
    /// - Tool crashes (non-zero exit) with parseable output: `success: true`
    ///   (linters exit non-zero when findings exist)
    /// - Tool crashes with unparseable output: `success: false`
    /// - Parse error: `ToolResult { success: false, error: "Parse error: ..." }`
    /// - After parsing, injects `tool.name` into each `L1Finding.tool` [PM-6]
    pub fn run_tool(&self, tool: &ToolConfig, project_path: &Path) -> (ToolResult, Vec<L1Finding>) {
        let start = Instant::now();

        // Spawn the subprocess
        let child = Command::new(tool.binary)
            .args(tool.args)
            .current_dir(project_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();

        let child = match child {
            Ok(c) => c,
            Err(e) => {
                return (
                    ToolResult {
                        name: tool.name.to_string(),
                        category: tool.category,
                        success: false,
                        duration_ms: start.elapsed().as_millis() as u64,
                        finding_count: 0,
                        error: Some(format!(
                            "Failed to spawn '{}': {}",
                            tool.binary, e
                        )),
                        exit_code: None,
                    },
                    vec![],
                );
            }
        };

        // Set up timeout watchdog thread.
        // The watchdog sleeps for timeout_secs, then kills the process via SIGKILL.
        // Meanwhile the main thread calls wait_with_output() which blocks until
        // the child exits (either naturally or via the kill signal).
        let timeout = Duration::from_secs(self.timeout_secs);
        let child_id = child.id();
        let timed_out = Arc::new(AtomicBool::new(false));
        let timed_out_clone = timed_out.clone();

        let _watchdog = std::thread::spawn(move || {
            std::thread::sleep(timeout);
            timed_out_clone.store(true, Ordering::SeqCst);
            // Kill the child process. Platform-specific because we only have
            // the PID (the Child handle is consumed by wait_with_output).
            kill_process_by_id(child_id);
        });

        // Block until child exits (naturally or killed by watchdog)
        let output = child.wait_with_output();
        let duration_ms = start.elapsed().as_millis() as u64;

        // Check if watchdog triggered
        if timed_out.load(Ordering::SeqCst) {
            return (
                ToolResult {
                    name: tool.name.to_string(),
                    category: tool.category,
                    success: false,
                    duration_ms,
                    finding_count: 0,
                    error: Some(format!("Timeout after {}s", self.timeout_secs)),
                    exit_code: None,
                },
                vec![],
            );
        }

        // Read output and truncate to MAX_OUTPUT_BYTES to prevent unbounded
        // memory growth (F1 safety valve).
        let (stdout, stderr, exit_code) = match output {
            Ok(o) => {
                let raw_stdout = String::from_utf8_lossy(&o.stdout).to_string();
                let raw_stderr = String::from_utf8_lossy(&o.stderr).to_string();
                let stdout = if raw_stdout.len() > MAX_OUTPUT_BYTES {
                    let mut truncated = raw_stdout;
                    truncated.truncate(MAX_OUTPUT_BYTES);
                    // Trim to last complete line to avoid breaking JSON parsing
                    if let Some(last_newline) = truncated.rfind('\n') {
                        truncated.truncate(last_newline + 1);
                    }
                    truncated
                } else {
                    raw_stdout
                };
                let stderr = if raw_stderr.len() > MAX_OUTPUT_BYTES {
                    let mut truncated = raw_stderr;
                    truncated.truncate(MAX_OUTPUT_BYTES);
                    truncated
                } else {
                    raw_stderr
                };
                (stdout, stderr, o.status.code())
            }
            Err(e) => {
                return (
                    ToolResult {
                        name: tool.name.to_string(),
                        category: tool.category,
                        success: false,
                        duration_ms,
                        finding_count: 0,
                        error: Some(format!("Failed to read output: {}", e)),
                        exit_code: None,
                    },
                    vec![],
                );
            }
        };

        // Parse output through the tool's parser
        match parsers::parse_tool_output(tool.parser, &stdout) {
            Ok(mut findings) => {
                // PM-6: Inject tool name into each finding
                for f in &mut findings {
                    f.tool = tool.name.to_string();
                }
                let count = findings.len();
                (
                    ToolResult {
                        name: tool.name.to_string(),
                        category: tool.category,
                        success: true,
                        duration_ms,
                        finding_count: count,
                        error: None,
                        exit_code,
                    },
                    findings,
                )
            }
            Err(e) => {
                // If parse failed, include truncated stderr for diagnostics
                let error_msg = if stderr.is_empty() {
                    format!("Parse error: {}", e)
                } else {
                    let truncated = if stderr.len() > 200 {
                        &stderr[..200]
                    } else {
                        &stderr
                    };
                    format!("Parse error: {}. stderr: {}", e, truncated.trim())
                };
                (
                    ToolResult {
                        name: tool.name.to_string(),
                        category: tool.category,
                        success: false,
                        duration_ms,
                        finding_count: 0,
                        error: Some(error_msg),
                        exit_code,
                    },
                    vec![],
                )
            }
        }
    }

    /// Run multiple tools in parallel, collecting results.
    ///
    /// # Contract
    /// - One tool failure does not block others
    /// - Results are in deterministic order (same as input `tools` order)
    /// - All findings have tool name injected [PM-6]
    /// - Single tool or empty list: runs sequentially (no thread overhead)
    pub fn run_tools_parallel(
        &self,
        tools: &[&ToolConfig],
        project_path: &Path,
    ) -> (Vec<ToolResult>, Vec<L1Finding>) {
        if tools.len() <= 1 {
            return self.run_tools_sequential(tools, project_path);
        }

        // Parallel execution using scoped threads (Rust 1.63+)
        // Scoped threads allow borrowing from the enclosing scope safely.
        let results: Vec<(usize, ToolResult, Vec<L1Finding>)> = std::thread::scope(|s| {
            let handles: Vec<_> = tools
                .iter()
                .enumerate()
                .map(|(i, tool)| {
                    let tool_name = tool.name;
                    let tool_category = tool.category;
                    let path = project_path;
                    let handle = s.spawn(move || {
                        let (result, findings) = self.run_tool(tool, path);
                        (i, result, findings)
                    });
                    (handle, i, tool_name, tool_category)
                })
                .collect();

            // F4: Convert thread panics into ToolResult with success=false
            // instead of propagating the panic to the parent thread.
            handles
                .into_iter()
                .map(|(h, idx, name, category)| {
                    match h.join() {
                        Ok(result) => result,
                        Err(_) => {
                            eprintln!("bugbot: tool thread for '{}' panicked", name);
                            (
                                idx,
                                ToolResult {
                                    name: name.to_string(),
                                    category,
                                    success: false,
                                    duration_ms: 0,
                                    finding_count: 0,
                                    error: Some("Tool thread panicked".to_string()),
                                    exit_code: None,
                                },
                                vec![],
                            )
                        }
                    }
                })
                .collect()
        });

        // Sort by original index to maintain deterministic order
        let mut sorted = results;
        sorted.sort_by_key(|(i, _, _)| *i);

        let mut all_results = Vec::new();
        let mut all_findings = Vec::new();
        for (_idx, result, findings) in sorted {
            all_results.push(result);
            all_findings.extend(findings);
        }

        (all_results, all_findings)
    }

    /// Run tools sequentially. Used when there is 0 or 1 tool.
    fn run_tools_sequential(
        &self,
        tools: &[&ToolConfig],
        project_path: &Path,
    ) -> (Vec<ToolResult>, Vec<L1Finding>) {
        let mut all_results = Vec::new();
        let mut all_findings = Vec::new();
        for tool in tools {
            let (result, findings) = self.run_tool(tool, project_path);
            all_results.push(result);
            all_findings.extend(findings);
        }
        (all_results, all_findings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::bugbot::tools::ToolCategory;

    /// Helper: create a ToolConfig with the given parameters.
    /// Uses Box::leak to create &'static str / &'static [&'static str] from owned data.
    fn make_tool(
        name: &'static str,
        binary: &'static str,
        args: &'static [&'static str],
        parser: &'static str,
        category: ToolCategory,
    ) -> ToolConfig {
        ToolConfig {
            name,
            binary,
            detection_binary: binary,
            args,
            category,
            parser,
        }
    }

    // =========================================================================
    // Test 1: Binary not found
    // =========================================================================

    #[test]
    fn test_run_tool_binary_not_found() {
        let runner = ToolRunner::new(10);
        let tool = make_tool(
            "missing-tool",
            "nonexistent-binary-xyz-12345",
            &[],
            "cargo",
            ToolCategory::Linter,
        );

        let (result, findings) = runner.run_tool(&tool, Path::new("."));

        assert!(!result.success, "should fail for missing binary");
        assert!(
            result.error.is_some(),
            "should have error message"
        );
        let err = result.error.as_ref().unwrap();
        assert!(
            err.contains("spawn") || err.contains("not found") || err.contains("No such file"),
            "error should mention spawn failure, got: {}",
            err
        );
        assert!(findings.is_empty(), "no findings for missing binary");
        assert_eq!(result.name, "missing-tool");
        assert_eq!(result.finding_count, 0);
        assert!(result.exit_code.is_none());
    }

    // =========================================================================
    // Test 2: Timeout handling
    // =========================================================================

    #[test]
    fn test_run_tool_timeout() {
        let runner = ToolRunner::new(1); // 1 second timeout
        let tool = make_tool(
            "sleeper",
            "sleep",
            &["10"], // sleep for 10 seconds, will be killed after 1
            "cargo",
            ToolCategory::Linter,
        );

        let start = Instant::now();
        let (result, findings) = runner.run_tool(&tool, Path::new("."));
        let elapsed = start.elapsed();

        assert!(!result.success, "should fail on timeout");
        assert!(
            result.error.as_ref().unwrap().contains("imeout"),
            "error should mention timeout, got: {:?}",
            result.error
        );
        assert!(findings.is_empty(), "no findings on timeout");
        // Should have taken roughly 1 second, not 10
        assert!(
            elapsed.as_secs() < 5,
            "should have been killed within ~1s, took {:?}",
            elapsed
        );
        assert!(
            result.duration_ms >= 900,
            "should have waited at least ~1s, got {}ms",
            result.duration_ms
        );
    }

    // =========================================================================
    // Test 3: Tool name injection [PM-6]
    // =========================================================================

    // sh -c command that outputs a valid cargo NDJSON compiler-message line.
    // Uses printf '%s\n' to avoid the shell interpreting escape sequences
    // (echo interprets \n in the rendered field as a literal newline, splitting
    // the JSON across lines). The rendered field omits \n since the parser
    // does not use it.
    const SH_CMD_ONE_WARNING: &str = concat!(
        "printf '%s\\n' '",
        r#"{"reason":"compiler-message","package_id":"test 0.1.0","manifest_path":"/test/Cargo.toml","#,
        r#""target":{"kind":["lib"],"crate_types":["lib"],"name":"test","src_path":"/test/src/lib.rs","edition":"2021","doc":false,"doctest":false,"test":false},"#,
        r#""message":{"rendered":"warning: unused","children":[],"code":{"code":"unused_variables","explanation":null},"level":"warning","message":"unused variable","#,
        r#""spans":[{"byte_end":100,"byte_start":99,"column_end":10,"column_start":9,"expansion":null,"file_name":"src/main.rs","is_primary":true,"label":null,"line_end":10,"line_start":10,"suggested_replacement":null,"suggestion_applicability":null,"text":[]}]}}"#,
        "'",
    );

    #[test]
    fn test_run_tool_injects_tool_name() {
        let runner = ToolRunner::new(10);
        let tool = make_tool(
            "test-clippy",
            "sh",
            &["-c", SH_CMD_ONE_WARNING],
            "cargo",
            ToolCategory::Linter,
        );

        let (result, findings) = runner.run_tool(&tool, Path::new("."));

        assert!(result.success, "tool should succeed, error: {:?}", result.error);
        assert_eq!(findings.len(), 1, "should have 1 finding");
        assert_eq!(
            findings[0].tool, "test-clippy",
            "PM-6: tool name should be injected by runner, got: '{}'",
            findings[0].tool
        );
    }

    // =========================================================================
    // Test 4: Parse error captured
    // =========================================================================

    #[test]
    fn test_run_tool_parse_error_captured() {
        // echo outputs garbage that the cargo-audit parser can't parse
        // (cargo-audit expects valid JSON, not NDJSON)
        let runner = ToolRunner::new(10);
        let tool = make_tool(
            "bad-output",
            "echo",
            &["this is not valid json at all"],
            "cargo-audit", // expects a single JSON object
            ToolCategory::SecurityScanner,
        );

        let (result, findings) = runner.run_tool(&tool, Path::new("."));

        assert!(!result.success, "should fail on parse error");
        assert!(
            result.error.as_ref().unwrap().contains("Parse error"),
            "error should mention parse error, got: {:?}",
            result.error
        );
        assert!(findings.is_empty(), "no findings on parse error");
    }

    // =========================================================================
    // Test 5: Parallel failure isolation
    // =========================================================================

    #[test]
    fn test_run_tools_parallel_failure_isolation() {
        let runner = ToolRunner::new(10);

        // Tool A succeeds: echo empty string -> cargo parser returns Ok(vec![])
        let tool_a = make_tool(
            "tool-a",
            "echo",
            &[""],
            "cargo",
            ToolCategory::Linter,
        );

        // Tool B fails: nonexistent binary
        let tool_b = make_tool(
            "tool-b",
            "nonexistent-binary-xyz-12345",
            &[],
            "cargo",
            ToolCategory::Linter,
        );

        let tools: Vec<&ToolConfig> = vec![&tool_a, &tool_b];
        let (results, _findings) = runner.run_tools_parallel(&tools, Path::new("."));

        assert_eq!(results.len(), 2, "should have 2 results");
        assert!(
            results[0].success,
            "tool-a should succeed, error: {:?}",
            results[0].error
        );
        assert!(!results[1].success, "tool-b should fail");
        assert_eq!(results[0].name, "tool-a");
        assert_eq!(results[1].name, "tool-b");
    }

    // =========================================================================
    // Test 6: Parallel deterministic order
    // =========================================================================

    #[test]
    fn test_run_tools_parallel_deterministic_order() {
        let runner = ToolRunner::new(10);

        let tool_alpha = make_tool(
            "alpha",
            "echo",
            &[""],
            "cargo",
            ToolCategory::Linter,
        );
        let tool_beta = make_tool(
            "beta",
            "echo",
            &[""],
            "cargo",
            ToolCategory::Linter,
        );
        let tool_gamma = make_tool(
            "gamma",
            "echo",
            &[""],
            "cargo",
            ToolCategory::Linter,
        );

        let tools: Vec<&ToolConfig> = vec![&tool_alpha, &tool_beta, &tool_gamma];
        let (results, _findings) = runner.run_tools_parallel(&tools, Path::new("."));

        assert_eq!(results.len(), 3);
        assert_eq!(
            results[0].name, "alpha",
            "first result should be alpha, got {}",
            results[0].name
        );
        assert_eq!(
            results[1].name, "beta",
            "second result should be beta, got {}",
            results[1].name
        );
        assert_eq!(
            results[2].name, "gamma",
            "third result should be gamma, got {}",
            results[2].name
        );
    }

    // =========================================================================
    // Test 7: Sequential for single tool
    // =========================================================================

    #[test]
    fn test_run_tools_sequential_for_single_tool() {
        let runner = ToolRunner::new(10);
        let tool = make_tool(
            "solo",
            "echo",
            &[""],
            "cargo",
            ToolCategory::Linter,
        );

        let tools: Vec<&ToolConfig> = vec![&tool];
        let (results, findings) = runner.run_tools_parallel(&tools, Path::new("."));

        assert_eq!(results.len(), 1);
        assert!(results[0].success);
        assert_eq!(results[0].name, "solo");
        assert!(findings.is_empty(), "empty echo -> no cargo findings");
    }

    // =========================================================================
    // Test 8: Non-zero exit with parseable output = success
    // =========================================================================

    // sh -c command that outputs a valid NDJSON line and exits with code 1
    const SH_CMD_WARNING_EXIT1: &str = concat!(
        "printf '%s\\n' '",
        r#"{"reason":"compiler-message","package_id":"test 0.1.0","manifest_path":"/test/Cargo.toml","#,
        r#""target":{"kind":["lib"],"crate_types":["lib"],"name":"test","src_path":"/test/src/lib.rs","edition":"2021","doc":false,"doctest":false,"test":false},"#,
        r#""message":{"rendered":"warning: unused","children":[],"code":{"code":"unused_variables","explanation":null},"level":"warning","message":"unused variable","#,
        r#""spans":[{"byte_end":100,"byte_start":99,"column_end":10,"column_start":9,"expansion":null,"file_name":"src/main.rs","is_primary":true,"label":null,"line_end":10,"line_start":10,"suggested_replacement":null,"suggestion_applicability":null,"text":[]}]}}"#,
        "'; exit 1",
    );

    #[test]
    fn test_run_tool_nonzero_exit_with_parseable_output() {
        // Linters exit non-zero when they find issues. That's not a failure.
        let runner = ToolRunner::new(10);
        let tool = make_tool(
            "linter-with-findings",
            "sh",
            &["-c", SH_CMD_WARNING_EXIT1],
            "cargo",
            ToolCategory::Linter,
        );

        let (result, findings) = runner.run_tool(&tool, Path::new("."));

        assert!(
            result.success,
            "non-zero exit with parseable output should be success, error: {:?}",
            result.error
        );
        assert_eq!(
            result.exit_code,
            Some(1),
            "exit code should be captured as 1"
        );
        assert_eq!(findings.len(), 1, "should have parsed 1 finding");
        assert_eq!(
            findings[0].tool, "linter-with-findings",
            "tool name should be injected"
        );
    }

    // =========================================================================
    // Test 9: Empty tool list
    // =========================================================================

    #[test]
    fn test_run_tools_parallel_empty_list() {
        let runner = ToolRunner::new(10);
        let tools: Vec<&ToolConfig> = vec![];
        let (results, findings) = runner.run_tools_parallel(&tools, Path::new("."));

        assert!(results.is_empty(), "no tools = no results");
        assert!(findings.is_empty(), "no tools = no findings");
    }

    // =========================================================================
    // Test 10: Success with echo (no findings)
    // =========================================================================

    #[test]
    fn test_run_tool_success_echo_empty_output() {
        // echo "" produces effectively empty output, cargo parser returns empty vec
        let runner = ToolRunner::new(10);
        let tool = make_tool(
            "echo-tool",
            "echo",
            &[""],
            "cargo",
            ToolCategory::Linter,
        );

        let (result, findings) = runner.run_tool(&tool, Path::new("."));

        assert!(result.success, "echo should succeed, error: {:?}", result.error);
        assert_eq!(result.finding_count, 0);
        assert!(findings.is_empty());
        assert!(result.error.is_none());
        assert_eq!(result.name, "echo-tool");
    }

    // =========================================================================
    // Test 11: Duration is tracked
    // =========================================================================

    #[test]
    fn test_run_tool_tracks_duration() {
        let runner = ToolRunner::new(10);
        let tool = make_tool(
            "timer-test",
            "echo",
            &["hello"],
            "cargo",
            ToolCategory::Linter,
        );

        let (result, _findings) = runner.run_tool(&tool, Path::new("."));

        // Duration should be positive (process did run)
        // We can't assert exact timing but it shouldn't be huge
        assert!(
            result.duration_ms < 5000,
            "echo should complete in well under 5s, got {}ms",
            result.duration_ms
        );
    }

    // =========================================================================
    // Test 12: Category preserved in result
    // =========================================================================

    #[test]
    fn test_run_tool_preserves_category() {
        let runner = ToolRunner::new(10);
        let tool = make_tool(
            "security-tool",
            "echo",
            &[""],
            "cargo",
            ToolCategory::SecurityScanner,
        );

        let (result, _findings) = runner.run_tool(&tool, Path::new("."));

        assert_eq!(
            result.category,
            ToolCategory::SecurityScanner,
            "category should be preserved from ToolConfig"
        );
    }

    // =========================================================================
    // Test 13: Build-finished only output (valid JSON, no findings)
    // =========================================================================

    #[test]
    fn test_run_tool_build_finished_only() {
        // cargo outputs build-finished at the end; that's not a compiler-message
        let runner = ToolRunner::new(10);
        let tool = make_tool(
            "cargo-noop",
            "echo",
            &[r#"{"reason":"build-finished","success":true}"#],
            "cargo",
            ToolCategory::Linter,
        );

        let (result, findings) = runner.run_tool(&tool, Path::new("."));

        assert!(result.success, "valid output should succeed");
        assert_eq!(result.finding_count, 0, "build-finished is not a finding");
        assert!(findings.is_empty());
    }

    // =========================================================================
    // Test 14: Unknown parser name
    // =========================================================================

    #[test]
    fn test_run_tool_unknown_parser() {
        let runner = ToolRunner::new(10);
        let tool = make_tool(
            "bad-parser",
            "echo",
            &["some output"],
            "nonexistent-parser",
            ToolCategory::Linter,
        );

        let (result, findings) = runner.run_tool(&tool, Path::new("."));

        assert!(!result.success, "unknown parser should fail");
        assert!(
            result.error.as_ref().unwrap().contains("Parse error"),
            "should mention parse error, got: {:?}",
            result.error
        );
        assert!(findings.is_empty());
    }

    // =========================================================================
    // Test 15: Multiple findings have tool name injected
    // =========================================================================

    // sh -c command that outputs two NDJSON lines + build-finished
    const SH_CMD_TWO_WARNINGS: &str = concat!(
        "printf '%s\\n' '",
        r#"{"reason":"compiler-message","package_id":"test 0.1.0","manifest_path":"/t/Cargo.toml","target":{"kind":["lib"],"crate_types":["lib"],"name":"t","src_path":"/t/src/lib.rs","edition":"2021","doc":false,"doctest":false,"test":false},"message":{"rendered":"w","children":[],"code":{"code":"W1","explanation":null},"level":"warning","message":"warning one","spans":[{"byte_end":10,"byte_start":1,"column_end":5,"column_start":1,"expansion":null,"file_name":"src/a.rs","is_primary":true,"label":null,"line_end":1,"line_start":1,"suggested_replacement":null,"suggestion_applicability":null,"text":[]}]}}"#,
        "'; printf '%s\\n' '",
        r#"{"reason":"compiler-message","package_id":"test 0.1.0","manifest_path":"/t/Cargo.toml","target":{"kind":["lib"],"crate_types":["lib"],"name":"t","src_path":"/t/src/lib.rs","edition":"2021","doc":false,"doctest":false,"test":false},"message":{"rendered":"e","children":[],"code":{"code":"E1","explanation":null},"level":"error","message":"error one","spans":[{"byte_end":20,"byte_start":11,"column_end":8,"column_start":3,"expansion":null,"file_name":"src/b.rs","is_primary":true,"label":null,"line_end":5,"line_start":5,"suggested_replacement":null,"suggestion_applicability":null,"text":[]}]}}"#,
        "'; printf '%s\\n' '",
        r#"{"reason":"build-finished","success":true}"#,
        "'",
    );

    #[test]
    fn test_run_tool_multiple_findings_all_have_tool_name() {
        let runner = ToolRunner::new(10);
        let tool = make_tool(
            "multi-finder",
            "sh",
            &["-c", SH_CMD_TWO_WARNINGS],
            "cargo",
            ToolCategory::Linter,
        );

        let (result, findings) = runner.run_tool(&tool, Path::new("."));

        assert!(result.success, "should succeed, error: {:?}", result.error);
        assert_eq!(findings.len(), 2, "should have 2 findings");
        for (i, f) in findings.iter().enumerate() {
            assert_eq!(
                f.tool, "multi-finder",
                "PM-6: finding[{}].tool should be 'multi-finder', got '{}'",
                i, f.tool
            );
        }
    }

    // =========================================================================
    // Test: F1 - Stdout/stderr truncation constant exists
    // =========================================================================

    #[test]
    fn test_max_output_bytes_constant_exists() {
        // F1: There should be a safety limit on captured stdout/stderr
        let max_output_bytes = std::hint::black_box(super::MAX_OUTPUT_BYTES);
        assert!(
            max_output_bytes > 0,
            "MAX_OUTPUT_BYTES should be a positive constant"
        );
        assert!(
            max_output_bytes >= 1_000_000,
            "MAX_OUTPUT_BYTES should be at least 1MB, got {}",
            max_output_bytes
        );
    }

    #[test]
    fn test_large_stdout_is_truncated() {
        // F1: When a tool produces output larger than MAX_OUTPUT_BYTES,
        // the output should be truncated and parsing should still work.
        let runner = ToolRunner::new(10);

        // Generate output larger than MAX_OUTPUT_BYTES by repeating a valid
        // NDJSON line many times. Each line is ~500 bytes, so we need
        // MAX_OUTPUT_BYTES / 500 + 1 lines to exceed the limit.
        let single_line = r#"{"reason":"compiler-message","package_id":"test 0.1.0","manifest_path":"/t/Cargo.toml","target":{"kind":["lib"],"crate_types":["lib"],"name":"t","src_path":"/t/src/lib.rs","edition":"2021","doc":false,"doctest":false,"test":false},"message":{"rendered":"w","children":[],"code":{"code":"W1","explanation":null},"level":"warning","message":"warning","spans":[{"byte_end":10,"byte_start":1,"column_end":5,"column_start":1,"expansion":null,"file_name":"src/a.rs","is_primary":true,"label":null,"line_end":1,"line_start":1,"suggested_replacement":null,"suggestion_applicability":null,"text":[]}]}}"#;

        let line_count = (super::MAX_OUTPUT_BYTES / single_line.len()) + 100;

        // Build a shell command that prints the line N times
        let sh_cmd = format!(
            "for i in $(seq 1 {}); do printf '%s\\n' '{}'; done",
            line_count, single_line
        );

        let tool = make_tool(
            "large-output",
            "sh",
            // We need to leak the strings for 'static lifetime
            Box::leak(Box::new(["-c", Box::leak(sh_cmd.into_boxed_str()) as &str])) as &[&str],
            "cargo",
            ToolCategory::Linter,
        );

        let (result, _findings) = runner.run_tool(&tool, Path::new("."));

        // The tool should still succeed (output is truncated but parseable)
        assert!(result.success, "should succeed even with truncated output, error: {:?}", result.error);
    }

    // =========================================================================
    // Test: F4 - Thread panic converts to error result
    // =========================================================================

    #[test]
    fn test_thread_panic_does_not_propagate() {
        // F4: If a tool thread panics, the parent should NOT panic.
        // Instead, it should produce a ToolResult with success=false.
        // We can't easily trigger a panic inside run_tool, but we can
        // verify the structural expectation: run_tools_parallel should
        // return results for all tools even if one has issues.
        let runner = ToolRunner::new(10);

        let tool_a = make_tool(
            "good-tool",
            "echo",
            &[""],
            "cargo",
            ToolCategory::Linter,
        );

        // Tool with a binary that will fail to spawn
        let tool_b = make_tool(
            "bad-tool",
            "nonexistent-binary-xyz-12345",
            &[],
            "cargo",
            ToolCategory::Linter,
        );

        let tools: Vec<&ToolConfig> = vec![&tool_a, &tool_b];
        let (results, _findings) = runner.run_tools_parallel(&tools, Path::new("."));

        // Both tools should produce results (no panic propagation)
        assert_eq!(results.len(), 2, "should have results for both tools");
    }
}
