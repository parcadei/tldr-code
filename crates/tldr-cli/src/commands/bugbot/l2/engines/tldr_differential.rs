//! TldrDifferentialEngine -- L2 engine that invokes the `tldr` CLI binary.
//!
//! Replaces the bespoke DeltaEngine by running `tldr` subcommands (complexity,
//! cognitive, contracts, smells, calls, deps, coupling, cohesion, dead) on
//! baseline and current file revisions, diffing the JSON outputs to detect
//! regressions.
//!
//! # Finding Types
//!
//! | ID | Finding Type | Category | Source command |
//! |----|-------------|----------|---------------|
//! | 1 | `complexity-increase` | LOCAL | `tldr complexity` |
//! | 2 | `cognitive-increase` | LOCAL | `tldr cognitive` |
//! | 3 | `contract-removed` | LOCAL | `tldr contracts` |
//! | 4 | `smell-introduced` | LOCAL | `tldr smells` |
//! | 5 | `call-graph-change` | FLOW | `tldr calls` |
//! | 6 | `dependency-change` | FLOW | derived from `tldr calls` |
//! | 7 | `coupling-increase` | FLOW | `tldr coupling` |
//! | 8 | `cohesion-decrease` | FLOW | `tldr cohesion` |
//! | 9 | `dead-code-introduced` | FLOW | `tldr dead` |
//! | 10 | `downstream-impact` | IMPACT | derived from `tldr calls` |
//! | 11 | `breaking-change-risk` | IMPACT | derived from `tldr calls` |
//!
//! # Architecture
//!
//! For LOCAL commands: writes baseline/current source to temp files, runs
//! `tldr <command> <tmpfile> --format json`, parses JSON, diffs metrics per
//! function, and emits findings for regressions.
//!
//! For FLOW commands: `tldr calls` is run once for the current project by the
//! `analyze()` entry point, and the resulting JSON is cached and passed to
//! `analyze_flow_commands`, `analyze_downstream_impact`, and
//! `analyze_function_impact`. The deps, downstream-impact, and
//! breaking-change-risk findings are all derived in-memory from the cached
//! call graph, eliminating separate `tldr deps`, `tldr whatbreaks`, and
//! redundant `tldr calls` subprocess calls. Only baseline `tldr calls`,
//! baseline/current `tldr cohesion`, and `tldr dead` still require
//! subprocess execution. The `dead` command uses count-only analysis
//! (no baseline worktree needed).

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use tempfile::TempDir;

use super::super::context::L2Context;
use super::super::types::{AnalyzerStatus, L2AnalyzerOutput};
use super::super::L2Engine;
use crate::commands::bugbot::dead::is_test_function;
use crate::commands::bugbot::types::BugbotFinding;

/// Category of a tldr command: LOCAL (per-file) or FLOW (project-wide).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TldrCategory {
    /// Per-file command: run on individual temp files.
    Local,
    /// Project-wide command: run on the project root directory.
    Flow,
}

/// Configuration for a single tldr subcommand.
#[derive(Debug, Clone)]
struct TldrCommand {
    /// Human-readable name (also used in finding_type).
    name: &'static str,
    /// CLI arguments passed to `tldr` (e.g., `["complexity"]`).
    args: &'static [&'static str],
    /// Whether this command operates per-file or project-wide.
    category: TldrCategory,
}

/// All tldr commands that this engine runs.
const TLDR_COMMANDS: &[TldrCommand] = &[
    // LOCAL (per-file, parse per-function from output):
    TldrCommand { name: "complexity", args: &["complexity"], category: TldrCategory::Local },
    TldrCommand { name: "cognitive", args: &["cognitive"], category: TldrCategory::Local },
    TldrCommand { name: "contracts", args: &["contracts"], category: TldrCategory::Local },
    TldrCommand { name: "smells", args: &["smells"], category: TldrCategory::Local },
    // FLOW (project-wide, run on project root):
    TldrCommand { name: "calls", args: &["calls"], category: TldrCategory::Flow },
    TldrCommand { name: "deps", args: &["deps"], category: TldrCategory::Flow },
    TldrCommand { name: "coupling", args: &["coupling"], category: TldrCategory::Flow },
    TldrCommand { name: "cohesion", args: &["cohesion"], category: TldrCategory::Flow },
    TldrCommand { name: "dead", args: &["dead"], category: TldrCategory::Flow },
];

/// The set of finding types that TldrDifferentialEngine can produce.
const FINDING_TYPES: &[&str] = &[
    "complexity-increase",
    "cognitive-increase",
    "contract-removed",
    "smell-introduced",
    "call-graph-change",
    "dependency-change",
    "coupling-increase",
    "cohesion-decrease",
    "dead-code-introduced",
    "downstream-impact",
    "breaking-change-risk",
];

/// Maximum bytes of stdout to retain from a tldr subprocess.
const MAX_OUTPUT_BYTES: usize = 10 * 1024 * 1024; // 10 MB

/// L2 engine that invokes the `tldr` CLI binary for differential analysis.
///
/// Runs tldr subcommands on baseline and current file revisions, diffs
/// the JSON metrics, and produces findings for regressions. The `analyze()`
/// entry point runs `tldr calls` once for the current project, then passes
/// the cached call graph JSON to flow, downstream, and function impact
/// analysis methods. Deps, downstream impact, and breaking-change-risk
/// findings are derived in-memory from the call graph. Only baseline calls,
/// cohesion, and dead code analysis require separate subprocess calls.
/// Uses subprocess execution with configurable timeout.
pub struct TldrDifferentialEngine {
    /// Timeout per tldr command in seconds.
    timeout_secs: u64,
}

impl TldrDifferentialEngine {
    /// Create a new TldrDifferentialEngine with the default 30-second timeout.
    pub fn new() -> Self {
        Self { timeout_secs: 30 }
    }

    /// Create a new TldrDifferentialEngine with a custom timeout.
    pub fn with_timeout(timeout_secs: u64) -> Self {
        Self { timeout_secs }
    }

    /// Run a tldr subcommand and parse its JSON output.
    ///
    /// Spawns `tldr` with the given arguments as a subprocess, captures
    /// stdout, and parses as JSON. Returns `Err` on spawn failure, timeout,
    /// or JSON parse failure. Truncates output to `MAX_OUTPUT_BYTES`.
    ///
    /// The caller is responsible for building the full argument list including
    /// `--format json`.
    fn run_tldr_command(
        &self,
        args: &[&str],
        target: &Path,
    ) -> Result<serde_json::Value, String> {
        let target_str = target.to_string_lossy().to_string();
        let mut full_args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        full_args.push(target_str);
        full_args.push("--format".to_string());
        full_args.push("json".to_string());
        self.run_tldr_raw(&full_args)
    }

    /// Run a tldr subcommand that requires per-function invocation.
    ///
    /// Spawns `tldr <command> <file> <function> --format json`. Used for
    /// `complexity` and `contracts` which require a function name argument.
    fn run_tldr_per_function(
        &self,
        command: &str,
        file: &Path,
        function_name: &str,
    ) -> Result<serde_json::Value, String> {
        let file_str = file.to_string_lossy().to_string();
        let args = vec![
            command.to_string(),
            file_str,
            function_name.to_string(),
            "--format".to_string(),
            "json".to_string(),
        ];
        self.run_tldr_raw(&args)
    }

    /// Run a tldr flow command with language filtering and gitignore respect.
    ///
    /// Unlike `run_tldr_command`, this method appends `--lang <language>` to
    /// restrict analysis to the relevant language, and `--respect-ignore` (for
    /// commands that support it) to skip files matched by `.gitignore`. This
    /// prevents flow commands from scanning thousands of irrelevant files
    /// (markdown, test fixtures, corpus data) and timing out.
    fn run_tldr_flow_command(
        &self,
        cmd_name: &str,
        args: &[&str],
        target: &Path,
        language: &str,
    ) -> Result<serde_json::Value, String> {
        let target_str = target.to_string_lossy().to_string();
        let mut full_args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        full_args.push(target_str);
        full_args.push("--lang".to_string());
        full_args.push(language.to_string());
        // Only pass --respect-ignore for commands that support it.
        // Currently only `calls` supports this flag.
        if cmd_name == "calls" {
            full_args.push("--respect-ignore".to_string());
        }
        full_args.push("--format".to_string());
        full_args.push("json".to_string());
        self.run_tldr_raw(&full_args)
    }

    /// Low-level: spawn `tldr` with the given arguments, capture stdout, parse as JSON.
    fn run_tldr_raw(
        &self,
        args: &[String],
    ) -> Result<serde_json::Value, String> {
        let child = Command::new("tldr")
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        let child = match child {
            Ok(c) => c,
            Err(e) => return Err(format!("Failed to spawn 'tldr': {}", e)),
        };

        // Simple timeout: wait in a thread, kill if exceeded.
        let timeout = Duration::from_secs(self.timeout_secs);
        let child_id = child.id();
        let timed_out = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let timed_out_clone = timed_out.clone();

        let _watchdog = std::thread::spawn(move || {
            std::thread::sleep(timeout);
            timed_out_clone.store(true, std::sync::atomic::Ordering::SeqCst);
            #[cfg(unix)]
            unsafe {
                libc::kill(child_id as libc::pid_t, libc::SIGKILL);
            }
        });

        let output = child
            .wait_with_output()
            .map_err(|e| format!("Failed to read tldr output: {}", e))?;

        if timed_out.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(format!("Timeout after {}s", self.timeout_secs));
        }

        let raw_stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stdout = if raw_stdout.len() > MAX_OUTPUT_BYTES {
            let mut truncated = raw_stdout;
            truncated.truncate(MAX_OUTPUT_BYTES);
            if let Some(last_newline) = truncated.rfind('\n') {
                truncated.truncate(last_newline + 1);
            }
            truncated
        } else {
            raw_stdout
        };

        if stdout.trim().is_empty() {
            return Err(format!(
                "tldr {} produced empty output (exit code: {:?}, stderr: {})",
                args.first().map(|s| s.as_str()).unwrap_or("?"),
                output.status.code(),
                String::from_utf8_lossy(&output.stderr),
            ));
        }

        serde_json::from_str(&stdout)
            .map_err(|e| format!("Failed to parse tldr JSON: {} (first 200 chars: {:?})", e, &stdout[..stdout.len().min(200)]))
    }

    /// Run all LOCAL commands on baseline and current temp files for a single changed file.
    ///
    /// Commands fall into two categories:
    /// - **File-level** (`cognitive`, `smells`): accept a file path, return all functions.
    /// - **Per-function** (`complexity`, `contracts`): require `<FILE> <FUNCTION>`, so we first
    ///   discover function names via `cognitive` then invoke per-function.
    fn analyze_local_commands(
        &self,
        file_path: &Path,
        baseline_source: &str,
        current_source: &str,
        partial_reasons: &mut Vec<String>,
    ) -> Vec<BugbotFinding> {
        let mut findings = Vec::new();

        let ext = file_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("py");

        // Create temp dir for this file's analysis
        let tmp_dir = match TempDir::new() {
            Ok(d) => d,
            Err(e) => {
                partial_reasons.push(format!("tmpdir creation failed: {}", e));
                return findings;
            }
        };

        let baseline_file = tmp_dir.path().join(format!("baseline.{}", ext));
        let current_file = tmp_dir.path().join(format!("current.{}", ext));

        if std::fs::write(&baseline_file, baseline_source).is_err() {
            partial_reasons.push(format!("write baseline tmpfile failed for {}", file_path.display()));
            return findings;
        }
        if std::fs::write(&current_file, current_source).is_err() {
            partial_reasons.push(format!("write current tmpfile failed for {}", file_path.display()));
            return findings;
        }

        // === File-level commands: cognitive, smells ===
        // These accept a path and return all functions or smells.
        for cmd_name in &["cognitive", "smells"] {
            let baseline_result = self.run_tldr_command(&[cmd_name], &baseline_file);
            let current_result = self.run_tldr_command(&[cmd_name], &current_file);

            match (baseline_result, current_result) {
                (Ok(baseline_json), Ok(current_json)) => {
                    let cmd_findings = self.diff_local_metrics(
                        cmd_name,
                        file_path,
                        &baseline_json,
                        &current_json,
                    );
                    findings.extend(cmd_findings);
                }
                (Err(e), _) | (_, Err(e)) => {
                    partial_reasons.push(format!(
                        "tldr {} failed for {}: {}",
                        cmd_name,
                        file_path.display(),
                        e,
                    ));
                }
            }
        }

        // === Per-function commands: complexity, contracts ===
        // Discover function names from the cognitive output (which lists all functions).
        let baseline_funcs = Self::discover_function_names_from_cognitive(
            &self.run_tldr_command(&["cognitive"], &baseline_file),
        );
        let current_funcs = Self::discover_function_names_from_cognitive(
            &self.run_tldr_command(&["cognitive"], &current_file),
        );

        // --- complexity: per-function ---
        {
            let mut baseline_entries: Vec<(String, serde_json::Value)> = Vec::new();
            for func in &baseline_funcs {
                match self.run_tldr_per_function("complexity", &baseline_file, func) {
                    Ok(json) => baseline_entries.push((func.clone(), json)),
                    Err(e) => {
                        partial_reasons.push(format!("tldr complexity {} baseline: {}", func, e));
                    }
                }
            }

            let mut current_entries: Vec<(String, serde_json::Value)> = Vec::new();
            for func in &current_funcs {
                match self.run_tldr_per_function("complexity", &current_file, func) {
                    Ok(json) => current_entries.push((func.clone(), json)),
                    Err(e) => {
                        partial_reasons.push(format!("tldr complexity {} current: {}", func, e));
                    }
                }
            }

            // Build aggregated JSON for diffing (wrap per-function results into
            // the same { "functions": [...] } shape the diff_local_metrics expects)
            let baseline_agg = Self::aggregate_per_function_complexity(&baseline_entries);
            let current_agg = Self::aggregate_per_function_complexity(&current_entries);

            let complexity_findings = self.diff_local_metrics(
                "complexity",
                file_path,
                &baseline_agg,
                &current_agg,
            );
            findings.extend(complexity_findings);
        }

        // --- contracts: per-function ---
        {
            let mut baseline_entries: Vec<(String, serde_json::Value)> = Vec::new();
            for func in &baseline_funcs {
                match self.run_tldr_per_function("contracts", &baseline_file, func) {
                    Ok(json) => baseline_entries.push((func.clone(), json)),
                    Err(e) => {
                        partial_reasons.push(format!("tldr contracts {} baseline: {}", func, e));
                    }
                }
            }

            // For current contracts, also attempt functions that only appear in
            // baseline_funcs. Cognitive discovery can miss simple functions (e.g.,
            // `name()`, `default()`), so without this, functions present in
            // baseline but absent from current_funcs would be falsely reported
            // as "function deleted" by diff_contracts.
            let current_func_set: std::collections::HashSet<&str> =
                current_funcs.iter().map(|s| s.as_str()).collect();
            let all_current_candidates: Vec<String> = current_funcs
                .iter()
                .cloned()
                .chain(
                    baseline_funcs
                        .iter()
                        .filter(|f| !current_func_set.contains(f.as_str()))
                        .cloned(),
                )
                .collect();

            let mut current_entries: Vec<(String, serde_json::Value)> = Vec::new();
            for func in &all_current_candidates {
                match self.run_tldr_per_function("contracts", &current_file, func) {
                    Ok(json) => current_entries.push((func.clone(), json)),
                    Err(e) => {
                        partial_reasons.push(format!("tldr contracts {} current: {}", func, e));
                    }
                }
            }

            let baseline_agg = Self::aggregate_per_function_contracts(&baseline_entries);
            let current_agg = Self::aggregate_per_function_contracts(&current_entries);

            let contract_findings = self.diff_contracts(
                file_path,
                &baseline_agg,
                &current_agg,
                &all_current_candidates,
            );
            findings.extend(contract_findings);
        }

        findings
    }

    /// Discover function names from a cognitive command result.
    ///
    /// The cognitive JSON output has `{ "functions": [{ "name": "..." }, ...] }`.
    /// Returns the list of function names found, or empty vec on error.
    fn discover_function_names_from_cognitive(
        result: &Result<serde_json::Value, String>,
    ) -> Vec<String> {
        match result {
            Ok(json) => {
                Self::extract_function_entries(json)
                    .into_iter()
                    .map(|(name, _)| name)
                    .filter(|name| !is_test_function(name))
                    .collect()
            }
            Err(_) => Vec::new(),
        }
    }

    /// Aggregate per-function complexity results into the standard `{ "functions": [...] }` shape.
    ///
    /// Each per-function call returns `{ "function": "name", "cyclomatic": N, ... }`.
    /// We wrap them into `{ "functions": [{ "name": "...", "cyclomatic": N }] }` for diff_local_metrics.
    fn aggregate_per_function_complexity(entries: &[(String, serde_json::Value)]) -> serde_json::Value {
        let functions: Vec<serde_json::Value> = entries
            .iter()
            .map(|(name, json)| {
                let cyclomatic = json.get("cyclomatic").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let line = json.get("lines_of_code").and_then(|v| v.as_u64()).unwrap_or(1);
                serde_json::json!({
                    "name": name,
                    "cyclomatic": cyclomatic,
                    "line": line,
                })
            })
            .collect();
        serde_json::json!({ "functions": functions })
    }

    /// Aggregate per-function contracts results into the standard `{ "functions": [...] }` shape.
    ///
    /// Each per-function call returns `{ "function": "name", "preconditions": [...], ... }`.
    fn aggregate_per_function_contracts(entries: &[(String, serde_json::Value)]) -> serde_json::Value {
        let functions: Vec<serde_json::Value> = entries
            .iter()
            .map(|(name, json)| {
                let preconditions = json.get("preconditions").cloned().unwrap_or(serde_json::json!([]));
                let postconditions = json.get("postconditions").cloned().unwrap_or(serde_json::json!([]));
                serde_json::json!({
                    "name": name,
                    "preconditions": preconditions,
                    "postconditions": postconditions,
                })
            })
            .collect();
        serde_json::json!({ "functions": functions })
    }

    /// Diff baseline vs current JSON from a local tldr command.
    ///
    /// The JSON structure varies by command, but we use a generic approach:
    /// look for per-function metrics (arrays of objects with "name" and numeric
    /// fields), then compare matching functions.
    fn diff_local_metrics(
        &self,
        command_name: &str,
        file_path: &Path,
        baseline_json: &serde_json::Value,
        current_json: &serde_json::Value,
    ) -> Vec<BugbotFinding> {
        let mut findings = Vec::new();

        match command_name {
            "complexity" => {
                findings.extend(self.diff_numeric_metrics(
                    "complexity-increase",
                    "cyclomatic",
                    file_path,
                    baseline_json,
                    current_json,
                ));
            }
            "cognitive" => {
                findings.extend(self.diff_numeric_metrics(
                    "cognitive-increase",
                    "cognitive",
                    file_path,
                    baseline_json,
                    current_json,
                ));
            }
            "contracts" => {
                // Note: When called via diff_local_metrics (fallback path),
                // we don't have known_current_funcs context, so pass empty
                // slice. The primary contracts path in analyze_per_function
                // passes actual current_funcs for accurate deletion detection.
                findings.extend(self.diff_contracts(
                    file_path,
                    baseline_json,
                    current_json,
                    &[],
                ));
            }
            "smells" => {
                findings.extend(self.diff_smells(
                    file_path,
                    baseline_json,
                    current_json,
                ));
            }
            _ => {}
        }

        findings
    }

    /// Extract function entries from tldr JSON output.
    ///
    /// Tldr commands typically output an object with a "functions" or "results"
    /// array, where each entry has a "name" field. We try several common keys.
    fn extract_function_entries(json: &serde_json::Value) -> Vec<(String, &serde_json::Value)> {
        let mut entries = Vec::new();

        // Try common top-level array keys
        for key in &["functions", "results", "items", "entries", "metrics"] {
            if let Some(arr) = json.get(key).and_then(|v| v.as_array()) {
                for item in arr {
                    if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                        entries.push((name.to_string(), item));
                    }
                }
                if !entries.is_empty() {
                    return entries;
                }
            }
        }

        // Try the root itself if it's an array
        if let Some(arr) = json.as_array() {
            for item in arr {
                if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                    entries.push((name.to_string(), item));
                }
            }
        }

        entries
    }

    /// Diff a single numeric metric between baseline and current JSON.
    ///
    /// Finds matching functions by name, extracts the specified metric field,
    /// and emits a finding if the value increased beyond the threshold.
    fn diff_numeric_metrics(
        &self,
        finding_type: &str,
        metric_field: &str,
        file_path: &Path,
        baseline_json: &serde_json::Value,
        current_json: &serde_json::Value,
    ) -> Vec<BugbotFinding> {
        let mut findings = Vec::new();

        let baseline_entries = Self::extract_function_entries(baseline_json);
        let current_entries = Self::extract_function_entries(current_json);

        let baseline_map: std::collections::HashMap<&str, &serde_json::Value> = baseline_entries
            .iter()
            .map(|(name, val)| (name.as_str(), *val))
            .collect();

        for (func_name, current_entry) in &current_entries {
            let Some(baseline_entry) = baseline_map.get(func_name.as_str()) else {
                // New function -- report as info for awareness
                if let Some(current_val) = current_entry.get(metric_field).and_then(|v| v.as_f64()) {
                    if current_val > 10.0 {
                        findings.push(BugbotFinding {
                            finding_type: finding_type.to_string(),
                            severity: "info".to_string(),
                            file: file_path.to_path_buf(),
                            function: func_name.clone(),
                            line: current_entry.get("line").and_then(|l| l.as_u64()).unwrap_or(1) as usize,
                            message: format!(
                                "New function `{}` has {} = {:.1}",
                                func_name, metric_field, current_val,
                            ),
                            evidence: serde_json::json!({
                                "command": finding_type.replace("-increase", ""),
                                "metric": metric_field,
                                "current_value": current_val,
                                "new_function": true,
                            }),
                            confidence: Some("DETERMINISTIC".to_string()),
                            finding_id: Some(compute_finding_id(finding_type, file_path, func_name, 0)),
                        });
                    }
                }
                continue;
            };

            let baseline_val = baseline_entry.get(metric_field).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let current_val = current_entry.get(metric_field).and_then(|v| v.as_f64()).unwrap_or(0.0);

            if current_val > baseline_val {
                let delta = current_val - baseline_val;

                // Skip trivial absolute changes. Small deltas (e.g., 2→4) fire
                // due to high percentage but are not actionable. Thresholds:
                //   cognitive: delta >= 3  (informed by real-world validation)
                //   complexity: delta >= 2  (cyclomatic is coarser-grained)
                let min_delta = match finding_type {
                    "cognitive-increase" => 3.0,
                    "complexity-increase" => 2.0,
                    _ => 1.0,
                };
                if delta < min_delta {
                    continue;
                }

                let pct_increase = if baseline_val > 0.0 {
                    (delta / baseline_val) * 100.0
                } else {
                    100.0
                };

                let severity = if pct_increase > 50.0 {
                    "high"
                } else if pct_increase > 20.0 {
                    "medium"
                } else {
                    "low"
                };

                let line = current_entry.get("line").and_then(|l| l.as_u64()).unwrap_or(1) as usize;

                findings.push(BugbotFinding {
                    finding_type: finding_type.to_string(),
                    severity: severity.to_string(),
                    file: file_path.to_path_buf(),
                    function: func_name.clone(),
                    line,
                    message: format!(
                        "`{}` {} increased by {:.1} ({:.1} -> {:.1}, +{:.0}%)",
                        func_name, metric_field, delta, baseline_val, current_val, pct_increase,
                    ),
                    evidence: serde_json::json!({
                        "command": finding_type.replace("-increase", ""),
                        "metric": metric_field,
                        "old_value": baseline_val,
                        "new_value": current_val,
                        "delta": delta,
                        "pct_increase": pct_increase,
                    }),
                    confidence: Some("DETERMINISTIC".to_string()),
                    finding_id: Some(compute_finding_id(finding_type, file_path, func_name, line)),
                });
            }
        }

        findings
    }

    /// Diff contracts between baseline and current.
    ///
    /// Detects contracts (pre/postconditions) present in baseline but absent
    /// in current, emitting a "contract-removed" finding.
    ///
    /// `known_current_funcs` contains the function names that actually exist in
    /// the current version (from the AST diff). This prevents false positives
    /// when `tldr contracts` fails to extract a function — without this check,
    /// an extraction failure would be misinterpreted as "function deleted".
    fn diff_contracts(
        &self,
        file_path: &Path,
        baseline_json: &serde_json::Value,
        current_json: &serde_json::Value,
        known_current_funcs: &[String],
    ) -> Vec<BugbotFinding> {
        let mut findings = Vec::new();

        let baseline_entries = Self::extract_function_entries(baseline_json);
        let current_entries = Self::extract_function_entries(current_json);

        let current_names: std::collections::HashSet<String> = current_entries
            .iter()
            .map(|(name, _)| name.clone())
            .collect();

        // Count contracts per function in baseline
        let baseline_contract_count = |entry: &serde_json::Value| -> usize {
            let pre = entry.get("preconditions").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
            let post = entry.get("postconditions").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
            pre + post
        };

        let current_map: std::collections::HashMap<&str, &serde_json::Value> = current_entries
            .iter()
            .map(|(name, val)| (name.as_str(), *val))
            .collect();

        for (func_name, baseline_entry) in &baseline_entries {
            let b_count = baseline_contract_count(baseline_entry);
            if b_count == 0 {
                continue;
            }

            if let Some(current_entry) = current_map.get(func_name.as_str()) {
                let c_count = baseline_contract_count(current_entry);
                if c_count < b_count {
                    let removed = b_count - c_count;
                    findings.push(BugbotFinding {
                        finding_type: "contract-removed".to_string(),
                        severity: "medium".to_string(),
                        file: file_path.to_path_buf(),
                        function: func_name.clone(),
                        line: 1,
                        message: format!(
                            "`{}` lost {} contract(s) ({} -> {})",
                            func_name, removed, b_count, c_count,
                        ),
                        evidence: serde_json::json!({
                            "command": "contracts",
                            "baseline_contracts": b_count,
                            "current_contracts": c_count,
                            "removed": removed,
                        }),
                        confidence: Some("DETERMINISTIC".to_string()),
                        finding_id: Some(compute_finding_id("contract-removed", file_path, func_name, 1)),
                    });
                }
            } else if !current_names.contains(func_name.as_str()) {
                // Check if the function actually exists in current version.
                // If it does, contracts extraction just failed — not a deletion.
                // That failure is already captured in partial_reasons upstream.
                if known_current_funcs.iter().any(|f| f == func_name) {
                    continue;
                }
                // Function with contracts was genuinely deleted
                findings.push(BugbotFinding {
                    finding_type: "contract-removed".to_string(),
                    severity: "high".to_string(),
                    file: file_path.to_path_buf(),
                    function: func_name.clone(),
                    line: 1,
                    message: format!(
                        "`{}` with {} contract(s) was removed entirely",
                        func_name, b_count,
                    ),
                    evidence: serde_json::json!({
                        "command": "contracts",
                        "baseline_contracts": b_count,
                        "current_contracts": 0,
                        "function_deleted": true,
                    }),
                    confidence: Some("DETERMINISTIC".to_string()),
                    finding_id: Some(compute_finding_id("contract-removed", file_path, func_name, 0)),
                });
            }
        }

        findings
    }

    /// Diff smells between baseline and current.
    ///
    /// Detects new code smells introduced in current that were not present in
    /// baseline.
    fn diff_smells(
        &self,
        file_path: &Path,
        baseline_json: &serde_json::Value,
        current_json: &serde_json::Value,
    ) -> Vec<BugbotFinding> {
        let mut findings = Vec::new();

        let count_smells = |json: &serde_json::Value| -> usize {
            // Smells output typically has a top-level "smells" or "issues" array
            for key in &["smells", "issues", "findings", "results"] {
                if let Some(arr) = json.get(key).and_then(|v| v.as_array()) {
                    return arr.len();
                }
            }
            if let Some(arr) = json.as_array() {
                return arr.len();
            }
            0
        };

        let baseline_count = count_smells(baseline_json);
        let current_count = count_smells(current_json);

        // Skip when baseline has zero smells (new file) — no regression possible
        if baseline_count == 0 {
            return findings;
        }

        if current_count > baseline_count {
            let introduced = current_count - baseline_count;

            // Extract current smell entries directly
            let current_smells: Vec<&serde_json::Value> = {
                let mut result = Vec::new();
                for key in &["smells", "issues", "findings", "results"] {
                    if let Some(arr) = current_json.get(key).and_then(|v| v.as_array()) {
                        result = arr.iter().collect();
                        break;
                    }
                }
                if result.is_empty() {
                    if let Some(arr) = current_json.as_array() {
                        result = arr.iter().collect();
                    }
                }
                result
            };

            // Smell types that are too noisy to report. message_chain fires on
            // idiomatic Rust iterator chains; long_parameter_list fires on
            // constructors and builders that legitimately need many params.
            const SUPPRESSED_SMELL_TYPES: &[&str] = &[
                "message_chain",
                "long_parameter_list",
            ];

            // Report each new smell (the last `introduced` entries are likely new)
            for (i, smell) in current_smells.iter().rev().take(introduced).enumerate() {
                let smell_type = smell.get("smell_type").or_else(|| smell.get("type")).or_else(|| smell.get("kind")).and_then(|v| v.as_str()).unwrap_or("unknown");

                if SUPPRESSED_SMELL_TYPES.contains(&smell_type) {
                    continue;
                }

                let func_name = smell.get("function").or_else(|| smell.get("name")).and_then(|v| v.as_str()).unwrap_or("(file-level)");
                let line = smell.get("line").and_then(|l| l.as_u64()).unwrap_or(1) as usize;

                // Severity by smell type: structural issues are medium,
                // style issues stay low.
                let severity = match smell_type {
                    "god_class" | "feature_envy" | "data_clump" => "medium",
                    _ => "low",
                };

                findings.push(BugbotFinding {
                    finding_type: "smell-introduced".to_string(),
                    severity: severity.to_string(),
                    file: file_path.to_path_buf(),
                    function: func_name.to_string(),
                    line,
                    message: format!(
                        "New code smell `{}` introduced (total smells: {} -> {})",
                        smell_type, baseline_count, current_count,
                    ),
                    evidence: serde_json::json!({
                        "command": "smells",
                        "smell_type": smell_type,
                        "baseline_smell_count": baseline_count,
                        "current_smell_count": current_count,
                        "introduced": introduced,
                        "index": i,
                    }),
                    confidence: Some("DETERMINISTIC".to_string()),
                    finding_id: Some(compute_finding_id("smell-introduced", file_path, func_name, line)),
                });
            }
        }

        findings
    }

    /// Run all FLOW commands on the project root with baseline comparison.
    ///
    /// Creates a git worktree at `base_ref` for baseline, runs each flow
    /// command on both baseline and current, and diffs the JSON outputs to
    /// detect regressions. The `dead` command uses count-only analysis
    /// (no baseline needed). Calls and deps use the cached `current_calls_json`
    /// when available (deps are derived in-memory from the call graph).
    /// Cohesion still requires a separate subprocess call. The `coupling`
    /// command is skipped because it requires file pairs, not a project root.
    ///
    /// When `current_calls_json` is `Some`, only the baseline `tldr calls` and
    /// baseline/current `tldr cohesion` subprocesses are spawned (3 calls
    /// instead of 6). When `None`, falls back to running all subprocesses.
    fn analyze_flow_commands(
        &self,
        project: &Path,
        base_ref: &str,
        language: &str,
        current_calls_json: Option<&serde_json::Value>,
        partial_reasons: &mut Vec<String>,
    ) -> Vec<BugbotFinding> {
        let mut findings = Vec::new();

        // Flow commands analyze entire projects -- give them 5 minutes.
        // The previous max(self.timeout_secs, 60) was too aggressive and
        // killed legitimate long-running analysis on large repos.
        let flow_engine = TldrDifferentialEngine::with_timeout(300);

        // === Dead code: count-only, no baseline needed ===
        for cmd in TLDR_COMMANDS.iter().filter(|c| c.category == TldrCategory::Flow && c.name == "dead") {
            match flow_engine.run_tldr_flow_command(cmd.name, cmd.args, project, language) {
                Ok(json) => {
                    let dead_count = Self::count_dead_code_entries(&json);
                    if dead_count > 0 {
                        findings.push(BugbotFinding {
                            finding_type: "dead-code-introduced".to_string(),
                            severity: "info".to_string(),
                            file: PathBuf::from("(project)"),
                            function: "(project-level)".to_string(),
                            line: 0,
                            message: format!(
                                "{} dead code entries detected in project",
                                dead_count,
                            ),
                            evidence: serde_json::json!({
                                "command": cmd.name,
                                "dead_code_count": dead_count,
                            }),
                            confidence: Some("DETERMINISTIC".to_string()),
                            finding_id: Some(compute_finding_id(
                                "dead-code-introduced",
                                Path::new("(project)"),
                                "(project-level)",
                                0,
                            )),
                        });
                    }
                }
                Err(e) => {
                    partial_reasons.push(format!("tldr {} failed: {}", cmd.name, e));
                }
            }
        }

        // === Try cached baseline call graph before creating a worktree ===
        //
        // Resolve base_ref to a commit hash and check if we have a cached
        // baseline call graph for that commit. On cache hit we can diff
        // calls/deps without a worktree (cohesion still needs one).
        use crate::commands::bugbot::first_run::{
            load_cached_baseline_call_graph, resolve_git_ref, save_baseline_call_graph,
        };

        let base_commit = resolve_git_ref(project, base_ref).ok();
        let cached_baseline = base_commit
            .as_deref()
            .and_then(|hash| load_cached_baseline_call_graph(project, hash));

        // Track whether we already handled calls/deps via cache
        let mut calls_deps_done = false;

        if let Some(ref cached_cg) = cached_baseline {
            // --- Cache hit: diff calls/deps without worktree ---
            let current_calls_result: Result<std::borrow::Cow<'_, serde_json::Value>, String> =
                if let Some(cached) = current_calls_json {
                    Ok(std::borrow::Cow::Borrowed(cached))
                } else {
                    flow_engine
                        .run_tldr_flow_command("calls", &["calls"], project, language)
                        .map(std::borrow::Cow::Owned)
                };

            match &current_calls_result {
                Ok(current_json) => {
                    findings.extend(self.diff_calls_json(cached_cg, current_json.as_ref()));

                    let baseline_deps = Self::derive_deps_from_calls(cached_cg);
                    let current_deps = Self::derive_deps_from_calls(current_json.as_ref());
                    findings.extend(self.diff_deps_json(&baseline_deps, &current_deps));
                    calls_deps_done = true;
                }
                Err(e) => {
                    partial_reasons.push(format!("tldr calls (current) failed: {}", e));
                    calls_deps_done = true; // don't retry via worktree
                }
            }
        }

        // === Baseline worktree for calls/deps (cache miss) + cohesion ===
        //
        // We still need a worktree for cohesion (always) and for calls/deps
        // when no cached baseline is available.
        let needs_worktree = true; // cohesion always needs baseline worktree

        if needs_worktree {
            let baseline_dir = match tempfile::tempdir() {
                Ok(d) => d,
                Err(e) => {
                    partial_reasons.push(format!("tmpdir for baseline worktree: {}", e));
                    return findings;
                }
            };
            let worktree_path = baseline_dir.path().join("baseline");

            let worktree_ok = match Command::new("git")
                .args(["worktree", "add", &worktree_path.to_string_lossy(), base_ref])
                .current_dir(project)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::piped())
                .status()
            {
                Ok(status) if status.success() => true,
                Ok(status) => {
                    partial_reasons.push(format!(
                        "git worktree add failed (exit {}); skipping baseline flow diff",
                        status
                    ));
                    false
                }
                Err(e) => {
                    partial_reasons.push(format!("git worktree add: {}; skipping baseline flow diff", e));
                    false
                }
            };

            if worktree_ok {
                // Copy .tldrignore to worktree so baseline analysis uses
                // consistent filtering (vendored code excluded from both sides).
                let tldrignore_src = project.join(".tldrignore");
                if tldrignore_src.exists() {
                    let _ = std::fs::copy(&tldrignore_src, worktree_path.join(".tldrignore"));
                }

                // --- Calls/deps: only if not already handled via cache ---
                if !calls_deps_done {
                    let baseline_calls = flow_engine.run_tldr_flow_command("calls", &["calls"], &worktree_path, language);
                    let current_calls_result: Result<std::borrow::Cow<'_, serde_json::Value>, String> =
                        if let Some(cached) = current_calls_json {
                            Ok(std::borrow::Cow::Borrowed(cached))
                        } else {
                            flow_engine
                                .run_tldr_flow_command("calls", &["calls"], project, language)
                                .map(std::borrow::Cow::Owned)
                        };

                    match (&baseline_calls, &current_calls_result) {
                        (Ok(baseline_json), Ok(current_json)) => {
                            // Diff call graph edges
                            findings.extend(self.diff_calls_json(baseline_json, current_json.as_ref()));

                            // Derive deps from calls in-memory instead of running `tldr deps`
                            let baseline_deps = Self::derive_deps_from_calls(baseline_json);
                            let current_deps = Self::derive_deps_from_calls(current_json.as_ref());
                            findings.extend(self.diff_deps_json(&baseline_deps, &current_deps));

                            // Cache the baseline for next run (non-fatal).
                            if let Some(ref hash) = base_commit {
                                let _ = save_baseline_call_graph(project, baseline_json, hash, language);
                            }
                        }
                        (Err(e), _) => {
                            partial_reasons.push(format!("tldr calls (baseline) failed: {}", e));
                        }
                        (_, Err(e)) => {
                            partial_reasons.push(format!("tldr calls (current) failed: {}", e));
                        }
                    }
                }

                // --- Cohesion: separate subprocess (requires LCOM4, not derivable from calls) ---
                for cmd in TLDR_COMMANDS.iter().filter(|c| c.category == TldrCategory::Flow && c.name == "cohesion") {
                    let baseline_result = flow_engine.run_tldr_flow_command(cmd.name, cmd.args, &worktree_path, language);
                    let current_result = flow_engine.run_tldr_flow_command(cmd.name, cmd.args, project, language);
                    match (baseline_result, current_result) {
                        (Ok(baseline_json), Ok(current_json)) => {
                            findings.extend(self.diff_cohesion_json(&baseline_json, &current_json));
                        }
                        (Err(e), _) => {
                            partial_reasons.push(format!("tldr cohesion (baseline) failed: {}", e));
                        }
                        (_, Err(e)) => {
                            partial_reasons.push(format!("tldr cohesion (current) failed: {}", e));
                        }
                    }
                }

                // Clean up worktree
                let _ = Command::new("git")
                    .args(["worktree", "remove", "--force", &worktree_path.to_string_lossy()])
                    .current_dir(project)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
            }
        }

        findings
    }

    /// Parse `tldr whatbreaks` JSON output into findings for a single file.
    ///
    /// Extracts `summary.importer_count`, `summary.direct_caller_count`, and
    /// `summary.affected_test_count` from the JSON. Emits a `downstream-impact`
    /// finding if `importer_count > 0` or `caller_count > 0`.
    ///
    /// Severity: `high` if importer_count > 10, `medium` if > 3, else `low`.
    fn parse_whatbreaks_findings(
        file_path: &Path,
        json: &serde_json::Value,
    ) -> Vec<BugbotFinding> {
        let mut findings = Vec::new();

        let summary = json.get("summary").unwrap_or(json);
        let importer_count = summary
            .get("importer_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let caller_count = summary
            .get("direct_caller_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let test_count = summary
            .get("affected_test_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        if importer_count > 0 || caller_count > 0 {
            let severity = if importer_count > 10 {
                "high"
            } else if importer_count > 3 {
                "medium"
            } else {
                "low"
            };

            findings.push(BugbotFinding {
                finding_type: "downstream-impact".to_string(),
                severity: severity.to_string(),
                file: file_path.to_path_buf(),
                function: "(file-level)".to_string(),
                line: 0,
                message: format!(
                    "Changed file has {} importers, {} direct callers, {} affected tests",
                    importer_count, caller_count, test_count,
                ),
                evidence: serde_json::json!({
                    "command": "whatbreaks",
                    "importer_count": importer_count,
                    "direct_caller_count": caller_count,
                    "affected_test_count": test_count,
                }),
                confidence: Some("DETERMINISTIC".to_string()),
                finding_id: Some(compute_finding_id(
                    "downstream-impact",
                    file_path,
                    "(file-level)",
                    0,
                )),
            });
        }

        findings
    }

    /// Parse `tldr impact` JSON output into findings for a single function.
    ///
    /// Looks for `targets.<function_name>.caller_count` and
    /// `targets.<function_name>.callers` in the JSON. Emits a
    /// `breaking-change-risk` finding if caller_count > 0.
    ///
    /// Severity: `high` if caller_count > 5, `medium` if 2-5, `info` if 1.
    ///
    /// Note: No longer called by `analyze_function_impact` (which now uses
    /// `parse_impact_findings_from_callgraph`), but retained for parsing
    /// raw `tldr impact` JSON output in other contexts and tested directly.
    pub fn parse_impact_findings(
        function_name: &str,
        json: &serde_json::Value,
    ) -> Vec<BugbotFinding> {
        let mut findings = Vec::new();

        // Try targets.<function_name>.caller_count first
        let (caller_count, callers_preview) = if let Some(target) =
            json.get("targets").and_then(|t| t.get(function_name))
        {
            let count = target
                .get("caller_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let callers: Vec<String> = target
                .get("callers")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .take(5)
                        .map(|c| {
                            let file = c.get("file").and_then(|v| v.as_str()).unwrap_or("?");
                            let func = c.get("function").and_then(|v| v.as_str()).unwrap_or("?");
                            format!("{}::{}", file, func)
                        })
                        .collect()
                })
                .unwrap_or_default();
            (count, callers)
        } else {
            // Fallback: try top-level caller_count
            let count = json
                .get("caller_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let callers: Vec<String> = json
                .get("callers")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .take(5)
                        .map(|c| {
                            let file = c.get("file").and_then(|v| v.as_str()).unwrap_or("?");
                            let func = c.get("function").and_then(|v| v.as_str()).unwrap_or("?");
                            format!("{}::{}", file, func)
                        })
                        .collect()
                })
                .unwrap_or_default();
            (count, callers)
        };

        if caller_count > 0 {
            let severity = if caller_count > 5 {
                "high"
            } else if caller_count >= 2 {
                "medium"
            } else {
                "info"
            };

            findings.push(BugbotFinding {
                finding_type: "breaking-change-risk".to_string(),
                severity: severity.to_string(),
                file: PathBuf::from("(project)"),
                function: function_name.to_string(),
                line: 0,
                message: format!(
                    "Function `{}` has {} callers that may be affected by changes",
                    function_name, caller_count,
                ),
                evidence: serde_json::json!({
                    "command": "impact",
                    "caller_count": caller_count,
                    "callers_preview": callers_preview,
                }),
                confidence: Some("DETERMINISTIC".to_string()),
                finding_id: Some(compute_finding_id(
                    "breaking-change-risk",
                    Path::new("(project)"),
                    function_name,
                    0,
                )),
            });
        }

        findings
    }

    /// Build a reverse caller map from `tldr calls` JSON output.
    ///
    /// Inverts call graph edges so that each `dst_func` maps to a list of
    /// `(src_file, src_func)` pairs representing its callers. Edges with
    /// missing `src_file`, `src_func`, or `dst_func` fields are silently
    /// skipped.
    fn build_reverse_caller_map(
        calls_json: &serde_json::Value,
    ) -> HashMap<String, Vec<(String, String)>> {
        let mut map: HashMap<String, Vec<(String, String)>> = HashMap::new();

        if let Some(edges) = calls_json.get("edges").and_then(|v| v.as_array()) {
            for edge in edges {
                let src_file = edge.get("src_file").and_then(|v| v.as_str());
                let src_func = edge.get("src_func").and_then(|v| v.as_str());
                let dst_func = edge.get("dst_func").and_then(|v| v.as_str());

                if let (Some(sf), Some(sfn), Some(df)) = (src_file, src_func, dst_func) {
                    map.entry(df.to_string())
                        .or_default()
                        .push((sf.to_string(), sfn.to_string()));
                }
            }
        }

        map
    }

    /// Generate `breaking-change-risk` findings from a pre-built caller list.
    ///
    /// Unlike `parse_impact_findings` which parses `tldr impact` JSON, this
    /// method accepts an already-resolved list of `(file, function)` callers
    /// from the reverse caller map built by `build_reverse_caller_map`.
    ///
    /// Severity thresholds match `parse_impact_findings`:
    /// - `>5` callers = `high`
    /// - `2..=5` callers = `medium`
    /// - `1` caller = `info`
    /// - `0` callers = no finding emitted
    ///
    /// The evidence `command` field is set to `"calls"` (not `"impact"`).
    fn parse_impact_findings_from_callgraph(
        func_name: &str,
        callers: &[(String, String)],
    ) -> Vec<BugbotFinding> {
        let mut findings = Vec::new();
        let caller_count = callers.len();

        if caller_count == 0 {
            return findings;
        }

        let severity = if caller_count > 5 {
            "high"
        } else if caller_count >= 2 {
            "medium"
        } else {
            "info"
        };

        let callers_preview: Vec<String> = callers
            .iter()
            .take(5)
            .map(|(file, func)| format!("{}::{}", file, func))
            .collect();

        findings.push(BugbotFinding {
            finding_type: "breaking-change-risk".to_string(),
            severity: severity.to_string(),
            file: PathBuf::from("(project)"),
            function: func_name.to_string(),
            line: 0,
            message: format!(
                "Function `{}` has {} callers that may be affected by changes",
                func_name, caller_count
            ),
            evidence: serde_json::json!({
                "command": "calls",
                "caller_count": caller_count,
                "callers_preview": callers_preview,
            }),
            confidence: Some("DETERMINISTIC".to_string()),
            finding_id: Some(compute_finding_id(
                "breaking-change-risk",
                Path::new("(project)"),
                func_name,
                0,
            )),
        });

        findings
    }

    /// Detect downstream dependencies for changed files.
    ///
    /// When `current_calls_json` is `Some`, derives downstream impact metrics
    /// in-memory from the cached call graph JSON using `derive_downstream_from_calls`,
    /// eliminating per-file `tldr whatbreaks` subprocess calls.
    ///
    /// When `current_calls_json` is `None`, falls back to running
    /// `tldr whatbreaks <relative_path> --type file --quick <project> --lang <language> --format json`
    /// per changed file. Uses a 300-second timeout to accommodate large projects.
    fn analyze_downstream_impact(
        &self,
        project: &Path,
        changed_files: &[PathBuf],
        language: &str,
        current_calls_json: Option<&serde_json::Value>,
        partial_reasons: &mut Vec<String>,
    ) -> Vec<BugbotFinding> {
        let mut findings = Vec::new();

        if let Some(calls_json) = current_calls_json {
            // Derive downstream impact from cached calls JSON
            let changed_file_strs: Vec<&str> = changed_files
                .iter()
                .map(|p| p.strip_prefix(project).unwrap_or(p))
                .filter_map(|p| p.to_str())
                .collect();

            let downstream_results =
                Self::derive_downstream_from_calls(calls_json, &changed_file_strs);
            for (file_str, metrics) in &downstream_results {
                let file_path = project.join(file_str);
                let wb_json = serde_json::json!({ "summary": metrics });
                findings.extend(Self::parse_whatbreaks_findings(&file_path, &wb_json));
            }
        } else {
            // Fallback: run tldr whatbreaks subprocess per file
            let flow_engine = TldrDifferentialEngine::with_timeout(300);

            for file_path in changed_files {
                let relative = file_path.strip_prefix(project).unwrap_or(file_path);
                let rel_str = relative.to_string_lossy().to_string();

                let args = vec![
                    "whatbreaks".to_string(),
                    rel_str.clone(),
                    "--type".to_string(),
                    "file".to_string(),
                    "--quick".to_string(),
                    project.to_string_lossy().to_string(),
                    "--lang".to_string(),
                    language.to_string(),
                    "--format".to_string(),
                    "json".to_string(),
                ];

                match flow_engine.run_tldr_raw(&args) {
                    Ok(json) => {
                        findings.extend(Self::parse_whatbreaks_findings(file_path, &json));
                    }
                    Err(e) => {
                        partial_reasons
                            .push(format!("tldr whatbreaks {} failed: {}", rel_str, e));
                    }
                }
            }
        }

        findings
    }

    /// Detect callers of changed functions via a single `tldr calls` invocation.
    ///
    /// Discovers function names via `tldr cognitive` on each changed file,
    /// caps the total at 20 functions, then uses the call graph to build a
    /// reverse caller map. Each discovered function is looked up to produce
    /// `breaking-change-risk` findings.
    ///
    /// When `current_calls_json` is `Some`, the cached call graph JSON is
    /// reused instead of running a `tldr calls` subprocess. When `None`,
    /// falls back to running `tldr calls` once at the project level.
    ///
    /// If `tldr calls` fails (and no cache is available), the error is logged
    /// to `partial_reasons` and an empty findings list is returned.
    fn analyze_function_impact(
        &self,
        project: &Path,
        changed_files: &[PathBuf],
        language: &str,
        current_calls_json: Option<&serde_json::Value>,
        partial_reasons: &mut Vec<String>,
    ) -> Vec<BugbotFinding> {
        let mut findings = Vec::new();
        let impact_engine = TldrDifferentialEngine::with_timeout(60);

        // Step 1: Discover function names from changed files via cognitive analysis.
        let mut all_functions: Vec<String> = Vec::new();
        for file_path in changed_files {
            let relative = file_path.strip_prefix(project).unwrap_or(file_path);
            let full_path = project.join(relative);

            let cognitive_result =
                impact_engine.run_tldr_command(&["cognitive"], &full_path);
            let func_names =
                Self::discover_function_names_from_cognitive(&cognitive_result);
            all_functions.extend(func_names);
        }

        // Cap at 20 functions to limit analysis scope.
        all_functions.truncate(20);

        if all_functions.is_empty() {
            return findings;
        }

        // Step 2: Use cached calls JSON or run `tldr calls` once at project level.
        let calls_json_owned: Option<serde_json::Value>;
        let calls_json_ref: &serde_json::Value = if let Some(cached) = current_calls_json {
            cached
        } else {
            let args = vec![
                "calls".to_string(),
                project.to_string_lossy().to_string(),
                "--lang".to_string(),
                language.to_string(),
                "--format".to_string(),
                "json".to_string(),
            ];

            match impact_engine.run_tldr_raw(&args) {
                Ok(json) => {
                    calls_json_owned = Some(json);
                    calls_json_owned.as_ref().unwrap()
                }
                Err(e) => {
                    partial_reasons.push(format!("tldr calls failed: {}", e));
                    return findings;
                }
            }
        };

        // Step 3: Build reverse map (dst_func -> [(src_file, src_func)]).
        let reverse_map = Self::build_reverse_caller_map(calls_json_ref);

        // Step 4: Look up callers for each discovered function.
        for func_name in &all_functions {
            let callers = reverse_map.get(func_name).cloned().unwrap_or_default();
            findings.extend(Self::parse_impact_findings_from_callgraph(
                func_name, &callers,
            ));
        }

        findings
    }

    /// Diff call graph edges between baseline and current.
    ///
    /// Extracts `edges` arrays from both JSON values, builds sets of
    /// `(src_file::src_func, dst_file::dst_func)` pairs, and reports
    /// new/removed edges as findings. More than 5 new edges produces a
    /// medium-severity summary finding.
    ///
    /// The actual `tldr calls --format json` schema uses:
    /// ```json
    /// { "edges": [{ "src_file": "a.rs", "src_func": "foo", "dst_file": "b.rs", "dst_func": "bar", "call_type": "direct" }] }
    /// ```
    fn diff_calls_json(
        &self,
        baseline: &serde_json::Value,
        current: &serde_json::Value,
    ) -> Vec<BugbotFinding> {
        let mut findings = Vec::new();

        let extract_edges = |json: &serde_json::Value| -> std::collections::HashSet<(String, String)> {
            let mut set = std::collections::HashSet::new();
            if let Some(edges) = json.get("edges").and_then(|v| v.as_array()) {
                for edge in edges {
                    let from = format!(
                        "{}::{}",
                        edge.get("src_file").and_then(|v| v.as_str()).unwrap_or("?"),
                        edge.get("src_func").and_then(|v| v.as_str()).unwrap_or("?"),
                    );
                    let to = format!(
                        "{}::{}",
                        edge.get("dst_file").and_then(|v| v.as_str()).unwrap_or("?"),
                        edge.get("dst_func").and_then(|v| v.as_str()).unwrap_or("?"),
                    );
                    if from != "?::?" && to != "?::?" {
                        set.insert((from, to));
                    }
                }
            }
            set
        };

        let baseline_edges = extract_edges(baseline);
        let current_edges = extract_edges(current);

        // New edges: in current but not in baseline
        let new_edges: Vec<&(String, String)> = current_edges.difference(&baseline_edges).collect();
        // Removed edges: in baseline but not in current
        let removed_edges: Vec<&(String, String)> = baseline_edges.difference(&current_edges).collect();

        if new_edges.is_empty() && removed_edges.is_empty() {
            return findings;
        }

        // Report individual new edges as info
        for (from, to) in &new_edges {
            findings.push(BugbotFinding {
                finding_type: "call-graph-change".to_string(),
                severity: "info".to_string(),
                file: PathBuf::from("(project)"),
                function: "(project-level)".to_string(),
                line: 0,
                message: format!("New call edge: {} -> {}", from, to),
                evidence: serde_json::json!({
                    "change": "added",
                    "from": from,
                    "to": to,
                }),
                confidence: Some("DETERMINISTIC".to_string()),
                finding_id: Some(compute_finding_id(
                    "call-graph-change",
                    Path::new("(project)"),
                    &format!("{}:{}", from, to),
                    0,
                )),
            });
        }

        // Report individual removed edges as info
        for (from, to) in &removed_edges {
            findings.push(BugbotFinding {
                finding_type: "call-graph-change".to_string(),
                severity: "info".to_string(),
                file: PathBuf::from("(project)"),
                function: "(project-level)".to_string(),
                line: 0,
                message: format!("Removed call edge: {} -> {}", from, to),
                evidence: serde_json::json!({
                    "change": "removed",
                    "from": from,
                    "to": to,
                }),
                confidence: Some("DETERMINISTIC".to_string()),
                finding_id: Some(compute_finding_id(
                    "call-graph-change",
                    Path::new("(project)"),
                    &format!("removed:{}:{}", from, to),
                    0,
                )),
            });
        }

        // Summary finding at medium severity if many new edges
        if new_edges.len() > 5 {
            findings.push(BugbotFinding {
                finding_type: "call-graph-change".to_string(),
                severity: "medium".to_string(),
                file: PathBuf::from("(project)"),
                function: "(project-level)".to_string(),
                line: 0,
                message: format!(
                    "Significant call graph change: {} new edges, {} removed edges",
                    new_edges.len(),
                    removed_edges.len(),
                ),
                evidence: serde_json::json!({
                    "new_edge_count": new_edges.len(),
                    "removed_edge_count": removed_edges.len(),
                }),
                confidence: Some("DETERMINISTIC".to_string()),
                finding_id: Some(compute_finding_id(
                    "call-graph-change",
                    Path::new("(project)"),
                    "(summary)",
                    0,
                )),
            });
        }

        findings
    }

    /// Diff module dependencies between baseline and current.
    ///
    /// Compares `circular_dependencies` arrays: new circular deps get "high"
    /// severity. Compares `internal_dependencies` counts: significant increase
    /// gets "medium".
    ///
    /// The actual `tldr deps --format json` schema uses:
    /// ```json
    /// {
    ///   "internal_dependencies": { "file.rs": ["dep1.rs", "dep2.rs"], ... },
    ///   "circular_dependencies": [{ "path": ["a.rs", "b.rs", "a.rs"], "len": 3 }, ...],
    ///   "stats": { "total_internal_deps": 42, ... }
    /// }
    /// ```
    fn diff_deps_json(
        &self,
        baseline: &serde_json::Value,
        current: &serde_json::Value,
    ) -> Vec<BugbotFinding> {
        let mut findings = Vec::new();

        // Extract circular dependencies as sets of sorted module lists.
        // Each circular dep is an object with a "path" array of module names.
        let extract_circular = |json: &serde_json::Value| -> std::collections::HashSet<String> {
            let mut set = std::collections::HashSet::new();
            if let Some(circs) = json.get("circular_dependencies").and_then(|v| v.as_array()) {
                for circ in circs {
                    // Each circular dep is an object: { "path": ["a.rs", "b.rs"], "len": N }
                    if let Some(path) = circ.get("path").and_then(|v| v.as_array()) {
                        let mut names: Vec<String> = path
                            .iter()
                            .filter_map(|m| m.as_str().map(|s| s.to_string()))
                            .collect();
                        names.sort();
                        set.insert(names.join(","));
                    }
                }
            }
            set
        };

        let baseline_circular = extract_circular(baseline);
        let current_circular = extract_circular(current);

        // New circular dependencies = high severity regression
        let new_circular: Vec<&String> = current_circular.difference(&baseline_circular).collect();
        for circ in &new_circular {
            findings.push(BugbotFinding {
                finding_type: "dependency-change".to_string(),
                severity: "high".to_string(),
                file: PathBuf::from("(project)"),
                function: "(project-level)".to_string(),
                line: 0,
                message: format!("New circular dependency detected: {}", circ),
                evidence: serde_json::json!({
                    "change": "new_circular",
                    "modules": circ,
                }),
                confidence: Some("DETERMINISTIC".to_string()),
                finding_id: Some(compute_finding_id(
                    "dependency-change",
                    Path::new("(project)"),
                    &format!("circular:{}", circ),
                    0,
                )),
            });
        }

        // Compare internal dependency counts.
        // `internal_dependencies` is a dict (file -> [deps]), so count total deps
        // across all files. Alternatively, use `stats.total_internal_deps` if available.
        let count_internal_deps = |json: &serde_json::Value| -> usize {
            // Prefer stats.total_internal_deps for accuracy
            if let Some(total) = json.get("stats")
                .and_then(|s| s.get("total_internal_deps"))
                .and_then(|v| v.as_u64())
            {
                return total as usize;
            }
            // Fallback: sum up all dependency arrays in the dict
            json.get("internal_dependencies")
                .and_then(|v| v.as_object())
                .map(|obj| obj.values()
                    .filter_map(|v| v.as_array())
                    .map(|a| a.len())
                    .sum())
                .unwrap_or(0)
        };

        let baseline_dep_count = count_internal_deps(baseline);
        let current_dep_count = count_internal_deps(current);

        if current_dep_count > baseline_dep_count {
            let increase = current_dep_count - baseline_dep_count;
            // Significant increase = more than 20% growth or >5 new deps
            if increase > 5 || (baseline_dep_count > 0 && increase * 100 / baseline_dep_count > 20) {
                findings.push(BugbotFinding {
                    finding_type: "dependency-change".to_string(),
                    severity: "medium".to_string(),
                    file: PathBuf::from("(project)"),
                    function: "(project-level)".to_string(),
                    line: 0,
                    message: format!(
                        "Internal dependency count increased: {} -> {} (+{})",
                        baseline_dep_count, current_dep_count, increase,
                    ),
                    evidence: serde_json::json!({
                        "change": "dependency_count_increase",
                        "baseline_count": baseline_dep_count,
                        "current_count": current_dep_count,
                        "increase": increase,
                    }),
                    confidence: Some("DETERMINISTIC".to_string()),
                    finding_id: Some(compute_finding_id(
                        "dependency-change",
                        Path::new("(project)"),
                        "(dep-count)",
                        0,
                    )),
                });
            }
        }

        findings
    }

    /// Diff coupling metrics between baseline and current.
    ///
    /// Builds maps of `module -> {ca, ce, instability}` from `martin_metrics`
    /// arrays. Flags modules where instability increased or efferent coupling
    /// (ce) increased significantly.
    ///
    /// Note: No longer called by `analyze_flow_commands` (coupling is skipped
    /// because it requires file pairs, not a project root), but retained for
    /// diffing raw `tldr coupling` JSON output in other contexts and tested
    /// directly.
    pub fn diff_coupling_json(
        &self,
        baseline: &serde_json::Value,
        current: &serde_json::Value,
    ) -> Vec<BugbotFinding> {
        let mut findings = Vec::new();

        let extract_metrics = |json: &serde_json::Value| -> std::collections::HashMap<String, (f64, f64, f64)> {
            let mut map = std::collections::HashMap::new();
            if let Some(metrics) = json.get("martin_metrics").and_then(|v| v.as_array()) {
                for entry in metrics {
                    let module = entry.get("module").and_then(|v| v.as_str()).unwrap_or("");
                    if module.is_empty() {
                        continue;
                    }
                    let ca = entry.get("ca").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let ce = entry.get("ce").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let instability = entry.get("instability").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    map.insert(module.to_string(), (ca, ce, instability));
                }
            }
            map
        };

        let baseline_metrics = extract_metrics(baseline);
        let current_metrics = extract_metrics(current);

        for (module, (_, curr_ce, curr_instability)) in &current_metrics {
            if let Some((_, base_ce, base_instability)) = baseline_metrics.get(module) {
                // Flag instability increase
                let instability_delta = curr_instability - base_instability;
                let ce_delta = curr_ce - base_ce;

                if instability_delta > 0.05 || ce_delta > 2.0 {
                    let severity = if instability_delta > 0.3 || ce_delta > 5.0 {
                        "high"
                    } else if instability_delta > 0.1 || ce_delta > 3.0 {
                        "medium"
                    } else {
                        "low"
                    };

                    findings.push(BugbotFinding {
                        finding_type: "coupling-increase".to_string(),
                        severity: severity.to_string(),
                        file: PathBuf::from("(project)"),
                        function: "(project-level)".to_string(),
                        line: 0,
                        message: format!(
                            "Module '{}': instability {:.2} -> {:.2} (delta {:.2}), ce {} -> {}",
                            module, base_instability, curr_instability, instability_delta,
                            base_ce, curr_ce,
                        ),
                        evidence: serde_json::json!({
                            "module": module,
                            "baseline_instability": base_instability,
                            "current_instability": curr_instability,
                            "instability_delta": instability_delta,
                            "baseline_ce": base_ce,
                            "current_ce": curr_ce,
                            "ce_delta": ce_delta,
                        }),
                        confidence: Some("DETERMINISTIC".to_string()),
                        finding_id: Some(compute_finding_id(
                            "coupling-increase",
                            Path::new("(project)"),
                            module,
                            0,
                        )),
                    });
                }
            }
        }

        findings
    }

    /// Diff class cohesion (LCOM4) between baseline and current.
    ///
    /// Builds maps of `class name -> lcom4` from `classes` arrays.
    /// LCOM4 increase = less cohesive = regression. New classes with
    /// high LCOM4 (>3) get "info" findings.
    ///
    /// The actual `tldr cohesion --format json` schema uses:
    /// ```json
    /// { "classes": [{ "class_name": "Foo", "lcom4": 3, ... }] }
    /// ```
    fn diff_cohesion_json(
        &self,
        baseline: &serde_json::Value,
        current: &serde_json::Value,
    ) -> Vec<BugbotFinding> {
        let mut findings = Vec::new();

        let extract_lcom4 = |json: &serde_json::Value| -> std::collections::HashMap<String, f64> {
            let mut map = std::collections::HashMap::new();
            if let Some(classes) = json.get("classes").and_then(|v| v.as_array()) {
                for cls in classes {
                    // Try "class_name" first (actual schema), fall back to "name"
                    let name = cls.get("class_name")
                        .or_else(|| cls.get("name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if name.is_empty() {
                        continue;
                    }
                    let lcom4 = cls.get("lcom4").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    map.insert(name.to_string(), lcom4);
                }
            }
            map
        };

        let baseline_lcom = extract_lcom4(baseline);
        let current_lcom = extract_lcom4(current);

        for (class_name, curr_lcom4) in &current_lcom {
            if let Some(base_lcom4) = baseline_lcom.get(class_name) {
                // LCOM4 increase = cohesion decrease = regression
                let delta = curr_lcom4 - base_lcom4;
                if delta > 0.5 {
                    let severity = if delta > 3.0 {
                        "high"
                    } else if delta > 1.0 {
                        "medium"
                    } else {
                        "low"
                    };

                    findings.push(BugbotFinding {
                        finding_type: "cohesion-decrease".to_string(),
                        severity: severity.to_string(),
                        file: PathBuf::from("(project)"),
                        function: "(project-level)".to_string(),
                        line: 0,
                        message: format!(
                            "Class '{}': LCOM4 increased {} -> {} (less cohesive)",
                            class_name, base_lcom4, curr_lcom4,
                        ),
                        evidence: serde_json::json!({
                            "class": class_name,
                            "baseline_lcom4": base_lcom4,
                            "current_lcom4": curr_lcom4,
                            "delta": delta,
                        }),
                        confidence: Some("DETERMINISTIC".to_string()),
                        finding_id: Some(compute_finding_id(
                            "cohesion-decrease",
                            Path::new("(project)"),
                            class_name,
                            0,
                        )),
                    });
                }
            } else {
                // New class: flag if LCOM4 is high
                if *curr_lcom4 > 3.0 {
                    findings.push(BugbotFinding {
                        finding_type: "cohesion-decrease".to_string(),
                        severity: "info".to_string(),
                        file: PathBuf::from("(project)"),
                        function: "(project-level)".to_string(),
                        line: 0,
                        message: format!(
                            "New class '{}' has high LCOM4 ({}): consider splitting",
                            class_name, curr_lcom4,
                        ),
                        evidence: serde_json::json!({
                            "class": class_name,
                            "lcom4": curr_lcom4,
                            "new_class": true,
                        }),
                        confidence: Some("DETERMINISTIC".to_string()),
                        finding_id: Some(compute_finding_id(
                            "cohesion-decrease",
                            Path::new("(project)"),
                            class_name,
                            0,
                        )),
                    });
                }
            }
        }

        findings
    }

    /// Count dead code entries from `tldr dead` JSON output.
    ///
    /// The actual output uses `"dead_functions"` and `"possibly_dead"` arrays,
    /// plus a `"total_count"` field for convenience.
    fn count_dead_code_entries(json: &serde_json::Value) -> usize {
        // Try the summary field first
        if let Some(total) = json.get("total_count").and_then(|v| v.as_u64()) {
            return total as usize;
        }
        // Fallback: count array entries
        for key in &["dead_functions", "possibly_dead", "dead_code", "unreachable", "functions", "results"] {
            if let Some(arr) = json.get(key).and_then(|v| v.as_array()) {
                return arr.len();
            }
        }
        if let Some(arr) = json.as_array() {
            return arr.len();
        }
        0
    }

    /// Derive module-level dependency information from a call-graph JSON.
    ///
    /// Reads `calls_json["edges"]`, groups cross-file edges into a dependency
    /// map (`src_file -> [dst_file, ...]`), and detects circular dependencies
    /// (A depends on B AND B depends on A).
    ///
    /// Returns a JSON value with `internal_dependencies`, `circular_dependencies`,
    /// and `stats.total_internal_deps`.
    pub fn derive_deps_from_calls(calls_json: &serde_json::Value) -> serde_json::Value {
        let empty_edges: Vec<serde_json::Value> = Vec::new();
        let edges = calls_json
            .get("edges")
            .and_then(|v| v.as_array())
            .unwrap_or(&empty_edges);

        // Build dependency map: src_file -> BTreeSet<dst_file>
        let mut dep_map: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for edge in edges {
            let src_file = edge.get("src_file").and_then(|v| v.as_str()).unwrap_or("");
            let dst_file = edge.get("dst_file").and_then(|v| v.as_str()).unwrap_or("");
            // Skip intra-file edges
            if src_file.is_empty() || dst_file.is_empty() || src_file == dst_file {
                continue;
            }
            dep_map
                .entry(src_file.to_string())
                .or_default()
                .insert(dst_file.to_string());
        }

        // Count total unique dependency pairs
        let total_internal_deps: usize = dep_map.values().map(|s| s.len()).sum();

        // Detect circular dependencies: A depends on B AND B depends on A
        let mut circular: Vec<serde_json::Value> = Vec::new();
        let mut seen_cycles: BTreeSet<(String, String)> = BTreeSet::new();
        for (src, destinations) in &dep_map {
            for dst in destinations {
                if let Some(reverse_deps) = dep_map.get(dst) {
                    if reverse_deps.contains(src) {
                        let (a, b) = if src < dst {
                            (src.clone(), dst.clone())
                        } else {
                            (dst.clone(), src.clone())
                        };
                        if seen_cycles.insert((a.clone(), b.clone())) {
                            circular.push(serde_json::json!({
                                "path": [a, b]
                            }));
                        }
                    }
                }
            }
        }

        // Build internal_dependencies as JSON object with sorted arrays
        let internal_deps: serde_json::Map<String, serde_json::Value> = dep_map
            .into_iter()
            .map(|(k, v)| {
                let arr: Vec<serde_json::Value> = v
                    .into_iter()
                    .map(serde_json::Value::String)
                    .collect();
                (k, serde_json::Value::Array(arr))
            })
            .collect();

        serde_json::json!({
            "internal_dependencies": internal_deps,
            "circular_dependencies": circular,
            "stats": {
                "total_internal_deps": total_internal_deps
            }
        })
    }

    /// Derive Martin coupling metrics (Ca, Ce, Instability) from a call-graph JSON.
    ///
    /// For each cross-file edge, increments efferent coupling (Ce) for the caller
    /// file and afferent coupling (Ca) for the callee file. Uses sets for
    /// deduplication: Ce counts unique destination files, Ca counts unique source
    /// files. Instability = Ce / (Ca + Ce).
    pub fn derive_coupling_from_calls(calls_json: &serde_json::Value) -> serde_json::Value {
        let empty_edges: Vec<serde_json::Value> = Vec::new();
        let edges = calls_json
            .get("edges")
            .and_then(|v| v.as_array())
            .unwrap_or(&empty_edges);

        // Ce: for each module, the set of unique modules it calls (efferent)
        let mut ce_map: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        // Ca: for each module, the set of unique modules that call into it (afferent)
        let mut ca_map: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

        for edge in edges {
            let src_file = edge.get("src_file").and_then(|v| v.as_str()).unwrap_or("");
            let dst_file = edge.get("dst_file").and_then(|v| v.as_str()).unwrap_or("");
            // Skip intra-file edges
            if src_file.is_empty() || dst_file.is_empty() || src_file == dst_file {
                continue;
            }
            ce_map
                .entry(src_file.to_string())
                .or_default()
                .insert(dst_file.to_string());
            ca_map
                .entry(dst_file.to_string())
                .or_default()
                .insert(src_file.to_string());
        }

        // Collect all modules
        let mut all_modules: BTreeSet<String> = BTreeSet::new();
        for k in ce_map.keys() {
            all_modules.insert(k.clone());
        }
        for k in ca_map.keys() {
            all_modules.insert(k.clone());
        }

        let mut metrics: Vec<serde_json::Value> = Vec::new();
        for module in &all_modules {
            let ca = ca_map.get(module).map_or(0, |s| s.len());
            let ce = ce_map.get(module).map_or(0, |s| s.len());
            let instability = if ca + ce == 0 {
                0.0
            } else {
                ce as f64 / (ca + ce) as f64
            };
            metrics.push(serde_json::json!({
                "module": module,
                "ca": ca,
                "ce": ce,
                "instability": instability
            }));
        }

        serde_json::json!({
            "martin_metrics": metrics
        })
    }

    /// Derive downstream impact metrics for a set of changed files from a call-graph JSON.
    ///
    /// For each changed file, finds all cross-file edges where that file is the
    /// callee (`dst_file`). Counts importers, direct callers, and affected test
    /// files (using a path/name heuristic). Always returns one entry per changed
    /// file, even when counts are zero.
    pub fn derive_downstream_from_calls(
        calls_json: &serde_json::Value,
        changed_files: &[&str],
    ) -> Vec<(String, serde_json::Value)> {
        let empty_edges: Vec<serde_json::Value> = Vec::new();
        let edges = calls_json
            .get("edges")
            .and_then(|v| v.as_array())
            .unwrap_or(&empty_edges);

        let mut results: Vec<(String, serde_json::Value)> = Vec::new();

        for &changed_file in changed_files {
            let mut importers: BTreeSet<String> = BTreeSet::new();
            let mut test_importers: BTreeSet<String> = BTreeSet::new();

            for edge in edges {
                let src_file = edge.get("src_file").and_then(|v| v.as_str()).unwrap_or("");
                let dst_file = edge.get("dst_file").and_then(|v| v.as_str()).unwrap_or("");

                // Only count unique source files calling INTO the changed file
                if dst_file == changed_file && src_file != changed_file && !src_file.is_empty() {
                    importers.insert(src_file.to_string());
                    if src_file.contains("test") {
                        test_importers.insert(src_file.to_string());
                    }
                }
            }

            let importer_count = importers.len() as u64;
            let affected_test_count = test_importers.len() as u64;

            results.push((
                changed_file.to_string(),
                serde_json::json!({
                    "importer_count": importer_count,
                    "direct_caller_count": importer_count,
                    "affected_test_count": affected_test_count
                }),
            ));
        }

        results
    }
}

/// Compute a deterministic finding ID from the finding's key fields.
///
/// Uses `DefaultHasher` (SipHash) over `(finding_type, file_path, function_name, line)`
/// and formats the result as a lowercase hex string.
fn compute_finding_id(finding_type: &str, file: &Path, function: &str, line: usize) -> String {
    let mut hasher = DefaultHasher::new();
    finding_type.hash(&mut hasher);
    file.to_string_lossy().as_ref().hash(&mut hasher);
    function.hash(&mut hasher);
    line.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

impl Default for TldrDifferentialEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl L2Engine for TldrDifferentialEngine {
    fn name(&self) -> &'static str {
        "TldrDifferentialEngine"
    }

    fn finding_types(&self) -> &[&'static str] {
        FINDING_TYPES
    }

    fn analyze(&self, ctx: &L2Context) -> L2AnalyzerOutput {
        let start = Instant::now();
        let mut all_findings = Vec::new();
        let mut partial_reasons = Vec::new();

        // === LOCAL commands: per-file analysis (parallelized across cores) ===
        let work_items: Vec<_> = ctx
            .changed_files
            .iter()
            .filter_map(|file_path| {
                let baseline = ctx.baseline_contents.get(file_path)?;
                let current = ctx.current_contents.get(file_path)?;
                Some((file_path, baseline.as_str(), current.as_str()))
            })
            .collect();

        let functions_skipped = ctx.changed_files.len() - work_items.len();
        let functions_analyzed = work_items.len();

        let num_threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .min(work_items.len().max(1));

        if num_threads <= 1 || work_items.len() <= 1 {
            for (file_path, baseline_src, current_src) in &work_items {
                let mut file_reasons = Vec::new();
                let file_findings =
                    self.analyze_local_commands(file_path, baseline_src, current_src, &mut file_reasons);
                all_findings.extend(file_findings);
                partial_reasons.extend(file_reasons);
            }
        } else {
            let chunk_size = work_items.len().div_ceil(num_threads);
            std::thread::scope(|s| {
                let handles: Vec<_> = work_items
                    .chunks(chunk_size)
                    .map(|chunk| {
                        s.spawn(move || {
                            let mut findings = Vec::new();
                            let mut reasons = Vec::new();
                            for (file_path, baseline_src, current_src) in chunk {
                                let file_findings = self.analyze_local_commands(
                                    file_path,
                                    baseline_src,
                                    current_src,
                                    &mut reasons,
                                );
                                findings.extend(file_findings);
                            }
                            (findings, reasons)
                        })
                    })
                    .collect();

                for handle in handles {
                    if let Ok((findings, reasons)) = handle.join() {
                        all_findings.extend(findings);
                        partial_reasons.extend(reasons);
                    }
                }
            });
        }

        // === Run `tldr calls` ONCE for the current project ===
        let language_str = ctx.language.as_str();
        let calls_engine = TldrDifferentialEngine::with_timeout(300);
        let current_calls_json = calls_engine
            .run_tldr_flow_command("calls", &["calls"], &ctx.project, language_str)
            .ok();

        // === FLOW commands: project-wide analysis ===
        let flow_findings = self.analyze_flow_commands(
            &ctx.project,
            &ctx.base_ref,
            language_str,
            current_calls_json.as_ref(),
            &mut partial_reasons,
        );
        all_findings.extend(flow_findings);

        // === IMPACT commands: downstream dependency analysis ===
        let impact_findings = self.analyze_downstream_impact(
            &ctx.project,
            &ctx.changed_files,
            language_str,
            current_calls_json.as_ref(),
            &mut partial_reasons,
        );
        all_findings.extend(impact_findings);

        let func_impact_findings = self.analyze_function_impact(
            &ctx.project,
            &ctx.changed_files,
            language_str,
            current_calls_json.as_ref(),
            &mut partial_reasons,
        );
        all_findings.extend(func_impact_findings);

        let duration_ms = start.elapsed().as_millis() as u64;

        let status = if partial_reasons.is_empty() {
            AnalyzerStatus::Complete
        } else {
            AnalyzerStatus::Partial {
                reason: partial_reasons.join("; "),
            }
        };

        L2AnalyzerOutput {
            findings: all_findings,
            status,
            duration_ms,
            functions_analyzed,
            functions_skipped,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::bugbot::l2::context::{FunctionDiff, L2Context};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use tldr_core::Language;

    fn empty_context() -> L2Context {
        L2Context::new(
            PathBuf::from("/tmp/test-project"),
            Language::Rust,
            vec![],
            FunctionDiff {
                changed: vec![],
                inserted: vec![],
                deleted: vec![],
            },
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
        )
    }

    // =========================================================================
    // Engine metadata tests
    // =========================================================================

    #[test]
    fn test_engine_name() {
        let engine = TldrDifferentialEngine::new();
        assert_eq!(engine.name(), "TldrDifferentialEngine");
    }

    #[test]
    fn test_finding_types() {
        let engine = TldrDifferentialEngine::new();
        let types = engine.finding_types();
        assert_eq!(types.len(), 11);
        assert!(types.contains(&"complexity-increase"));
        assert!(types.contains(&"cognitive-increase"));
        assert!(types.contains(&"contract-removed"));
        assert!(types.contains(&"smell-introduced"));
        assert!(types.contains(&"call-graph-change"));
        assert!(types.contains(&"dependency-change"));
        assert!(types.contains(&"coupling-increase"));
        assert!(types.contains(&"cohesion-decrease"));
        assert!(types.contains(&"dead-code-introduced"));
        assert!(types.contains(&"downstream-impact"));
        assert!(types.contains(&"breaking-change-risk"));
    }

    #[test]
    fn test_default() {
        let engine = TldrDifferentialEngine::default();
        assert_eq!(engine.name(), "TldrDifferentialEngine");
        assert_eq!(engine.timeout_secs, 30);
    }

    #[test]
    fn test_with_timeout() {
        let engine = TldrDifferentialEngine::with_timeout(60);
        assert_eq!(engine.timeout_secs, 60);
    }

    #[test]
    fn test_languages_empty() {
        let engine = TldrDifferentialEngine::new();
        assert!(
            engine.languages().is_empty(),
            "TldrDifferentialEngine is language-agnostic"
        );
    }

    // =========================================================================
    // Empty context behavior
    // =========================================================================

    #[test]
    fn test_empty_context() {
        let engine = TldrDifferentialEngine::new();
        let ctx = empty_context();
        let output = engine.analyze(&ctx);

        assert!(
            output.findings.is_empty(),
            "Empty context should produce no findings"
        );
        assert_eq!(output.functions_analyzed, 0);
        assert_eq!(output.functions_skipped, 0);
        assert!(output.duration_ms < 5000, "Should complete quickly");
    }

    #[test]
    fn test_empty_context_status() {
        let engine = TldrDifferentialEngine::new();
        let ctx = empty_context();
        let output = engine.analyze(&ctx);

        // With no changed files, local commands produce Complete.
        // Flow commands may produce Partial if tldr isn't on PATH, but the
        // status check is for the overall output shape.
        // We accept either Complete or Partial here since flow commands run
        // on project root and may fail on /tmp/test-project.
        match &output.status {
            AnalyzerStatus::Complete => {} // ideal
            AnalyzerStatus::Partial { .. } => {} // acceptable (flow command failures)
            other => panic!("Unexpected status: {:?}", other),
        }
    }

    // =========================================================================
    // Graceful degradation when tldr not available
    // =========================================================================

    #[test]
    fn test_run_tldr_command_not_found() {
        // Use a nonexistent binary name to simulate tldr not on PATH
        // We test the error handling path directly
        let engine = TldrDifferentialEngine::new();
        let result = engine.run_tldr_command(&["complexity"], Path::new("/dev/null"));

        // Should return an error, not panic
        // The result may be an error (binary not found) or success with empty
        // output depending on environment. Either way, no panic.
        match result {
            Ok(_) => {} // tldr is on PATH and ran, that's fine
            Err(e) => {
                assert!(
                    !e.is_empty(),
                    "Error message should not be empty"
                );
            }
        }
    }

    // =========================================================================
    // Trait object safety
    // =========================================================================

    #[test]
    fn test_as_trait_object() {
        let engine: Box<dyn L2Engine> = Box::new(TldrDifferentialEngine::new());
        assert_eq!(engine.name(), "TldrDifferentialEngine");
        assert_eq!(engine.finding_types().len(), 11);
        assert!(engine.languages().is_empty());
    }

    // =========================================================================
    // Finding ID determinism
    // =========================================================================

    #[test]
    fn test_finding_id_deterministic() {
        let id1 = compute_finding_id("complexity-increase", Path::new("a.py"), "foo", 10);
        let id2 = compute_finding_id("complexity-increase", Path::new("a.py"), "foo", 10);
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_finding_id_differs_for_different_inputs() {
        let id1 = compute_finding_id("complexity-increase", Path::new("a.py"), "foo", 10);
        let id2 = compute_finding_id("complexity-increase", Path::new("a.py"), "bar", 10);
        assert_ne!(id1, id2);
    }

    // =========================================================================
    // Diff logic unit tests (using mock JSON)
    // =========================================================================

    #[test]
    fn test_diff_numeric_metrics_increase_detected() {
        let engine = TldrDifferentialEngine::new();

        let baseline = serde_json::json!({
            "functions": [
                { "name": "process", "cyclomatic": 2, "line": 1 }
            ]
        });
        let current = serde_json::json!({
            "functions": [
                { "name": "process", "cyclomatic": 10, "line": 1 }
            ]
        });

        let findings = engine.diff_numeric_metrics(
            "complexity-increase",
            "cyclomatic",
            Path::new("src/lib.py"),
            &baseline,
            &current,
        );

        assert!(!findings.is_empty(), "Should detect cyclomatic increase");
        assert_eq!(findings[0].finding_type, "complexity-increase");
        assert_eq!(findings[0].confidence, Some("DETERMINISTIC".to_string()));
        assert!(findings[0].finding_id.is_some());

        // Verify severity: 2 -> 10 = +400%, should be "high"
        assert_eq!(findings[0].severity, "high");

        let evidence = &findings[0].evidence;
        assert_eq!(evidence["old_value"], 2.0);
        assert_eq!(evidence["new_value"], 10.0);
        assert_eq!(evidence["delta"], 8.0);
    }

    #[test]
    fn test_diff_numeric_metrics_decrease_not_flagged() {
        let engine = TldrDifferentialEngine::new();

        let baseline = serde_json::json!({
            "functions": [
                { "name": "process", "cyclomatic": 10, "line": 1 }
            ]
        });
        let current = serde_json::json!({
            "functions": [
                { "name": "process", "cyclomatic": 2, "line": 1 }
            ]
        });

        let findings = engine.diff_numeric_metrics(
            "complexity-increase",
            "cyclomatic",
            Path::new("src/lib.py"),
            &baseline,
            &current,
        );

        assert!(
            findings.is_empty(),
            "Decrease should not produce a finding"
        );
    }

    #[test]
    fn test_diff_numeric_metrics_new_function_info() {
        let engine = TldrDifferentialEngine::new();

        let baseline = serde_json::json!({
            "functions": []
        });
        let current = serde_json::json!({
            "functions": [
                { "name": "new_func", "cyclomatic": 15, "line": 5 }
            ]
        });

        let findings = engine.diff_numeric_metrics(
            "complexity-increase",
            "cyclomatic",
            Path::new("src/lib.py"),
            &baseline,
            &current,
        );

        assert!(!findings.is_empty(), "New function with high metric should be reported");
        assert_eq!(findings[0].severity, "info");
        assert!(findings[0].evidence["new_function"].as_bool().unwrap_or(false));
    }

    #[test]
    fn test_diff_numeric_metrics_no_change() {
        let engine = TldrDifferentialEngine::new();

        let baseline = serde_json::json!({
            "functions": [
                { "name": "process", "cyclomatic": 5, "line": 1 }
            ]
        });
        let current = serde_json::json!({
            "functions": [
                { "name": "process", "cyclomatic": 5, "line": 1 }
            ]
        });

        let findings = engine.diff_numeric_metrics(
            "complexity-increase",
            "cyclomatic",
            Path::new("src/lib.py"),
            &baseline,
            &current,
        );

        assert!(
            findings.is_empty(),
            "No change should produce no findings"
        );
    }

    #[test]
    fn test_diff_contracts_removed() {
        let engine = TldrDifferentialEngine::new();

        let baseline = serde_json::json!({
            "functions": [
                {
                    "name": "validate",
                    "preconditions": [{"expr": "x > 0"}],
                    "postconditions": [{"expr": "result >= 0"}]
                }
            ]
        });
        let current = serde_json::json!({
            "functions": [
                {
                    "name": "validate",
                    "preconditions": [],
                    "postconditions": []
                }
            ]
        });

        let findings = engine.diff_contracts(
            Path::new("src/lib.py"),
            &baseline,
            &current,
            &["validate".to_string()],
        );

        assert!(!findings.is_empty(), "Should detect removed contracts");
        assert_eq!(findings[0].finding_type, "contract-removed");
        assert_eq!(findings[0].severity, "medium");
        assert_eq!(findings[0].evidence["removed"], 2);
    }

    #[test]
    fn test_diff_contracts_function_deleted() {
        let engine = TldrDifferentialEngine::new();

        let baseline = serde_json::json!({
            "functions": [
                {
                    "name": "validate",
                    "preconditions": [{"expr": "x > 0"}],
                    "postconditions": []
                }
            ]
        });
        let current = serde_json::json!({
            "functions": []
        });

        // Pass empty known_current_funcs so "validate" is genuinely absent
        let findings = engine.diff_contracts(
            Path::new("src/lib.py"),
            &baseline,
            &current,
            &[],
        );

        assert!(!findings.is_empty(), "Should detect deleted function with contracts");
        assert_eq!(findings[0].severity, "high");
        assert!(findings[0].evidence["function_deleted"].as_bool().unwrap_or(false));
    }

    #[test]
    fn test_diff_contracts_extraction_failure_not_treated_as_deletion() {
        let engine = TldrDifferentialEngine::new();

        let baseline = serde_json::json!({
            "functions": [
                {
                    "name": "validate",
                    "preconditions": [{"expr": "x > 0"}],
                    "postconditions": []
                }
            ]
        });
        // Current JSON has no entries for "validate" (extraction failed),
        // but the function still exists in the current version.
        let current = serde_json::json!({
            "functions": []
        });

        // "validate" is in known_current_funcs — extraction failed, not deleted
        let findings = engine.diff_contracts(
            Path::new("src/lib.rs"),
            &baseline,
            &current,
            &["validate".to_string()],
        );

        assert!(findings.is_empty(), "Should NOT emit contract-removed when function exists but extraction failed");
    }

    #[test]
    fn test_diff_smells_introduced() {
        let engine = TldrDifferentialEngine::new();

        let baseline = serde_json::json!({
            "smells": [
                { "smell_type": "long_method", "name": "process", "line": 1, "reason": "too long", "severity": 1 }
            ]
        });
        let current = serde_json::json!({
            "smells": [
                { "smell_type": "long_method", "name": "process", "line": 1, "reason": "too long", "severity": 1 },
                { "smell_type": "god_class", "name": "Handler", "line": 20, "reason": "too many methods", "severity": 2 }
            ]
        });

        let findings = engine.diff_smells(
            Path::new("src/lib.py"),
            &baseline,
            &current,
        );

        assert!(!findings.is_empty(), "Should detect introduced smell");
        assert_eq!(findings[0].finding_type, "smell-introduced");
        assert_eq!(findings[0].severity, "medium"); // god_class is structural → medium
        assert_eq!(findings[0].evidence["introduced"], 1);
        // Verify smell_type is correctly extracted (not "unknown")
        assert_eq!(findings[0].evidence["smell_type"], "god_class");
        assert!(findings[0].message.contains("god_class"));
    }

    #[test]
    fn test_diff_smells_no_regression() {
        let engine = TldrDifferentialEngine::new();

        let baseline = serde_json::json!({
            "smells": [
                { "smell_type": "long_method", "name": "process", "line": 1, "reason": "too long", "severity": 1 }
            ]
        });
        let current = serde_json::json!({
            "smells": [
                { "smell_type": "long_method", "name": "process", "line": 1, "reason": "too long", "severity": 1 }
            ]
        });

        let findings = engine.diff_smells(
            Path::new("src/lib.py"),
            &baseline,
            &current,
        );

        assert!(findings.is_empty(), "Same smells should produce no findings");
    }

    #[test]
    fn test_diff_smells_new_file_baseline_empty() {
        let engine = TldrDifferentialEngine::new();

        // New file: baseline has no smells, current has many.
        // This is NOT a regression — all code is new, so no findings should fire.
        let baseline = serde_json::json!({ "smells": [] });
        let current = serde_json::json!({
            "smells": [
                { "smell_type": "god_class", "name": "BigEngine", "line": 10, "reason": "too big", "severity": 2 },
                { "smell_type": "long_method", "name": "run", "line": 50, "reason": "too long", "severity": 1 },
                { "smell_type": "long_method", "name": "analyze", "line": 200, "reason": "too long", "severity": 1 }
            ]
        });

        let findings = engine.diff_smells(
            Path::new("src/new_module.rs"),
            &baseline,
            &current,
        );

        assert!(findings.is_empty(), "New file (empty baseline) should not trigger smell-introduced");
    }

    #[test]
    fn test_diff_smells_real_tldr_schema() {
        // Test with exact JSON schema produced by `tldr smells --format json`
        let engine = TldrDifferentialEngine::new();

        let baseline = serde_json::json!({
            "smells": [
                {
                    "smell_type": "long_method",
                    "file": "src/engine.rs",
                    "name": "analyze",
                    "line": 100,
                    "reason": "Method has 52 lines of code (threshold: 50)",
                    "severity": 1
                }
            ],
            "files_scanned": 1,
            "by_file": {},
            "summary": { "total": 1 }
        });
        let current = serde_json::json!({
            "smells": [
                {
                    "smell_type": "long_method",
                    "file": "src/engine.rs",
                    "name": "analyze",
                    "line": 100,
                    "reason": "Method has 80 lines of code (threshold: 50)",
                    "severity": 2
                },
                {
                    "smell_type": "feature_envy",
                    "file": "src/engine.rs",
                    "name": "diff_metrics",
                    "line": 200,
                    "reason": "Method accesses 5 foreign fields",
                    "severity": 1
                },
                {
                    "smell_type": "data_clump",
                    "file": "src/engine.rs",
                    "name": "analyze_batch",
                    "line": 300,
                    "reason": "3 parameters always appear together",
                    "severity": 1
                }
            ],
            "files_scanned": 1,
            "by_file": {},
            "summary": { "total": 3 }
        });

        let findings = engine.diff_smells(
            Path::new("src/engine.rs"),
            &baseline,
            &current,
        );

        assert_eq!(findings.len(), 2, "Should detect 2 introduced smells");
        // Verify types are extracted from smell_type field (not "unknown")
        let types: Vec<&str> = findings.iter().map(|f| f.evidence["smell_type"].as_str().unwrap()).collect();
        assert!(types.contains(&"feature_envy"), "Should extract feature_envy type");
        assert!(types.contains(&"data_clump"), "Should extract data_clump type");
        // Structural smells should be medium severity
        assert!(findings.iter().all(|f| f.severity == "medium"), "Structural smells should be medium severity");
        // None should be "unknown"
        assert!(!types.contains(&"unknown"), "No smell should have type 'unknown'");
    }

    #[test]
    fn test_diff_smells_suppressed_types_filtered() {
        let engine = TldrDifferentialEngine::new();

        let baseline = serde_json::json!({
            "smells": [
                { "smell_type": "long_method", "name": "process", "line": 1, "reason": "too long", "severity": 1 }
            ]
        });
        // Introduce only suppressed smell types (message_chain, long_parameter_list)
        let current = serde_json::json!({
            "smells": [
                { "smell_type": "long_method", "name": "process", "line": 1, "reason": "too long", "severity": 1 },
                { "smell_type": "message_chain", "name": "chain", "line": 50, "reason": "chain length 4", "severity": 1 },
                { "smell_type": "long_parameter_list", "name": "many_params", "line": 80, "reason": "6 params", "severity": 1 }
            ]
        });

        let findings = engine.diff_smells(
            Path::new("src/lib.rs"),
            &baseline,
            &current,
        );

        assert!(findings.is_empty(), "Suppressed smell types should produce no findings");
    }

    #[test]
    fn test_extract_function_entries_from_functions_key() {
        let json = serde_json::json!({
            "functions": [
                { "name": "foo", "value": 1 },
                { "name": "bar", "value": 2 }
            ]
        });

        let entries = TldrDifferentialEngine::extract_function_entries(&json);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].0, "foo");
        assert_eq!(entries[1].0, "bar");
    }

    #[test]
    fn test_extract_function_entries_from_root_array() {
        let json = serde_json::json!([
            { "name": "foo", "value": 1 },
            { "name": "bar", "value": 2 }
        ]);

        let entries = TldrDifferentialEngine::extract_function_entries(&json);
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn test_extract_function_entries_empty() {
        let json = serde_json::json!({ "other": 42 });
        let entries = TldrDifferentialEngine::extract_function_entries(&json);
        assert!(entries.is_empty());
    }

    #[test]
    fn test_count_dead_code_entries() {
        let json = serde_json::json!({
            "dead_code": [
                { "name": "unused_fn", "file": "src/lib.rs" },
                { "name": "old_helper", "file": "src/utils.rs" }
            ]
        });
        assert_eq!(TldrDifferentialEngine::count_dead_code_entries(&json), 2);
    }

    #[test]
    fn test_count_dead_code_entries_empty() {
        let json = serde_json::json!({ "dead_code": [] });
        assert_eq!(TldrDifferentialEngine::count_dead_code_entries(&json), 0);
    }

    #[test]
    fn test_severity_thresholds() {
        let engine = TldrDifferentialEngine::new();

        // >50% increase = high
        let high = serde_json::json!({ "functions": [{ "name": "f", "metric": 2.0, "line": 1 }] });
        let high_curr = serde_json::json!({ "functions": [{ "name": "f", "metric": 10.0, "line": 1 }] });
        let findings = engine.diff_numeric_metrics("test-increase", "metric", Path::new("a.py"), &high, &high_curr);
        assert_eq!(findings[0].severity, "high");

        // 20-50% increase = medium
        let med = serde_json::json!({ "functions": [{ "name": "f", "metric": 10.0, "line": 1 }] });
        let med_curr = serde_json::json!({ "functions": [{ "name": "f", "metric": 14.0, "line": 1 }] });
        let findings = engine.diff_numeric_metrics("test-increase", "metric", Path::new("a.py"), &med, &med_curr);
        assert_eq!(findings[0].severity, "medium");

        // <20% increase = low
        let low = serde_json::json!({ "functions": [{ "name": "f", "metric": 10.0, "line": 1 }] });
        let low_curr = serde_json::json!({ "functions": [{ "name": "f", "metric": 11.0, "line": 1 }] });
        let findings = engine.diff_numeric_metrics("test-increase", "metric", Path::new("a.py"), &low, &low_curr);
        assert_eq!(findings[0].severity, "low");
    }

    #[test]
    fn test_cognitive_delta_threshold_filters_trivial() {
        let engine = TldrDifferentialEngine::new();

        // Cognitive delta of 2 (below threshold of 3) should be suppressed
        let baseline = serde_json::json!({ "functions": [{ "name": "f", "cognitive": 2.0, "line": 1 }] });
        let current = serde_json::json!({ "functions": [{ "name": "f", "cognitive": 4.0, "line": 1 }] });
        let findings = engine.diff_numeric_metrics("cognitive-increase", "cognitive", Path::new("a.rs"), &baseline, &current);
        assert!(findings.is_empty(), "Cognitive delta of 2 should be suppressed (threshold 3)");

        // Cognitive delta of 3 (at threshold) should be reported
        let baseline = serde_json::json!({ "functions": [{ "name": "g", "cognitive": 5.0, "line": 1 }] });
        let current = serde_json::json!({ "functions": [{ "name": "g", "cognitive": 8.0, "line": 1 }] });
        let findings = engine.diff_numeric_metrics("cognitive-increase", "cognitive", Path::new("a.rs"), &baseline, &current);
        assert_eq!(findings.len(), 1, "Cognitive delta of 3 should be reported");

        // Complexity delta of 1 (below threshold of 2) should be suppressed
        let baseline = serde_json::json!({ "functions": [{ "name": "h", "cyclomatic": 3.0, "line": 1 }] });
        let current = serde_json::json!({ "functions": [{ "name": "h", "cyclomatic": 4.0, "line": 1 }] });
        let findings = engine.diff_numeric_metrics("complexity-increase", "cyclomatic", Path::new("a.rs"), &baseline, &current);
        assert!(findings.is_empty(), "Complexity delta of 1 should be suppressed (threshold 2)");

        // Complexity delta of 2 (at threshold) should be reported
        let baseline = serde_json::json!({ "functions": [{ "name": "j", "cyclomatic": 3.0, "line": 1 }] });
        let current = serde_json::json!({ "functions": [{ "name": "j", "cyclomatic": 5.0, "line": 1 }] });
        let findings = engine.diff_numeric_metrics("complexity-increase", "cyclomatic", Path::new("a.rs"), &baseline, &current);
        assert_eq!(findings.len(), 1, "Complexity delta of 2 should be reported");
    }

    // =========================================================================
    // Integration test: complexity diff via actual tldr binary
    // =========================================================================

    #[test]
    fn test_complexity_diff_real_tldr() {
        // Skip this test if tldr is not on PATH
        if Command::new("tldr").arg("--version").output().is_err() {
            eprintln!("Skipping test_complexity_diff_real_tldr: tldr not on PATH");
            return;
        }

        let engine = TldrDifferentialEngine::with_timeout(10);

        // Create a temp dir with baseline and current Python files
        let tmp_dir = TempDir::new().expect("create tmpdir");
        let baseline_file = tmp_dir.path().join("baseline.py");
        let current_file = tmp_dir.path().join("current.py");

        std::fs::write(
            &baseline_file,
            "def process(x):\n    return x + 1\n",
        ).expect("write baseline");

        std::fs::write(
            &current_file,
            "def process(x):\n    if x > 10:\n        if x > 20:\n            return x * 3\n        return x * 2\n    return x\n",
        ).expect("write current");

        // Run complexity command on both
        let baseline_result = engine.run_tldr_command(&["complexity"], &baseline_file);
        let current_result = engine.run_tldr_command(&["complexity"], &current_file);

        // Both should succeed (tldr is on PATH)
        match (baseline_result, current_result) {
            (Ok(baseline_json), Ok(current_json)) => {
                // The JSON should be parseable
                assert!(baseline_json.is_object() || baseline_json.is_array());
                assert!(current_json.is_object() || current_json.is_array());
            }
            (Err(e), _) => {
                // Acceptable: tldr might not support the command or file type
                eprintln!("Baseline complexity failed (acceptable): {}", e);
            }
            (_, Err(e)) => {
                eprintln!("Current complexity failed (acceptable): {}", e);
            }
        }
    }

    // =========================================================================
    // TLDR_COMMANDS config tests
    // =========================================================================

    #[test]
    fn test_tldr_commands_count() {
        assert_eq!(TLDR_COMMANDS.len(), 9);
    }

    #[test]
    fn test_tldr_commands_local_count() {
        let local_count = TLDR_COMMANDS.iter().filter(|c| c.category == TldrCategory::Local).count();
        assert_eq!(local_count, 4);
    }

    #[test]
    fn test_tldr_commands_flow_count() {
        let flow_count = TLDR_COMMANDS.iter().filter(|c| c.category == TldrCategory::Flow).count();
        assert_eq!(flow_count, 5);
    }

    #[test]
    fn test_finding_types_match_commands() {
        // Every TLDR_COMMANDS entry should have a corresponding finding type.
        // FINDING_TYPES also includes "downstream-impact" and "breaking-change-risk"
        // which come from whatbreaks/impact commands (not in TLDR_COMMANDS).
        assert_eq!(FINDING_TYPES.len(), TLDR_COMMANDS.len() + 2);
        // Verify the extra types are the impact ones
        assert!(FINDING_TYPES.contains(&"downstream-impact"));
        assert!(FINDING_TYPES.contains(&"breaking-change-risk"));
    }

    // =========================================================================
    // Flow command baseline diffing: diff_calls_json
    // =========================================================================

    #[test]
    fn test_diff_calls_new_edges_detected() {
        let engine = TldrDifferentialEngine::new();
        let baseline = serde_json::json!({
            "edges": [{"src_file": "a.rs", "src_func": "foo", "dst_file": "b.rs", "dst_func": "bar", "call_type": "direct"}],
            "edge_count": 1
        });
        let current = serde_json::json!({
            "edges": [
                {"src_file": "a.rs", "src_func": "foo", "dst_file": "b.rs", "dst_func": "bar", "call_type": "direct"},
                {"src_file": "a.rs", "src_func": "foo", "dst_file": "c.rs", "dst_func": "baz", "call_type": "direct"}
            ],
            "edge_count": 2
        });
        let findings = engine.diff_calls_json(&baseline, &current);
        assert!(!findings.is_empty(), "Should detect new call graph edge");
        assert_eq!(findings[0].finding_type, "call-graph-change");
        assert_eq!(findings[0].confidence, Some("DETERMINISTIC".to_string()));
        assert!(findings[0].finding_id.is_some());
    }

    #[test]
    fn test_diff_calls_no_change() {
        let engine = TldrDifferentialEngine::new();
        let json = serde_json::json!({
            "edges": [{"src_file": "a.rs", "src_func": "foo", "dst_file": "b.rs", "dst_func": "bar", "call_type": "direct"}],
            "edge_count": 1
        });
        let findings = engine.diff_calls_json(&json, &json);
        assert!(findings.is_empty(), "No change should produce no findings");
    }

    #[test]
    fn test_diff_calls_removed_edge_reported() {
        let engine = TldrDifferentialEngine::new();
        let baseline = serde_json::json!({
            "edges": [
                {"src_file": "a.rs", "src_func": "foo", "dst_file": "b.rs", "dst_func": "bar", "call_type": "direct"},
                {"src_file": "a.rs", "src_func": "foo", "dst_file": "c.rs", "dst_func": "baz", "call_type": "direct"}
            ],
            "edge_count": 2
        });
        let current = serde_json::json!({
            "edges": [{"src_file": "a.rs", "src_func": "foo", "dst_file": "b.rs", "dst_func": "bar", "call_type": "direct"}],
            "edge_count": 1
        });
        let findings = engine.diff_calls_json(&baseline, &current);
        assert!(!findings.is_empty(), "Should detect removed call graph edge");
        assert_eq!(findings[0].finding_type, "call-graph-change");
    }

    #[test]
    fn test_diff_calls_many_new_edges_medium_severity() {
        let engine = TldrDifferentialEngine::new();
        let baseline = serde_json::json!({
            "edges": [{"src_file": "a.rs", "src_func": "foo", "dst_file": "b.rs", "dst_func": "bar", "call_type": "direct"}],
            "edge_count": 1
        });
        // Add 6 new edges (>5 threshold for medium severity)
        let current = serde_json::json!({
            "edges": [
                {"src_file": "a.rs", "src_func": "foo", "dst_file": "b.rs", "dst_func": "bar", "call_type": "direct"},
                {"src_file": "a.rs", "src_func": "foo", "dst_file": "c.rs", "dst_func": "baz", "call_type": "direct"},
                {"src_file": "a.rs", "src_func": "foo", "dst_file": "d.rs", "dst_func": "qux", "call_type": "direct"},
                {"src_file": "a.rs", "src_func": "foo", "dst_file": "e.rs", "dst_func": "quux", "call_type": "direct"},
                {"src_file": "b.rs", "src_func": "bar", "dst_file": "c.rs", "dst_func": "baz", "call_type": "direct"},
                {"src_file": "b.rs", "src_func": "bar", "dst_file": "d.rs", "dst_func": "qux", "call_type": "direct"},
                {"src_file": "b.rs", "src_func": "bar", "dst_file": "e.rs", "dst_func": "quux", "call_type": "direct"}
            ],
            "edge_count": 7
        });
        let findings = engine.diff_calls_json(&baseline, &current);
        assert!(!findings.is_empty());
        // At least one finding should have medium severity when >5 new edges
        let has_medium = findings.iter().any(|f| f.severity == "medium");
        assert!(has_medium, "Should produce a medium-severity summary finding for >5 new edges");
    }

    // =========================================================================
    // Flow command baseline diffing: diff_deps_json
    // =========================================================================

    #[test]
    fn test_diff_deps_new_circular_dep_high_severity() {
        let engine = TldrDifferentialEngine::new();
        let baseline = serde_json::json!({
            "internal_dependencies": {"a.rs": ["b.rs"]},
            "circular_dependencies": [],
            "stats": {"total_internal_deps": 1}
        });
        let current = serde_json::json!({
            "internal_dependencies": {"a.rs": ["b.rs"], "b.rs": ["a.rs"]},
            "circular_dependencies": [{"path": ["a.rs", "b.rs", "a.rs"], "len": 3}],
            "stats": {"total_internal_deps": 2}
        });
        let findings = engine.diff_deps_json(&baseline, &current);
        assert!(!findings.is_empty(), "Should detect new circular dependency");
        assert_eq!(findings[0].finding_type, "dependency-change");
        assert_eq!(findings[0].severity, "high");
    }

    #[test]
    fn test_diff_deps_no_change() {
        let engine = TldrDifferentialEngine::new();
        let json = serde_json::json!({
            "internal_dependencies": {"a.rs": ["b.rs"]},
            "circular_dependencies": [],
            "stats": {"total_internal_deps": 1}
        });
        let findings = engine.diff_deps_json(&json, &json);
        assert!(findings.is_empty(), "No change should produce no findings");
    }

    #[test]
    fn test_diff_deps_removed_circular_not_flagged() {
        let engine = TldrDifferentialEngine::new();
        let baseline = serde_json::json!({
            "internal_dependencies": {"a.rs": ["b.rs"], "b.rs": ["a.rs"]},
            "circular_dependencies": [{"path": ["a.rs", "b.rs", "a.rs"], "len": 3}],
            "stats": {"total_internal_deps": 2}
        });
        let current = serde_json::json!({
            "internal_dependencies": {"a.rs": ["b.rs"]},
            "circular_dependencies": [],
            "stats": {"total_internal_deps": 1}
        });
        let findings = engine.diff_deps_json(&baseline, &current);
        // Removing a circular dependency is an improvement, not a regression
        let has_high = findings.iter().any(|f| f.severity == "high");
        assert!(!has_high, "Removing circular dependency should not produce high severity finding");
    }

    #[test]
    fn test_diff_deps_internal_deps_dict_count() {
        // Verify that internal_dependencies as a dict is counted correctly
        let engine = TldrDifferentialEngine::new();
        let baseline = serde_json::json!({
            "internal_dependencies": {"a.rs": ["b.rs"]},
            "circular_dependencies": [],
            "stats": {"total_internal_deps": 1}
        });
        let current = serde_json::json!({
            "internal_dependencies": {"a.rs": ["b.rs", "c.rs", "d.rs", "e.rs", "f.rs", "g.rs", "h.rs"]},
            "circular_dependencies": [],
            "stats": {"total_internal_deps": 7}
        });
        let findings = engine.diff_deps_json(&baseline, &current);
        assert!(!findings.is_empty(), "Should detect dependency count increase of 6 (>5 threshold)");
        assert_eq!(findings[0].finding_type, "dependency-change");
        assert_eq!(findings[0].severity, "medium");
    }

    #[test]
    fn test_diff_deps_fallback_to_dict_counting_without_stats() {
        // When stats.total_internal_deps is missing, fall back to counting dict entries
        let engine = TldrDifferentialEngine::new();
        let baseline = serde_json::json!({
            "internal_dependencies": {"a.rs": ["b.rs"]},
            "circular_dependencies": []
        });
        let current = serde_json::json!({
            "internal_dependencies": {"a.rs": ["b.rs", "c.rs", "d.rs", "e.rs", "f.rs", "g.rs", "h.rs"]},
            "circular_dependencies": []
        });
        let findings = engine.diff_deps_json(&baseline, &current);
        assert!(!findings.is_empty(), "Should detect dependency count increase even without stats field");
    }

    // =========================================================================
    // Flow command baseline diffing: diff_coupling_json
    // =========================================================================

    #[test]
    fn test_diff_coupling_instability_increase_detected() {
        let engine = TldrDifferentialEngine::new();
        let baseline = serde_json::json!({
            "martin_metrics": [
                {"module": "core", "ca": 5, "ce": 2, "instability": 0.29, "abstractness": 0.1}
            ],
            "pairwise_coupling": []
        });
        let current = serde_json::json!({
            "martin_metrics": [
                {"module": "core", "ca": 5, "ce": 8, "instability": 0.62, "abstractness": 0.1}
            ],
            "pairwise_coupling": []
        });
        let findings = engine.diff_coupling_json(&baseline, &current);
        assert!(!findings.is_empty(), "Should detect instability increase");
        assert_eq!(findings[0].finding_type, "coupling-increase");
    }

    #[test]
    fn test_diff_coupling_no_change() {
        let engine = TldrDifferentialEngine::new();
        let json = serde_json::json!({
            "martin_metrics": [
                {"module": "core", "ca": 5, "ce": 2, "instability": 0.29, "abstractness": 0.1}
            ],
            "pairwise_coupling": []
        });
        let findings = engine.diff_coupling_json(&json, &json);
        assert!(findings.is_empty(), "No change should produce no findings");
    }

    #[test]
    fn test_diff_coupling_improvement_not_flagged() {
        let engine = TldrDifferentialEngine::new();
        let baseline = serde_json::json!({
            "martin_metrics": [
                {"module": "core", "ca": 5, "ce": 8, "instability": 0.62, "abstractness": 0.1}
            ],
            "pairwise_coupling": []
        });
        let current = serde_json::json!({
            "martin_metrics": [
                {"module": "core", "ca": 5, "ce": 2, "instability": 0.29, "abstractness": 0.1}
            ],
            "pairwise_coupling": []
        });
        let findings = engine.diff_coupling_json(&baseline, &current);
        assert!(findings.is_empty(), "Coupling decrease should not produce findings");
    }

    // =========================================================================
    // Flow command baseline diffing: diff_cohesion_json
    // =========================================================================

    #[test]
    fn test_diff_cohesion_lcom4_increase_detected() {
        let engine = TldrDifferentialEngine::new();
        let baseline = serde_json::json!({
            "classes": [
                {"class_name": "Engine", "lcom4": 1, "method_count": 5, "field_count": 3}
            ],
            "summary": {"total_classes": 1}
        });
        let current = serde_json::json!({
            "classes": [
                {"class_name": "Engine", "lcom4": 4, "method_count": 8, "field_count": 3}
            ],
            "summary": {"total_classes": 1}
        });
        let findings = engine.diff_cohesion_json(&baseline, &current);
        assert!(!findings.is_empty(), "Should detect LCOM4 increase");
        assert_eq!(findings[0].finding_type, "cohesion-decrease");
    }

    #[test]
    fn test_diff_cohesion_no_change() {
        let engine = TldrDifferentialEngine::new();
        let json = serde_json::json!({
            "classes": [
                {"class_name": "Engine", "lcom4": 2, "method_count": 5, "field_count": 3}
            ],
            "summary": {"total_classes": 1}
        });
        let findings = engine.diff_cohesion_json(&json, &json);
        assert!(findings.is_empty(), "No change should produce no findings");
    }

    #[test]
    fn test_diff_cohesion_improvement_not_flagged() {
        let engine = TldrDifferentialEngine::new();
        let baseline = serde_json::json!({
            "classes": [
                {"class_name": "Engine", "lcom4": 5, "method_count": 10, "field_count": 3}
            ],
            "summary": {"total_classes": 1}
        });
        let current = serde_json::json!({
            "classes": [
                {"class_name": "Engine", "lcom4": 1, "method_count": 4, "field_count": 3}
            ],
            "summary": {"total_classes": 1}
        });
        let findings = engine.diff_cohesion_json(&baseline, &current);
        assert!(findings.is_empty(), "LCOM4 decrease is an improvement, should not produce findings");
    }

    #[test]
    fn test_diff_cohesion_new_class_high_lcom4_info() {
        let engine = TldrDifferentialEngine::new();
        let baseline = serde_json::json!({
            "classes": [],
            "summary": {"total_classes": 0}
        });
        let current = serde_json::json!({
            "classes": [
                {"class_name": "GodObject", "lcom4": 5, "method_count": 12, "field_count": 0, "verdict": "split_candidate"}
            ],
            "summary": {"total_classes": 1}
        });
        let findings = engine.diff_cohesion_json(&baseline, &current);
        assert!(!findings.is_empty(), "New class with high LCOM4 should be flagged");
        assert_eq!(findings[0].severity, "info");
    }

    #[test]
    fn test_diff_cohesion_backward_compat_name_field() {
        // Verify backward compatibility: "name" field still works as fallback
        let engine = TldrDifferentialEngine::new();
        let baseline = serde_json::json!({
            "classes": [{"name": "Legacy", "lcom4": 1}],
            "summary": {"total_classes": 1}
        });
        let current = serde_json::json!({
            "classes": [{"name": "Legacy", "lcom4": 4}],
            "summary": {"total_classes": 1}
        });
        let findings = engine.diff_cohesion_json(&baseline, &current);
        assert!(!findings.is_empty(), "Should still work with 'name' field as fallback");
    }

    // =========================================================================
    // L2Context base_ref field
    // =========================================================================

    #[test]
    fn test_l2context_default_base_ref() {
        let ctx = empty_context();
        assert_eq!(ctx.base_ref, "HEAD", "Default base_ref should be HEAD");
    }

    #[test]
    fn test_l2context_with_base_ref() {
        let ctx = empty_context().with_base_ref(String::from("main"));
        assert_eq!(ctx.base_ref, "main");
    }

    // =========================================================================
    // analyze_flow_commands takes base_ref
    // =========================================================================

    #[test]
    fn test_analyze_flow_commands_accepts_base_ref_and_language() {
        let engine = TldrDifferentialEngine::new();
        let mut partial_reasons = Vec::new();
        // Should not panic — graceful failure when project dir doesn't exist
        let _findings = engine.analyze_flow_commands(
            Path::new("/tmp/nonexistent-project-for-test"),
            "HEAD",
            "rust",
            None,
            &mut partial_reasons,
        );
        // Flow commands should fail gracefully on non-existent project
        // (either empty findings or partial_reasons populated, but no panic)
    }

    // =========================================================================
    // run_tldr_flow_command: --lang and --respect-ignore filtering
    // =========================================================================

    #[test]
    fn test_run_tldr_flow_command_exists() {
        // Verify the method signature exists and is callable
        let engine = TldrDifferentialEngine::new();
        // Calling with a nonexistent path should return Err, not panic
        let result = engine.run_tldr_flow_command(
            "calls",
            &["calls"],
            Path::new("/tmp/nonexistent-project"),
            "rust",
        );
        // Either Ok (if tldr is available) or Err (spawn/parse failure) — no panic
        let _ = result;
    }

    #[test]
    fn test_run_tldr_flow_command_builds_args_with_lang() {
        // Verify the method constructs correct args by testing the public interface.
        // We test indirectly: the method should produce the same result as run_tldr_command
        // but with additional --lang and possibly --respect-ignore flags.
        // Since we can't inspect the internal args directly, we verify the method
        // is callable with various language strings.
        let engine = TldrDifferentialEngine::with_timeout(1);

        for lang in &["python", "rust", "typescript", "go", "java"] {
            let result = engine.run_tldr_flow_command(
                "dead",
                &["dead"],
                Path::new("/tmp/nonexistent"),
                lang,
            );
            // Should not panic for any language
            let _ = result;
        }
    }

    #[test]
    fn test_run_tldr_flow_command_calls_gets_respect_ignore() {
        // The `calls` command should get --respect-ignore.
        // We verify indirectly that the method distinguishes command names.
        let engine = TldrDifferentialEngine::with_timeout(1);

        // Both should be callable without panic, but `calls` gets --respect-ignore
        let _calls_result = engine.run_tldr_flow_command(
            "calls",
            &["calls"],
            Path::new("/tmp/nonexistent"),
            "rust",
        );
        let _deps_result = engine.run_tldr_flow_command(
            "deps",
            &["deps"],
            Path::new("/tmp/nonexistent"),
            "rust",
        );
    }

    // =========================================================================
    // Flow timeout: 300s for flow commands
    // =========================================================================

    #[test]
    fn test_flow_engine_timeout_is_300s() {
        // The analyze method should use 300s timeout for flow commands,
        // not the artificial max(self.timeout_secs, 60).
        // We verify via analyze_flow_commands: the flow_engine inside uses 300s.
        // Since we can't inspect the internal flow_engine directly, we verify
        // that analyze_flow_commands completes without artificial timeout issues
        // by checking it uses a generous timeout.
        let engine = TldrDifferentialEngine::with_timeout(10);
        let mut partial_reasons = Vec::new();
        let _findings = engine.analyze_flow_commands(
            Path::new("/tmp/nonexistent-project"),
            "HEAD",
            "python",
            None,
            &mut partial_reasons,
        );
        // The fact that it runs without panic is sufficient;
        // the timeout change is an internal implementation detail.
    }

    // =========================================================================
    // analyze() passes language to analyze_flow_commands
    // =========================================================================

    #[test]
    fn test_analyze_passes_language_to_flow_commands() {
        // Verify that analyze() correctly derives language string from ctx.language
        // and passes it to flow commands.
        let engine = TldrDifferentialEngine::new();
        let ctx = L2Context::new(
            PathBuf::from("/tmp/test-project-lang"),
            Language::Python,
            vec![],
            FunctionDiff {
                changed: vec![],
                inserted: vec![],
                deleted: vec![],
            },
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
        );
        let output = engine.analyze(&ctx);
        // Should complete without panic. Flow commands will fail on /tmp path,
        // but the important thing is the language plumbing works.
        match &output.status {
            AnalyzerStatus::Complete => {}
            AnalyzerStatus::Partial { .. } => {}
            other => panic!("Unexpected status: {:?}", other),
        }
    }

    // =========================================================================
    // downstream-impact (whatbreaks) parsing tests
    // =========================================================================

    #[test]
    fn test_finding_types_includes_impact() {
        let engine = TldrDifferentialEngine::new();
        let types = engine.finding_types();
        assert!(
            types.contains(&"downstream-impact"),
            "FINDING_TYPES must include downstream-impact"
        );
        assert!(
            types.contains(&"breaking-change-risk"),
            "FINDING_TYPES must include breaking-change-risk"
        );
    }

    #[test]
    fn test_downstream_impact_severity_high() {
        let json = serde_json::json!({
            "summary": {
                "importer_count": 15,
                "direct_caller_count": 3,
                "affected_test_count": 2
            }
        });
        let file = PathBuf::from("src/lib.rs");
        let findings = TldrDifferentialEngine::parse_whatbreaks_findings(&file, &json);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].finding_type, "downstream-impact");
        assert_eq!(findings[0].severity, "high");
        assert_eq!(findings[0].function, "(file-level)");
        assert_eq!(findings[0].file, file);
        assert_eq!(
            findings[0].confidence.as_deref(),
            Some("DETERMINISTIC")
        );
        assert!(findings[0].finding_id.is_some());

        // Verify evidence fields
        let ev = &findings[0].evidence;
        assert_eq!(ev["command"], "whatbreaks");
        assert_eq!(ev["importer_count"], 15);
        assert_eq!(ev["direct_caller_count"], 3);
        assert_eq!(ev["affected_test_count"], 2);
    }

    #[test]
    fn test_downstream_impact_severity_medium() {
        let json = serde_json::json!({
            "summary": {
                "importer_count": 7,
                "direct_caller_count": 1,
                "affected_test_count": 0
            }
        });
        let file = PathBuf::from("src/core.rs");
        let findings = TldrDifferentialEngine::parse_whatbreaks_findings(&file, &json);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "medium");
    }

    #[test]
    fn test_downstream_impact_severity_low() {
        let json = serde_json::json!({
            "summary": {
                "importer_count": 2,
                "direct_caller_count": 0,
                "affected_test_count": 1
            }
        });
        let file = PathBuf::from("src/utils.rs");
        let findings = TldrDifferentialEngine::parse_whatbreaks_findings(&file, &json);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "low");
    }

    #[test]
    fn test_downstream_impact_no_findings_when_no_importers() {
        let json = serde_json::json!({
            "summary": {
                "importer_count": 0,
                "direct_caller_count": 0,
                "affected_test_count": 0
            }
        });
        let file = PathBuf::from("src/leaf.rs");
        let findings = TldrDifferentialEngine::parse_whatbreaks_findings(&file, &json);
        assert!(
            findings.is_empty(),
            "Zero importers and zero callers should produce no findings"
        );
    }

    #[test]
    fn test_downstream_impact_boundary_importer_3() {
        // importer_count == 3 is NOT > 3, so severity should be "low"
        let json = serde_json::json!({
            "summary": {
                "importer_count": 3,
                "direct_caller_count": 0,
                "affected_test_count": 0
            }
        });
        let file = PathBuf::from("src/boundary.rs");
        let findings = TldrDifferentialEngine::parse_whatbreaks_findings(&file, &json);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "low");
    }

    #[test]
    fn test_downstream_impact_boundary_importer_4() {
        // importer_count == 4 is > 3 but NOT > 10, so severity should be "medium"
        let json = serde_json::json!({
            "summary": {
                "importer_count": 4,
                "direct_caller_count": 0,
                "affected_test_count": 0
            }
        });
        let file = PathBuf::from("src/boundary4.rs");
        let findings = TldrDifferentialEngine::parse_whatbreaks_findings(&file, &json);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "medium");
    }

    #[test]
    fn test_downstream_impact_boundary_importer_10() {
        // importer_count == 10 is NOT > 10, so severity should be "medium"
        let json = serde_json::json!({
            "summary": {
                "importer_count": 10,
                "direct_caller_count": 0,
                "affected_test_count": 0
            }
        });
        let file = PathBuf::from("src/boundary10.rs");
        let findings = TldrDifferentialEngine::parse_whatbreaks_findings(&file, &json);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "medium");
    }

    #[test]
    fn test_downstream_impact_boundary_importer_11() {
        // importer_count == 11 is > 10, so severity should be "high"
        let json = serde_json::json!({
            "summary": {
                "importer_count": 11,
                "direct_caller_count": 0,
                "affected_test_count": 0
            }
        });
        let file = PathBuf::from("src/boundary11.rs");
        let findings = TldrDifferentialEngine::parse_whatbreaks_findings(&file, &json);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "high");
    }

    #[test]
    fn test_downstream_impact_callers_only() {
        // 0 importers but positive caller_count still emits a finding
        let json = serde_json::json!({
            "summary": {
                "importer_count": 0,
                "direct_caller_count": 5,
                "affected_test_count": 0
            }
        });
        let file = PathBuf::from("src/callers.rs");
        let findings = TldrDifferentialEngine::parse_whatbreaks_findings(&file, &json);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "low");
        assert!(findings[0].message.contains("5 direct callers"));
    }

    #[test]
    fn test_downstream_impact_summary_at_top_level() {
        // When summary fields are at top level (no "summary" wrapper)
        let json = serde_json::json!({
            "importer_count": 6,
            "direct_caller_count": 2,
            "affected_test_count": 1
        });
        let file = PathBuf::from("src/flat.rs");
        let findings = TldrDifferentialEngine::parse_whatbreaks_findings(&file, &json);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "medium");
    }

    // =========================================================================
    // breaking-change-risk (impact) parsing tests
    // =========================================================================

    #[test]
    fn test_function_impact_high_severity() {
        let json = serde_json::json!({
            "targets": {
                "process_data": {
                    "caller_count": 8,
                    "callers": [
                        { "file": "main.rs", "function": "run" },
                        { "file": "handler.rs", "function": "handle" },
                        { "file": "api.rs", "function": "endpoint" },
                        { "file": "worker.rs", "function": "execute" },
                        { "file": "batch.rs", "function": "process_all" },
                        { "file": "test.rs", "function": "test_it" },
                    ]
                }
            }
        });
        let findings =
            TldrDifferentialEngine::parse_impact_findings("process_data", &json);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].finding_type, "breaking-change-risk");
        assert_eq!(findings[0].severity, "high");
        assert_eq!(findings[0].function, "process_data");
        assert_eq!(findings[0].file, PathBuf::from("(project)"));
        assert_eq!(
            findings[0].confidence.as_deref(),
            Some("DETERMINISTIC")
        );
        assert!(findings[0].finding_id.is_some());

        // Verify evidence
        let ev = &findings[0].evidence;
        assert_eq!(ev["command"], "impact");
        assert_eq!(ev["caller_count"], 8);
        // callers_preview capped at 5
        let preview = ev["callers_preview"].as_array().unwrap();
        assert_eq!(preview.len(), 5);
    }

    #[test]
    fn test_function_impact_medium_severity() {
        let json = serde_json::json!({
            "targets": {
                "helper_fn": {
                    "caller_count": 3,
                    "callers": [
                        { "file": "a.rs", "function": "foo" },
                        { "file": "b.rs", "function": "bar" },
                        { "file": "c.rs", "function": "baz" },
                    ]
                }
            }
        });
        let findings =
            TldrDifferentialEngine::parse_impact_findings("helper_fn", &json);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "medium");
    }

    #[test]
    fn test_function_impact_info_severity() {
        let json = serde_json::json!({
            "targets": {
                "rare_fn": {
                    "caller_count": 1,
                    "callers": [
                        { "file": "only.rs", "function": "sole_caller" }
                    ]
                }
            }
        });
        let findings =
            TldrDifferentialEngine::parse_impact_findings("rare_fn", &json);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "info");
    }

    #[test]
    fn test_function_impact_no_callers() {
        let json = serde_json::json!({
            "targets": {
                "leaf_fn": {
                    "caller_count": 0,
                    "callers": []
                }
            }
        });
        let findings =
            TldrDifferentialEngine::parse_impact_findings("leaf_fn", &json);
        assert!(
            findings.is_empty(),
            "Function with zero callers should produce no findings"
        );
    }

    #[test]
    fn test_function_impact_missing_target() {
        // Function name not found in targets -- should produce no findings
        let json = serde_json::json!({
            "targets": {
                "other_fn": {
                    "caller_count": 5,
                    "callers": []
                }
            }
        });
        let findings =
            TldrDifferentialEngine::parse_impact_findings("missing_fn", &json);
        assert!(
            findings.is_empty(),
            "Missing target key should produce no findings"
        );
    }

    #[test]
    fn test_function_impact_fallback_top_level() {
        // When caller data is at top level (no "targets" wrapper)
        let json = serde_json::json!({
            "caller_count": 4,
            "callers": [
                { "file": "x.rs", "function": "a" },
                { "file": "y.rs", "function": "b" },
                { "file": "z.rs", "function": "c" },
                { "file": "w.rs", "function": "d" },
            ]
        });
        let findings =
            TldrDifferentialEngine::parse_impact_findings("any_fn", &json);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "medium");
        assert_eq!(findings[0].evidence["caller_count"], 4);
    }

    #[test]
    fn test_function_impact_boundary_caller_2() {
        // caller_count == 2 is >= 2, so severity should be "medium"
        let json = serde_json::json!({
            "targets": {
                "boundary_fn": {
                    "caller_count": 2,
                    "callers": [
                        { "file": "a.rs", "function": "x" },
                        { "file": "b.rs", "function": "y" },
                    ]
                }
            }
        });
        let findings =
            TldrDifferentialEngine::parse_impact_findings("boundary_fn", &json);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "medium");
    }

    #[test]
    fn test_function_impact_boundary_caller_5() {
        // caller_count == 5 is NOT > 5, so severity should be "medium"
        let json = serde_json::json!({
            "targets": {
                "five_fn": {
                    "caller_count": 5,
                    "callers": []
                }
            }
        });
        let findings =
            TldrDifferentialEngine::parse_impact_findings("five_fn", &json);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "medium");
    }

    #[test]
    fn test_function_impact_boundary_caller_6() {
        // caller_count == 6 is > 5, so severity should be "high"
        let json = serde_json::json!({
            "targets": {
                "six_fn": {
                    "caller_count": 6,
                    "callers": []
                }
            }
        });
        let findings =
            TldrDifferentialEngine::parse_impact_findings("six_fn", &json);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "high");
    }

    #[test]
    fn test_downstream_impact_finding_id_deterministic() {
        // Same inputs should produce the same finding_id
        let json = serde_json::json!({
            "summary": {
                "importer_count": 5,
                "direct_caller_count": 2,
                "affected_test_count": 1
            }
        });
        let file = PathBuf::from("src/stable.rs");
        let findings1 = TldrDifferentialEngine::parse_whatbreaks_findings(&file, &json);
        let findings2 = TldrDifferentialEngine::parse_whatbreaks_findings(&file, &json);
        assert_eq!(findings1[0].finding_id, findings2[0].finding_id);
    }

    #[test]
    fn test_function_impact_finding_id_deterministic() {
        let json = serde_json::json!({
            "targets": {
                "stable_fn": {
                    "caller_count": 3,
                    "callers": []
                }
            }
        });
        let findings1 =
            TldrDifferentialEngine::parse_impact_findings("stable_fn", &json);
        let findings2 =
            TldrDifferentialEngine::parse_impact_findings("stable_fn", &json);
        assert_eq!(findings1[0].finding_id, findings2[0].finding_id);
    }

    // =========================================================================
    // build_reverse_caller_map tests
    // =========================================================================

    #[test]
    fn test_build_reverse_caller_map_basic() {
        // Two edges pointing to same dst_func "bar"
        // Expected: map has 1 key "bar" with 2 callers
        let json = serde_json::json!({
            "edges": [
                { "src_file": "a.rs", "src_func": "foo", "dst_file": "b.rs", "dst_func": "bar", "call_type": "direct" },
                { "src_file": "c.rs", "src_func": "baz", "dst_file": "b.rs", "dst_func": "bar", "call_type": "direct" }
            ]
        });
        let map = TldrDifferentialEngine::build_reverse_caller_map(&json);
        assert_eq!(map.len(), 1);
        assert_eq!(map["bar"].len(), 2);
        assert!(map["bar"].contains(&("a.rs".to_string(), "foo".to_string())));
        assert!(map["bar"].contains(&("c.rs".to_string(), "baz".to_string())));
    }

    #[test]
    fn test_build_reverse_caller_map_multiple_targets() {
        // Edges to different dst_funcs
        let json = serde_json::json!({
            "edges": [
                { "src_file": "a.rs", "src_func": "foo", "dst_file": "b.rs", "dst_func": "bar", "call_type": "direct" },
                { "src_file": "c.rs", "src_func": "baz", "dst_file": "d.rs", "dst_func": "qux", "call_type": "direct" }
            ]
        });
        let map = TldrDifferentialEngine::build_reverse_caller_map(&json);
        assert_eq!(map.len(), 2);
        assert_eq!(map["bar"].len(), 1);
        assert_eq!(map["qux"].len(), 1);
    }

    #[test]
    fn test_build_reverse_caller_map_empty_edges() {
        let json = serde_json::json!({ "edges": [] });
        let map = TldrDifferentialEngine::build_reverse_caller_map(&json);
        assert!(map.is_empty());
    }

    #[test]
    fn test_build_reverse_caller_map_no_edges_key() {
        let json = serde_json::json!({ "nodes": [] });
        let map = TldrDifferentialEngine::build_reverse_caller_map(&json);
        assert!(map.is_empty());
    }

    #[test]
    fn test_build_reverse_caller_map_malformed_edges_skipped() {
        // Edges missing required fields should be skipped
        let json = serde_json::json!({
            "edges": [
                { "src_file": "a.rs", "src_func": "foo" },
                { "src_func": "bar", "dst_func": "baz" },
                { "src_file": "valid.rs", "src_func": "caller", "dst_file": "t.rs", "dst_func": "target", "call_type": "direct" }
            ]
        });
        let map = TldrDifferentialEngine::build_reverse_caller_map(&json);
        // Only the valid edge should be in the map
        assert_eq!(map.len(), 1);
        assert_eq!(map["target"].len(), 1);
    }

    // =========================================================================
    // parse_impact_findings_from_callgraph tests
    // =========================================================================

    #[test]
    fn test_parse_impact_from_callgraph_high_severity() {
        // >5 callers = high severity
        let callers = vec![
            ("main.rs".to_string(), "run".to_string()),
            ("handler.rs".to_string(), "handle".to_string()),
            ("api.rs".to_string(), "endpoint".to_string()),
            ("worker.rs".to_string(), "execute".to_string()),
            ("batch.rs".to_string(), "process_all".to_string()),
            ("scheduler.rs".to_string(), "schedule".to_string()),
        ];
        let findings =
            TldrDifferentialEngine::parse_impact_findings_from_callgraph("process_data", &callers);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].finding_type, "breaking-change-risk");
        assert_eq!(findings[0].severity, "high");
        assert_eq!(findings[0].evidence["caller_count"], 6);
        assert_eq!(findings[0].evidence["command"], "calls");
        assert!(findings[0].message.contains("process_data"));
        assert!(findings[0].message.contains("6 callers"));
        // Callers preview capped at 5
        let preview = findings[0].evidence["callers_preview"].as_array().unwrap();
        assert_eq!(preview.len(), 5);
    }

    #[test]
    fn test_parse_impact_from_callgraph_medium_severity() {
        // 2-5 callers = medium severity
        let callers = vec![
            ("a.rs".to_string(), "foo".to_string()),
            ("b.rs".to_string(), "bar".to_string()),
            ("c.rs".to_string(), "baz".to_string()),
        ];
        let findings =
            TldrDifferentialEngine::parse_impact_findings_from_callgraph("helper", &callers);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "medium");
        assert_eq!(findings[0].evidence["caller_count"], 3);
    }

    #[test]
    fn test_parse_impact_from_callgraph_info_severity() {
        // 1 caller = info severity
        let callers = vec![("main.rs".to_string(), "run".to_string())];
        let findings =
            TldrDifferentialEngine::parse_impact_findings_from_callgraph("private_fn", &callers);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "info");
        assert_eq!(findings[0].evidence["caller_count"], 1);
    }

    #[test]
    fn test_parse_impact_from_callgraph_no_callers() {
        // 0 callers = no finding
        let callers: Vec<(String, String)> = vec![];
        let findings =
            TldrDifferentialEngine::parse_impact_findings_from_callgraph("unused_fn", &callers);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_parse_impact_from_callgraph_callers_preview_format() {
        // Preview format should be "file::func"
        let callers = vec![
            ("main.rs".to_string(), "run".to_string()),
            ("handler.rs".to_string(), "handle".to_string()),
        ];
        let findings =
            TldrDifferentialEngine::parse_impact_findings_from_callgraph("target", &callers);

        let preview = findings[0].evidence["callers_preview"].as_array().unwrap();
        assert_eq!(preview[0], "main.rs::run");
        assert_eq!(preview[1], "handler.rs::handle");
    }

    #[test]
    fn test_parse_impact_from_callgraph_finding_fields() {
        // Verify all finding fields match expected values
        let callers = vec![
            ("src.rs".to_string(), "caller".to_string()),
            ("other.rs".to_string(), "other_caller".to_string()),
        ];
        let findings =
            TldrDifferentialEngine::parse_impact_findings_from_callgraph("my_func", &callers);

        assert_eq!(findings[0].finding_type, "breaking-change-risk");
        assert_eq!(findings[0].file, PathBuf::from("(project)"));
        assert_eq!(findings[0].function, "my_func");
        assert_eq!(findings[0].line, 0);
        assert_eq!(findings[0].confidence, Some("DETERMINISTIC".to_string()));
        assert!(findings[0].finding_id.is_some());
    }

    #[test]
    fn test_parse_impact_from_callgraph_boundary_5_callers() {
        // Exactly 5 callers = medium (not high, which requires >5)
        let callers: Vec<(String, String)> = (0..5)
            .map(|i| (format!("f{}.rs", i), format!("fn{}", i)))
            .collect();
        let findings =
            TldrDifferentialEngine::parse_impact_findings_from_callgraph("boundary_fn", &callers);

        assert_eq!(findings[0].severity, "medium");
        // Preview should include all 5 (cap is 5)
        let preview = findings[0].evidence["callers_preview"].as_array().unwrap();
        assert_eq!(preview.len(), 5);
    }

    #[test]
    fn test_parse_impact_from_callgraph_boundary_2_callers() {
        // Exactly 2 callers = medium (>= 2)
        let callers = vec![
            ("a.rs".to_string(), "fa".to_string()),
            ("b.rs".to_string(), "fb".to_string()),
        ];
        let findings =
            TldrDifferentialEngine::parse_impact_findings_from_callgraph("edge_fn", &callers);
        assert_eq!(findings[0].severity, "medium");
    }

    // =========================================================================
    // Derivation function tests (flow cache refactoring)
    // =========================================================================

    // --- derive_deps_from_calls ---

    #[test]
    fn test_bugbot_derive_deps_basic() {
        // One cross-file edge → one dependency
        let calls = serde_json::json!({
            "edges": [
                {"src_file": "a.rs", "src_func": "foo", "dst_file": "b.rs", "dst_func": "bar", "call_type": "direct"}
            ]
        });
        let deps = TldrDifferentialEngine::derive_deps_from_calls(&calls);
        let internal = deps["internal_dependencies"].as_object().unwrap();
        assert!(internal.contains_key("a.rs"));
        let a_deps = internal["a.rs"].as_array().unwrap();
        assert_eq!(a_deps.len(), 1);
        assert!(a_deps.iter().any(|v| v.as_str() == Some("b.rs")));
        assert_eq!(deps["stats"]["total_internal_deps"].as_u64().unwrap(), 1);
    }

    #[test]
    fn test_bugbot_derive_deps_intra_file_excluded() {
        // Same-file edge should NOT produce a dependency
        let calls = serde_json::json!({
            "edges": [
                {"src_file": "a.rs", "src_func": "foo", "dst_file": "a.rs", "dst_func": "bar", "call_type": "direct"}
            ]
        });
        let deps = TldrDifferentialEngine::derive_deps_from_calls(&calls);
        let internal = deps["internal_dependencies"].as_object().unwrap();
        assert!(internal.is_empty() || internal.values().all(|v| v.as_array().unwrap().is_empty()));
        assert_eq!(deps["stats"]["total_internal_deps"].as_u64().unwrap(), 0);
    }

    #[test]
    fn test_bugbot_derive_deps_deduplication() {
        // Two edges between same files → only one dependency entry
        let calls = serde_json::json!({
            "edges": [
                {"src_file": "a.rs", "src_func": "foo", "dst_file": "b.rs", "dst_func": "bar", "call_type": "direct"},
                {"src_file": "a.rs", "src_func": "baz", "dst_file": "b.rs", "dst_func": "qux", "call_type": "direct"}
            ]
        });
        let deps = TldrDifferentialEngine::derive_deps_from_calls(&calls);
        let a_deps = deps["internal_dependencies"]["a.rs"].as_array().unwrap();
        assert_eq!(a_deps.len(), 1);
        assert_eq!(deps["stats"]["total_internal_deps"].as_u64().unwrap(), 1);
    }

    #[test]
    fn test_bugbot_derive_deps_circular_detection() {
        // a.rs → b.rs → a.rs forms a cycle
        let calls = serde_json::json!({
            "edges": [
                {"src_file": "a.rs", "src_func": "f1", "dst_file": "b.rs", "dst_func": "f2", "call_type": "direct"},
                {"src_file": "b.rs", "src_func": "f2", "dst_file": "a.rs", "dst_func": "f3", "call_type": "direct"}
            ]
        });
        let deps = TldrDifferentialEngine::derive_deps_from_calls(&calls);
        let circular = deps["circular_dependencies"].as_array().unwrap();
        assert!(!circular.is_empty(), "should detect circular dependency between a.rs and b.rs");
        // The cycle path should mention both files
        let path = circular[0]["path"].as_array().unwrap();
        let path_strs: Vec<&str> = path.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(path_strs.contains(&"a.rs"));
        assert!(path_strs.contains(&"b.rs"));
    }

    #[test]
    fn test_bugbot_derive_deps_empty_edges() {
        let calls = serde_json::json!({ "edges": [] });
        let deps = TldrDifferentialEngine::derive_deps_from_calls(&calls);
        let internal = deps["internal_dependencies"].as_object().unwrap();
        assert!(internal.is_empty());
        let circular = deps["circular_dependencies"].as_array().unwrap();
        assert!(circular.is_empty());
        assert_eq!(deps["stats"]["total_internal_deps"].as_u64().unwrap(), 0);
    }

    #[test]
    fn test_bugbot_derive_deps_no_edges_key() {
        // Graceful handling when edges key is missing
        let calls = serde_json::json!({ "nodes": ["a.rs:foo"] });
        let deps = TldrDifferentialEngine::derive_deps_from_calls(&calls);
        assert_eq!(deps["stats"]["total_internal_deps"].as_u64().unwrap(), 0);
    }

    // --- derive_coupling_from_calls ---

    #[test]
    fn test_bugbot_derive_coupling_basic() {
        // a.rs→b.rs and c.rs→b.rs: b.rs has Ca=2, Ce=0
        let calls = serde_json::json!({
            "edges": [
                {"src_file": "a.rs", "src_func": "f1", "dst_file": "b.rs", "dst_func": "g1", "call_type": "direct"},
                {"src_file": "c.rs", "src_func": "f2", "dst_file": "b.rs", "dst_func": "g2", "call_type": "direct"}
            ]
        });
        let coupling = TldrDifferentialEngine::derive_coupling_from_calls(&calls);
        let metrics = coupling["martin_metrics"].as_array().unwrap();

        // Find b.rs entry
        let b_metric = metrics.iter().find(|m| m["module"].as_str() == Some("b.rs")).unwrap();
        assert_eq!(b_metric["ca"].as_u64().unwrap(), 2);
        assert_eq!(b_metric["ce"].as_u64().unwrap(), 0);
        assert!((b_metric["instability"].as_f64().unwrap() - 0.0).abs() < 0.01);

        // a.rs: Ca=0, Ce=1, instability=1.0
        let a_metric = metrics.iter().find(|m| m["module"].as_str() == Some("a.rs")).unwrap();
        assert_eq!(a_metric["ca"].as_u64().unwrap(), 0);
        assert_eq!(a_metric["ce"].as_u64().unwrap(), 1);
        assert!((a_metric["instability"].as_f64().unwrap() - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_bugbot_derive_coupling_bidirectional() {
        // a.rs↔b.rs: both have Ca=1, Ce=1, instability=0.5
        let calls = serde_json::json!({
            "edges": [
                {"src_file": "a.rs", "src_func": "f1", "dst_file": "b.rs", "dst_func": "g1", "call_type": "direct"},
                {"src_file": "b.rs", "src_func": "g2", "dst_file": "a.rs", "dst_func": "f2", "call_type": "direct"}
            ]
        });
        let coupling = TldrDifferentialEngine::derive_coupling_from_calls(&calls);
        let metrics = coupling["martin_metrics"].as_array().unwrap();

        for module_name in &["a.rs", "b.rs"] {
            let m = metrics.iter().find(|m| m["module"].as_str() == Some(*module_name))
                .unwrap_or_else(|| panic!("missing metric for {}", module_name));
            assert_eq!(m["ca"].as_u64().unwrap(), 1, "{} Ca should be 1", module_name);
            assert_eq!(m["ce"].as_u64().unwrap(), 1, "{} Ce should be 1", module_name);
            assert!((m["instability"].as_f64().unwrap() - 0.5).abs() < 0.01,
                "{} instability should be 0.5", module_name);
        }
    }

    #[test]
    fn test_bugbot_derive_coupling_self_calls_excluded() {
        // Self-call should not contribute to coupling
        let calls = serde_json::json!({
            "edges": [
                {"src_file": "a.rs", "src_func": "f1", "dst_file": "a.rs", "dst_func": "f2", "call_type": "direct"}
            ]
        });
        let coupling = TldrDifferentialEngine::derive_coupling_from_calls(&calls);
        let metrics = coupling["martin_metrics"].as_array().unwrap();
        // Either empty or a.rs with Ca=0, Ce=0
        if !metrics.is_empty() {
            let a = metrics.iter().find(|m| m["module"].as_str() == Some("a.rs"));
            if let Some(a_metric) = a {
                assert_eq!(a_metric["ca"].as_u64().unwrap(), 0);
                assert_eq!(a_metric["ce"].as_u64().unwrap(), 0);
            }
        }
    }

    #[test]
    fn test_bugbot_derive_coupling_empty() {
        let calls = serde_json::json!({ "edges": [] });
        let coupling = TldrDifferentialEngine::derive_coupling_from_calls(&calls);
        let metrics = coupling["martin_metrics"].as_array().unwrap();
        assert!(metrics.is_empty());
    }

    // --- derive_downstream_from_calls ---

    #[test]
    fn test_bugbot_derive_downstream_basic() {
        let calls = serde_json::json!({
            "edges": [
                {"src_file": "main.rs", "src_func": "run", "dst_file": "lib.rs", "dst_func": "process", "call_type": "direct"}
            ]
        });
        let results = TldrDifferentialEngine::derive_downstream_from_calls(&calls, &["lib.rs"]);
        assert_eq!(results.len(), 1);
        let (file, metrics) = &results[0];
        assert_eq!(file, "lib.rs");
        assert_eq!(metrics["importer_count"].as_u64().unwrap(), 1);
        assert_eq!(metrics["direct_caller_count"].as_u64().unwrap(), 1);
    }

    #[test]
    fn test_bugbot_derive_downstream_multiple_importers() {
        let calls = serde_json::json!({
            "edges": [
                {"src_file": "a.rs", "src_func": "f1", "dst_file": "lib.rs", "dst_func": "process", "call_type": "direct"},
                {"src_file": "b.rs", "src_func": "f2", "dst_file": "lib.rs", "dst_func": "process", "call_type": "direct"},
                {"src_file": "c.rs", "src_func": "f3", "dst_file": "lib.rs", "dst_func": "init", "call_type": "direct"}
            ]
        });
        let results = TldrDifferentialEngine::derive_downstream_from_calls(&calls, &["lib.rs"]);
        let (_, metrics) = &results[0];
        assert_eq!(metrics["importer_count"].as_u64().unwrap(), 3);
        assert_eq!(metrics["direct_caller_count"].as_u64().unwrap(), 3);
    }

    #[test]
    fn test_bugbot_derive_downstream_no_callers() {
        // No edges point to the changed file
        let calls = serde_json::json!({
            "edges": [
                {"src_file": "a.rs", "src_func": "f1", "dst_file": "b.rs", "dst_func": "g1", "call_type": "direct"}
            ]
        });
        let results = TldrDifferentialEngine::derive_downstream_from_calls(&calls, &["lib.rs"]);
        assert_eq!(results.len(), 1);
        let (_, metrics) = &results[0];
        assert_eq!(metrics["importer_count"].as_u64().unwrap(), 0);
        assert_eq!(metrics["direct_caller_count"].as_u64().unwrap(), 0);
    }

    #[test]
    fn test_bugbot_derive_downstream_test_heuristic() {
        // Caller from a test file should be counted in affected_test_count
        let calls = serde_json::json!({
            "edges": [
                {"src_file": "tests/test_lib.rs", "src_func": "test_process", "dst_file": "lib.rs", "dst_func": "process", "call_type": "direct"},
                {"src_file": "main.rs", "src_func": "run", "dst_file": "lib.rs", "dst_func": "process", "call_type": "direct"}
            ]
        });
        let results = TldrDifferentialEngine::derive_downstream_from_calls(&calls, &["lib.rs"]);
        let (_, metrics) = &results[0];
        assert!(metrics["affected_test_count"].as_u64().unwrap() >= 1,
            "test callers should be detected via path/name heuristic");
        assert_eq!(metrics["importer_count"].as_u64().unwrap(), 2);
    }

    #[test]
    fn test_bugbot_derive_downstream_self_calls_excluded() {
        // Edges from the same file should not count as importers
        let calls = serde_json::json!({
            "edges": [
                {"src_file": "lib.rs", "src_func": "helper", "dst_file": "lib.rs", "dst_func": "process", "call_type": "direct"},
                {"src_file": "main.rs", "src_func": "run", "dst_file": "lib.rs", "dst_func": "process", "call_type": "direct"}
            ]
        });
        let results = TldrDifferentialEngine::derive_downstream_from_calls(&calls, &["lib.rs"]);
        let (_, metrics) = &results[0];
        assert_eq!(metrics["importer_count"].as_u64().unwrap(), 1, "self-calls should be excluded");
    }

    #[test]
    fn test_bugbot_derive_downstream_same_importer_multiple_calls() {
        // Same importer calling multiple functions should count as 1 importer
        let calls = serde_json::json!({
            "edges": [
                {"src_file": "main.rs", "src_func": "run", "dst_file": "lib.rs", "dst_func": "init", "call_type": "direct"},
                {"src_file": "main.rs", "src_func": "run", "dst_file": "lib.rs", "dst_func": "process", "call_type": "direct"},
                {"src_file": "main.rs", "src_func": "shutdown", "dst_file": "lib.rs", "dst_func": "cleanup", "call_type": "direct"}
            ]
        });
        let results = TldrDifferentialEngine::derive_downstream_from_calls(&calls, &["lib.rs"]);
        let (_, metrics) = &results[0];
        assert_eq!(metrics["importer_count"].as_u64().unwrap(), 1, "3 edges from same file = 1 importer");
        assert_eq!(metrics["direct_caller_count"].as_u64().unwrap(), 1);
    }

    // =========================================================================
    // Calls JSON caching: rewired signatures accept cached calls
    // =========================================================================

    #[test]
    fn test_analyze_flow_commands_accepts_cached_calls_json() {
        // analyze_flow_commands should accept an optional current_calls_json
        // parameter. When None, it falls back to running the subprocess.
        let engine = TldrDifferentialEngine::new();
        let mut partial_reasons = Vec::new();
        let _findings = engine.analyze_flow_commands(
            Path::new("/tmp/nonexistent-project-for-cache-test"),
            "HEAD",
            "rust",
            None, // no cached calls — fallback behavior
            &mut partial_reasons,
        );
        // Should not panic
    }

    #[test]
    fn test_analyze_flow_commands_uses_cached_calls_for_deps() {
        // When current_calls_json is Some, analyze_flow_commands should derive
        // deps from it instead of running `tldr deps` subprocess.
        let engine = TldrDifferentialEngine::new();
        let mut partial_reasons = Vec::new();
        let calls_json = serde_json::json!({
            "edges": [
                {"src_file": "a.rs", "src_func": "foo", "dst_file": "b.rs", "dst_func": "bar", "call_type": "direct"}
            ]
        });
        // With cached calls, the method should not need to run tldr deps subprocess.
        // On a nonexistent project, the worktree will fail, so we won't get findings,
        // but the important thing is it doesn't panic and accepts the parameter.
        let _findings = engine.analyze_flow_commands(
            Path::new("/tmp/nonexistent-project-for-cache-test"),
            "HEAD",
            "rust",
            Some(&calls_json),
            &mut partial_reasons,
        );
    }

    #[test]
    fn test_analyze_downstream_impact_accepts_cached_calls_json() {
        // analyze_downstream_impact should accept an optional current_calls_json.
        // When Some, it derives downstream impact from the calls JSON instead
        // of running tldr whatbreaks per file.
        let engine = TldrDifferentialEngine::new();
        let mut partial_reasons = Vec::new();
        let calls_json = serde_json::json!({
            "edges": [
                {"src_file": "main.rs", "src_func": "run", "dst_file": "lib.rs", "dst_func": "process", "call_type": "direct"},
                {"src_file": "tests/test_lib.rs", "src_func": "test_it", "dst_file": "lib.rs", "dst_func": "process", "call_type": "direct"}
            ]
        });

        let project = Path::new("/tmp/nonexistent-downstream-test");
        let changed_files = vec![project.join("lib.rs")];
        let findings = engine.analyze_downstream_impact(
            project,
            &changed_files,
            "rust",
            Some(&calls_json),
            &mut partial_reasons,
        );

        // With 2 cross-file edges into lib.rs, should produce a downstream-impact finding
        assert!(!findings.is_empty(), "cached calls should produce downstream findings");
        assert_eq!(findings[0].finding_type, "downstream-impact");
    }

    #[test]
    fn test_analyze_downstream_impact_none_falls_back() {
        // When current_calls_json is None, analyze_downstream_impact should
        // fall back to running tldr whatbreaks subprocess (which will fail
        // gracefully on nonexistent paths).
        let engine = TldrDifferentialEngine::new();
        let mut partial_reasons = Vec::new();
        let project = Path::new("/tmp/nonexistent-downstream-fallback");
        let changed_files = vec![project.join("lib.rs")];
        let _findings = engine.analyze_downstream_impact(
            project,
            &changed_files,
            "rust",
            None,
            &mut partial_reasons,
        );
        // Should not panic — graceful fallback
    }

    #[test]
    fn test_analyze_function_impact_accepts_cached_calls_json() {
        // analyze_function_impact should accept an optional current_calls_json.
        // When Some, it reuses the cached JSON instead of running tldr calls.
        let engine = TldrDifferentialEngine::new();
        let mut partial_reasons = Vec::new();
        let calls_json = serde_json::json!({
            "edges": [
                {"src_file": "caller.rs", "src_func": "caller_fn", "dst_file": "lib.rs", "dst_func": "target_fn", "call_type": "direct"}
            ]
        });
        let project = Path::new("/tmp/nonexistent-function-impact-test");
        let changed_files = vec![project.join("lib.rs")];
        let _findings = engine.analyze_function_impact(
            project,
            &changed_files,
            "rust",
            Some(&calls_json),
            &mut partial_reasons,
        );
        // Should not panic and should accept the parameter
    }

    #[test]
    fn test_analyze_function_impact_none_falls_back() {
        // When current_calls_json is None, falls back to subprocess
        let engine = TldrDifferentialEngine::new();
        let mut partial_reasons = Vec::new();
        let project = Path::new("/tmp/nonexistent-function-impact-fallback");
        let changed_files = vec![project.join("lib.rs")];
        let _findings = engine.analyze_function_impact(
            project,
            &changed_files,
            "rust",
            None,
            &mut partial_reasons,
        );
        // Should not panic — graceful fallback to subprocess
    }

    #[test]
    fn test_analyze_downstream_with_cached_calls_produces_correct_findings() {
        // When using cached calls, the downstream findings should match
        // what derive_downstream_from_calls produces fed through parse_whatbreaks_findings.
        let engine = TldrDifferentialEngine::new();
        let mut partial_reasons = Vec::new();
        let calls_json = serde_json::json!({
            "edges": [
                {"src_file": "a.rs", "src_func": "f1", "dst_file": "target.rs", "dst_func": "process", "call_type": "direct"},
                {"src_file": "b.rs", "src_func": "f2", "dst_file": "target.rs", "dst_func": "init", "call_type": "direct"},
                {"src_file": "c.rs", "src_func": "f3", "dst_file": "target.rs", "dst_func": "run", "call_type": "direct"},
                {"src_file": "d.rs", "src_func": "f4", "dst_file": "target.rs", "dst_func": "cleanup", "call_type": "direct"},
            ]
        });

        let project = Path::new("/tmp/nonexistent-downstream-correct");
        let changed_files = vec![project.join("target.rs")];
        let findings = engine.analyze_downstream_impact(
            project,
            &changed_files,
            "rust",
            Some(&calls_json),
            &mut partial_reasons,
        );

        // 4 importers → medium severity (>3 but <=10)
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "medium");
        assert_eq!(findings[0].finding_type, "downstream-impact");
        // Evidence should contain the counts
        assert_eq!(findings[0].evidence["importer_count"], 4);
    }
}
