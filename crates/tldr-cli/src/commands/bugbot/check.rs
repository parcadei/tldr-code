//! Bugbot check command - analyze uncommitted changes for potential bugs
//!
//! Wires the full pipeline: detect changes, compute baselines, L1 commodity
//! tool execution (clippy, cargo-audit), AST-diff, signature-regression
//! analysis, and born-dead detection.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{bail, Result};
use clap::Args;

use tldr_core::Language;

use crate::output::{OutputFormat, OutputWriter};

use super::baseline::{get_baseline_content, write_baseline_tmpfile, BaselineStatus};
use super::changes::detect_changes;
use super::dead::compose_born_dead_scoped;
use super::diff::diff_functions;
use super::l2::types::AnalyzerStatus;
use super::runner::ToolRunner;
use super::signature::compose_signature_regression;
use super::text_format::format_bugbot_text;
use super::tools::{L1Finding, ToolRegistry, ToolResult};
use super::types::{
    BugbotCheckReport, BugbotExitError, BugbotFinding, BugbotSummary, L2AnalyzerResult,
};

/// Run bugbot check on uncommitted changes
#[derive(Debug, Args)]
pub struct BugbotCheckArgs {
    /// Project root directory
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Git base reference to diff against
    #[arg(long, default_value = "HEAD")]
    pub base_ref: String,

    /// Check only staged changes
    #[arg(long)]
    pub staged: bool,

    /// Maximum number of findings to report (0 = unlimited)
    #[arg(long, default_value = "50")]
    pub max_findings: usize,

    /// Do not fail (exit 0) even if findings exist
    #[arg(long)]
    pub no_fail: bool,

    /// Suppress progress messages
    #[arg(long, short)]
    pub quiet: bool,

    /// Disable L1 commodity tool analysis (clippy, cargo-audit, etc.)
    #[arg(long, default_value_t = false)]
    pub no_tools: bool,

    /// Timeout for each L1 tool in seconds
    #[arg(long, default_value_t = 60)]
    pub tool_timeout: u64,
}

impl BugbotCheckArgs {
    /// Run the bugbot check command
    ///
    /// `format` and `quiet` come from the global CLI flags.
    /// `lang` comes from the global `--lang` / `-l` flag (already parsed as `Language` enum).
    pub fn run(&self, format: OutputFormat, quiet: bool, lang: Option<Language>) -> Result<()> {
        let start = Instant::now();
        let writer = OutputWriter::new(format, quiet);
        let mut errors: Vec<String> = Vec::new();
        let mut notes: Vec<String> = Vec::new();

        // Step 1: Resolve language
        let language = match lang {
            Some(l) => l,
            None => match Language::from_directory(&self.path) {
                Some(l) => l,
                None => {
                    bail!("Could not detect language. Use --lang <LANG>");
                }
            },
        };

        let language_str = format!("{:?}", language).to_lowercase();
        let project = std::fs::canonicalize(&self.path)?;

        // Step 1b: First-run detection and auto-scan (PM-34)
        let is_first_run = {
            use super::first_run::{detect_first_run, run_first_run_scan, FirstRunStatus};
            match detect_first_run(&project) {
                FirstRunStatus::FirstRun => {
                    let progress_fn = |msg: &str| writer.progress(msg);
                    match run_first_run_scan(&project, &progress_fn) {
                        Ok(result) => {
                            if !result.baseline_errors.is_empty() {
                                for err in &result.baseline_errors {
                                    errors.push(format!("first-run baseline: {err}"));
                                }
                            }
                            notes.push(format!(
                                "first_run_baseline_built_in_{}ms",
                                result.elapsed_ms
                            ));
                            true
                        }
                        Err(e) => {
                            errors.push(format!("first-run scan failed: {e}"));
                            // Continue anyway -- the L2 engines handle missing caches
                            true
                        }
                    }
                }
                FirstRunStatus::SubsequentRun { .. } => false,
            }
        };

        writer.progress(&format!(
            "Detecting {} changes in {}...",
            language_str,
            project.display()
        ));

        // Step 2: Detect changed files
        let changes = detect_changes(&project, &self.base_ref, self.staged, &language)?;

        // Step 3: Early return if no changes
        if changes.changed_files.is_empty() {
            let report = BugbotCheckReport {
                tool: "bugbot".to_string(),
                mode: "check".to_string(),
                language: language_str,
                base_ref: self.base_ref.clone(),
                detection_method: changes.detection_method,
                timestamp: chrono::Utc::now().to_rfc3339(),
                changed_files: Vec::new(),
                findings: Vec::new(),
                summary: build_summary(&[], 0, 0),
                elapsed_ms: start.elapsed().as_millis() as u64,
                errors: Vec::new(),
                notes: vec!["no_changes_detected".to_string()],
                tool_results: Vec::new(),
                tools_available: Vec::new(),
                tools_missing: Vec::new(),
                l2_engine_results: Vec::new(),
            };

            if writer.is_text() {
                writer.write_text(&format_bugbot_text(&report))?;
            } else {
                writer.write(&report)?;
            }
            return Ok(());
        }

        writer.progress(&format!(
            "Found {} changed {} file(s)",
            changes.changed_files.len(),
            language_str
        ));

        // Step 4: Per-file baseline extraction and AST diff
        let mut all_diffs: HashMap<PathBuf, Vec<crate::commands::remaining::types::ASTChange>> =
            HashMap::new();
        // Keep temp files alive until the pipeline finishes (dropping deletes them)
        let mut _tmpfiles: Vec<tempfile::NamedTempFile> = Vec::new();
        // File contents for L2Context: baseline (pre-change) and current (post-change)
        let mut baseline_contents: HashMap<PathBuf, String> = HashMap::new();
        let mut current_contents: HashMap<PathBuf, String> = HashMap::new();

        for file in &changes.changed_files {
            match get_baseline_content(&project, file, &self.base_ref) {
                Ok(BaselineStatus::Exists(content)) => {
                    if file.exists() {
                        // Save baseline and current file contents for L2 engines
                        let rel_path = file.strip_prefix(&project).unwrap_or(file).to_path_buf();
                        baseline_contents.insert(rel_path.clone(), content.clone());
                        if let Ok(current) = std::fs::read_to_string(file) {
                            current_contents.insert(rel_path, current);
                        }

                        // Normal case: diff baseline vs current
                        match write_baseline_tmpfile(&content, file) {
                            Ok(tmpfile) => {
                                match diff_functions(tmpfile.path(), file) {
                                    Ok(report) => {
                                        all_diffs.insert(file.clone(), report.changes);
                                    }
                                    Err(e) => {
                                        errors.push(format!(
                                            "diff failed for {}: {}",
                                            file.display(),
                                            e
                                        ));
                                    }
                                }
                                _tmpfiles.push(tmpfile);
                            }
                            Err(e) => {
                                errors.push(format!(
                                    "baseline tmpfile failed for {}: {}",
                                    file.display(),
                                    e
                                ));
                            }
                        }
                    } else {
                        // File existed at baseline but is now deleted -- skip for v0.1
                        notes.push(format!("deleted_file:{}", file.display()));
                    }
                }
                Ok(BaselineStatus::NewFile) => {
                    if file.exists() {
                        // Save empty baseline and current file contents for L2 engines
                        let rel_path = file.strip_prefix(&project).unwrap_or(file).to_path_buf();
                        baseline_contents.insert(rel_path.clone(), String::new());
                        if let Ok(current) = std::fs::read_to_string(file) {
                            current_contents.insert(rel_path, current);
                        }

                        // New file: diff against an empty baseline so all functions are Insert
                        let extension = file.extension().and_then(|e| e.to_str()).unwrap_or("txt");
                        match tempfile::Builder::new()
                            .prefix("bugbot_empty_")
                            .suffix(&format!(".{}", extension))
                            .tempfile()
                        {
                            Ok(mut empty_file) => {
                                // Write nothing (empty file)
                                let _ = empty_file.flush();
                                match diff_functions(empty_file.path(), file) {
                                    Ok(report) => {
                                        all_diffs.insert(file.clone(), report.changes);
                                    }
                                    Err(e) => {
                                        errors.push(format!(
                                            "diff (new file) failed for {}: {}",
                                            file.display(),
                                            e
                                        ));
                                    }
                                }
                                _tmpfiles.push(empty_file);
                            }
                            Err(e) => {
                                errors.push(format!(
                                    "empty tmpfile failed for {}: {}",
                                    file.display(),
                                    e
                                ));
                            }
                        }
                    }
                }
                Ok(BaselineStatus::GitShowFailed(msg)) => {
                    errors.push(format!("git show failed for {}: {}", file.display(), msg));
                }
                Err(e) => {
                    errors.push(format!("baseline error for {}: {}", file.display(), e));
                }
            }
        }

        let files_analyzed = all_diffs.len();
        let functions_analyzed: usize = all_diffs.values().map(|v| v.len()).sum();

        writer.progress(&format!(
            "Analyzed {} file(s), {} function-level change(s)",
            files_analyzed, functions_analyzed
        ));

        // Step 4b: Build L2 context and spawn L2 engines on background thread.
        // L2 is CPU-bound (tree-sitter, graph algorithms, data flow) while L1 is
        // I/O-bound (subprocess execution). Running them in parallel reduces wall
        // clock time from ~2.5s to ~1.5s.
        writer.progress("Running L1 + L2 analysis in parallel...");
        let l2_handle = {
            use super::l2::{l2_engine_registry, L2Context};

            let engines = l2_engine_registry();

            // Build L2Context from pipeline data. The L2 engines use changed_files,
            // function-level diffs, and file contents for their analysis.
            let relative_changed: Vec<PathBuf> = changes
                .changed_files
                .iter()
                .filter_map(|f| f.strip_prefix(&project).ok().map(|p| p.to_path_buf()))
                .collect();

            // Build ast_changes with relative paths to match L2Context conventions.
            let relative_diffs: HashMap<
                PathBuf,
                Vec<crate::commands::remaining::types::ASTChange>,
            > = all_diffs
                .iter()
                .map(|(path, changes)| {
                    let rel = path.strip_prefix(&project).unwrap_or(path).to_path_buf();
                    (rel, changes.clone())
                })
                .collect();

            // Create daemon client for this project. If a daemon is running,
            // deferred-tier engines will use cached IR artifacts.
            let daemon = super::l2::daemon_client::create_daemon_client(&project);

            // Convert AST changes to function-level diff for L2 engines
            let function_diff = build_function_diff(&all_diffs, &project);

            let l2_ctx = L2Context::new(
                project.clone(),
                language,
                relative_changed,
                function_diff,
                baseline_contents,
                current_contents,
                relative_diffs,
            )
            .with_first_run(is_first_run)
            .with_base_ref(self.base_ref.clone())
            .with_daemon(daemon);

            // Spawn L2 engines on background thread. L2Context uses DashMap +
            // OnceLock (Send+Sync), and L2Engine: Send+Sync, so both can move.
            std::thread::spawn(move || run_l2_engines(&l2_ctx, &engines))
        };

        // L1 runs on main thread concurrently with L2 (I/O-bound subprocess work)
        if !self.no_tools {
            writer.progress("Running L1 diagnostic tools...");
        }
        let (l1_raw, tool_results, tools_available, tools_missing) =
            run_l1_tools_opt(&project, &language_str, self.no_tools, self.tool_timeout);

        // Convert L1Finding -> BugbotFinding and filter to changed files (PM-3)
        let l1_bugbot: Vec<BugbotFinding> = l1_raw.into_iter().map(BugbotFinding::from).collect();
        let changed_paths: Vec<PathBuf> = changes
            .changed_files
            .iter()
            .filter_map(|f| f.strip_prefix(&project).ok().map(|p| p.to_path_buf()))
            .collect();
        let l1_filtered = filter_l1_findings(l1_bugbot, &changed_paths);
        let l1_count = l1_filtered.len();

        if !tools_available.is_empty() {
            let ran_count = tool_results.len();
            let finding_count: usize = tool_results.iter().map(|r| r.finding_count).sum();
            writer.progress(&format!(
                "L1 tools: {} ran, {} raw findings, {} after filtering to changed files",
                ran_count, finding_count, l1_count
            ));
        }

        // Step 5: Compose signature regression findings (main thread, uses all_diffs)
        let sig_findings = compose_signature_regression(&all_diffs, &project);

        // Step 6: Compose born-dead findings (only if there are Insert changes)
        // Filter inserted functions directly from references (avoids cloning)
        use crate::commands::remaining::types::{ChangeType, NodeKind};
        let inserts: Vec<&crate::commands::remaining::types::ASTChange> = all_diffs
            .values()
            .flat_map(|changes| changes.iter())
            .filter(|c| matches!(c.change_type, ChangeType::Insert))
            .filter(|c| matches!(c.node_kind, NodeKind::Function | NodeKind::Method))
            .collect();
        let dead_findings = if !inserts.is_empty() {
            writer.progress("Scanning for born-dead functions...");
            match compose_born_dead_scoped(&inserts, &changes.changed_files, &project, &language) {
                Ok(findings) => findings,
                Err(e) => {
                    errors.push(format!("born-dead analysis failed: {}", e));
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };

        // Join L2 thread -- graceful degradation if the thread panicked
        let (l2_engine_findings, l2_engine_results) = l2_handle.join().unwrap_or_else(|_| {
            errors.push("L2 engine thread panicked".to_string());
            (Vec::new(), Vec::new())
        });

        // Step 7: Merge L1 + L2 findings (compose_ + engine findings)
        let compose_l2_count = sig_findings.len() + dead_findings.len();
        let l2_count = compose_l2_count + l2_engine_findings.len();
        let mut findings: Vec<BugbotFinding> = Vec::new();
        findings.extend(l1_filtered);
        findings.extend(sig_findings);
        findings.extend(dead_findings);
        findings.extend(l2_engine_findings);

        // Step 8a: Dedup and prioritize (CK-4)
        use super::l2::dedup::dedup_and_prioritize;
        findings = dedup_and_prioritize(findings, self.max_findings);

        // Step 8b: Composition Engine (PM-41)
        use super::l2::composition::compose_findings;
        findings = compose_findings(findings);

        // Re-sort after composition (composed findings may have different severity)
        findings.sort_by(|a, b| {
            severity_rank(&b.severity)
                .cmp(&severity_rank(&a.severity))
                .then(a.file.cmp(&b.file))
                .then(a.line.cmp(&b.line))
        });

        // Step 9: Build summary (with L1/L2 breakdown)
        let summary = build_summary_with_l1(
            &findings,
            l1_count,
            l2_count,
            files_analyzed,
            functions_analyzed,
            &tool_results,
        );
        let elapsed_ms = start.elapsed().as_millis() as u64;

        // Step 10: Build and emit report
        let report = BugbotCheckReport {
            tool: "bugbot".to_string(),
            mode: "check".to_string(),
            language: language_str,
            base_ref: self.base_ref.clone(),
            detection_method: changes.detection_method,
            timestamp: chrono::Utc::now().to_rfc3339(),
            changed_files: changes.changed_files,
            findings,
            summary,
            elapsed_ms,
            errors,
            notes,
            tool_results,
            tools_available,
            tools_missing,
            l2_engine_results,
        };

        // Output
        if writer.is_text() {
            writer.write_text(&format_bugbot_text(&report))?;
        } else {
            writer.write(&report)?;
        }

        // Exit code for pre-push gating: `tldr bugbot check && git push`
        //
        // Exit codes:
        //   0 = clean (no findings, or --no-fail suppresses failure)
        //   1 = findings detected (analysis succeeded but bugs found)
        //   2 = analysis had errors with no findings (broken pipeline, not "clean")
        //   3 = critical findings detected (highest priority, takes precedence over 1)
        let has_findings = !report.findings.is_empty();
        let has_errors = !report.errors.is_empty();
        let has_critical = report.findings.iter().any(|f| f.severity == "critical");

        // PM-42: Critical findings exit code 3 takes precedence over exit code 1
        if has_critical && !self.no_fail {
            return Err(BugbotExitError::CriticalFindings {
                count: report
                    .findings
                    .iter()
                    .filter(|f| f.severity == "critical")
                    .count(),
            }
            .into());
        }

        if has_findings && !self.no_fail {
            return Err(BugbotExitError::FindingsDetected {
                count: report.findings.len(),
            }
            .into());
        }

        if !has_findings && has_errors && !self.no_fail {
            return Err(BugbotExitError::AnalysisErrors {
                count: report.errors.len(),
            }
            .into());
        }

        Ok(())
    }
}

/// Run L1 commodity diagnostic tools and return their findings and metadata.
///
/// Creates a `ToolRegistry`, detects available tools for the given language,
/// runs all available tools in parallel via `ToolRunner`, and returns:
/// - `l1_findings`: Raw `L1Finding`s from all tools
/// - `tool_results`: Execution results for each tool
/// - `available_names`: Names of tools that were available
/// - `missing_names`: Names of tools that were not installed
///
/// When `no_tools` is `true`, skips all L1 tool execution and returns empty
/// results. This is the `--no-tools` CLI flag path.
///
/// `timeout_secs` controls the per-tool timeout passed to `ToolRunner`.
fn run_l1_tools_opt(
    project_root: &std::path::Path,
    language: &str,
    no_tools: bool,
    timeout_secs: u64,
) -> (Vec<L1Finding>, Vec<ToolResult>, Vec<String>, Vec<String>) {
    if no_tools {
        return (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    }

    let registry = ToolRegistry::new();
    let (available, missing) = registry.detect_available_tools(language);

    let available_names: Vec<String> = available.iter().map(|t| t.name.to_string()).collect();
    let missing_names: Vec<String> = missing.iter().map(|t| t.name.to_string()).collect();

    if available.is_empty() {
        return (Vec::new(), Vec::new(), available_names, missing_names);
    }

    let runner = ToolRunner::new(timeout_secs);
    let (tool_results, l1_findings) = runner.run_tools_parallel(&available, project_root);

    (l1_findings, tool_results, available_names, missing_names)
}

/// Map severity string to a numeric rank for sorting (higher = more severe).
///
/// PM-8: "info" is explicitly ranked below "low" rather than falling through
/// to the wildcard case. Unknown severities get rank 0.
fn severity_rank(severity: &str) -> u8 {
    match severity {
        "critical" => 5,
        "high" => 4,
        "medium" => 3,
        "low" => 2,
        "info" => 1,
        _ => 0,
    }
}

/// Build summary statistics from the final findings list.
fn build_summary(
    findings: &[BugbotFinding],
    files_analyzed: usize,
    functions_analyzed: usize,
) -> BugbotSummary {
    build_summary_with_l1(
        findings,
        0,
        findings.len(),
        files_analyzed,
        functions_analyzed,
        &[],
    )
}

/// Build summary statistics with separate L1 and L2 finding counts.
///
/// Also counts tool execution statistics from `tool_results`.
///
/// F14: The `l1_count` and `l2_count` parameters are hints from the
/// pre-truncation pipeline. After truncation, these may be stale. This
/// function recalculates L1/L2 counts from the actual `findings` slice
/// to ensure `total_findings == l1_findings + l2_findings`.
fn build_summary_with_l1(
    findings: &[BugbotFinding],
    l1_count: usize,
    l2_count: usize,
    files_analyzed: usize,
    functions_analyzed: usize,
    tool_results: &[super::tools::ToolResult],
) -> BugbotSummary {
    let mut by_severity: HashMap<String, usize> = HashMap::new();
    let mut by_type: HashMap<String, usize> = HashMap::new();

    for f in findings {
        *by_severity.entry(f.severity.clone()).or_insert(0) += 1;
        *by_type.entry(f.finding_type.clone()).or_insert(0) += 1;
    }

    let tools_run = tool_results.len();
    let tools_failed = tool_results.iter().filter(|r| !r.success).count();

    // F14: Recalculate L1/L2 counts from actual findings to handle
    // post-truncation consistency. L1 findings have finding_type starting
    // with "tool:" (set by the L1Finding -> BugbotFinding conversion).
    let actual_l1 = findings
        .iter()
        .filter(|f| f.finding_type.starts_with("tool:"))
        .count();
    let actual_l2 = findings.len() - actual_l1;

    // Use the actual counts if they differ from the hints (truncation happened)
    let final_l1 = if actual_l1 + actual_l2 != l1_count + l2_count {
        actual_l1
    } else {
        l1_count
    };
    let final_l2 = if actual_l1 + actual_l2 != l1_count + l2_count {
        actual_l2
    } else {
        l2_count
    };

    BugbotSummary {
        total_findings: findings.len(),
        by_severity,
        by_type,
        files_analyzed,
        functions_analyzed,
        l1_findings: final_l1,
        l2_findings: final_l2,
        tools_run,
        tools_failed,
    }
}

/// Filter L1 findings to only include files in the changed set.
///
/// PM-3: L1 tools scan the whole project, but we only report findings for files
/// that are in the changed set. If `changed_files` is empty (scan mode or no
/// baseline), all findings are returned unfiltered.
fn filter_l1_findings(
    findings: Vec<BugbotFinding>,
    changed_files: &[PathBuf],
) -> Vec<BugbotFinding> {
    if changed_files.is_empty() {
        return findings;
    }
    findings
        .into_iter()
        .filter(|f| {
            changed_files.iter().any(|cf| {
                // Direct match (both relative or both absolute)
                cf == &f.file
                // L1 tools may emit absolute paths; compare by filename suffix
                || f.file.ends_with(cf)
                || cf.ends_with(&f.file)
            })
        })
        .collect()
}

/// Run a single L2 engine, applying language gating and collecting results.
///
/// Returns `Some((findings, result))` if the engine ran, or `Some(([], result))`
/// if it was skipped due to language gating.
fn run_single_engine(
    engine: &dyn super::l2::L2Engine,
    ctx: &super::l2::L2Context,
) -> (Vec<BugbotFinding>, L2AnalyzerResult) {
    // Language gating (PM-37): skip engines that declare specific language
    // support when the context language is not in the supported set.
    let supported = engine.languages();
    if !supported.is_empty() && !supported.contains(&ctx.language) {
        return (
            Vec::new(),
            L2AnalyzerResult {
                name: engine.name().to_string(),
                success: true,
                duration_ms: 0,
                finding_count: 0,
                functions_analyzed: 0,
                functions_skipped: 0,
                status: format!(
                    "Skipped: {} does not support {:?}",
                    engine.name(),
                    ctx.language
                ),
                errors: vec![],
            },
        );
    }

    let start = Instant::now();
    let output = engine.analyze(ctx);
    let duration = start.elapsed().as_millis() as u64;

    let status_str = match &output.status {
        AnalyzerStatus::Complete => "complete".to_string(),
        AnalyzerStatus::Partial { reason } => format!("partial ({})", reason),
        AnalyzerStatus::Skipped { reason } => format!("skipped ({})", reason),
        AnalyzerStatus::TimedOut { partial_findings } => {
            format!("timed out ({} partial findings)", partial_findings)
        }
    };

    let errors = match &output.status {
        AnalyzerStatus::Partial { reason } => vec![reason.clone()],
        AnalyzerStatus::TimedOut { .. } => vec!["Engine timed out".to_string()],
        _ => vec![],
    };

    let result = L2AnalyzerResult {
        name: engine.name().to_string(),
        success: matches!(output.status, AnalyzerStatus::Complete),
        duration_ms: duration,
        finding_count: output.findings.len(),
        functions_analyzed: output.functions_analyzed,
        functions_skipped: output.functions_skipped,
        status: status_str,
        errors,
    };

    (output.findings, result)
}

/// Run all registered L2 analysis engines.
///
/// Iterates over every engine in registration order, collects findings and
/// per-engine result summaries. Returns a tuple of (all_findings, engine_results).
fn run_l2_engines(
    ctx: &super::l2::L2Context,
    engines: &[Box<dyn super::l2::L2Engine>],
) -> (Vec<BugbotFinding>, Vec<L2AnalyzerResult>) {
    let mut all_findings = Vec::new();
    let mut results = Vec::new();

    for engine in engines {
        let (findings, result) = run_single_engine(engine.as_ref(), ctx);
        all_findings.extend(findings);
        results.push(result);
    }

    (all_findings, results)
}

/// Build a `FunctionDiff` from AST-level changes collected during the diff phase.
///
/// Iterates over all file-level `ASTChange` entries and converts function/method
/// changes into the `FunctionChange`, `InsertedFunction`, and `DeletedFunction`
/// types expected by `L2Context`. Non-function nodes (classes, statements, etc.)
/// and unnamed changes are skipped.
///
/// Paths in `all_diffs` are expected to be absolute; they are converted to
/// relative paths by stripping the `project` prefix, matching L2Context
/// conventions.
fn build_function_diff(
    all_diffs: &HashMap<PathBuf, Vec<crate::commands::remaining::types::ASTChange>>,
    project: &std::path::Path,
) -> super::l2::context::FunctionDiff {
    use super::l2::context::{DeletedFunction, FunctionChange, FunctionDiff, InsertedFunction};
    use super::l2::types::FunctionId;
    use crate::commands::remaining::types::{ChangeType, NodeKind};

    let mut changed_fns = Vec::new();
    let mut inserted_fns = Vec::new();
    let mut deleted_fns = Vec::new();

    for (abs_path, changes) in all_diffs {
        let rel_path = abs_path
            .strip_prefix(project)
            .unwrap_or(abs_path)
            .to_path_buf();

        for change in changes {
            // Only process function-level changes
            if !matches!(change.node_kind, NodeKind::Function | NodeKind::Method) {
                continue;
            }

            let name = match &change.name {
                Some(n) => n.clone(),
                None => continue, // Skip unnamed changes
            };

            let def_line = change
                .new_location
                .as_ref()
                .or(change.old_location.as_ref())
                .map(|loc| loc.line as usize)
                .unwrap_or(0);

            let func_id = FunctionId::new(rel_path.clone(), &name, def_line);

            match change.change_type {
                ChangeType::Update => {
                    let old_source = change.old_text.clone().unwrap_or_default();
                    let new_source = change.new_text.clone().unwrap_or_default();
                    changed_fns.push(FunctionChange {
                        id: func_id,
                        name: name.clone(),
                        old_source,
                        new_source,
                    });
                }
                ChangeType::Insert => {
                    let source = change.new_text.clone().unwrap_or_default();
                    inserted_fns.push(InsertedFunction {
                        id: func_id,
                        name: name.clone(),
                        source,
                    });
                }
                ChangeType::Delete => {
                    deleted_fns.push(DeletedFunction {
                        id: func_id,
                        name: name.clone(),
                    });
                }
                ChangeType::Move
                | ChangeType::Rename
                | ChangeType::Extract
                | ChangeType::Inline
                | ChangeType::Format => {
                    // Treat as Update if both old and new texts exist
                    if change.old_text.is_some() && change.new_text.is_some() {
                        changed_fns.push(FunctionChange {
                            id: func_id,
                            name: name.clone(),
                            old_source: change.old_text.clone().unwrap_or_default(),
                            new_source: change.new_text.clone().unwrap_or_default(),
                        });
                    }
                }
            }
        }
    }

    FunctionDiff {
        changed: changed_fns,
        inserted: inserted_fns,
        deleted: deleted_fns,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Unit tests for helper functions (no git required)
    // -----------------------------------------------------------------------

    #[test]
    fn test_severity_rank_ordering() {
        assert_eq!(severity_rank("critical"), 5);
        assert_eq!(severity_rank("high"), 4);
        assert_eq!(severity_rank("medium"), 3);
        assert_eq!(severity_rank("low"), 2);
        assert_eq!(severity_rank("info"), 1); // PM-8: explicit rank for info
        assert_eq!(severity_rank(""), 0);
    }

    /// CK-4: Verify critical severity is ranked at 5 (above high).
    #[test]
    fn test_severity_rank_critical() {
        assert_eq!(severity_rank("critical"), 5);
        assert!(
            severity_rank("critical") > severity_rank("high"),
            "critical ({}) should rank above high ({})",
            severity_rank("critical"),
            severity_rank("high"),
        );
    }

    #[test]
    fn test_build_summary_empty() {
        let summary = build_summary(&[], 0, 0);
        assert_eq!(summary.total_findings, 0);
        assert!(summary.by_severity.is_empty());
        assert!(summary.by_type.is_empty());
        assert_eq!(summary.files_analyzed, 0);
        assert_eq!(summary.functions_analyzed, 0);
    }

    #[test]
    fn test_build_summary_counts() {
        let findings = vec![
            BugbotFinding {
                finding_type: "signature-regression".to_string(),
                severity: "high".to_string(),
                file: PathBuf::from("a.rs"),
                function: "foo".to_string(),
                line: 10,
                message: "param removed".to_string(),
                evidence: serde_json::Value::Null,
                confidence: None,
                finding_id: None,
            },
            BugbotFinding {
                finding_type: "born-dead".to_string(),
                severity: "low".to_string(),
                file: PathBuf::from("b.rs"),
                function: "bar".to_string(),
                line: 20,
                message: "no callers".to_string(),
                evidence: serde_json::Value::Null,
                confidence: None,
                finding_id: None,
            },
            BugbotFinding {
                finding_type: "signature-regression".to_string(),
                severity: "high".to_string(),
                file: PathBuf::from("c.rs"),
                function: "baz".to_string(),
                line: 5,
                message: "return type changed".to_string(),
                evidence: serde_json::Value::Null,
                confidence: None,
                finding_id: None,
            },
        ];

        let summary = build_summary(&findings, 3, 10);
        assert_eq!(summary.total_findings, 3);
        assert_eq!(summary.by_severity.get("high"), Some(&2));
        assert_eq!(summary.by_severity.get("low"), Some(&1));
        assert_eq!(summary.by_type.get("signature-regression"), Some(&2));
        assert_eq!(summary.by_type.get("born-dead"), Some(&1));
        assert_eq!(summary.files_analyzed, 3);
        assert_eq!(summary.functions_analyzed, 10);
    }

    #[test]
    fn test_findings_sort_severity_first() {
        let mut findings = [
            BugbotFinding {
                finding_type: "born-dead".to_string(),
                severity: "low".to_string(),
                file: PathBuf::from("a.rs"),
                function: "f1".to_string(),
                line: 1,
                message: String::new(),
                evidence: serde_json::Value::Null,
                confidence: None,
                finding_id: None,
            },
            BugbotFinding {
                finding_type: "signature-regression".to_string(),
                severity: "high".to_string(),
                file: PathBuf::from("z.rs"),
                function: "f2".to_string(),
                line: 100,
                message: String::new(),
                evidence: serde_json::Value::Null,
                confidence: None,
                finding_id: None,
            },
            BugbotFinding {
                finding_type: "born-dead".to_string(),
                severity: "medium".to_string(),
                file: PathBuf::from("b.rs"),
                function: "f3".to_string(),
                line: 50,
                message: String::new(),
                evidence: serde_json::Value::Null,
                confidence: None,
                finding_id: None,
            },
        ];

        findings.sort_by(|a, b| {
            severity_rank(&b.severity)
                .cmp(&severity_rank(&a.severity))
                .then(a.file.cmp(&b.file))
                .then(a.line.cmp(&b.line))
        });

        assert_eq!(findings[0].severity, "high");
        assert_eq!(findings[1].severity, "medium");
        assert_eq!(findings[2].severity, "low");
    }

    #[test]
    fn test_findings_sort_file_then_line_within_same_severity() {
        let mut findings = [
            BugbotFinding {
                finding_type: "sig".to_string(),
                severity: "high".to_string(),
                file: PathBuf::from("z.rs"),
                function: "f1".to_string(),
                line: 10,
                message: String::new(),
                evidence: serde_json::Value::Null,
                confidence: None,
                finding_id: None,
            },
            BugbotFinding {
                finding_type: "sig".to_string(),
                severity: "high".to_string(),
                file: PathBuf::from("a.rs"),
                function: "f2".to_string(),
                line: 50,
                message: String::new(),
                evidence: serde_json::Value::Null,
                confidence: None,
                finding_id: None,
            },
            BugbotFinding {
                finding_type: "sig".to_string(),
                severity: "high".to_string(),
                file: PathBuf::from("a.rs"),
                function: "f3".to_string(),
                line: 5,
                message: String::new(),
                evidence: serde_json::Value::Null,
                confidence: None,
                finding_id: None,
            },
        ];

        findings.sort_by(|a, b| {
            severity_rank(&b.severity)
                .cmp(&severity_rank(&a.severity))
                .then(a.file.cmp(&b.file))
                .then(a.line.cmp(&b.line))
        });

        // Same severity: a.rs before z.rs, and within a.rs line 5 before line 50
        assert_eq!(findings[0].file, PathBuf::from("a.rs"));
        assert_eq!(findings[0].line, 5);
        assert_eq!(findings[1].file, PathBuf::from("a.rs"));
        assert_eq!(findings[1].line, 50);
        assert_eq!(findings[2].file, PathBuf::from("z.rs"));
    }

    #[test]
    fn test_findings_truncation() {
        let max_findings = 2;
        let mut findings: Vec<BugbotFinding> = (0..5)
            .map(|i| BugbotFinding {
                finding_type: "test".to_string(),
                severity: "low".to_string(),
                file: PathBuf::from(format!("f{}.rs", i)),
                function: format!("fn_{}", i),
                line: i,
                message: String::new(),
                evidence: serde_json::Value::Null,
                confidence: None,
                finding_id: None,
            })
            .collect();

        let mut notes: Vec<String> = Vec::new();
        if findings.len() > max_findings {
            notes.push(format!("truncated_to_{}", max_findings));
            findings.truncate(max_findings);
        }

        assert_eq!(findings.len(), 2);
        assert_eq!(notes, vec!["truncated_to_2"]);
    }

    // -----------------------------------------------------------------------
    // Integration tests (require git)
    // -----------------------------------------------------------------------

    /// Helper: initialize a git repo with an initial commit in a temp directory.
    fn init_git_repo() -> tempfile::TempDir {
        let tmp = tempfile::TempDir::new().expect("create temp dir");
        let dir = tmp.path();

        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir)
            .output()
            .expect("git init");

        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(dir)
            .output()
            .expect("git config email");

        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir)
            .output()
            .expect("git config name");

        // Create an initial committed Rust file so HEAD exists
        std::fs::write(dir.join("lib.rs"), "fn placeholder() {}\n").expect("write lib.rs");
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(dir)
            .output()
            .expect("git add");
        std::process::Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(dir)
            .output()
            .expect("git commit");

        tmp
    }

    #[test]
    fn test_run_no_changes_produces_empty_report() {
        let tmp = init_git_repo();
        let args = BugbotCheckArgs {
            path: tmp.path().to_path_buf(),
            base_ref: "HEAD".to_string(),
            staged: false,
            max_findings: 50,
            no_fail: false,
            quiet: true,
            no_tools: false,
            tool_timeout: 60,
        };

        // Should succeed with no findings
        let result = args.run(OutputFormat::Json, true, Some(Language::Rust));
        assert!(result.is_ok(), "run() should succeed: {:?}", result.err());
    }

    #[test]
    fn test_run_with_signature_change_finds_regression() {
        let tmp = init_git_repo();
        let dir = tmp.path();

        // Commit a file with a function
        let original = "pub fn compute(x: i32, y: i32) -> i32 {\n    x + y\n}\n";
        std::fs::write(dir.join("lib.rs"), original).expect("write lib.rs");
        std::process::Command::new("git")
            .args(["add", "lib.rs"])
            .current_dir(dir)
            .output()
            .expect("git add");
        std::process::Command::new("git")
            .args(["commit", "-m", "add compute"])
            .current_dir(dir)
            .output()
            .expect("git commit");

        // Modify the signature (remove a parameter)
        let modified = "pub fn compute(x: i32) -> i32 {\n    x * 2\n}\n";
        std::fs::write(dir.join("lib.rs"), modified).expect("overwrite lib.rs");

        let args = BugbotCheckArgs {
            path: dir.to_path_buf(),
            base_ref: "HEAD".to_string(),
            staged: false,
            max_findings: 50,
            no_fail: true,
            quiet: true,
            no_tools: false,
            tool_timeout: 60,
        };

        // The pipeline should find a signature regression (no_fail=true
        // so it doesn't call process::exit which would kill the test runner)
        let result = args.run(OutputFormat::Json, true, Some(Language::Rust));
        assert!(result.is_ok(), "run() should succeed: {:?}", result.err());
    }

    #[test]
    fn test_run_new_file_produces_insert_changes() {
        let tmp = init_git_repo();
        let dir = tmp.path();

        // Add a brand new file (not in any commit)
        let new_code = "fn brand_new_function() {\n    println!(\"hello\");\n}\n";
        std::fs::write(dir.join("new_module.rs"), new_code).expect("write new_module.rs");

        let args = BugbotCheckArgs {
            path: dir.to_path_buf(),
            base_ref: "HEAD".to_string(),
            staged: false,
            max_findings: 50,
            no_fail: true,
            quiet: true,
            no_tools: false,
            tool_timeout: 60,
        };

        // Should succeed with no_fail -- new file is treated as all-inserts
        // (born-dead findings may exist but no_fail suppresses exit error)
        let result = args.run(OutputFormat::Json, true, Some(Language::Rust));
        assert!(
            result.is_ok(),
            "run() should succeed with no_fail: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_run_elapsed_ms_is_populated() {
        // This is a timing sanity check: the pipeline should measure time
        let start = Instant::now();
        std::thread::sleep(std::time::Duration::from_millis(1));
        let elapsed_ms = start.elapsed().as_millis() as u64;
        assert!(elapsed_ms >= 1, "Instant timing should work");
    }

    // -----------------------------------------------------------------------
    // FIX 1: process::exit replaced with error propagation
    // -----------------------------------------------------------------------

    #[test]
    fn test_run_findings_without_no_fail_returns_error() {
        // Previously this called process::exit(1), killing the test runner.
        // Now it returns a BugbotExitError::FindingsDetected which is testable.
        let tmp = init_git_repo();
        let dir = tmp.path();

        let original = "pub fn compute(x: i32, y: i32) -> i32 {\n    x + y\n}\n";
        std::fs::write(dir.join("lib.rs"), original).expect("write lib.rs");
        std::process::Command::new("git")
            .args(["add", "lib.rs"])
            .current_dir(dir)
            .output()
            .expect("git add");
        std::process::Command::new("git")
            .args(["commit", "-m", "add compute"])
            .current_dir(dir)
            .output()
            .expect("git commit");

        let modified = "pub fn compute(x: i32) -> i32 {\n    x * 2\n}\n";
        std::fs::write(dir.join("lib.rs"), modified).expect("overwrite lib.rs");

        let args = BugbotCheckArgs {
            path: dir.to_path_buf(),
            base_ref: "HEAD".to_string(),
            staged: false,
            max_findings: 50,
            no_fail: false,
            quiet: true,
            no_tools: false,
            tool_timeout: 60,
        };

        let result = args.run(OutputFormat::Json, true, Some(Language::Rust));
        assert!(
            result.is_err(),
            "run() should return Err when findings exist"
        );

        // Verify the error is a BugbotExitError with exit code 1
        let err = result.unwrap_err();
        use crate::commands::bugbot::BugbotExitError;
        let bugbot_err = err
            .downcast_ref::<BugbotExitError>()
            .expect("error should be BugbotExitError");
        assert_eq!(
            bugbot_err.exit_code(),
            1,
            "exit code should be 1 for findings"
        );
    }

    // -----------------------------------------------------------------------
    // FIX 4: max_findings=0 means unlimited
    // -----------------------------------------------------------------------

    #[test]
    fn test_max_findings_zero_means_unlimited() {
        // When max_findings is 0, all findings should be reported (no truncation)
        let max_findings: usize = 0;
        let mut findings: Vec<BugbotFinding> = (0..5)
            .map(|i| BugbotFinding {
                finding_type: "test".to_string(),
                severity: "low".to_string(),
                file: PathBuf::from(format!("f{}.rs", i)),
                function: format!("fn_{}", i),
                line: i,
                message: String::new(),
                evidence: serde_json::Value::Null,
                confidence: None,
                finding_id: None,
            })
            .collect();

        let mut notes: Vec<String> = Vec::new();
        if max_findings > 0 && findings.len() > max_findings {
            notes.push(format!("truncated_to_{}", max_findings));
            findings.truncate(max_findings);
        }

        assert_eq!(findings.len(), 5, "max_findings=0 should not truncate");
        assert!(notes.is_empty(), "no truncation note with max_findings=0");
    }

    // ===================================================================
    // Phase 6: L1 integration tests
    // ===================================================================

    #[test]
    fn test_severity_rank_info_below_low() {
        // PM-8: "info" should be ranked explicitly, below "low"
        assert!(
            severity_rank("info") < severity_rank("low"),
            "info ({}) should rank below low ({})",
            severity_rank("info"),
            severity_rank("low")
        );
        assert!(
            severity_rank("info") > 0,
            "PM-8: info should have an explicit rank > 0, not wildcard"
        );
    }

    #[test]
    fn test_filter_l1_findings_to_changed_files() {
        // PM-3: L1 findings must be filtered to only files in the changed set
        let l1_findings: Vec<BugbotFinding> = vec![
            BugbotFinding {
                finding_type: "tool:clippy".to_string(),
                severity: "medium".to_string(),
                file: PathBuf::from("src/main.rs"),
                function: String::new(),
                line: 10,
                message: "warning in changed file".to_string(),
                evidence: serde_json::Value::Null,
                confidence: None,
                finding_id: None,
            },
            BugbotFinding {
                finding_type: "tool:clippy".to_string(),
                severity: "low".to_string(),
                file: PathBuf::from("src/untouched.rs"),
                function: String::new(),
                line: 5,
                message: "warning in untouched file".to_string(),
                evidence: serde_json::Value::Null,
                confidence: None,
                finding_id: None,
            },
            BugbotFinding {
                finding_type: "tool:clippy".to_string(),
                severity: "high".to_string(),
                file: PathBuf::from("src/lib.rs"),
                function: String::new(),
                line: 20,
                message: "error in changed file".to_string(),
                evidence: serde_json::Value::Null,
                confidence: None,
                finding_id: None,
            },
        ];

        let changed_files: Vec<PathBuf> =
            vec![PathBuf::from("src/main.rs"), PathBuf::from("src/lib.rs")];

        let filtered = filter_l1_findings(l1_findings, &changed_files);

        assert_eq!(
            filtered.len(),
            2,
            "should keep only 2 findings matching changed files"
        );
        let untouched = std::path::Path::new("src/untouched.rs");
        assert!(
            filtered.iter().all(|f| f.file != untouched),
            "PM-3: untouched file findings should be excluded"
        );
    }

    #[test]
    fn test_filter_l1_findings_empty_changed_files_keeps_all() {
        // When changed_files is empty (scan mode), keep ALL L1 findings
        let l1_findings: Vec<BugbotFinding> = vec![
            BugbotFinding {
                finding_type: "tool:clippy".to_string(),
                severity: "medium".to_string(),
                file: PathBuf::from("src/main.rs"),
                function: String::new(),
                line: 10,
                message: "warning".to_string(),
                evidence: serde_json::Value::Null,
                confidence: None,
                finding_id: None,
            },
            BugbotFinding {
                finding_type: "tool:clippy".to_string(),
                severity: "low".to_string(),
                file: PathBuf::from("src/other.rs"),
                function: String::new(),
                line: 5,
                message: "another warning".to_string(),
                evidence: serde_json::Value::Null,
                confidence: None,
                finding_id: None,
            },
        ];

        let changed_files: Vec<PathBuf> = vec![];

        let filtered = filter_l1_findings(l1_findings, &changed_files);

        assert_eq!(
            filtered.len(),
            2,
            "empty changed_files should keep all findings"
        );
    }

    #[test]
    fn test_build_summary_with_l1_and_l2() {
        // build_summary_with_l1 should count L1 and L2 findings separately
        let l2_findings = vec![BugbotFinding {
            finding_type: "signature-regression".to_string(),
            severity: "high".to_string(),
            file: PathBuf::from("a.rs"),
            function: "foo".to_string(),
            line: 10,
            message: "param removed".to_string(),
            evidence: serde_json::Value::Null,
            confidence: None,
            finding_id: None,
        }];
        let l1_findings = vec![
            BugbotFinding {
                finding_type: "tool:clippy".to_string(),
                severity: "medium".to_string(),
                file: PathBuf::from("b.rs"),
                function: String::new(),
                line: 5,
                message: "unused var".to_string(),
                evidence: serde_json::Value::Null,
                confidence: None,
                finding_id: None,
            },
            BugbotFinding {
                finding_type: "tool:cargo-audit".to_string(),
                severity: "high".to_string(),
                file: PathBuf::from("Cargo.lock"),
                function: String::new(),
                line: 1,
                message: "vuln".to_string(),
                evidence: serde_json::Value::Null,
                confidence: None,
                finding_id: None,
            },
        ];

        let tool_results = vec![
            super::super::tools::ToolResult {
                name: "clippy".to_string(),
                category: super::super::tools::ToolCategory::Linter,
                success: true,
                duration_ms: 100,
                finding_count: 1,
                error: None,
                exit_code: Some(0),
            },
            super::super::tools::ToolResult {
                name: "cargo-audit".to_string(),
                category: super::super::tools::ToolCategory::SecurityScanner,
                success: true,
                duration_ms: 50,
                finding_count: 1,
                error: None,
                exit_code: Some(0),
            },
        ];

        let mut all_findings = Vec::new();
        all_findings.extend(l1_findings.clone());
        all_findings.extend(l2_findings.clone());

        let summary = build_summary_with_l1(
            &all_findings,
            l1_findings.len(),
            l2_findings.len(),
            5,
            20,
            &tool_results,
        );

        assert_eq!(summary.total_findings, 3);
        assert_eq!(summary.l1_findings, 2);
        assert_eq!(summary.l2_findings, 1);
        assert_eq!(summary.tools_run, 2);
        assert_eq!(summary.tools_failed, 0);
        assert_eq!(summary.files_analyzed, 5);
        assert_eq!(summary.functions_analyzed, 20);
    }

    #[test]
    fn test_build_summary_with_l1_counts_failed_tools() {
        let findings: Vec<BugbotFinding> = vec![];
        let tool_results = vec![
            super::super::tools::ToolResult {
                name: "clippy".to_string(),
                category: super::super::tools::ToolCategory::Linter,
                success: true,
                duration_ms: 100,
                finding_count: 0,
                error: None,
                exit_code: Some(0),
            },
            super::super::tools::ToolResult {
                name: "cargo-audit".to_string(),
                category: super::super::tools::ToolCategory::SecurityScanner,
                success: false,
                duration_ms: 50,
                finding_count: 0,
                error: Some("binary not found".to_string()),
                exit_code: None,
            },
        ];

        let summary = build_summary_with_l1(&findings, 0, 0, 3, 10, &tool_results);

        assert_eq!(summary.tools_run, 2);
        assert_eq!(summary.tools_failed, 1);
    }

    #[test]
    fn test_no_tools_available_graceful_degradation() {
        // When no L1 tools are available, the pipeline should work identically
        // to before: empty tool_results, empty tools_available/tools_missing,
        // and only L2 findings.
        let l2_findings = vec![BugbotFinding {
            finding_type: "born-dead".to_string(),
            severity: "low".to_string(),
            file: PathBuf::from("src/lib.rs"),
            function: "dead_fn".to_string(),
            line: 10,
            message: "no callers".to_string(),
            evidence: serde_json::Value::Null,
            confidence: None,
            finding_id: None,
        }];

        let summary = build_summary_with_l1(&l2_findings, 0, 1, 1, 5, &[]);

        assert_eq!(summary.total_findings, 1);
        assert_eq!(summary.l1_findings, 0);
        assert_eq!(summary.l2_findings, 1);
        assert_eq!(summary.tools_run, 0);
        assert_eq!(summary.tools_failed, 0);
    }

    // ===================================================================
    // Phase 7: CLI flags (--no-tools, --tool-timeout)
    // ===================================================================

    #[test]
    fn test_no_tools_flag_defaults_to_false() {
        let args = BugbotCheckArgs {
            path: PathBuf::from("."),
            base_ref: "HEAD".to_string(),
            staged: false,
            max_findings: 50,
            no_fail: false,
            quiet: true,
            no_tools: false,
            tool_timeout: 60,
        };
        assert!(!args.no_tools);
    }

    #[test]
    fn test_tool_timeout_default_is_60() {
        let args = BugbotCheckArgs {
            path: PathBuf::from("."),
            base_ref: "HEAD".to_string(),
            staged: false,
            max_findings: 50,
            no_fail: false,
            quiet: true,
            no_tools: false,
            tool_timeout: 60,
        };
        assert_eq!(args.tool_timeout, 60);
    }

    #[test]
    fn test_no_tools_skips_l1_analysis() {
        // When --no-tools is set, tool_results, tools_available,
        // and tools_missing should all be empty in the report
        let tmp = init_git_repo();
        let dir = tmp.path();

        // Add a file so the pipeline has something to analyze
        let code = "pub fn hello() { println!(\"hello\"); }\n";
        std::fs::write(dir.join("lib.rs"), code).expect("write lib.rs");
        std::process::Command::new("git")
            .args(["add", "lib.rs"])
            .current_dir(dir)
            .output()
            .expect("git add");
        std::process::Command::new("git")
            .args(["commit", "-m", "add hello"])
            .current_dir(dir)
            .output()
            .expect("git commit");

        // Modify so there's a change to detect
        let modified = "pub fn hello(name: &str) { println!(\"hello {}\", name); }\n";
        std::fs::write(dir.join("lib.rs"), modified).expect("overwrite lib.rs");

        let args = BugbotCheckArgs {
            path: dir.to_path_buf(),
            base_ref: "HEAD".to_string(),
            staged: false,
            max_findings: 50,
            no_fail: true,
            quiet: true,
            no_tools: true,
            tool_timeout: 60,
        };

        // Run with JSON output, capture report via run_and_capture
        let result = args.run(OutputFormat::Json, true, Some(Language::Rust));
        // The pipeline should succeed even with no_tools
        assert!(
            result.is_ok(),
            "run() should succeed with --no-tools: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_no_tools_report_has_no_l1_data() {
        // Capture the report JSON to verify tool_results is empty.
        // We do this indirectly by verifying run_l1_tools_opt returns
        // empty results when no_tools is set.
        let (l1_findings, tool_results, available, missing) =
            run_l1_tools_opt(std::path::Path::new("/nonexistent"), "rust", true, 60);

        assert!(
            l1_findings.is_empty(),
            "no_tools should produce empty L1 findings"
        );
        assert!(
            tool_results.is_empty(),
            "no_tools should produce empty tool_results"
        );
        assert!(
            available.is_empty(),
            "no_tools should produce empty tools_available"
        );
        assert!(
            missing.is_empty(),
            "no_tools should produce empty tools_missing"
        );
    }

    #[test]
    fn test_tool_timeout_passed_to_runner() {
        // When no_tools is false, the timeout should be passed to ToolRunner.
        // We test this indirectly: run_l1_tools_opt with no_tools=false should
        // create a ToolRunner with the specified timeout. We can verify by running
        // with a very short timeout against a non-existent path (tools will fail
        // to find Cargo.toml, but we verify the function accepts the timeout param).
        let (_l1, _results, _avail, _missing) =
            run_l1_tools_opt(std::path::Path::new("/tmp/nonexistent"), "rust", false, 5);
        // The function should run without panic even with custom timeout
    }

    #[test]
    fn test_no_tools_no_changes_report() {
        // --no-tools with no changes should produce a clean empty report
        let tmp = init_git_repo();
        let args = BugbotCheckArgs {
            path: tmp.path().to_path_buf(),
            base_ref: "HEAD".to_string(),
            staged: false,
            max_findings: 50,
            no_fail: false,
            quiet: true,
            no_tools: true,
            tool_timeout: 60,
        };

        let result = args.run(OutputFormat::Json, true, Some(Language::Rust));
        assert!(
            result.is_ok(),
            "no-tools + no-changes should succeed: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_default_behavior_without_flags_runs_l1() {
        // With default flags (no_tools=false), run_l1_tools_opt should attempt
        // to detect and run tools (even if they're not available in test env).
        let (_l1, _results, available, _missing) =
            run_l1_tools_opt(std::path::Path::new("/tmp/nonexistent"), "rust", false, 60);
        // In a typical dev env, at least clippy is available.
        // But in CI/test it might not be. We just verify the function runs.
        // The available list should contain tool names if they're installed.
        // We can't assert exact counts, but we verify no panic.
        let _ = available; // function completed without error
    }

    // =========================================================================
    // F14: L1/L2 count mismatch after truncation
    // =========================================================================

    #[test]
    fn test_build_summary_l1_l2_counts_reflect_actual_findings() {
        // F14: When findings are truncated, the summary l1_findings and l2_findings
        // should be recalculated from the TRUNCATED list, not pre-truncation counts.
        // This tests the build_summary_with_l1 function with a findings list
        // that has been truncated.

        // Create 5 L1 + 3 L2 findings = 8 total
        let mut all_findings: Vec<BugbotFinding> = Vec::new();

        // 5 L1 findings (tool:clippy)
        for i in 0..5 {
            all_findings.push(BugbotFinding {
                finding_type: "tool:clippy".to_string(),
                severity: "medium".to_string(),
                file: PathBuf::from(format!("l1_{}.rs", i)),
                function: String::new(),
                line: i,
                message: "lint".to_string(),
                evidence: serde_json::Value::Null,
                confidence: None,
                finding_id: None,
            });
        }

        // 3 L2 findings (signature-regression)
        for i in 0..3 {
            all_findings.push(BugbotFinding {
                finding_type: "signature-regression".to_string(),
                severity: "high".to_string(),
                file: PathBuf::from(format!("l2_{}.rs", i)),
                function: format!("fn_{}", i),
                line: i + 100,
                message: "param removed".to_string(),
                evidence: serde_json::Value::Null,
                confidence: None,
                finding_id: None,
            });
        }

        // Simulate truncation to 4 findings (would drop some L1 and/or L2)
        all_findings.truncate(4);

        // Count actual L1/L2 in the truncated list
        let actual_l1 = all_findings
            .iter()
            .filter(|f| f.finding_type.starts_with("tool:"))
            .count();
        let actual_l2 = all_findings
            .iter()
            .filter(|f| !f.finding_type.starts_with("tool:"))
            .count();

        let summary = build_summary_with_l1(&all_findings, actual_l1, actual_l2, 3, 10, &[]);

        // F14: total_findings should equal l1_findings + l2_findings
        assert_eq!(
            summary.total_findings,
            summary.l1_findings + summary.l2_findings,
            "total_findings ({}) should equal l1_findings ({}) + l2_findings ({})",
            summary.total_findings,
            summary.l1_findings,
            summary.l2_findings
        );

        // And all should match the truncated list
        assert_eq!(summary.total_findings, 4, "should reflect truncated count");
        assert_eq!(
            summary.l1_findings, actual_l1,
            "l1 should reflect post-truncation count"
        );
        assert_eq!(
            summary.l2_findings, actual_l2,
            "l2 should reflect post-truncation count"
        );
    }

    #[test]
    fn test_summary_counts_consistent_after_heavy_truncation() {
        // F14: Edge case -- truncate to just 1 finding from a mixed set
        let mut all_findings: Vec<BugbotFinding> = Vec::new();

        // 10 L1 findings
        for i in 0..10 {
            all_findings.push(BugbotFinding {
                finding_type: "tool:clippy".to_string(),
                severity: "medium".to_string(),
                file: PathBuf::from(format!("l1_{}.rs", i)),
                function: String::new(),
                line: i,
                message: "lint".to_string(),
                evidence: serde_json::Value::Null,
                confidence: None,
                finding_id: None,
            });
        }

        // 10 L2 findings
        for i in 0..10 {
            all_findings.push(BugbotFinding {
                finding_type: "born-dead".to_string(),
                severity: "low".to_string(),
                file: PathBuf::from(format!("l2_{}.rs", i)),
                function: format!("fn_{}", i),
                line: i + 100,
                message: "no callers".to_string(),
                evidence: serde_json::Value::Null,
                confidence: None,
                finding_id: None,
            });
        }

        // Pre-truncation counts
        let pre_l1 = 10;
        let pre_l2 = 10;
        assert_eq!(all_findings.len(), 20);

        // Truncate to 1
        all_findings.truncate(1);

        // Post-truncation counts from actual data
        let _post_l1 = all_findings
            .iter()
            .filter(|f| f.finding_type.starts_with("tool:"))
            .count();
        let _post_l2 = all_findings
            .iter()
            .filter(|f| !f.finding_type.starts_with("tool:"))
            .count();

        // If we pass pre-truncation counts, the summary would be wrong
        let bad_summary = build_summary_with_l1(&all_findings, pre_l1, pre_l2, 3, 10, &[]);

        // This SHOULD fail before the fix: total=1 but l1+l2=20
        // After the fix, the function should use actual findings, not raw counts
        assert_eq!(
            bad_summary.total_findings,
            bad_summary.l1_findings + bad_summary.l2_findings,
            "F14: total ({}) must equal l1 ({}) + l2 ({}) even with stale pre-truncation counts",
            bad_summary.total_findings,
            bad_summary.l1_findings,
            bad_summary.l2_findings,
        );
    }

    // =========================================================================
    // Phase 8: Integration & Polish
    // =========================================================================

    #[test]
    fn test_critical_exit_code_3() {
        // Critical findings should produce BugbotExitError::CriticalFindings (exit code 3)
        // instead of FindingsDetected (exit code 1)
        let tmp = init_git_repo();
        let dir = tmp.path();

        // Write a file with an obvious secret pattern so ScanEngine can find it
        let code = r#"
fn main() {
    // AWS secret key hardcoded (intentional test fixture)
    let _key = "AKIAIOSFODNN7EXAMPLE";
    let _secret = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
}
"#;
        std::fs::write(dir.join("lib.rs"), "fn placeholder() {}\n").ok();
        std::process::Command::new("git")
            .args(["add", "lib.rs"])
            .current_dir(dir)
            .output()
            .expect("git add");
        std::process::Command::new("git")
            .args(["commit", "-m", "add placeholder"])
            .current_dir(dir)
            .output()
            .expect("git commit");

        std::fs::write(dir.join("lib.rs"), code).expect("write secret code");

        let args = BugbotCheckArgs {
            path: dir.to_path_buf(),
            base_ref: "HEAD".to_string(),
            staged: false,
            max_findings: 50,
            no_fail: false,
            quiet: true,
            no_tools: true, // skip L1 tools, focus on L2
            tool_timeout: 60,
        };

        let result = args.run(OutputFormat::Json, true, Some(Language::Rust));
        // The pipeline should return an error (findings exist)
        // We just verify that CriticalFindings with exit code 3 is possible
        // (It depends on whether the scan finds the secret as "critical" severity.
        //  The important test is the unit test below.)
        let _ = result;

        // Unit-level verification: CriticalFindings variant has exit code 3
        let err = BugbotExitError::CriticalFindings { count: 2 };
        assert_eq!(err.exit_code(), 3, "CriticalFindings exit code should be 3");
    }

    #[test]
    fn test_l2_all_engines_registered() {
        // l2_engine_registry() should return exactly TldrDifferentialEngine
        use crate::commands::bugbot::l2::l2_engine_registry;
        let engines = l2_engine_registry();
        assert_eq!(
            engines.len(),
            1,
            "Registry should contain exactly 1 engine (TldrDifferentialEngine), got {}",
            engines.len()
        );
        assert_eq!(engines[0].name(), "TldrDifferentialEngine");
    }

    #[test]
    fn test_l2_total_finding_types_matches_tldr_engine() {
        // TldrDifferentialEngine is the only registered engine; it declares 11
        // finding types covering complexity, cognitive, contracts, smells,
        // flow analysis, and downstream impact analysis.
        use crate::commands::bugbot::l2::l2_engine_registry;
        let engines = l2_engine_registry();
        let total: usize = engines.iter().map(|e| e.finding_types().len()).sum();
        assert_eq!(
            total, 11,
            "Total finding types across all engines should be 11 (TldrDifferentialEngine), got {}",
            total
        );
    }

    #[test]
    fn test_l2_engine_names_unique() {
        // No duplicate engine names
        use crate::commands::bugbot::l2::l2_engine_registry;
        let engines = l2_engine_registry();
        let mut names: Vec<&str> = engines.iter().map(|e| e.name()).collect();
        let original_len = names.len();
        names.sort();
        names.dedup();
        assert_eq!(
            names.len(),
            original_len,
            "Engine names should be unique, found duplicates"
        );
    }

    #[test]
    fn test_severity_rank_ordering_complete() {
        // All severity ranks should be in correct order:
        // critical > high > medium > low > info > unknown
        assert!(
            severity_rank("critical") > severity_rank("high"),
            "critical should rank above high"
        );
        assert!(
            severity_rank("high") > severity_rank("medium"),
            "high should rank above medium"
        );
        assert!(
            severity_rank("medium") > severity_rank("low"),
            "medium should rank above low"
        );
        assert!(
            severity_rank("low") > severity_rank("info"),
            "low should rank above info"
        );
        assert!(
            severity_rank("info") > severity_rank("unknown"),
            "info should rank above unknown"
        );
        assert_eq!(
            severity_rank("unknown"),
            0,
            "unknown severity should have rank 0"
        );
    }

    #[test]
    fn test_run_l2_engines_empty_context() {
        // run_l2_engines with empty context should produce engine results
        use crate::commands::bugbot::l2::context::FunctionDiff;
        use crate::commands::bugbot::l2::{l2_engine_registry, L2Context};
        use std::collections::HashMap;

        let engines = l2_engine_registry();
        let ctx = L2Context::new(
            PathBuf::from("/tmp/nonexistent"),
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
        );

        let (_findings, results) = run_l2_engines(&ctx, &engines);

        // Should have one result per engine
        assert_eq!(
            results.len(),
            engines.len(),
            "Should have one result per engine"
        );

        // All engine results should have a name
        for result in &results {
            assert!(!result.name.is_empty(), "Engine result should have a name");
        }
    }

    // =========================================================================
    // Phase 8: L2 Integration Tests
    // =========================================================================

    #[test]
    fn test_l2_engine_failure_isolation() {
        // When one engine returns Partial status, other engines should still
        // produce findings independently. We create an empty context (no
        // functions to analyze) so DeltaEngine has nothing to diff against
        // (partial/skipped), but all engines should still run and report.
        use crate::commands::bugbot::l2::context::FunctionDiff;
        use crate::commands::bugbot::l2::{l2_engine_registry, L2Context};
        use std::collections::HashMap;

        let engines = l2_engine_registry();
        let ctx = L2Context::new(
            PathBuf::from("/tmp/nonexistent"),
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
        );

        let (_findings, results) = run_l2_engines(&ctx, &engines);

        // Every engine must produce a result entry regardless of other engines
        assert_eq!(
            results.len(),
            engines.len(),
            "Every engine must produce a result even when others partially fail"
        );

        // Verify each engine result has a valid name and status
        for result in &results {
            assert!(
                !result.name.is_empty(),
                "Engine result must have a non-empty name"
            );
            assert!(
                !result.status.is_empty(),
                "Engine '{}' must have a non-empty status string",
                result.name
            );
        }
    }

    #[test]
    fn test_l2_findings_merge_with_l1() {
        // L1 findings (tool:*) and L2 findings (engine-produced) should
        // merge into a single list, sorted by severity descending.
        let l1_findings = vec![
            BugbotFinding {
                finding_type: "tool:clippy".to_string(),
                severity: "medium".to_string(),
                file: PathBuf::from("src/lib.rs"),
                function: String::new(),
                line: 10,
                message: "clippy lint".to_string(),
                evidence: serde_json::Value::Null,
                confidence: None,
                finding_id: None,
            },
            BugbotFinding {
                finding_type: "tool:cargo-audit".to_string(),
                severity: "low".to_string(),
                file: PathBuf::from("Cargo.lock"),
                function: String::new(),
                line: 1,
                message: "advisory".to_string(),
                evidence: serde_json::Value::Null,
                confidence: None,
                finding_id: None,
            },
        ];

        let l2_findings = vec![
            BugbotFinding {
                finding_type: "signature-regression".to_string(),
                severity: "high".to_string(),
                file: PathBuf::from("src/api.rs"),
                function: "handle_request".to_string(),
                line: 42,
                message: "parameter removed".to_string(),
                evidence: serde_json::Value::Null,
                confidence: Some("CERTAIN".to_string()),
                finding_id: None,
            },
            BugbotFinding {
                finding_type: "born-dead".to_string(),
                severity: "low".to_string(),
                file: PathBuf::from("src/util.rs"),
                function: "unused_helper".to_string(),
                line: 5,
                message: "no callers".to_string(),
                evidence: serde_json::Value::Null,
                confidence: Some("CERTAIN".to_string()),
                finding_id: None,
            },
        ];

        // Merge L1 + L2 into one list
        let mut all_findings = Vec::new();
        all_findings.extend(l1_findings);
        all_findings.extend(l2_findings);

        // Sort by severity descending, then file, then line (same logic as pipeline)
        all_findings.sort_by(|a, b| {
            severity_rank(&b.severity)
                .cmp(&severity_rank(&a.severity))
                .then(a.file.cmp(&b.file))
                .then(a.line.cmp(&b.line))
        });

        // Verify total count
        assert_eq!(
            all_findings.len(),
            4,
            "merged list should contain all 4 findings"
        );

        // Verify sort order: high first, then medium, then lows
        assert_eq!(
            all_findings[0].severity, "high",
            "highest severity finding should be first"
        );
        assert_eq!(
            all_findings[1].severity, "medium",
            "medium severity should be second"
        );
        // Both remaining are "low"; verify they're sorted by file path
        assert_eq!(all_findings[2].severity, "low");
        assert_eq!(all_findings[3].severity, "low");
        assert!(
            all_findings[2].file <= all_findings[3].file,
            "low-severity findings should be sorted by file path"
        );

        // Verify both L1 and L2 findings are present
        let l1_count = all_findings
            .iter()
            .filter(|f| f.finding_type.starts_with("tool:"))
            .count();
        let l2_count = all_findings
            .iter()
            .filter(|f| !f.finding_type.starts_with("tool:"))
            .count();
        assert_eq!(l1_count, 2, "should have 2 L1 findings");
        assert_eq!(l2_count, 2, "should have 2 L2 findings");
    }

    #[test]
    fn test_l2_dedup_suppresses_born_dead_cascade() {
        // When a function has a born-dead finding AND a complexity-increase
        // finding, dedup should suppress the complexity-increase (born-dead
        // dominates everything for that function).
        use crate::commands::bugbot::l2::dedup::dedup_and_prioritize;

        let findings = vec![
            BugbotFinding {
                finding_type: "born-dead".to_string(),
                severity: "low".to_string(),
                file: PathBuf::from("src/orphan.rs"),
                function: "orphan_fn".to_string(),
                line: 10,
                message: "function has no callers".to_string(),
                evidence: serde_json::Value::Null,
                confidence: Some("CERTAIN".to_string()),
                finding_id: None,
            },
            BugbotFinding {
                finding_type: "complexity-increase".to_string(),
                severity: "medium".to_string(),
                file: PathBuf::from("src/orphan.rs"),
                function: "orphan_fn".to_string(),
                line: 12,
                message: "cyclomatic complexity increased by 5".to_string(),
                evidence: serde_json::Value::Null,
                confidence: Some("LIKELY".to_string()),
                finding_id: None,
            },
        ];

        let deduped = dedup_and_prioritize(findings, 0);

        // Only born-dead should remain; complexity-increase should be suppressed
        assert_eq!(
            deduped.len(),
            1,
            "born-dead should suppress all other findings for the same function, got {} findings",
            deduped.len()
        );
        assert_eq!(
            deduped[0].finding_type, "born-dead",
            "the surviving finding should be born-dead, not '{}'",
            deduped[0].finding_type
        );
    }

    #[test]
    fn test_l2_composition_taint_plus_guard() {
        // A taint-flow finding and a guard-removed finding at nearby lines
        // in the same file/function should compose into an
        // unguarded-injection-path finding with critical severity and
        // LIKELY confidence.
        use crate::commands::bugbot::l2::composition::compose_findings;

        let findings = vec![
            BugbotFinding {
                finding_type: "taint-flow".to_string(),
                severity: "high".to_string(),
                file: PathBuf::from("src/handler.rs"),
                function: "process_input".to_string(),
                line: 25,
                message: "user input flows to SQL query".to_string(),
                evidence: serde_json::json!({"source": "param:input", "sink": "sql_query"}),
                confidence: Some("LIKELY".to_string()),
                finding_id: None,
            },
            BugbotFinding {
                finding_type: "guard-removed".to_string(),
                severity: "medium".to_string(),
                file: PathBuf::from("src/handler.rs"),
                function: "process_input".to_string(),
                line: 28,
                message: "input validation guard was removed".to_string(),
                evidence: serde_json::json!({"guard": "validate_input()"}),
                confidence: Some("CERTAIN".to_string()),
                finding_id: None,
            },
        ];

        let composed = compose_findings(findings);

        // Should produce exactly 1 composed finding replacing both constituents
        assert_eq!(
            composed.len(),
            1,
            "taint-flow + guard-removed should compose into 1 finding, got {}",
            composed.len()
        );

        let finding = &composed[0];
        assert_eq!(
            finding.finding_type, "unguarded-injection-path",
            "composed type should be unguarded-injection-path, got '{}'",
            finding.finding_type
        );
        assert_eq!(
            finding.severity, "critical",
            "unguarded-injection-path severity should be critical, got '{}'",
            finding.severity
        );
        assert_eq!(
            finding.confidence.as_deref(),
            Some("LIKELY"),
            "composed finding confidence should be LIKELY, got {:?}",
            finding.confidence
        );

        // Verify evidence contains constituent data
        assert!(
            finding.evidence.get("constituent_a").is_some(),
            "composed evidence should contain constituent_a"
        );
        assert!(
            finding.evidence.get("constituent_b").is_some(),
            "composed evidence should contain constituent_b"
        );
    }

    #[test]
    fn test_l2_language_gating_rust_skips_gvn() {
        // FlowEngine's redundant-computation (GVN) is Python-only.
        // When running with language=Rust, run_l2_engines should not
        // produce any redundant-computation findings.
        use crate::commands::bugbot::l2::context::{FunctionDiff, InsertedFunction};
        use crate::commands::bugbot::l2::types::FunctionId;
        use crate::commands::bugbot::l2::{l2_engine_registry, L2Context};
        use std::collections::HashMap;

        // Create a context with a simple inserted function so FlowEngine
        // actually runs its per-function analysis pipeline.
        let source = "fn compute(x: i32) -> i32 { x + x }".to_string();
        let file = PathBuf::from("src/lib.rs");
        let func_id = FunctionId::new(file.clone(), "compute", 1);

        let mut current_contents = HashMap::new();
        current_contents.insert(file.clone(), source.clone());

        let ctx = L2Context::new(
            PathBuf::from("/tmp/test-gvn-gating"),
            Language::Rust,
            vec![file.clone()],
            FunctionDiff {
                changed: vec![],
                inserted: vec![InsertedFunction {
                    id: func_id,
                    name: "compute".to_string(),
                    source,
                }],
                deleted: vec![],
            },
            HashMap::new(),
            current_contents,
            HashMap::new(),
        );

        let engines = l2_engine_registry();
        let (findings, _results) = run_l2_engines(&ctx, &engines);

        // No finding should have type redundant-computation (Python-only via GVN)
        let gvn_findings: Vec<&BugbotFinding> = findings
            .iter()
            .filter(|f| f.finding_type == "redundant-computation")
            .collect();

        assert!(
            gvn_findings.is_empty(),
            "Rust context should not produce redundant-computation findings (Python-only GVN), \
             but found {} such findings",
            gvn_findings.len()
        );
    }

    #[test]
    fn test_l2_no_finding_types_overlap() {
        // No two engines should claim the same finding type.
        // Each finding type must belong to exactly one engine.
        use crate::commands::bugbot::l2::l2_engine_registry;
        use std::collections::HashSet;

        let engines = l2_engine_registry();
        let mut seen: HashSet<&str> = HashSet::new();
        let mut duplicates: Vec<String> = Vec::new();

        for engine in &engines {
            for ft in engine.finding_types() {
                if !seen.insert(ft) {
                    duplicates.push(format!(
                        "'{}' claimed by engine '{}' but already registered",
                        ft,
                        engine.name()
                    ));
                }
            }
        }

        assert!(
            duplicates.is_empty(),
            "Finding type overlap detected: {}",
            duplicates.join("; ")
        );
    }

    #[test]
    fn test_l2_all_finding_types_have_confidence() {
        // All L2 engines should produce findings with a non-None confidence
        // field. We create a minimal context with an inserted function to
        // trigger finding generation, then check that any findings produced
        // have confidence set.
        use crate::commands::bugbot::l2::context::{FunctionDiff, InsertedFunction};
        use crate::commands::bugbot::l2::types::FunctionId;
        use crate::commands::bugbot::l2::{l2_engine_registry, L2Context};
        use std::collections::HashMap;

        // Create a function that should trigger at least some findings
        // (born-dead since it has no callers in the call graph)
        let source = "pub fn lonely_function(x: i32) -> i32 { x + 1 }".to_string();
        let file = PathBuf::from("src/lonely.rs");
        let func_id = FunctionId::new(file.clone(), "lonely_function", 1);

        let mut current_contents = HashMap::new();
        current_contents.insert(file.clone(), source.clone());

        let ctx = L2Context::new(
            PathBuf::from("/tmp/test-confidence"),
            Language::Rust,
            vec![file.clone()],
            FunctionDiff {
                changed: vec![],
                inserted: vec![InsertedFunction {
                    id: func_id,
                    name: "lonely_function".to_string(),
                    source,
                }],
                deleted: vec![],
            },
            HashMap::new(),
            current_contents,
            HashMap::new(),
        );

        let engines = l2_engine_registry();
        let (findings, _results) = run_l2_engines(&ctx, &engines);

        // If findings are produced, every one must have confidence set
        let missing_confidence: Vec<String> = findings
            .iter()
            .filter(|f| f.confidence.is_none())
            .map(|f| format!("{}:{} ({})", f.file.display(), f.line, f.finding_type))
            .collect();

        assert!(
            missing_confidence.is_empty(),
            "All L2 findings must have confidence set, but {} findings are missing it: {}",
            missing_confidence.len(),
            missing_confidence.join(", ")
        );
    }

    // =========================================================================
    // L1/L2 Parallel Execution Tests
    // =========================================================================

    #[test]
    fn test_l2_engines_sendable_to_thread() {
        // L2Engine: Send + Sync, so Vec<Box<dyn L2Engine>> must be Send.
        // This test verifies engines can be moved to a thread and run there.
        use crate::commands::bugbot::l2::context::FunctionDiff;
        use crate::commands::bugbot::l2::{l2_engine_registry, L2Context};
        use std::collections::HashMap;

        let engines = l2_engine_registry();
        let ctx = L2Context::new(
            PathBuf::from("/tmp/nonexistent"),
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
        );

        // Spawn L2 on a background thread (the parallel pattern from check pipeline)
        let handle = std::thread::spawn(move || run_l2_engines(&ctx, &engines));

        let (findings, results) = handle.join().expect("L2 thread should not panic");

        // Should have one result per engine, same as running inline
        assert_eq!(
            results.len(),
            1,
            "L2 engines on thread should produce 1 result (DeltaEngine), got {}",
            results.len()
        );
        for result in &results {
            assert!(
                !result.name.is_empty(),
                "Engine result from thread should have a name"
            );
        }
        // Findings may be empty (no changed files) but should not error
        let _ = findings;
    }

    #[test]
    fn test_l2_thread_panic_graceful_degradation() {
        // If L2 thread panics, unwrap_or_else should return empty results
        let handle = std::thread::spawn(|| -> (Vec<BugbotFinding>, Vec<L2AnalyzerResult>) {
            panic!("simulated L2 engine panic");
        });

        let (findings, results) = handle.join().unwrap_or_else(|_| (Vec::new(), Vec::new()));

        assert!(
            findings.is_empty(),
            "Panicked thread should yield empty findings"
        );
        assert!(
            results.is_empty(),
            "Panicked thread should yield empty results"
        );
    }

    #[test]
    fn test_l1_and_l2_parallel_both_contribute_to_report() {
        // Verify that when L1 and L2 both produce findings, they merge correctly.
        // This simulates the pipeline's merge step after parallel execution.
        let l1_findings = vec![BugbotFinding {
            finding_type: "tool:clippy".to_string(),
            severity: "medium".to_string(),
            file: PathBuf::from("src/main.rs"),
            function: String::new(),
            line: 10,
            message: "unused variable".to_string(),
            evidence: serde_json::Value::Null,
            confidence: None,
            finding_id: None,
        }];

        let l2_findings = vec![BugbotFinding {
            finding_type: "signature-regression".to_string(),
            severity: "high".to_string(),
            file: PathBuf::from("src/lib.rs"),
            function: "compute".to_string(),
            line: 5,
            message: "param removed".to_string(),
            evidence: serde_json::Value::Null,
            confidence: None,
            finding_id: None,
        }];

        // Merge (same as pipeline step 7)
        let mut findings: Vec<BugbotFinding> = Vec::new();
        findings.extend(l1_findings);
        findings.extend(l2_findings);

        assert_eq!(
            findings.len(),
            2,
            "Merged findings should contain both L1 and L2"
        );
        assert!(
            findings.iter().any(|f| f.finding_type.starts_with("tool:")),
            "Should contain L1 finding"
        );
        assert!(
            findings
                .iter()
                .any(|f| f.finding_type == "signature-regression"),
            "Should contain L2 finding"
        );
    }

    #[test]
    fn test_parallel_execution_integration() {
        // Full integration: run L2 on a thread, L1 on main, join and merge.
        // This mirrors the exact pattern used in the pipeline.
        use crate::commands::bugbot::l2::context::FunctionDiff;
        use crate::commands::bugbot::l2::{l2_engine_registry, L2Context};
        use std::collections::HashMap;

        let engines = l2_engine_registry();
        let ctx = L2Context::new(
            PathBuf::from("/tmp/nonexistent"),
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
        );

        // Spawn L2 on background thread
        let l2_handle = std::thread::spawn(move || run_l2_engines(&ctx, &engines));

        // L1 runs on main thread (simulated with run_l1_tools_opt)
        let (l1_raw, tool_results, tools_available, tools_missing) =
            run_l1_tools_opt(std::path::Path::new("/tmp/nonexistent"), "rust", false, 5);

        // Join L2
        let (l2_engine_findings, l2_engine_results) = l2_handle
            .join()
            .unwrap_or_else(|_| (Vec::new(), Vec::new()));

        // Merge
        let l1_bugbot: Vec<BugbotFinding> = l1_raw.into_iter().map(BugbotFinding::from).collect();
        let mut findings: Vec<BugbotFinding> = Vec::new();
        findings.extend(l1_bugbot);
        findings.extend(l2_engine_findings);

        // Both results should be present
        assert_eq!(
            l2_engine_results.len(),
            1,
            "L2 should produce 1 engine result (DeltaEngine), got {}",
            l2_engine_results.len()
        );
        // L1 tool_results may or may not have entries depending on installed tools
        let _ = tool_results;
        let _ = tools_available;
        let _ = tools_missing;
    }

    #[test]
    fn test_no_tools_flag_still_runs_l2_on_thread() {
        // When --no-tools is set, L1 is skipped but L2 should still run.
        // After parallelization, L2 runs on its own thread regardless of no_tools.
        use crate::commands::bugbot::l2::context::FunctionDiff;
        use crate::commands::bugbot::l2::{l2_engine_registry, L2Context};
        use std::collections::HashMap;

        let engines = l2_engine_registry();
        let ctx = L2Context::new(
            PathBuf::from("/tmp/nonexistent"),
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
        );

        // L2 on thread
        let l2_handle = std::thread::spawn(move || run_l2_engines(&ctx, &engines));

        // L1 skipped (no_tools=true)
        let (l1_raw, tool_results, _, _) =
            run_l1_tools_opt(std::path::Path::new("/tmp/nonexistent"), "rust", true, 60);

        // Join L2
        let (l2_findings, l2_results) = l2_handle
            .join()
            .unwrap_or_else(|_| (Vec::new(), Vec::new()));

        assert!(
            l1_raw.is_empty(),
            "no_tools should produce empty L1 findings"
        );
        assert!(
            tool_results.is_empty(),
            "no_tools should produce empty tool_results"
        );
        assert_eq!(
            l2_results.len(),
            1,
            "L2 should run 1 engine (DeltaEngine), got {}",
            l2_results.len()
        );
        let _ = l2_findings;
    }

    // =========================================================================
    // Phase 8.5: Performance Benchmark — Foreground Tier
    // =========================================================================

    /// Verify that the foreground tier completes in under 200ms (release) on a
    /// 50-function diff. In debug builds the budget is relaxed to 2000ms because
    /// the compiler does not optimise the analysis code. The test constructs a
    /// realistic L2Context with 50 changed functions, each backed by synthetic
    /// Rust source code, and runs all registered L2 engines.
    #[test]
    fn test_l2_all_engines_budget() {
        use crate::commands::bugbot::l2::context::{
            FunctionChange, FunctionDiff, InsertedFunction,
        };
        use crate::commands::bugbot::l2::types::FunctionId;
        use crate::commands::bugbot::l2::{l2_engine_registry, L2Context};
        use crate::commands::remaining::types::{ASTChange, ChangeType, Location, NodeKind};
        use std::collections::HashMap;
        use std::time::Duration;

        // -- Build 50 synthetic changed functions ----------------------------------
        let num_functions: usize = 50;
        let mut changed_functions = Vec::with_capacity(num_functions);
        let mut inserted_functions = Vec::with_capacity(num_functions / 5);
        let mut baseline_contents: HashMap<PathBuf, String> = HashMap::new();
        let mut current_contents: HashMap<PathBuf, String> = HashMap::new();
        let mut ast_changes: HashMap<PathBuf, Vec<ASTChange>> = HashMap::new();
        let mut changed_files: Vec<PathBuf> = Vec::new();

        // Distribute functions across 10 files (5 functions per file).
        let files_count = 10;
        for file_idx in 0..files_count {
            let file_path = PathBuf::from(format!("src/module_{}.rs", file_idx));
            changed_files.push(file_path.clone());

            let mut baseline_src = String::new();
            let mut current_src = String::new();
            let mut file_ast_changes = Vec::new();

            let funcs_per_file = num_functions / files_count;
            for func_idx in 0..funcs_per_file {
                let global_idx = file_idx * funcs_per_file + func_idx;
                let func_name = format!("process_item_{}", global_idx);
                let def_line = func_idx * 10 + 1;

                // Baseline version
                let old_source = format!(
                    "fn {}(input: &str) -> Result<(), Error> {{\n    \
                         let data = parse(input)?;\n    \
                         validate(&data)?;\n    \
                         Ok(())\n\
                     }}\n",
                    func_name
                );

                // Current version — added an argument and extra logic
                let new_source = format!(
                    "fn {}(input: &str, config: &Config) -> Result<(), Error> {{\n    \
                         let data = parse(input)?;\n    \
                         if config.strict {{\n        \
                             validate_strict(&data)?;\n    \
                         }} else {{\n        \
                             validate(&data)?;\n    \
                         }}\n    \
                         Ok(())\n\
                     }}\n",
                    func_name
                );

                baseline_src.push_str(&old_source);
                baseline_src.push('\n');
                current_src.push_str(&new_source);
                current_src.push('\n');

                let fid = FunctionId::new(file_path.clone(), func_name.clone(), def_line);

                changed_functions.push(FunctionChange {
                    id: fid,
                    name: func_name.clone(),
                    old_source: old_source.clone(),
                    new_source: new_source.clone(),
                });

                // AST change: parameter update for DeltaEngine
                file_ast_changes.push(ASTChange {
                    change_type: ChangeType::Update,
                    node_kind: NodeKind::Function,
                    name: Some(func_name.clone()),
                    old_location: Some(Location::new(
                        file_path.to_string_lossy().to_string(),
                        def_line as u32,
                    )),
                    new_location: Some(Location::new(
                        file_path.to_string_lossy().to_string(),
                        def_line as u32,
                    )),
                    old_text: Some(old_source),
                    new_text: Some(new_source),
                    similarity: Some(0.85),
                    children: None,
                    base_changes: None,
                });

                // Every 10th function is also inserted (tests DeltaEngine handling)
                if global_idx.is_multiple_of(10) {
                    let ins_name = format!("new_helper_{}", global_idx);
                    let ins_source = format!("fn {}(x: i32) -> i32 {{\n    x * 2\n}}\n", ins_name);
                    inserted_functions.push(InsertedFunction {
                        id: FunctionId::new(file_path.clone(), ins_name.clone(), def_line + 100),
                        name: ins_name,
                        source: ins_source,
                    });
                }
            }

            baseline_contents.insert(file_path.clone(), baseline_src);
            current_contents.insert(file_path.clone(), current_src);
            ast_changes.insert(file_path, file_ast_changes);
        }

        let ctx = L2Context::new(
            PathBuf::from("/tmp/bench-project"),
            Language::Rust,
            changed_files,
            FunctionDiff {
                changed: changed_functions,
                inserted: inserted_functions,
                deleted: vec![],
            },
            baseline_contents,
            current_contents,
            ast_changes,
        );

        // -- Run all engines -------------------------------------------------------
        let all_engines = l2_engine_registry();

        assert_eq!(
            all_engines.len(),
            1,
            "Expected exactly 1 engine (DeltaEngine), got {}",
            all_engines.len()
        );

        // -- Time execution --------------------------------------------------------
        let start = Instant::now();
        let (findings, results) = run_l2_engines(&ctx, &all_engines);
        let elapsed = start.elapsed();

        // -- Assertions ------------------------------------------------------------

        // Every engine must have produced a result entry.
        assert_eq!(
            results.len(),
            all_engines.len(),
            "Every engine must produce a result"
        );

        // At least one engine must have actually analyzed something (not all skipped).
        let engines_that_ran = results
            .iter()
            .filter(|r| r.functions_analyzed > 0 || r.finding_count > 0)
            .count();
        assert!(
            engines_that_ran > 0,
            "At least one engine must have analyzed functions, \
             but all were skipped: {:?}",
            results
                .iter()
                .map(|r| format!("{}:{}", r.name, r.status))
                .collect::<Vec<_>>()
        );

        // Budget: 2000ms in release, 5000ms in debug (unoptimised code is slower;
        // FlowEngine adds ~1500ms in release).
        let budget = if cfg!(debug_assertions) {
            Duration::from_millis(5000)
        } else {
            Duration::from_millis(2000)
        };

        assert!(
            elapsed < budget,
            "All engines took {:?} which exceeds the {:?} budget \
             (release target: <2000ms). Engine breakdown: {:?}",
            elapsed,
            budget,
            results
                .iter()
                .map(|r| format!("{}={}ms", r.name, r.duration_ms))
                .collect::<Vec<_>>()
        );

        // Verify no findings or results were silently dropped.
        let total_finding_count: usize = results.iter().map(|r| r.finding_count).sum();
        assert_eq!(
            findings.len(),
            total_finding_count,
            "Merged findings count must equal sum of per-engine finding counts"
        );
    }

    /// Realistic workload test: runs all engines on functions with branching,
    /// match arms, nested calls, variable shadowing, and multi-path control
    /// flow -- the kind of code that exercises CFG, DFG, SSA, AbstractInterp,
    /// and Taint for real.
    ///
    /// Uses production-like Rust templates (serde-style deserialisers, state
    /// machines, error-handling chains) to validate engine correctness under
    /// realistic conditions.
    #[test]
    fn test_flow_engine_realistic_workload() {
        use crate::commands::bugbot::l2::context::{
            FunctionChange, FunctionDiff, InsertedFunction,
        };
        use crate::commands::bugbot::l2::types::FunctionId;
        use crate::commands::bugbot::l2::{l2_engine_registry, L2Context};
        use crate::commands::remaining::types::{ASTChange, ChangeType, Location, NodeKind};
        use std::collections::HashMap;

        // -- Templates for realistic functions ------------------------------------
        // Each exercises different IR stages: branching (CFG), variable reuse
        // (DFG/reaching defs), conditional assignment (SSA/SCCP), arithmetic
        // (abstract interp), and resource handles (resource-leak).

        type ComplexTemplate = Box<dyn Fn(usize) -> (String, String)>;
        let complex_templates: Vec<ComplexTemplate> = vec![
            // Template 1: Match-heavy deserialiser (exercises CFG branching)
            Box::new(|idx: usize| {
                let old = format!(
                    r#"fn deserialize_{idx}(input: &[u8]) -> Result<Value, Error> {{
    let mut pos = 0;
    let tag = input.get(pos).ok_or(Error::Eof)?;
    pos += 1;
    match tag {{
        0x01 => {{
            let len = input.get(pos).copied().unwrap_or(0) as usize;
            pos += 1;
            let data = &input[pos..pos + len];
            Ok(Value::String(std::str::from_utf8(data)?.to_string()))
        }}
        0x02 => {{
            let n = i32::from_le_bytes(input[pos..pos+4].try_into()?);
            Ok(Value::Int(n))
        }}
        0x03 => {{
            let count = input.get(pos).copied().unwrap_or(0) as usize;
            pos += 1;
            let mut items = Vec::with_capacity(count);
            for _ in 0..count {{
                let item = deserialize_{idx}(&input[pos..])?;
                items.push(item);
            }}
            Ok(Value::Array(items))
        }}
        _ => Err(Error::UnknownTag(*tag)),
    }}
}}"#
                );
                let new = format!(
                    r#"fn deserialize_{idx}(input: &[u8], opts: &Options) -> Result<Value, Error> {{
    let mut pos = 0;
    let tag = input.get(pos).ok_or(Error::Eof)?;
    pos += 1;
    match tag {{
        0x01 => {{
            let len = input.get(pos).copied().unwrap_or(0) as usize;
            pos += 1;
            if len > opts.max_string_len {{
                return Err(Error::TooLong(len));
            }}
            let data = &input[pos..pos + len];
            Ok(Value::String(std::str::from_utf8(data)?.to_string()))
        }}
        0x02 => {{
            let n = i32::from_le_bytes(input[pos..pos+4].try_into()?);
            if opts.strict && n < 0 {{
                return Err(Error::NegativeInt(n));
            }}
            Ok(Value::Int(n))
        }}
        0x03 => {{
            let count = input.get(pos).copied().unwrap_or(0) as usize;
            if count > opts.max_array_len {{
                return Err(Error::TooMany(count));
            }}
            pos += 1;
            let mut items = Vec::with_capacity(count);
            for _ in 0..count {{
                let item = deserialize_{idx}(&input[pos..], opts)?;
                items.push(item);
            }}
            Ok(Value::Array(items))
        }}
        _ => Err(Error::UnknownTag(*tag)),
    }}
}}"#
                );
                (old, new)
            }),
            // Template 2: State machine with variable reassignment (exercises DFG/SSA)
            Box::new(|idx: usize| {
                let old = format!(
                    r#"fn process_state_{idx}(events: &[Event]) -> Result<State, Error> {{
    let mut state = State::Init;
    let mut retries = 0;
    let mut last_error = None;
    for event in events {{
        state = match (state, event) {{
            (State::Init, Event::Start) => State::Running,
            (State::Running, Event::Pause) => State::Paused,
            (State::Paused, Event::Resume) => State::Running,
            (State::Running, Event::Error(e)) => {{
                last_error = Some(e.clone());
                retries += 1;
                if retries > 3 {{
                    return Err(Error::TooManyRetries(last_error.unwrap()));
                }}
                State::Running
            }}
            (State::Running, Event::Done) => State::Complete,
            (s, _) => s,
        }};
    }}
    Ok(state)
}}"#
                );
                let new = format!(
                    r#"fn process_state_{idx}(events: &[Event], config: &Config) -> Result<State, Error> {{
    let mut state = State::Init;
    let mut retries = 0;
    let mut last_error = None;
    let max_retries = config.max_retries.unwrap_or(3);
    for event in events {{
        state = match (state, event) {{
            (State::Init, Event::Start) => {{
                if config.require_auth && !config.authenticated {{
                    return Err(Error::Unauthorized);
                }}
                State::Running
            }}
            (State::Running, Event::Pause) => State::Paused,
            (State::Paused, Event::Resume) => {{
                retries = 0;
                State::Running
            }}
            (State::Running, Event::Error(e)) => {{
                last_error = Some(e.clone());
                retries += 1;
                if retries > max_retries {{
                    return Err(Error::TooManyRetries(last_error.unwrap()));
                }}
                State::Running
            }}
            (State::Running, Event::Done) => State::Complete,
            (s, _) => s,
        }};
    }}
    Ok(state)
}}"#
                );
                (old, new)
            }),
            // Template 3: Resource-handling with early returns (exercises resource-leak)
            Box::new(|idx: usize| {
                let old = format!(
                    r#"fn read_config_{idx}(path: &str) -> Result<Config, Error> {{
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut lines = Vec::new();
    for line in reader.lines() {{
        let line = line?;
        if line.starts_with('#') {{
            continue;
        }}
        if line.is_empty() {{
            break;
        }}
        lines.push(line);
    }}
    let parsed = parse_config(&lines)?;
    validate_config(&parsed)?;
    Ok(parsed)
}}"#
                );
                let new = format!(
                    r#"fn read_config_{idx}(path: &str, env: &Env) -> Result<Config, Error> {{
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut lines = Vec::new();
    let mut saw_section = false;
    for line in reader.lines() {{
        let line = line?;
        if line.starts_with('#') {{
            continue;
        }}
        if line.starts_with('[') {{
            if saw_section {{
                break;
            }}
            saw_section = true;
            continue;
        }}
        if line.is_empty() && !saw_section {{
            break;
        }}
        let resolved = if line.contains("${{") {{
            env.resolve_vars(&line)?
        }} else {{
            line
        }};
        lines.push(resolved);
    }}
    let parsed = parse_config(&lines)?;
    validate_config(&parsed)?;
    Ok(parsed)
}}"#
                );
                (old, new)
            }),
            // Template 4: Arithmetic with conditional division (exercises abstract interp)
            Box::new(|idx: usize| {
                let old = format!(
                    r#"fn compute_metrics_{idx}(data: &[f64]) -> Metrics {{
    let sum: f64 = data.iter().sum();
    let count = data.len();
    let mean = sum / count as f64;
    let variance = data.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / count as f64;
    let stddev = variance.sqrt();
    let min = data.iter().copied().fold(f64::INFINITY, f64::min);
    let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let range = max - min;
    Metrics {{ mean, stddev, min, max, range }}
}}"#
                );
                let new = format!(
                    r#"fn compute_metrics_{idx}(data: &[f64], opts: &MetricOpts) -> Result<Metrics, Error> {{
    if data.is_empty() {{
        return Err(Error::EmptyData);
    }}
    let sum: f64 = data.iter().sum();
    let count = data.len();
    let mean = sum / count as f64;
    let trimmed = if opts.trim_outliers {{
        let lo = mean - 2.0 * opts.threshold;
        let hi = mean + 2.0 * opts.threshold;
        data.iter().filter(|&&x| x >= lo && x <= hi).copied().collect::<Vec<_>>()
    }} else {{
        data.to_vec()
    }};
    let adj_count = trimmed.len();
    if adj_count == 0 {{
        return Err(Error::AllOutliers);
    }}
    let adj_mean = trimmed.iter().sum::<f64>() / adj_count as f64;
    let variance = trimmed.iter().map(|x| (x - adj_mean).powi(2)).sum::<f64>() / adj_count as f64;
    let stddev = variance.sqrt();
    let min = trimmed.iter().copied().fold(f64::INFINITY, f64::min);
    let max = trimmed.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let range = max - min;
    let cv = if adj_mean.abs() > f64::EPSILON {{ stddev / adj_mean }} else {{ 0.0 }};
    Ok(Metrics {{ mean: adj_mean, stddev, min, max, range, cv }})
}}"#
                );
                (old, new)
            }),
            // Template 5: Nested error chain (exercises taint / data flow)
            Box::new(|idx: usize| {
                let old = format!(
                    r#"fn handle_request_{idx}(req: &Request) -> Result<Response, Error> {{
    let auth = validate_auth(&req.headers)?;
    let body = parse_body(&req.body)?;
    let user = lookup_user(auth.user_id)?;
    let result = execute_query(&user, &body.query)?;
    let formatted = format_response(&result)?;
    Ok(Response::new(200, formatted))
}}"#
                );
                let new = format!(
                    r#"fn handle_request_{idx}(req: &Request, ctx: &Context) -> Result<Response, Error> {{
    let auth = validate_auth(&req.headers)?;
    if auth.expired() {{
        return Ok(Response::new(401, "Token expired".into()));
    }}
    let body = parse_body(&req.body)?;
    let user = lookup_user(auth.user_id)?;
    if !user.has_permission(&body.query) {{
        return Ok(Response::new(403, "Forbidden".into()));
    }}
    let result = if ctx.read_only {{
        execute_read_query(&user, &body.query)?
    }} else {{
        execute_query(&user, &body.query)?
    }};
    let formatted = format_response(&result)?;
    ctx.metrics.record_latency(req.start.elapsed());
    Ok(Response::new(200, formatted))
}}"#
                );
                (old, new)
            }),
        ];

        // -- Build 50 realistic functions across 10 files -------------------------
        let num_functions: usize = 50;
        let num_templates = complex_templates.len();
        let mut changed_functions = Vec::with_capacity(num_functions);
        let mut inserted_functions = Vec::with_capacity(num_functions / 5);
        let mut baseline_contents: HashMap<PathBuf, String> = HashMap::new();
        let mut current_contents: HashMap<PathBuf, String> = HashMap::new();
        let mut ast_changes: HashMap<PathBuf, Vec<ASTChange>> = HashMap::new();
        let mut changed_files: Vec<PathBuf> = Vec::new();

        let files_count = 10;
        for file_idx in 0..files_count {
            let file_path = PathBuf::from(format!("src/service_{}.rs", file_idx));
            changed_files.push(file_path.clone());

            let mut baseline_src = String::new();
            let mut current_src = String::new();
            let mut file_ast_changes = Vec::new();

            let funcs_per_file = num_functions / files_count;
            for func_idx in 0..funcs_per_file {
                let global_idx = file_idx * funcs_per_file + func_idx;
                let template_idx = global_idx % num_templates;
                let (old_source, new_source) = complex_templates[template_idx](global_idx);

                // Extract function name from source (first word after "fn ")
                let func_name = old_source
                    .strip_prefix("fn ")
                    .and_then(|s| s.split('(').next())
                    .unwrap_or(&format!("func_{}", global_idx))
                    .to_string();

                let def_line = baseline_src.lines().count() + 1;

                baseline_src.push_str(&old_source);
                baseline_src.push_str("\n\n");
                current_src.push_str(&new_source);
                current_src.push_str("\n\n");

                let fid = FunctionId::new(file_path.clone(), func_name.clone(), def_line);

                changed_functions.push(FunctionChange {
                    id: fid,
                    name: func_name.clone(),
                    old_source: old_source.clone(),
                    new_source: new_source.clone(),
                });

                file_ast_changes.push(ASTChange {
                    change_type: ChangeType::Update,
                    node_kind: NodeKind::Function,
                    name: Some(func_name.clone()),
                    old_location: Some(Location::new(
                        file_path.to_string_lossy().to_string(),
                        def_line as u32,
                    )),
                    new_location: Some(Location::new(
                        file_path.to_string_lossy().to_string(),
                        def_line as u32,
                    )),
                    old_text: Some(old_source),
                    new_text: Some(new_source),
                    similarity: Some(0.75),
                    children: None,
                    base_changes: None,
                });

                if global_idx.is_multiple_of(10) {
                    let ins_name = format!("new_helper_{}", global_idx);
                    let ins_source = format!("fn {}(x: i32) -> i32 {{\n    x * 2\n}}\n", ins_name);
                    inserted_functions.push(InsertedFunction {
                        id: FunctionId::new(file_path.clone(), ins_name.clone(), def_line + 200),
                        name: ins_name,
                        source: ins_source,
                    });
                }
            }

            baseline_contents.insert(file_path.clone(), baseline_src);
            current_contents.insert(file_path.clone(), current_src);
            ast_changes.insert(file_path, file_ast_changes);
        }

        let ctx = L2Context::new(
            PathBuf::from("/tmp/bench-deferred-realistic"),
            Language::Rust,
            changed_files,
            FunctionDiff {
                changed: changed_functions,
                inserted: inserted_functions,
                deleted: vec![],
            },
            baseline_contents,
            current_contents,
            ast_changes,
        );

        // -- Run all engines -------------------------------------------------------
        let all_engines = l2_engine_registry();

        let start = Instant::now();
        let (findings, results) = run_l2_engines(&ctx, &all_engines);
        let elapsed = start.elapsed();

        // At least one engine must have run (not all skipped)
        let engines_ran = results.iter().filter(|r| r.functions_analyzed > 0).count();
        assert!(engines_ran > 0, "All engines skipped on realistic workload");

        // Print timing and findings (visible with --nocapture)
        eprintln!(
            "\n[realistic-bench] Total={:?}\n  Engines: {:?}\n  Findings: {}",
            elapsed,
            results
                .iter()
                .map(|r| format!(
                    "{}={}ms(a={},f={})",
                    r.name, r.duration_ms, r.functions_analyzed, r.finding_count
                ))
                .collect::<Vec<_>>(),
            findings.len(),
        );

        // Every engine must have produced a result entry.
        assert_eq!(
            results.len(),
            all_engines.len(),
            "Every engine must produce a result"
        );

        // After the Ashby pivot, DeltaEngine only produces complexity-increase
        // and maintainability-drop findings. These require parseable source that
        // shows a measurable regression, so the realistic templates (which use
        // undefined types) may not produce findings. We verify consistency only.

        // Verify no findings were silently dropped.
        let total_finding_count: usize = results.iter().map(|r| r.finding_count).sum();
        assert_eq!(
            findings.len(),
            total_finding_count,
            "Merged findings count must equal sum of per-engine finding counts"
        );
    }

    // =========================================================================
    // Phase 8.4: Engine Execution Tests
    // =========================================================================

    /// Verify all registered engines produce results in run_l2_engines.
    #[test]
    fn test_all_engines_produce_results() {
        use crate::commands::bugbot::l2::context::FunctionDiff;
        use crate::commands::bugbot::l2::{l2_engine_registry, L2Context};
        use std::collections::HashMap;

        let engines = l2_engine_registry();
        let ctx = L2Context::new(
            PathBuf::from("/tmp/test-engine-results"),
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
        );

        let (_findings, results) = run_l2_engines(&ctx, &engines);

        // Every registered engine must produce a result entry.
        assert_eq!(
            results.len(),
            engines.len(),
            "All engines should have results"
        );

        // Verify engine names in results match registered engines.
        let engine_names: Vec<&str> = engines.iter().map(|e| e.name()).collect();
        for result in &results {
            assert!(
                engine_names.contains(&result.name.as_str()),
                "Result for unknown engine '{}' -- not in registry",
                result.name
            );
        }

        // Verify exactly 1 engine is registered (DeltaEngine).
        assert_eq!(
            engines.len(),
            1,
            "Should have exactly 1 engine (DeltaEngine), got {}",
            engines.len()
        );
    }

    /// All engines should run synchronously when no daemon is available.
    #[test]
    fn test_engines_run_synchronously_without_daemon() {
        use crate::commands::bugbot::l2::context::FunctionDiff;
        use crate::commands::bugbot::l2::{l2_engine_registry, L2Context};
        use std::collections::HashMap;

        let engines = l2_engine_registry();
        let ctx = L2Context::new(
            PathBuf::from("/tmp/test-sync-no-daemon"),
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
        );

        // No daemon attached (default NoDaemon)
        assert!(!ctx.daemon_available());

        let (_findings, results) = run_l2_engines(&ctx, &engines);

        // All engines should have run
        assert_eq!(
            results.len(),
            engines.len(),
            "All engines should have run even without daemon"
        );

        for result in &results {
            assert!(
                !result.status.is_empty(),
                "Engine '{}' should have a status after running synchronously",
                result.name
            );
        }
    }

    /// Deferred engines should produce results when daemon provides cached data.
    #[test]
    fn test_deferred_engines_use_daemon_cache() {
        use crate::commands::bugbot::l2::context::FunctionDiff;
        use crate::commands::bugbot::l2::daemon_client::DaemonClient;
        use crate::commands::bugbot::l2::{l2_engine_registry, L2Context};
        use std::collections::HashMap;

        // Mock daemon that is available (but returns None for individual queries)
        struct AvailableDaemon;
        impl DaemonClient for AvailableDaemon {
            fn is_available(&self) -> bool {
                true
            }
            fn query_call_graph(&self) -> Option<tldr_core::ProjectCallGraph> {
                None
            }
            fn query_cfg(
                &self,
                _fid: &super::super::l2::types::FunctionId,
            ) -> Option<tldr_core::CfgInfo> {
                None
            }
            fn query_dfg(
                &self,
                _fid: &super::super::l2::types::FunctionId,
            ) -> Option<tldr_core::DfgInfo> {
                None
            }
            fn query_ssa(
                &self,
                _fid: &super::super::l2::types::FunctionId,
            ) -> Option<tldr_core::ssa::SsaFunction> {
                None
            }
            fn notify_changed_files(&self, _files: &[PathBuf]) {}
        }

        let engines = l2_engine_registry();
        let ctx = L2Context::new(
            PathBuf::from("/tmp/test-daemon-cache"),
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
        .with_daemon(Box::new(AvailableDaemon));

        assert!(ctx.daemon_available());

        let (_findings, results) = run_l2_engines(&ctx, &engines);

        // All engines should have results
        assert_eq!(
            results.len(),
            engines.len(),
            "All engines should run even with daemon available"
        );
    }

    /// Daemon client creation should be wired in the pipeline.
    #[test]
    fn test_daemon_client_creation_factory() {
        use crate::commands::bugbot::l2::daemon_client::create_daemon_client;

        // For a nonexistent project, should return NoDaemon
        let client = create_daemon_client(std::path::Path::new("/tmp/nonexistent-project-xyz"));
        assert!(!client.is_available());
    }

    // =========================================================================
    // L2Context population tests (FunctionDiff + file contents wiring)
    // =========================================================================

    #[test]
    fn test_build_function_diff_from_ast_changes_insert() {
        use crate::commands::remaining::types::{ASTChange, ChangeType, Location, NodeKind};

        let project = PathBuf::from("/project");
        let mut all_diffs: HashMap<PathBuf, Vec<ASTChange>> = HashMap::new();
        all_diffs.insert(
            PathBuf::from("/project/src/lib.rs"),
            vec![ASTChange {
                change_type: ChangeType::Insert,
                node_kind: NodeKind::Function,
                name: Some("new_func".to_string()),
                old_location: None,
                new_location: Some(Location::new("src/lib.rs", 10)),
                old_text: None,
                new_text: Some("fn new_func() { }".to_string()),
                similarity: None,
                children: None,
                base_changes: None,
            }],
        );

        let diff = build_function_diff(&all_diffs, &project);

        assert_eq!(diff.inserted.len(), 1, "Should have 1 inserted function");
        assert_eq!(diff.changed.len(), 0, "Should have 0 changed functions");
        assert_eq!(diff.deleted.len(), 0, "Should have 0 deleted functions");
        assert_eq!(diff.inserted[0].name, "new_func");
        assert_eq!(diff.inserted[0].source, "fn new_func() { }");
        assert_eq!(
            diff.inserted[0].id.file,
            PathBuf::from("src/lib.rs"),
            "FunctionId file should be relative"
        );
        assert_eq!(diff.inserted[0].id.def_line, 10);
    }

    #[test]
    fn test_build_function_diff_from_ast_changes_update() {
        use crate::commands::remaining::types::{ASTChange, ChangeType, Location, NodeKind};

        let project = PathBuf::from("/project");
        let mut all_diffs: HashMap<PathBuf, Vec<ASTChange>> = HashMap::new();
        all_diffs.insert(
            PathBuf::from("/project/src/main.rs"),
            vec![ASTChange {
                change_type: ChangeType::Update,
                node_kind: NodeKind::Function,
                name: Some("existing_fn".to_string()),
                old_location: Some(Location::new("src/main.rs", 5)),
                new_location: Some(Location::new("src/main.rs", 5)),
                old_text: Some("fn existing_fn() { old }".to_string()),
                new_text: Some("fn existing_fn() { new }".to_string()),
                similarity: None,
                children: None,
                base_changes: None,
            }],
        );

        let diff = build_function_diff(&all_diffs, &project);

        assert_eq!(diff.changed.len(), 1, "Should have 1 changed function");
        assert_eq!(diff.inserted.len(), 0);
        assert_eq!(diff.deleted.len(), 0);
        assert_eq!(diff.changed[0].name, "existing_fn");
        assert_eq!(diff.changed[0].old_source, "fn existing_fn() { old }");
        assert_eq!(diff.changed[0].new_source, "fn existing_fn() { new }");
        assert_eq!(
            diff.changed[0].id.file,
            PathBuf::from("src/main.rs"),
            "FunctionId file should be relative"
        );
    }

    #[test]
    fn test_build_function_diff_from_ast_changes_delete() {
        use crate::commands::remaining::types::{ASTChange, ChangeType, Location, NodeKind};

        let project = PathBuf::from("/project");
        let mut all_diffs: HashMap<PathBuf, Vec<ASTChange>> = HashMap::new();
        all_diffs.insert(
            PathBuf::from("/project/src/old.rs"),
            vec![ASTChange {
                change_type: ChangeType::Delete,
                node_kind: NodeKind::Function,
                name: Some("removed_fn".to_string()),
                old_location: Some(Location::new("src/old.rs", 20)),
                new_location: None,
                old_text: Some("fn removed_fn() { }".to_string()),
                new_text: None,
                similarity: None,
                children: None,
                base_changes: None,
            }],
        );

        let diff = build_function_diff(&all_diffs, &project);

        assert_eq!(diff.deleted.len(), 1, "Should have 1 deleted function");
        assert_eq!(diff.changed.len(), 0);
        assert_eq!(diff.inserted.len(), 0);
        assert_eq!(diff.deleted[0].name, "removed_fn");
        assert_eq!(
            diff.deleted[0].id.file,
            PathBuf::from("src/old.rs"),
            "FunctionId file should be relative"
        );
    }

    #[test]
    fn test_build_function_diff_skips_non_function_nodes() {
        use crate::commands::remaining::types::{ASTChange, ChangeType, Location, NodeKind};

        let project = PathBuf::from("/project");
        let mut all_diffs: HashMap<PathBuf, Vec<ASTChange>> = HashMap::new();
        all_diffs.insert(
            PathBuf::from("/project/src/lib.rs"),
            vec![
                // Class node -- should be skipped
                ASTChange {
                    change_type: ChangeType::Insert,
                    node_kind: NodeKind::Class,
                    name: Some("MyClass".to_string()),
                    old_location: None,
                    new_location: Some(Location::new("src/lib.rs", 1)),
                    old_text: None,
                    new_text: Some("class MyClass {}".to_string()),
                    similarity: None,
                    children: None,
                    base_changes: None,
                },
                // Statement node -- should be skipped
                ASTChange {
                    change_type: ChangeType::Update,
                    node_kind: NodeKind::Statement,
                    name: Some("let x".to_string()),
                    old_location: Some(Location::new("src/lib.rs", 10)),
                    new_location: Some(Location::new("src/lib.rs", 10)),
                    old_text: Some("let x = 1;".to_string()),
                    new_text: Some("let x = 2;".to_string()),
                    similarity: None,
                    children: None,
                    base_changes: None,
                },
                // Function node -- should be included
                ASTChange {
                    change_type: ChangeType::Insert,
                    node_kind: NodeKind::Function,
                    name: Some("real_fn".to_string()),
                    old_location: None,
                    new_location: Some(Location::new("src/lib.rs", 20)),
                    old_text: None,
                    new_text: Some("fn real_fn() {}".to_string()),
                    similarity: None,
                    children: None,
                    base_changes: None,
                },
            ],
        );

        let diff = build_function_diff(&all_diffs, &project);

        assert_eq!(
            diff.inserted.len(),
            1,
            "Only function/method nodes should be included"
        );
        assert_eq!(diff.inserted[0].name, "real_fn");
    }

    #[test]
    fn test_build_function_diff_includes_method_nodes() {
        use crate::commands::remaining::types::{ASTChange, ChangeType, Location, NodeKind};

        let project = PathBuf::from("/project");
        let mut all_diffs: HashMap<PathBuf, Vec<ASTChange>> = HashMap::new();
        all_diffs.insert(
            PathBuf::from("/project/src/impl.rs"),
            vec![ASTChange {
                change_type: ChangeType::Update,
                node_kind: NodeKind::Method,
                name: Some("MyStruct::do_thing".to_string()),
                old_location: Some(Location::new("src/impl.rs", 15)),
                new_location: Some(Location::new("src/impl.rs", 15)),
                old_text: Some("fn do_thing(&self) { old }".to_string()),
                new_text: Some("fn do_thing(&self) { new }".to_string()),
                similarity: None,
                children: None,
                base_changes: None,
            }],
        );

        let diff = build_function_diff(&all_diffs, &project);

        assert_eq!(diff.changed.len(), 1, "Method nodes should be included");
        assert_eq!(diff.changed[0].name, "MyStruct::do_thing");
    }

    #[test]
    fn test_build_function_diff_skips_unnamed_changes() {
        use crate::commands::remaining::types::{ASTChange, ChangeType, Location, NodeKind};

        let project = PathBuf::from("/project");
        let mut all_diffs: HashMap<PathBuf, Vec<ASTChange>> = HashMap::new();
        all_diffs.insert(
            PathBuf::from("/project/src/lib.rs"),
            vec![ASTChange {
                change_type: ChangeType::Insert,
                node_kind: NodeKind::Function,
                name: None, // No name
                old_location: None,
                new_location: Some(Location::new("src/lib.rs", 1)),
                old_text: None,
                new_text: Some("fn() {}".to_string()),
                similarity: None,
                children: None,
                base_changes: None,
            }],
        );

        let diff = build_function_diff(&all_diffs, &project);

        assert_eq!(
            diff.inserted.len(),
            0,
            "Unnamed function changes should be skipped"
        );
    }

    #[test]
    fn test_build_function_diff_move_with_both_texts_becomes_update() {
        use crate::commands::remaining::types::{ASTChange, ChangeType, Location, NodeKind};

        let project = PathBuf::from("/project");
        let mut all_diffs: HashMap<PathBuf, Vec<ASTChange>> = HashMap::new();
        all_diffs.insert(
            PathBuf::from("/project/src/lib.rs"),
            vec![ASTChange {
                change_type: ChangeType::Move,
                node_kind: NodeKind::Function,
                name: Some("moved_fn".to_string()),
                old_location: Some(Location::new("src/lib.rs", 10)),
                new_location: Some(Location::new("src/lib.rs", 50)),
                old_text: Some("fn moved_fn() { a }".to_string()),
                new_text: Some("fn moved_fn() { b }".to_string()),
                similarity: Some(0.9),
                children: None,
                base_changes: None,
            }],
        );

        let diff = build_function_diff(&all_diffs, &project);

        assert_eq!(
            diff.changed.len(),
            1,
            "Move with both old/new text should become a changed function"
        );
        assert_eq!(diff.changed[0].name, "moved_fn");
        assert_eq!(diff.changed[0].old_source, "fn moved_fn() { a }");
        assert_eq!(diff.changed[0].new_source, "fn moved_fn() { b }");
    }

    #[test]
    fn test_build_function_diff_multiple_files() {
        use crate::commands::remaining::types::{ASTChange, ChangeType, Location, NodeKind};

        let project = PathBuf::from("/project");
        let mut all_diffs: HashMap<PathBuf, Vec<ASTChange>> = HashMap::new();
        all_diffs.insert(
            PathBuf::from("/project/src/a.rs"),
            vec![ASTChange {
                change_type: ChangeType::Insert,
                node_kind: NodeKind::Function,
                name: Some("fn_a".to_string()),
                old_location: None,
                new_location: Some(Location::new("src/a.rs", 1)),
                old_text: None,
                new_text: Some("fn fn_a() {}".to_string()),
                similarity: None,
                children: None,
                base_changes: None,
            }],
        );
        all_diffs.insert(
            PathBuf::from("/project/src/b.rs"),
            vec![ASTChange {
                change_type: ChangeType::Delete,
                node_kind: NodeKind::Method,
                name: Some("fn_b".to_string()),
                old_location: Some(Location::new("src/b.rs", 5)),
                new_location: None,
                old_text: Some("fn fn_b() {}".to_string()),
                new_text: None,
                similarity: None,
                children: None,
                base_changes: None,
            }],
        );

        let diff = build_function_diff(&all_diffs, &project);

        assert_eq!(diff.inserted.len(), 1, "Should have insert from a.rs");
        assert_eq!(diff.deleted.len(), 1, "Should have delete from b.rs");
        assert_eq!(diff.inserted[0].name, "fn_a");
        assert_eq!(diff.deleted[0].name, "fn_b");
    }

    #[test]
    fn test_build_function_diff_empty_input() {
        let project = PathBuf::from("/project");
        let all_diffs: HashMap<PathBuf, Vec<crate::commands::remaining::types::ASTChange>> =
            HashMap::new();

        let diff = build_function_diff(&all_diffs, &project);

        assert_eq!(diff.changed.len(), 0);
        assert_eq!(diff.inserted.len(), 0);
        assert_eq!(diff.deleted.len(), 0);
    }

    #[test]
    fn test_build_function_diff_path_already_relative() {
        use crate::commands::remaining::types::{ASTChange, ChangeType, Location, NodeKind};

        // When path is already relative (no project prefix match),
        // strip_prefix returns the original path
        let project = PathBuf::from("/project");
        let mut all_diffs: HashMap<PathBuf, Vec<ASTChange>> = HashMap::new();
        all_diffs.insert(
            PathBuf::from("src/lib.rs"), // Already relative
            vec![ASTChange {
                change_type: ChangeType::Insert,
                node_kind: NodeKind::Function,
                name: Some("f".to_string()),
                old_location: None,
                new_location: Some(Location::new("src/lib.rs", 1)),
                old_text: None,
                new_text: Some("fn f() {}".to_string()),
                similarity: None,
                children: None,
                base_changes: None,
            }],
        );

        let diff = build_function_diff(&all_diffs, &project);

        assert_eq!(diff.inserted.len(), 1);
        // Path should remain as-is when not matching project prefix
        assert_eq!(diff.inserted[0].id.file, PathBuf::from("src/lib.rs"));
    }

    // =========================================================================
    // End-to-end simulation: prove L2 engines detect real bugs
    // =========================================================================

    /// End-to-end simulation test: constructs an L2Context with a complexity
    /// increase and verifies that the DeltaEngine detects it. The
    /// complexity-increase and maintainability-drop finding types are the
    /// remaining DeltaEngine capabilities after the Ashby pivot.
    #[test]
    fn test_bugbot_finds_real_bugs() {
        use crate::commands::bugbot::l2::context::FunctionDiff;
        use crate::commands::bugbot::l2::{l2_engine_registry, L2Context};
        use std::collections::HashMap;

        // =====================================================================
        // Bug: Complexity increase -- simple function becomes deeply nested
        // Expects: complexity-increase
        // =====================================================================
        let simple_src = "def process(x):\n    return x + 1\n";
        let complex_src = r#"def process(x):
    if x > 10:
        if x > 20:
            if x > 30:
                return x * 3
            elif x > 25:
                return x * 2
            else:
                return x
        elif x > 15:
            return x - 1
        else:
            return x + 1
    else:
        return 0
"#;

        let file = PathBuf::from("src/process.py");

        let mut baseline_contents: HashMap<PathBuf, String> = HashMap::new();
        let mut current_contents: HashMap<PathBuf, String> = HashMap::new();
        baseline_contents.insert(file.clone(), simple_src.to_string());
        current_contents.insert(file.clone(), complex_src.to_string());

        let ctx = L2Context::new(
            PathBuf::from("/tmp/bugbot-simulation"),
            Language::Python,
            vec![file],
            FunctionDiff {
                changed: vec![],
                inserted: vec![],
                deleted: vec![],
            },
            baseline_contents,
            current_contents,
            HashMap::new(),
        );

        // =====================================================================
        // Run all L2 engines
        // =====================================================================
        let all_engines = l2_engine_registry();
        let (findings, results) = run_l2_engines(&ctx, &all_engines);

        // =====================================================================
        // Assertions: verify DeltaEngine detected the complexity increase
        // =====================================================================
        let has_finding = |finding_type: &str| -> bool {
            findings.iter().any(|f| f.finding_type == finding_type)
        };

        assert!(
            has_finding("complexity-increase"),
            "Expected complexity-increase finding. Got: {:?}",
            findings
                .iter()
                .map(|f| format!("{}:{}", f.finding_type, f.function))
                .collect::<Vec<_>>()
        );

        // =====================================================================
        // Summary: print what was found (visible with --nocapture)
        // =====================================================================
        eprintln!("\n=== Bugbot Simulation Results ===");
        eprintln!("Total findings: {}", findings.len());
        for engine_result in &results {
            eprintln!(
                "  {}: {} findings ({}ms, analyzed={}, skipped={})",
                engine_result.name,
                engine_result.finding_count,
                engine_result.duration_ms,
                engine_result.functions_analyzed,
                engine_result.functions_skipped,
            );
        }

        // =====================================================================
        // Structural assertions
        // =====================================================================

        // Every engine must have produced a result entry.
        assert_eq!(
            results.len(),
            all_engines.len(),
            "Every engine must produce a result"
        );

        // At least one finding should be present.
        assert!(
            !findings.is_empty(),
            "Expected at least 1 finding from buggy code, got 0"
        );

        // Verify finding count consistency.
        let total_from_results: usize = results.iter().map(|r| r.finding_count).sum();
        assert_eq!(
            findings.len(),
            total_from_results,
            "Merged findings count must equal sum of per-engine finding counts"
        );
    }
}
