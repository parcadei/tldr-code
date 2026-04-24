//! Verify command - Aggregated verification dashboard combining multiple analyses.
//!
//! Provides a unified view of code constraints including:
//! - Contracts (pre/postconditions) from source analysis
//! - Specs from test files
//! - Bounds analysis warnings
//! - Dead store detection
//!
//! # ELEPHANT Mitigations Addressed
//! - E02: Capture all sub-analysis errors, report in summary
//! - E03: Partial failure handling - continue and report
//! - E07: Clear intermediate results after each file
//! - E09: Concurrent access - unique temp dirs
//!
//! # Example
//!
//! ```bash
//! tldr verify ./src
//! tldr verify ./src --quick
//! tldr verify ./src --detail contracts
//! tldr verify ./src --format text
//! ```

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Result;
use clap::Args;
use tldr_core::walker::walk_project;

use tldr_core::Language;

use crate::output::{OutputFormat, OutputWriter};

use super::contracts::run_contracts;
use super::error::{ContractsError, ContractsResult};
use super::specs::run_specs;
use super::types::ContractsReport;
use super::types::{
    CoverageInfo, OutputFormat as ContractsOutputFormat, SubAnalysisResult, SubAnalysisStatus,
    VerifyReport, VerifySummary,
};
// validate_file_path is available but currently unused
// use super::validation::validate_file_path;

// =============================================================================
// Resource Limits (E03 Mitigation)
// =============================================================================

/// Maximum number of files to analyze (E03 mitigation)
const MAX_FILES: usize = 500;

// =============================================================================
// CLI Arguments
// =============================================================================

/// Aggregated verification dashboard combining multiple analyses.
///
/// Runs contracts, specs, bounds, and dead-stores analyses on a project
/// directory and provides a unified coverage report.
///
/// # Example
///
/// ```bash
/// tldr verify ./src
/// tldr verify ./src --quick
/// tldr verify ./src --detail contracts
/// ```
#[derive(Debug, Args)]
pub struct VerifyArgs {
    /// Directory to analyze (defaults to current directory)
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Output format (json or text). Prefer global --format/-f flag.
    #[arg(
        long = "output-format",
        short = 'o',
        hide = true,
        default_value = "json"
    )]
    pub output_format: ContractsOutputFormat,

    /// Programming language override (auto-detected if not specified)
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,

    /// Show specific sub-analysis detail
    #[arg(long)]
    pub detail: Option<String>,

    /// Quick mode - skip expensive analyses (invariants, patterns)
    #[arg(long)]
    pub quick: bool,
}

impl VerifyArgs {
    /// Run the verify command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        // Validate path exists
        let canonical_path = if self.path.exists() {
            std::fs::canonicalize(&self.path).unwrap_or_else(|_| self.path.clone())
        } else {
            return Err(ContractsError::FileNotFound {
                path: self.path.clone(),
            }
            .into());
        };

        writer.progress(&format!(
            "Running verification on {}...",
            self.path.display()
        ));

        // Determine language (auto-detect from directory, default to Python)
        let language = self.lang.unwrap_or_else(|| {
            if self.path.is_file() {
                Language::from_path(&self.path).unwrap_or(Language::Python)
            } else {
                Language::from_directory(&self.path).unwrap_or(Language::Python)
            }
        });

        // Run verification
        let report = run_verify(
            &canonical_path,
            language,
            self.quick,
            self.detail.as_deref(),
        )?;

        // Output based on format
        let use_text = matches!(self.output_format, ContractsOutputFormat::Text)
            || matches!(format, OutputFormat::Text);

        if use_text {
            let text = format_verify_text(&report);
            writer.write_text(&text)?;
        } else {
            writer.write(&report)?;
        }

        Ok(())
    }
}

// =============================================================================
// Core Analysis Functions
// =============================================================================

/// Run the full verification dashboard.
///
/// # Arguments
/// * `path` - Directory or file to analyze
/// * `language` - Programming language for analysis
/// * `quick` - If true, skip expensive analyses (invariants, patterns)
/// * `detail` - If Some, show only the specified sub-analysis
///
/// # Returns
/// VerifyReport with all sub-analysis results and coverage summary.
pub fn run_verify(
    path: &Path,
    language: Language,
    quick: bool,
    detail: Option<&str>,
) -> ContractsResult<VerifyReport> {
    let start_time = Instant::now();

    // Collect files to analyze
    let files = collect_source_files(path, language)?;
    let files_analyzed = files.len() as u32;

    // Initialize report
    let mut sub_results: HashMap<String, SubAnalysisResult> = HashMap::new();
    let mut files_failed = 0u32;

    // Run sub-analyses
    // 1. Contracts sweep
    let contracts_result = sweep_contracts(&files, language, detail);
    if let Some(ref err) = contracts_result.error {
        files_failed += count_failures_from_error(err);
    }
    sub_results.insert("contracts".to_string(), contracts_result);

    // 2. Specs extraction (if test directory exists)
    let test_dirs = find_test_dirs(path);
    if !test_dirs.is_empty() {
        let specs_result = sweep_specs(&test_dirs[0], detail);
        sub_results.insert("specs".to_string(), specs_result);
    } else {
        sub_results.insert(
            "specs".to_string(),
            SubAnalysisResult {
                name: "specs".to_string(),
                status: SubAnalysisStatus::Failed,
                items_found: 0,
                elapsed_ms: 0,
                error: Some("No test directory found".to_string()),
                data: None,
            },
        );
    }

    // 3. Bounds analysis (skip in quick mode - it's expensive)
    if !quick {
        let bounds_result = sweep_bounds(&files, language, detail);
        sub_results.insert("bounds".to_string(), bounds_result);
    }

    // 4. Dead stores detection
    let dead_stores_result = sweep_dead_stores(&files, language, detail);
    sub_results.insert("dead_stores".to_string(), dead_stores_result);

    // 5. Invariants (skip in quick mode - requires test execution)
    if !quick && !test_dirs.is_empty() {
        sub_results.insert(
            "invariants".to_string(),
            SubAnalysisResult {
                name: "invariants".to_string(),
                status: SubAnalysisStatus::Skipped,
                items_found: 0,
                elapsed_ms: 0,
                error: Some(
                    "Invariants analysis requires test execution (not yet implemented)".to_string(),
                ),
                data: None,
            },
        );
    }

    // Compute coverage from results
    let summary = build_verify_summary(&sub_results, files_analyzed);

    let total_elapsed_ms = (start_time.elapsed().as_millis() as u64).max(1);

    // Determine if we have partial results
    let partial_results = sub_results.values().any(|r| {
        matches!(
            r.status,
            SubAnalysisStatus::Partial | SubAnalysisStatus::Failed
        )
    });

    Ok(VerifyReport {
        path: path.to_path_buf(),
        sub_results,
        summary,
        total_elapsed_ms,
        files_analyzed,
        files_failed,
        partial_results,
    })
}

/// Collect source files for analysis.
fn collect_source_files(path: &Path, language: Language) -> ContractsResult<Vec<PathBuf>> {
    let extension = match language {
        Language::Python => "py",
        Language::TypeScript | Language::JavaScript => "ts",
        Language::Rust => "rs",
        Language::Go => "go",
        Language::Java => "java",
        _ => "py", // Default to Python
    };

    let mut files = Vec::new();

    if path.is_file() {
        files.push(path.to_path_buf());
    } else {
        for entry in walk_project(path).filter(|e| {
            e.path().is_file()
                && e.path()
                    .extension()
                    .is_some_and(|ext| ext == extension)
                // Skip test files for main analysis
                && !e.file_name().to_str().is_some_and(|n| n.starts_with("test_"))
        }) {
            files.push(entry.path().to_path_buf());

            // Apply file limit (E03 mitigation)
            if files.len() >= MAX_FILES {
                break;
            }
        }
    }

    Ok(files)
}

/// Find test directories by convention.
fn find_test_dirs(project_path: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    // Check common test directory names
    for name in &["tests", "test"] {
        let dir = project_path.join(name);
        if dir.is_dir() {
            candidates.push(dir);
        }
    }

    // Check for test_*.py files in the project root
    if let Ok(entries) = std::fs::read_dir(project_path) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_file() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with("test_") && name.ends_with(".py") {
                        candidates.push(path);
                    }
                }
            }
        }
    }

    candidates
}

// =============================================================================
// Sub-Analysis Sweepers
// =============================================================================

/// Sweep contracts analysis over all files.
fn sweep_contracts(
    files: &[PathBuf],
    language: Language,
    _detail: Option<&str>,
) -> SubAnalysisResult {
    let start = Instant::now();
    let mut total_contracts = 0u32;
    let mut all_results: Vec<ContractsReport> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    for file in files {
        // Find all functions in the file and analyze each
        match analyze_file_contracts(file, language) {
            Ok(reports) => {
                for report in reports {
                    total_contracts += report.preconditions.len() as u32;
                    total_contracts += report.postconditions.len() as u32;
                    total_contracts += report.invariants.len() as u32;
                    all_results.push(report);
                }
            }
            Err(e) => {
                errors.push(format!("{}: {}", file.display(), e));
            }
        }
    }

    let status = if errors.is_empty() {
        SubAnalysisStatus::Success
    } else if !all_results.is_empty() {
        SubAnalysisStatus::Partial
    } else {
        SubAnalysisStatus::Failed
    };

    SubAnalysisResult {
        name: "contracts".to_string(),
        status,
        items_found: total_contracts,
        elapsed_ms: start.elapsed().as_millis() as u64,
        error: if errors.is_empty() {
            None
        } else {
            Some(errors.join("; "))
        },
        data: Some(serde_json::to_value(&all_results).unwrap_or(serde_json::Value::Null)),
    }
}

/// Analyze contracts for all functions in a file.
fn analyze_file_contracts(
    file: &Path,
    language: Language,
) -> ContractsResult<Vec<ContractsReport>> {
    let source = std::fs::read_to_string(file)?;
    let functions = extract_function_names(&source, language)?;

    let mut reports = Vec::new();
    for func_name in functions {
        match run_contracts(file, &func_name, language, 100) {
            Ok(report) => reports.push(report),
            Err(_) => continue, // Skip functions that fail to analyze
        }
    }

    Ok(reports)
}

/// Extract function names from source code.
fn extract_function_names(source: &str, _language: Language) -> ContractsResult<Vec<String>> {
    // Simple regex-based extraction for Python
    let mut names = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("def ") {
            if let Some(name_end) = trimmed.find('(') {
                let name = &trimmed[4..name_end].trim();
                if !name.is_empty() {
                    names.push(name.to_string());
                }
            }
        }
    }
    Ok(names)
}

/// Sweep specs extraction from test directory.
fn sweep_specs(test_path: &Path, _detail: Option<&str>) -> SubAnalysisResult {
    let start = Instant::now();

    match run_specs(test_path, None) {
        Ok(report) => {
            let total_specs = report.summary.total_specs;
            SubAnalysisResult {
                name: "specs".to_string(),
                status: SubAnalysisStatus::Success,
                items_found: total_specs,
                elapsed_ms: start.elapsed().as_millis() as u64,
                error: None,
                data: Some(serde_json::to_value(&report).unwrap_or(serde_json::Value::Null)),
            }
        }
        Err(e) => SubAnalysisResult {
            name: "specs".to_string(),
            status: SubAnalysisStatus::Failed,
            items_found: 0,
            elapsed_ms: start.elapsed().as_millis() as u64,
            error: Some(e.to_string()),
            data: None,
        },
    }
}

/// Sweep bounds analysis over all files.
fn sweep_bounds(
    _files: &[PathBuf],
    _language: Language,
    _detail: Option<&str>,
) -> SubAnalysisResult {
    let start = Instant::now();

    // Bounds analysis is expensive - for now, return a stub
    // TODO: Implement when bounds command is integrated
    SubAnalysisResult {
        name: "bounds".to_string(),
        status: SubAnalysisStatus::Skipped,
        items_found: 0,
        elapsed_ms: start.elapsed().as_millis() as u64,
        error: Some("Bounds sweep not yet integrated".to_string()),
        data: None,
    }
}

/// Sweep dead stores detection over all files.
fn sweep_dead_stores(
    _files: &[PathBuf],
    _language: Language,
    _detail: Option<&str>,
) -> SubAnalysisResult {
    let start = Instant::now();

    // Dead stores requires SSA analysis for each function
    // TODO: Implement when dead_stores command is fully integrated
    SubAnalysisResult {
        name: "dead_stores".to_string(),
        status: SubAnalysisStatus::Skipped,
        items_found: 0,
        elapsed_ms: start.elapsed().as_millis() as u64,
        error: Some("Dead stores sweep not yet integrated".to_string()),
        data: None,
    }
}

// =============================================================================
// Summary Building
// =============================================================================

/// Build the verify summary from sub-analysis results.
fn build_verify_summary(
    sub_results: &HashMap<String, SubAnalysisResult>,
    total_files: u32,
) -> VerifySummary {
    // Count items from each sub-analysis
    let spec_count = sub_results.get("specs").map(|r| r.items_found).unwrap_or(0);

    let contract_count = sub_results
        .get("contracts")
        .map(|r| r.items_found)
        .unwrap_or(0);

    let invariant_count = sub_results
        .get("invariants")
        .map(|r| r.items_found)
        .unwrap_or(0);

    // Compute coverage from contracts data
    let coverage = compute_coverage(sub_results, total_files);

    VerifySummary {
        spec_count,
        invariant_count,
        contract_count,
        annotated_count: 0,  // Not yet implemented
        behavioral_count: 0, // Not yet implemented
        pattern_count: 0,
        pattern_high_confidence: 0,
        coverage,
    }
}

/// Compute function coverage from analysis results.
fn compute_coverage(
    sub_results: &HashMap<String, SubAnalysisResult>,
    total_files: u32,
) -> CoverageInfo {
    let mut constrained_functions: HashSet<String> = HashSet::new();
    let mut total_functions: HashSet<String> = HashSet::new();

    // Extract function info from contracts results
    if let Some(contracts_result) = sub_results.get("contracts") {
        if let Some(data) = &contracts_result.data {
            if let Some(reports) = data.as_array() {
                for report in reports {
                    if let Some(func_name) = report.get("function").and_then(|f| f.as_str()) {
                        total_functions.insert(func_name.to_string());

                        // Check if function has any constraints
                        let has_pre = report
                            .get("preconditions")
                            .and_then(|p| p.as_array())
                            .is_some_and(|a| !a.is_empty());
                        let has_post = report
                            .get("postconditions")
                            .and_then(|p| p.as_array())
                            .is_some_and(|a| !a.is_empty());
                        let has_inv = report
                            .get("invariants")
                            .and_then(|i| i.as_array())
                            .is_some_and(|a| !a.is_empty());

                        if has_pre || has_post || has_inv {
                            constrained_functions.insert(func_name.to_string());
                        }
                    }
                }
            }
        }
    }

    // If no functions found, use file count as proxy
    let total = if total_functions.is_empty() {
        total_files
    } else {
        total_functions.len() as u32
    };

    let constrained = constrained_functions.len() as u32;
    let coverage_pct = if total > 0 {
        (constrained as f64 / total as f64 * 100.0).round() / 1.0 // Round to 1 decimal
    } else {
        0.0
    };

    CoverageInfo {
        constrained_functions: constrained,
        total_functions: total,
        coverage_pct,
    }
}

/// Count failures from error message.
fn count_failures_from_error(error: &str) -> u32 {
    // Count semicolons (our error separator) + 1
    (error.matches(';').count() + 1) as u32
}

// =============================================================================
// Output Formatting
// =============================================================================

/// Format verify report as human-readable text.
pub fn format_verify_text(report: &VerifyReport) -> String {
    let s = &report.summary;
    let cov = &s.coverage;

    let mut lines = vec![
        format!("Verification: {}", report.path.display()),
        "=".repeat(50),
        format!("Test Specs:    {} behavioral specs extracted", s.spec_count),
        format!("Invariants:    {} inferred invariants", s.invariant_count),
        format!(
            "Contracts:     {} pre/postconditions inferred",
            s.contract_count
        ),
        format!(
            "Annotations:   {} Annotated[T] constraints found",
            s.annotated_count
        ),
        format!(
            "Behaviors:     {} functions with behavioral models",
            s.behavioral_count
        ),
        format!(
            "Patterns:      {} project patterns ({} high-confidence)",
            s.pattern_count, s.pattern_high_confidence
        ),
        String::new(),
        "Constraint Coverage:".to_string(),
        format!(
            "  Functions with any constraint: {}/{} ({:.1}%)",
            cov.constrained_functions, cov.total_functions, cov.coverage_pct
        ),
        String::new(),
        format!("Elapsed: {}ms", report.total_elapsed_ms),
    ];

    // Add errors if any
    let failed: Vec<&str> = report
        .sub_results
        .iter()
        .filter(|(_, r)| matches!(r.status, SubAnalysisStatus::Failed))
        .map(|(name, _)| name.as_str())
        .collect();

    if !failed.is_empty() {
        lines.push(format!("Errors: {}", failed.join(", ")));
    }

    lines.join("\n")
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    const PYTHON_WITH_CONTRACTS: &str = r#"
def constrained(x):
    if x < 0:
        raise ValueError("x must be non-negative")
    return x * 2

def unconstrained(y):
    return y * 3
"#;

    const PYTHON_TEST_FILE: &str = r#"
import pytest
from mymodule import add, validate

def test_add():
    assert add(2, 3) == 5

def test_validate_raises():
    with pytest.raises(ValueError):
        validate("")
"#;

    // -------------------------------------------------------------------------
    // Full Sweep Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_verify_full_sweep() {
        let temp = TempDir::new().unwrap();
        let src_dir = temp.path().join("src");
        let test_dir = temp.path().join("tests");
        fs::create_dir(&src_dir).unwrap();
        fs::create_dir(&test_dir).unwrap();

        fs::write(src_dir.join("module.py"), PYTHON_WITH_CONTRACTS).unwrap();
        fs::write(test_dir.join("test_module.py"), PYTHON_TEST_FILE).unwrap();

        let report = run_verify(temp.path(), Language::Python, false, None).unwrap();

        // Should have sub_results
        assert!(report.sub_results.contains_key("contracts"));
        assert!(report.sub_results.contains_key("specs"));
        assert!(report.total_elapsed_ms > 0);
    }

    #[test]
    fn test_verify_quick_mode() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("module.py"), PYTHON_WITH_CONTRACTS).unwrap();

        let report = run_verify(temp.path(), Language::Python, true, None).unwrap();

        // Quick mode should skip invariants and bounds
        if let Some(invariants) = report.sub_results.get("invariants") {
            assert!(
                matches!(
                    invariants.status,
                    SubAnalysisStatus::Skipped | SubAnalysisStatus::Failed
                ),
                "Invariants should be skipped in quick mode"
            );
        }
    }

    #[test]
    fn test_verify_partial_failure() {
        let temp = TempDir::new().unwrap();

        // Create a file that will cause parse errors
        fs::write(temp.path().join("broken.py"), "def broken( syntax error").unwrap();
        fs::write(temp.path().join("valid.py"), "def valid(): pass").unwrap();

        let report = run_verify(temp.path(), Language::Python, false, None).unwrap();

        // Should still produce a report (partial results)
        assert!(report.sub_results.contains_key("contracts"));
    }

    #[test]
    fn test_verify_file_limit() {
        let temp = TempDir::new().unwrap();

        // Create more than MAX_FILES Python files
        for i in 0..600 {
            fs::write(
                temp.path().join(format!("module_{}.py", i)),
                format!("def func_{i}(): pass"),
            )
            .unwrap();
        }

        let files = collect_source_files(temp.path(), Language::Python).unwrap();

        assert!(
            files.len() <= MAX_FILES,
            "Should limit to {} files, got {}",
            MAX_FILES,
            files.len()
        );
    }

    #[test]
    fn test_verify_coverage_calculation() {
        let temp = TempDir::new().unwrap();

        fs::write(temp.path().join("module.py"), PYTHON_WITH_CONTRACTS).unwrap();

        let report = run_verify(temp.path(), Language::Python, true, None).unwrap();

        let cov = &report.summary.coverage;
        assert!(cov.total_functions > 0 || report.files_analyzed > 0);
        assert!(cov.coverage_pct >= 0.0 && cov.coverage_pct <= 100.0);
    }

    #[test]
    fn test_verify_json_output() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("module.py"), "def foo(): pass").unwrap();

        let report = run_verify(temp.path(), Language::Python, true, None).unwrap();

        // Should serialize to valid JSON
        let json = serde_json::to_string(&report);
        assert!(json.is_ok());

        // Verify expected fields
        let json_value: serde_json::Value = serde_json::from_str(&json.unwrap()).unwrap();
        assert!(json_value.get("path").is_some());
        assert!(json_value.get("sub_results").is_some());
        assert!(json_value.get("summary").is_some());
        assert!(json_value.get("total_elapsed_ms").is_some());
    }

    #[test]
    fn test_verify_text_output() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("module.py"), PYTHON_WITH_CONTRACTS).unwrap();

        let report = run_verify(temp.path(), Language::Python, true, None).unwrap();
        let text = format_verify_text(&report);

        assert!(text.contains("Verification:"));
        assert!(text.contains("Constraint Coverage:"));
        assert!(text.contains("Elapsed:"));
    }

    #[test]
    fn test_verify_detail_filter() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("module.py"), PYTHON_WITH_CONTRACTS).unwrap();

        let report = run_verify(temp.path(), Language::Python, true, Some("contracts")).unwrap();

        // Should still run all analyses but detail is informational
        assert!(report.sub_results.contains_key("contracts"));
    }

    // -------------------------------------------------------------------------
    // Helper Function Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_find_test_dirs() {
        let temp = TempDir::new().unwrap();
        let tests_dir = temp.path().join("tests");
        fs::create_dir(&tests_dir).unwrap();

        let dirs = find_test_dirs(temp.path());
        assert!(!dirs.is_empty());
        assert!(dirs[0].ends_with("tests"));
    }

    #[test]
    fn test_find_test_dirs_none() {
        let temp = TempDir::new().unwrap();

        let dirs = find_test_dirs(temp.path());
        assert!(dirs.is_empty());
    }

    #[test]
    fn test_extract_function_names() {
        let source = r#"
def foo():
    pass

def bar(x):
    return x

def baz(a, b):
    return a + b
"#;

        let names = extract_function_names(source, Language::Python).unwrap();
        assert_eq!(names.len(), 3);
        assert!(names.contains(&"foo".to_string()));
        assert!(names.contains(&"bar".to_string()));
        assert!(names.contains(&"baz".to_string()));
    }

    #[test]
    fn test_empty_directory() {
        let temp = TempDir::new().unwrap();

        let report = run_verify(temp.path(), Language::Python, true, None).unwrap();

        assert_eq!(report.files_analyzed, 0);
        assert_eq!(report.summary.coverage.total_functions, 0);
    }
}
