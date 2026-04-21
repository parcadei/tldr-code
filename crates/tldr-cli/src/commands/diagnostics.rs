//! Diagnostics command - Unified type checking and linting across languages
//!
//! Session 6 Phase 10: CLI command for running diagnostic tools.
//!
//! # Features
//! - Auto-detect available tools (pyright, ruff, tsc, eslint, etc.)
//! - Run type checkers and linters in parallel
//! - Unified diagnostic output format
//! - Severity filtering (error, warning, info, hint)
//! - Multiple output formats (JSON, text, SARIF, GitHub Actions)
//!
//! # Exit Codes (documented in --help, S6-R52 mitigation)
//! - 0: Success (no errors, or only warnings without --strict)
//! - 1: Errors found (or warnings with --strict)
//! - 60: No diagnostic tools available
//! - 61: All tools failed to run

use anyhow::{anyhow, Result};
use clap::Args;
use std::path::PathBuf;

use tldr_core::diagnostics::{
    compute_exit_code, compute_summary, dedupe_diagnostics, detect_available_tools,
    filter_diagnostics_by_severity, run_tools_parallel, tools_for_language, DiagnosticsReport,
    Severity, ToolConfig,
};
use tldr_core::Language;

use crate::output::{format_diagnostics_text, OutputFormat, OutputWriter};

/// Run type checking and linting
///
/// Runs diagnostic tools (type checkers and linters) and produces unified output.
/// Tools are detected automatically based on language and availability.
///
/// # Exit Codes
///
/// - 0: Success (no errors, or only warnings without --strict)
/// - 1: Errors found (or warnings with --strict)
/// - 60: No diagnostic tools available for language
/// - 61: All tools failed to run
#[derive(Debug, Args)]
pub struct DiagnosticsArgs {
    /// File or directory to analyze
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Programming language (auto-detect if not specified)
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,

    // === Tool Selection ===
    /// Specific tools to run (comma-separated, e.g., "pyright,ruff")
    #[arg(long, value_delimiter = ',')]
    pub tools: Vec<String>,

    /// Skip type checking (linters only)
    #[arg(long)]
    pub no_typecheck: bool,

    /// Skip linting (type checkers only)
    #[arg(long)]
    pub no_lint: bool,

    // === Filtering ===
    /// Minimum severity to report (error, warning, info, hint)
    #[arg(long, short = 's', value_enum, default_value = "hint")]
    pub severity: SeverityFilter,

    /// Ignore specific error codes (comma-separated)
    #[arg(long, value_delimiter = ',')]
    pub ignore: Vec<String>,

    // === Output Options ===
    /// Additional output format (sarif, github-actions)
    #[arg(long, value_enum)]
    pub output: Option<DiagnosticOutput>,

    /// Analyze entire project (not just specified path)
    #[arg(long)]
    pub project: bool,

    /// Maximum number of annotations for GitHub Actions output
    #[arg(long, default_value = "50")]
    pub max_annotations: usize,

    // === Execution ===
    /// Timeout per tool in seconds
    #[arg(long, default_value = "60")]
    pub timeout: u64,

    /// Fail on warnings (not just errors)
    #[arg(long)]
    pub strict: bool,

    // === Baseline Comparison (Phase 12) ===
    /// Compare against baseline file (show only new issues)
    #[arg(long)]
    pub baseline: Option<PathBuf>,

    /// Save current results as baseline
    #[arg(long)]
    pub save_baseline: Option<PathBuf>,
}

/// Severity filter for CLI (maps to core Severity)
#[derive(Debug, Clone, Copy, clap::ValueEnum, Default)]
pub enum SeverityFilter {
    /// Show only errors
    Error,
    /// Show errors and warnings
    Warning,
    /// Show errors, warnings, and info
    Info,
    /// Show all diagnostics including hints
    #[default]
    Hint,
}

impl From<SeverityFilter> for Severity {
    fn from(filter: SeverityFilter) -> Self {
        match filter {
            SeverityFilter::Error => Severity::Error,
            SeverityFilter::Warning => Severity::Warning,
            SeverityFilter::Info => Severity::Information,
            SeverityFilter::Hint => Severity::Hint,
        }
    }
}

/// Additional output formats for diagnostics
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum DiagnosticOutput {
    /// SARIF 2.1.0 format for GitHub/GitLab Code Scanning
    Sarif,
    /// GitHub Actions workflow commands (::error::, ::warning::)
    GithubActions,
}

impl DiagnosticsArgs {
    /// Run the diagnostics command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        // 1. Detect language (default to Python if not specified and can't detect)
        let language = self.lang.unwrap_or_else(|| {
            if self.path.is_file() {
                Language::from_path(&self.path).unwrap_or(Language::Python)
            } else {
                Language::from_directory(&self.path).unwrap_or(Language::Python)
            }
        });

        writer.progress(&format!("Detecting tools for {:?}...", language));

        // 2. Get available tools
        let mut tools: Vec<ToolConfig> = if self.tools.is_empty() {
            detect_available_tools(language)
        } else {
            // Filter to requested tools
            tools_for_language(language)
                .into_iter()
                .filter(|t| {
                    self.tools
                        .iter()
                        .any(|name| t.name.eq_ignore_ascii_case(name))
                })
                .collect()
        };

        // 3. Apply type/lint filtering
        if self.no_typecheck {
            tools.retain(|t| !t.is_type_checker);
        }
        if self.no_lint {
            tools.retain(|t| !t.is_linter);
        }

        // 4. Check if we have tools to run
        if tools.is_empty() {
            // Exit code 60: No diagnostic tools available (S6-R36 mitigation)
            eprintln!(
                "Error: No diagnostic tools available for {:?}. Install one of:",
                language
            );
            for tool in tools_for_language(language) {
                eprintln!(
                    "  - {} ({})",
                    tool.name,
                    tldr_core::diagnostics::get_install_suggestion(tool.name)
                );
            }
            std::process::exit(60);
        }

        writer.progress(&format!(
            "Running diagnostics: {}",
            tools.iter().map(|t| t.name).collect::<Vec<_>>().join(", ")
        ));

        // 5. Run tools in parallel
        let mut report = run_tools_parallel(&tools, &self.path, self.timeout)?;

        // Check if all tools failed (exit code 61)
        if report.tools_run.iter().all(|t| !t.success) {
            eprintln!("Error: All diagnostic tools failed to run.");
            for result in &report.tools_run {
                if let Some(err) = &result.error {
                    eprintln!("  - {}: {}", result.name, err);
                }
            }
            std::process::exit(61);
        }

        // 6. Deduplicate diagnostics
        report.diagnostics = dedupe_diagnostics(report.diagnostics);

        // 7. Filter by severity
        let min_severity: Severity = self.severity.into();
        let unfiltered_count = report.diagnostics.len();
        report.diagnostics = filter_diagnostics_by_severity(&report.diagnostics, min_severity);

        // 8. Filter by ignored codes
        if !self.ignore.is_empty() {
            report.diagnostics.retain(|d| {
                if let Some(code) = &d.code {
                    !self.ignore.iter().any(|ignored| code == ignored)
                } else {
                    true
                }
            });
        }

        // 9. Apply baseline comparison (Phase 12)
        if let Some(baseline_path) = &self.baseline {
            report = apply_baseline(report, baseline_path)?;
        }

        // 10. Recompute summary after filtering (S6-R28 mitigation)
        report.summary = compute_summary(&report.diagnostics);

        // 11. Save baseline if requested
        if let Some(save_path) = &self.save_baseline {
            save_baseline(&report, save_path)?;
            writer.progress(&format!("Baseline saved to: {}", save_path.display()));
        }

        // 12. Calculate filtered count for display (S6-R47 mitigation)
        let filtered_count = unfiltered_count - report.diagnostics.len();

        // 13. Output based on format
        match self.output {
            Some(DiagnosticOutput::Sarif) => {
                let sarif = to_sarif(&report);
                // Warn if SARIF exceeds 10MB estimate (S6-R56 mitigation)
                let estimated_size = serde_json::to_string(&sarif).map(|s| s.len()).unwrap_or(0);
                if estimated_size > 10 * 1024 * 1024 {
                    eprintln!(
                        "Warning: SARIF output is large (~{}MB). GitHub may reject files over 10MB.",
                        estimated_size / (1024 * 1024)
                    );
                }
                println!("{}", serde_json::to_string_pretty(&sarif)?);
            }
            Some(DiagnosticOutput::GithubActions) => {
                output_github_actions(&report, self.max_annotations);
            }
            None => {
                if writer.is_text() {
                    let text = format_diagnostics_text(&report, filtered_count);
                    writer.write_text(&text)?;
                } else {
                    writer.write(&report)?;
                }
            }
        }

        // 14. Compute exit code (S6-R36 mitigation: distinct codes)
        let exit_code = compute_exit_code(&report.summary, self.strict);
        if exit_code != 0 {
            std::process::exit(exit_code);
        }

        Ok(())
    }
}

// =============================================================================
// Phase 11: SARIF Output Format
// =============================================================================

/// SARIF 2.1.0 output structure
#[derive(Debug, serde::Serialize)]
struct SarifReport {
    #[serde(rename = "$schema")]
    schema: &'static str,
    version: &'static str,
    runs: Vec<SarifRun>,
}

#[derive(Debug, serde::Serialize)]
struct SarifRun {
    tool: SarifTool,
    results: Vec<SarifResult>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifTool {
    driver: SarifDriver,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifDriver {
    name: String,
    version: String,
    information_uri: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifResult {
    rule_id: String,
    level: String,
    message: SarifMessage,
    locations: Vec<SarifLocation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    help_uri: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct SarifMessage {
    text: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifLocation {
    physical_location: SarifPhysicalLocation,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifPhysicalLocation {
    artifact_location: SarifArtifactLocation,
    region: SarifRegion,
}

#[derive(Debug, serde::Serialize)]
struct SarifArtifactLocation {
    uri: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifRegion {
    start_line: u32,
    start_column: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    end_line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    end_column: Option<u32>,
}

/// Convert DiagnosticsReport to SARIF 2.1.0 format
fn to_sarif(report: &DiagnosticsReport) -> SarifReport {
    let results: Vec<SarifResult> = report
        .diagnostics
        .iter()
        .map(|d| {
            // Map severity to SARIF level (S6-R35 mitigation)
            let level = match d.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
                Severity::Information => "note",
                Severity::Hint => "note",
            };

            // Use relative path for URI (S6-R23 mitigation)
            let uri = d.file.display().to_string();
            let relative_uri = if uri.starts_with('/') {
                // Strip absolute path prefix - try common prefixes
                uri.trim_start_matches('/')
                    .split_once('/')
                    .map(|(_, rest)| rest.to_string())
                    .unwrap_or(uri)
            } else {
                uri
            };

            SarifResult {
                rule_id: d.code.clone().unwrap_or_else(|| d.source.clone()),
                level: level.to_string(),
                message: SarifMessage {
                    text: d.message.clone(),
                },
                locations: vec![SarifLocation {
                    physical_location: SarifPhysicalLocation {
                        artifact_location: SarifArtifactLocation { uri: relative_uri },
                        region: SarifRegion {
                            start_line: d.line,
                            start_column: d.column,
                            end_line: d.end_line,
                            end_column: d.end_column,
                        },
                    },
                }],
                help_uri: d.url.clone(),
            }
        })
        .collect();

    SarifReport {
        schema: "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/master/Schemata/sarif-schema-2.1.0.json",
        version: "2.1.0",
        runs: vec![SarifRun {
            tool: SarifTool {
                driver: SarifDriver {
                    name: "tldr-diagnostics".to_string(),
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    information_uri: "https://github.com/user/tldr".to_string(),
                },
            },
            results,
        }],
    }
}

// =============================================================================
// Phase 11: GitHub Actions Output Format
// =============================================================================

/// Output diagnostics as GitHub Actions workflow commands
fn output_github_actions(report: &DiagnosticsReport, max_annotations: usize) {
    // Warn if exceeding annotation limit (S6-R55 mitigation)
    if report.diagnostics.len() > max_annotations {
        eprintln!(
            "Warning: {} diagnostics found, but GitHub Actions limits annotations to {}. \
             Only first {} will be shown. Use --max-annotations to adjust.",
            report.diagnostics.len(),
            max_annotations,
            max_annotations
        );
    }

    for diag in report.diagnostics.iter().take(max_annotations) {
        let severity = match diag.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Information => "notice",
            Severity::Hint => "notice",
        };

        // GitHub Actions format: ::severity file=path,line=N,col=M::message
        // Escape message for GH Actions (newlines become %0A)
        let escaped_message = diag
            .message
            .replace('\n', "%0A")
            .replace('\r', "%0D")
            .replace('%', "%25");

        println!(
            "::{} file={},line={},col={}::{}",
            severity,
            diag.file.display(),
            diag.line,
            diag.column,
            escaped_message
        );
    }

    // Output summary as a group
    println!("::group::Diagnostics Summary");
    println!(
        "Errors: {}, Warnings: {}, Info: {}, Hints: {}",
        report.summary.errors, report.summary.warnings, report.summary.info, report.summary.hints
    );
    println!("::endgroup::");
}

// =============================================================================
// Phase 12: Baseline Comparison
// =============================================================================

/// Baseline file structure for JSON serialization
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct BaselineFile {
    version: u32,
    created_at: String,
    diagnostics: Vec<BaselineDiagnostic>,
}

/// Simplified diagnostic for baseline storage
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq, Hash)]
struct BaselineDiagnostic {
    /// Relative file path
    file: String,
    /// Start line
    line: u32,
    /// Start column
    column: u32,
    /// Hash of message for comparison
    message_hash: u64,
    /// Original message (for resolved diagnostics)
    message: String,
    /// Error code
    code: Option<String>,
}

impl From<&tldr_core::diagnostics::Diagnostic> for BaselineDiagnostic {
    fn from(d: &tldr_core::diagnostics::Diagnostic) -> Self {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        d.message.hash(&mut hasher);
        let message_hash = hasher.finish();

        BaselineDiagnostic {
            file: d.file.display().to_string(),
            line: d.line,
            column: d.column,
            message_hash,
            message: d.message.clone(),
            code: d.code.clone(),
        }
    }
}

/// Apply baseline comparison to filter out known issues
fn apply_baseline(
    mut report: DiagnosticsReport,
    baseline_path: &PathBuf,
) -> Result<DiagnosticsReport> {
    // Read baseline file
    let baseline_content = std::fs::read_to_string(baseline_path).map_err(|e| {
        anyhow!(
            "Failed to read baseline file '{}': {}",
            baseline_path.display(),
            e
        )
    })?;

    // Parse baseline (S6-R25 mitigation: validate on load)
    let baseline: BaselineFile = serde_json::from_str(&baseline_content).map_err(|e| {
        anyhow!(
            "Invalid baseline JSON in '{}': {}",
            baseline_path.display(),
            e
        )
    })?;

    // Check version compatibility
    if baseline.version != 1 {
        return Err(anyhow!(
            "Unsupported baseline version: {}. Expected version 1.",
            baseline.version
        ));
    }

    // Convert current diagnostics to baseline format for comparison
    let current_set: std::collections::HashSet<BaselineDiagnostic> =
        report.diagnostics.iter().map(|d| d.into()).collect();

    let baseline_set: std::collections::HashSet<BaselineDiagnostic> =
        baseline.diagnostics.into_iter().collect();

    // Find new diagnostics (in current but not in baseline)
    let new_diagnostics: std::collections::HashSet<_> =
        current_set.difference(&baseline_set).cloned().collect();

    // Find resolved diagnostics (in baseline but not in current)
    let resolved: Vec<_> = baseline_set.difference(&current_set).collect();

    if !resolved.is_empty() {
        eprintln!(
            "Info: {} issues from baseline have been resolved.",
            resolved.len()
        );
    }

    // Filter report to only new diagnostics
    report.diagnostics.retain(|d| {
        let bd: BaselineDiagnostic = d.into();
        new_diagnostics.contains(&bd)
    });

    Ok(report)
}

/// Save current diagnostics as baseline file
fn save_baseline(report: &DiagnosticsReport, path: &PathBuf) -> Result<()> {
    let baseline = BaselineFile {
        version: 1,
        created_at: chrono::Utc::now().to_rfc3339(),
        diagnostics: report.diagnostics.iter().map(|d| d.into()).collect(),
    };

    let json = serde_json::to_string_pretty(&baseline)?;
    std::fs::write(path, json)
        .map_err(|e| anyhow!("Failed to write baseline file '{}': {}", path.display(), e))?;

    Ok(())
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_severity_filter_conversion() {
        assert_eq!(Severity::from(SeverityFilter::Error), Severity::Error);
        assert_eq!(Severity::from(SeverityFilter::Warning), Severity::Warning);
        assert_eq!(Severity::from(SeverityFilter::Info), Severity::Information);
        assert_eq!(Severity::from(SeverityFilter::Hint), Severity::Hint);
    }

    #[test]
    fn test_args_default_values() {
        use clap::Parser;

        #[derive(Debug, Parser)]
        struct TestCli {
            #[command(flatten)]
            args: DiagnosticsArgs,
        }

        let cli = TestCli::try_parse_from(["test"]).unwrap();
        assert_eq!(cli.args.path, PathBuf::from("."));
        assert!(!cli.args.no_typecheck);
        assert!(!cli.args.no_lint);
        assert!(!cli.args.strict);
        assert_eq!(cli.args.timeout, 60);
        assert!(matches!(cli.args.severity, SeverityFilter::Hint));
    }

    #[test]
    fn test_sarif_severity_mapping() {
        use tldr_core::diagnostics::Diagnostic;

        let diag = Diagnostic {
            file: PathBuf::from("test.py"),
            line: 1,
            column: 1,
            end_line: None,
            end_column: None,
            severity: Severity::Error,
            message: "test error".to_string(),
            code: Some("E001".to_string()),
            source: "test".to_string(),
            url: None,
        };

        let report = DiagnosticsReport {
            diagnostics: vec![diag],
            summary: tldr_core::diagnostics::DiagnosticsSummary {
                errors: 1,
                warnings: 0,
                info: 0,
                hints: 0,
                total: 1,
            },
            tools_run: vec![],
            files_analyzed: 1,
        };

        let sarif = to_sarif(&report);
        assert_eq!(sarif.version, "2.1.0");
        assert_eq!(sarif.runs.len(), 1);
        assert_eq!(sarif.runs[0].results.len(), 1);
        assert_eq!(sarif.runs[0].results[0].level, "error");
    }

    #[test]
    fn test_baseline_diagnostic_hash() {
        use tldr_core::diagnostics::Diagnostic;

        let diag1 = Diagnostic {
            file: PathBuf::from("test.py"),
            line: 10,
            column: 5,
            end_line: None,
            end_column: None,
            severity: Severity::Warning,
            message: "test warning".to_string(),
            code: Some("W001".to_string()),
            source: "test".to_string(),
            url: None,
        };

        let diag2 = Diagnostic {
            file: PathBuf::from("test.py"),
            line: 10,
            column: 5,
            end_line: None,
            end_column: None,
            severity: Severity::Warning,
            message: "test warning".to_string(), // Same message
            code: Some("W001".to_string()),
            source: "test".to_string(),
            url: None,
        };

        let bd1: BaselineDiagnostic = (&diag1).into();
        let bd2: BaselineDiagnostic = (&diag2).into();

        assert_eq!(bd1, bd2);
        assert_eq!(bd1.message_hash, bd2.message_hash);
    }
}
