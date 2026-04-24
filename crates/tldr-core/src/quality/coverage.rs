//! Coverage report parsing module
//!
//! Parses coverage reports in multiple formats:
//! - Cobertura XML (GitLab/Jenkins standard)
//! - LCOV (llvm-cov, gcov)
//! - coverage.py JSON
//!
//! # Security
//! The XML parser (quick-xml) does NOT support DTD/external entities by default,
//! making it safe from XXE attacks (CM-4 mitigation).
//!
//! # Example
//! ```ignore
//! use tldr_core::quality::coverage::{parse_coverage, CoverageFormat};
//!
//! let report = parse_coverage(Path::new("coverage.xml"), None)?;
//! println!("Line coverage: {:.1}%", report.summary.line_coverage);
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::TldrError;
use crate::TldrResult;

// =============================================================================
// Types
// =============================================================================

/// Supported coverage report formats
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CoverageFormat {
    /// Cobertura XML format
    Cobertura,
    /// LCOV format (llvm-cov, gcov)
    Lcov,
    /// coverage.py JSON format
    CoveragePy,
}

impl std::fmt::Display for CoverageFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CoverageFormat::Cobertura => write!(f, "cobertura"),
            CoverageFormat::Lcov => write!(f, "lcov"),
            CoverageFormat::CoveragePy => write!(f, "coveragepy"),
        }
    }
}

/// Line coverage information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineCoverage {
    /// Line number (1-based)
    pub line: u32,
    /// Number of times this line was executed
    pub hits: u64,
}

/// Function coverage information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCoverage {
    /// Function name
    pub name: String,
    /// Starting line number
    pub line: u32,
    /// Number of times the function was executed
    pub hits: u64,
}

/// Coverage data for a single file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileCoverage {
    /// File path (as recorded in the coverage report)
    pub path: String,
    /// Line coverage percentage (0.0 - 100.0)
    pub line_coverage: f64,
    /// Branch coverage percentage (0.0 - 100.0), if available
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_coverage: Option<f64>,
    /// Total lines tracked
    pub total_lines: u32,
    /// Covered lines count
    pub covered_lines: u32,
    /// Total branches (if tracked)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_branches: Option<u32>,
    /// Covered branches count (if tracked)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub covered_branches: Option<u32>,
    /// List of uncovered line numbers
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub uncovered_lines: Vec<u32>,
    /// Function coverage data
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub functions: Vec<FunctionCoverage>,
    /// Whether this file exists on disk
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_exists: Option<bool>,
}

/// Uncovered function information for reporting
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UncoveredFunction {
    /// File containing the function
    pub file: String,
    /// Function name
    pub name: String,
    /// Starting line number
    pub line: u32,
}

/// Range of uncovered lines for compact reporting
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UncoveredLineRange {
    /// File containing the uncovered lines
    pub file: String,
    /// Start of uncovered range (1-based, inclusive)
    pub start: u32,
    /// End of uncovered range (1-based, inclusive)
    pub end: u32,
}

/// Summary of uncovered code
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UncoveredSummary {
    /// List of uncovered functions
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub functions: Vec<UncoveredFunction>,
    /// Consolidated uncovered line ranges
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub line_ranges: Vec<UncoveredLineRange>,
}

/// Summary statistics for the coverage report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageSummary {
    /// Overall line coverage percentage (0.0 - 100.0)
    pub line_coverage: f64,
    /// Overall branch coverage percentage, if available
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_coverage: Option<f64>,
    /// Overall function coverage percentage, if available
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_coverage: Option<f64>,
    /// Total lines across all files
    pub total_lines: u32,
    /// Total covered lines
    pub covered_lines: u32,
    /// Total branches (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_branches: Option<u32>,
    /// Covered branches (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub covered_branches: Option<u32>,
    /// Total functions (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_functions: Option<u32>,
    /// Covered functions (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub covered_functions: Option<u32>,
    /// Whether the threshold was met
    pub threshold_met: bool,
}

/// Complete coverage report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageReport {
    /// Format of the source report
    pub format: CoverageFormat,
    /// Overall summary statistics
    pub summary: CoverageSummary,
    /// Per-file coverage data
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub files: Vec<FileCoverage>,
    /// Uncovered code summary
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uncovered: Option<UncoveredSummary>,
    /// Warnings encountered during parsing
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub warnings: Vec<String>,
}

/// Options for coverage parsing
#[derive(Debug, Clone, Default)]
pub struct CoverageOptions {
    /// Minimum coverage threshold (0.0 - 100.0)
    pub threshold: f64,
    /// Include per-file breakdown
    pub by_file: bool,
    /// Include uncovered code details
    pub include_uncovered: bool,
    /// Filter to files matching these patterns
    pub filter: Vec<String>,
    /// Base path for resolving file paths
    pub base_path: Option<PathBuf>,
}

// =============================================================================
// Main API
// =============================================================================

/// Parse a coverage report file
///
/// # Arguments
/// * `path` - Path to the coverage report file
/// * `format` - Optional format hint (auto-detect if None)
/// * `options` - Parsing options
///
/// # Returns
/// * `Ok(CoverageReport)` - Parsed coverage data
/// * `Err(TldrError)` - On file not found or parse errors
pub fn parse_coverage(
    path: &Path,
    format: Option<CoverageFormat>,
    options: &CoverageOptions,
) -> TldrResult<CoverageReport> {
    // Check file exists
    if !path.exists() {
        return Err(TldrError::PathNotFound(path.to_path_buf()));
    }

    // Read file content
    let content = std::fs::read_to_string(path).map_err(|e| TldrError::ParseError {
        file: path.to_path_buf(),
        line: None,
        message: format!("Failed to read file: {}", e),
    })?;

    // Auto-detect format if not specified
    let detected_format = format.unwrap_or_else(|| detect_format(&content));

    // Parse based on format
    let mut report = match detected_format {
        CoverageFormat::Cobertura => parse_cobertura(&content)?,
        CoverageFormat::Lcov => parse_lcov(&content)?,
        CoverageFormat::CoveragePy => parse_coverage_py_json(&content)?,
    };

    // Apply options
    report.summary.threshold_met = report.summary.line_coverage >= options.threshold;

    // Filter files if patterns specified
    if !options.filter.is_empty() {
        report.files.retain(|f| {
            options
                .filter
                .iter()
                .any(|pattern| f.path.contains(pattern))
        });
    }

    // Check file existence and add warnings
    if let Some(base_path) = &options.base_path {
        for file in &mut report.files {
            let full_path = base_path.join(&file.path);
            let exists = full_path.exists();
            file.file_exists = Some(exists);
            if !exists {
                report
                    .warnings
                    .push(format!("File not found on disk: {}", file.path));
            }
        }
    }

    // Build uncovered summary if requested
    if options.include_uncovered {
        report.uncovered = Some(build_uncovered_summary(&report.files));
    }

    // Clear per-file data if not requested
    if !options.by_file {
        report.files.clear();
    }

    Ok(report)
}

/// Detect coverage format from file content
pub fn detect_format(content: &str) -> CoverageFormat {
    let trimmed = content.trim();

    // Check for XML (Cobertura)
    if trimmed.starts_with("<?xml") || trimmed.starts_with("<coverage") {
        return CoverageFormat::Cobertura;
    }

    // Check for LCOV format markers
    if trimmed.contains("SF:") && trimmed.contains("end_of_record") {
        return CoverageFormat::Lcov;
    }

    // Check for JSON (coverage.py)
    if trimmed.starts_with('{') {
        return CoverageFormat::CoveragePy;
    }

    // Default to Cobertura as it's most common
    CoverageFormat::Cobertura
}

// =============================================================================
// Cobertura XML Parser
// =============================================================================

/// Parse Cobertura XML format
pub fn parse_cobertura(xml: &str) -> TldrResult<CoverageReport> {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut files: Vec<FileCoverage> = Vec::new();
    let warnings: Vec<String> = Vec::new();

    // Root-level attributes
    let mut root_line_rate: Option<f64> = None;
    let mut root_branch_rate: Option<f64> = None;
    let mut root_lines_valid: Option<u32> = None;
    let mut root_lines_covered: Option<u32> = None;

    // Current file being parsed
    let mut current_file: Option<FileCoverage> = None;
    let mut current_lines: HashMap<u32, u64> = HashMap::new();
    let mut current_functions: Vec<FunctionCoverage> = Vec::new();

    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let tag_name = e.name();
                let tag_name_str = std::str::from_utf8(tag_name.as_ref()).unwrap_or("");

                match tag_name_str {
                    "coverage" => {
                        // Parse root attributes
                        for attr in e.attributes().filter_map(|a| a.ok()) {
                            let key = std::str::from_utf8(attr.key.as_ref()).unwrap_or("");
                            let value = std::str::from_utf8(&attr.value).unwrap_or("");

                            match key {
                                "line-rate" => {
                                    root_line_rate = value.parse::<f64>().ok().map(|r| r * 100.0)
                                }
                                "branch-rate" => {
                                    root_branch_rate = value.parse::<f64>().ok().map(|r| r * 100.0)
                                }
                                "lines-valid" => root_lines_valid = value.parse().ok(),
                                "lines-covered" => root_lines_covered = value.parse().ok(),
                                _ => {}
                            }
                        }
                    }
                    "class" => {
                        // Start a new file/class
                        let mut filename = String::new();
                        let mut line_rate = 0.0;
                        let mut branch_rate: Option<f64> = None;

                        for attr in e.attributes().filter_map(|a| a.ok()) {
                            let key = std::str::from_utf8(attr.key.as_ref()).unwrap_or("");
                            let value = std::str::from_utf8(&attr.value).unwrap_or("");

                            match key {
                                "filename" => filename = value.to_string(),
                                "line-rate" => {
                                    line_rate = value.parse::<f64>().unwrap_or(0.0) * 100.0
                                }
                                "branch-rate" => {
                                    branch_rate = value.parse::<f64>().ok().map(|r| r * 100.0)
                                }
                                _ => {}
                            }
                        }

                        current_file = Some(FileCoverage {
                            path: filename,
                            line_coverage: line_rate,
                            branch_coverage: branch_rate,
                            total_lines: 0,
                            covered_lines: 0,
                            total_branches: None,
                            covered_branches: None,
                            uncovered_lines: Vec::new(),
                            functions: Vec::new(),
                            file_exists: None,
                        });
                        current_lines.clear();
                        current_functions.clear();
                    }
                    "method" => {
                        // Parse method/function coverage
                        let mut name = String::new();
                        let mut line_rate = 0.0;

                        for attr in e.attributes().filter_map(|a| a.ok()) {
                            let key = std::str::from_utf8(attr.key.as_ref()).unwrap_or("");
                            let value = std::str::from_utf8(&attr.value).unwrap_or("");

                            match key {
                                "name" => name = value.to_string(),
                                "line-rate" => line_rate = value.parse::<f64>().unwrap_or(0.0),
                                _ => {}
                            }
                        }

                        if !name.is_empty() {
                            // We'll get the line number from the first line inside
                            current_functions.push(FunctionCoverage {
                                name,
                                line: 0, // Will be updated from line elements
                                hits: if line_rate > 0.0 { 1 } else { 0 },
                            });
                        }
                    }
                    "line" => {
                        // Parse line coverage
                        let mut line_num: u32 = 0;
                        let mut hits: u64 = 0;

                        for attr in e.attributes().filter_map(|a| a.ok()) {
                            let key = std::str::from_utf8(attr.key.as_ref()).unwrap_or("");
                            let value = std::str::from_utf8(&attr.value).unwrap_or("");

                            match key {
                                "number" => line_num = value.parse().unwrap_or(0),
                                "hits" => hits = value.parse().unwrap_or(0),
                                _ => {}
                            }
                        }

                        if line_num > 0 {
                            // Use the last value if there are conflicts (per spec)
                            current_lines.insert(line_num, hits);

                            // Update function line if this is the first line
                            if let Some(func) = current_functions.last_mut() {
                                if func.line == 0 {
                                    func.line = line_num;
                                    func.hits = hits;
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::End(e)) => {
                let name_bytes = e.name();
                let tag_name = std::str::from_utf8(name_bytes.as_ref()).unwrap_or("");

                if tag_name == "class" {
                    // Finalize current file
                    if let Some(mut file) = current_file.take() {
                        file.total_lines = current_lines.len() as u32;
                        file.covered_lines =
                            current_lines.values().filter(|&&h| h > 0).count() as u32;
                        file.uncovered_lines = current_lines
                            .iter()
                            .filter(|(_, &h)| h == 0)
                            .map(|(&l, _)| l)
                            .collect();
                        file.uncovered_lines.sort();
                        file.functions = std::mem::take(&mut current_functions);

                        // Recalculate line coverage from actual data
                        if file.total_lines > 0 {
                            file.line_coverage =
                                (file.covered_lines as f64 / file.total_lines as f64) * 100.0;
                        }

                        files.push(file);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(TldrError::CoverageParseError {
                    format: "cobertura".to_string(),
                    detail: format!(
                        "XML parse error at position {}: {}",
                        reader.buffer_position(),
                        e
                    ),
                });
            }
            _ => {}
        }
        buf.clear();
    }

    // Calculate summary
    let (total_lines, covered_lines) = match (root_lines_valid, root_lines_covered) {
        (Some(valid), Some(covered)) => (valid, covered),
        _ => files.iter().fold((0u32, 0u32), |(tl, cl), f| {
            (tl + f.total_lines, cl + f.covered_lines)
        }),
    };

    let line_coverage = root_line_rate.unwrap_or_else(|| {
        if total_lines > 0 {
            (covered_lines as f64 / total_lines as f64) * 100.0
        } else {
            0.0
        }
    });

    // Count functions
    let (total_functions, covered_functions): (u32, u32) =
        files.iter().fold((0, 0), |(tf, cf), f| {
            let covered = f.functions.iter().filter(|func| func.hits > 0).count() as u32;
            (tf + f.functions.len() as u32, cf + covered)
        });

    let function_coverage = if total_functions > 0 {
        Some((covered_functions as f64 / total_functions as f64) * 100.0)
    } else {
        None
    };

    let summary = CoverageSummary {
        line_coverage,
        branch_coverage: root_branch_rate,
        function_coverage,
        total_lines,
        covered_lines,
        total_branches: None,
        covered_branches: None,
        total_functions: if total_functions > 0 {
            Some(total_functions)
        } else {
            None
        },
        covered_functions: if total_functions > 0 {
            Some(covered_functions)
        } else {
            None
        },
        threshold_met: false, // Will be set by parse_coverage
    };

    Ok(CoverageReport {
        format: CoverageFormat::Cobertura,
        summary,
        files,
        uncovered: None,
        warnings,
    })
}

// =============================================================================
// LCOV Parser
// =============================================================================

/// Parse LCOV format
pub fn parse_lcov(content: &str) -> TldrResult<CoverageReport> {
    let mut files: Vec<FileCoverage> = Vec::new();
    let warnings: Vec<String> = Vec::new();
    let mut state = LcovParseState::default();

    for line in content.lines().map(str::trim) {
        if let Some(path) = line.strip_prefix("SF:") {
            state.reset(path.to_string());
            continue;
        }
        if let Some(payload) = line.strip_prefix("FN:") {
            state.parse_function_definition(payload);
            continue;
        }
        if let Some(payload) = line.strip_prefix("FNDA:") {
            state.parse_function_hits(payload);
            continue;
        }
        if let Some(payload) = line.strip_prefix("DA:") {
            state.parse_line_hits(payload);
            continue;
        }
        if let Some(payload) = line.strip_prefix("LF:") {
            state.lf = payload.parse().unwrap_or(0);
            continue;
        }
        if let Some(payload) = line.strip_prefix("LH:") {
            state.lh = payload.parse().unwrap_or(0);
            continue;
        }
        if let Some(payload) = line.strip_prefix("BRF:") {
            state.brf = payload.parse().ok();
            continue;
        }
        if let Some(payload) = line.strip_prefix("BRH:") {
            state.brh = payload.parse().ok();
            continue;
        }
        if line == "end_of_record" {
            if let Some(file_coverage) = state.finalize_current_file() {
                files.push(file_coverage);
            }
        }
    }

    let summary = summarize_lcov_files(&files);

    Ok(CoverageReport {
        format: CoverageFormat::Lcov,
        summary,
        files,
        uncovered: None,
        warnings,
    })
}

#[derive(Default)]
struct LcovParseState {
    current_file: Option<String>,
    current_lines: HashMap<u32, u64>,
    current_functions: Vec<FunctionCoverage>,
    lf: u32,
    lh: u32,
    brf: Option<u32>,
    brh: Option<u32>,
}

impl LcovParseState {
    fn reset(&mut self, file_path: String) {
        self.current_file = Some(file_path);
        self.current_lines.clear();
        self.current_functions.clear();
        self.lf = 0;
        self.lh = 0;
        self.brf = None;
        self.brh = None;
    }

    fn parse_function_definition(&mut self, payload: &str) {
        let parts: Vec<&str> = payload.splitn(2, ',').collect();
        if parts.len() != 2 {
            return;
        }
        let Ok(line_num) = parts[0].parse::<u32>() else {
            return;
        };
        self.current_functions.push(FunctionCoverage {
            name: parts[1].to_string(),
            line: line_num,
            hits: 0,
        });
    }

    fn parse_function_hits(&mut self, payload: &str) {
        let parts: Vec<&str> = payload.splitn(2, ',').collect();
        if parts.len() != 2 {
            return;
        }
        let Ok(hits) = parts[0].parse::<u64>() else {
            return;
        };
        if let Some(func) = self
            .current_functions
            .iter_mut()
            .find(|f| f.name == parts[1])
        {
            func.hits = hits;
        }
    }

    fn parse_line_hits(&mut self, payload: &str) {
        let parts: Vec<&str> = payload.splitn(2, ',').collect();
        if parts.len() < 2 {
            return;
        }
        let (Ok(line_num), Ok(hits)) = (parts[0].parse::<u32>(), parts[1].parse::<u64>()) else {
            return;
        };
        self.current_lines.insert(line_num, hits);
    }

    fn finalize_current_file(&mut self) -> Option<FileCoverage> {
        let path = self.current_file.take()?;
        let total_lines = if self.lf > 0 {
            self.lf
        } else {
            self.current_lines.len() as u32
        };
        let covered_lines = if self.lh > 0 {
            self.lh
        } else {
            self.current_lines.values().filter(|&&h| h > 0).count() as u32
        };
        let line_coverage = if total_lines > 0 {
            (covered_lines as f64 / total_lines as f64) * 100.0
        } else {
            0.0
        };
        let branch_coverage = match (self.brf, self.brh) {
            (Some(total), Some(hit)) if total > 0 => Some((hit as f64 / total as f64) * 100.0),
            _ => None,
        };
        let uncovered_lines: Vec<u32> = self
            .current_lines
            .iter()
            .filter(|(_, &hits)| hits == 0)
            .map(|(&line, _)| line)
            .collect();

        Some(FileCoverage {
            path,
            line_coverage,
            branch_coverage,
            total_lines,
            covered_lines,
            total_branches: self.brf,
            covered_branches: self.brh,
            uncovered_lines,
            functions: std::mem::take(&mut self.current_functions),
            file_exists: None,
        })
    }
}

fn summarize_lcov_files(files: &[FileCoverage]) -> CoverageSummary {
    let (total_lines, covered_lines) = files.iter().fold((0u32, 0u32), |(tl, cl), file| {
        (tl + file.total_lines, cl + file.covered_lines)
    });
    let line_coverage = if total_lines > 0 {
        (covered_lines as f64 / total_lines as f64) * 100.0
    } else {
        0.0
    };

    let (total_branches, covered_branches) = files.iter().fold((0u32, 0u32), |(tb, cb), file| {
        (
            tb + file.total_branches.unwrap_or(0),
            cb + file.covered_branches.unwrap_or(0),
        )
    });
    let branch_coverage = if total_branches > 0 {
        Some((covered_branches as f64 / total_branches as f64) * 100.0)
    } else {
        None
    };

    let (total_functions, covered_functions) = files.iter().fold((0u32, 0u32), |(tf, cf), file| {
        let covered = file.functions.iter().filter(|func| func.hits > 0).count() as u32;
        (tf + file.functions.len() as u32, cf + covered)
    });
    let function_coverage = if total_functions > 0 {
        Some((covered_functions as f64 / total_functions as f64) * 100.0)
    } else {
        None
    };

    CoverageSummary {
        line_coverage,
        branch_coverage,
        function_coverage,
        total_lines,
        covered_lines,
        total_branches: (total_branches > 0).then_some(total_branches),
        covered_branches: (covered_branches > 0).then_some(covered_branches),
        total_functions: (total_functions > 0).then_some(total_functions),
        covered_functions: (total_functions > 0).then_some(covered_functions),
        threshold_met: false,
    }
}

// =============================================================================
// coverage.py JSON Parser
// =============================================================================

/// Intermediate structure for coverage.py JSON
#[derive(Debug, Deserialize)]
struct CoveragePyJson {
    #[serde(default)]
    files: HashMap<String, CoveragePyFile>,
    #[serde(default)]
    totals: CoveragePyTotals,
}

#[derive(Debug, Default, Deserialize)]
struct CoveragePyFile {
    #[serde(default)]
    executed_lines: Vec<u32>,
    #[serde(default)]
    missing_lines: Vec<u32>,
    #[serde(default)]
    summary: Option<CoveragePyFileSummary>,
}

#[derive(Debug, Default, Deserialize)]
struct CoveragePyFileSummary {
    #[serde(default)]
    percent_covered: f64,
}

#[derive(Debug, Default, Deserialize)]
struct CoveragePyTotals {
    #[serde(default)]
    covered_lines: u32,
    #[serde(default)]
    num_statements: u32,
    #[serde(default)]
    percent_covered: f64,
}

/// Parse coverage.py JSON format
pub fn parse_coverage_py_json(json_str: &str) -> TldrResult<CoverageReport> {
    let parsed: CoveragePyJson =
        serde_json::from_str(json_str).map_err(|e| TldrError::CoverageParseError {
            format: "coveragepy".to_string(),
            detail: format!("JSON parse error: {}", e),
        })?;

    let mut files: Vec<FileCoverage> = Vec::new();
    let warnings: Vec<String> = Vec::new();

    for (path, file_data) in parsed.files {
        let total_lines =
            file_data.executed_lines.len() as u32 + file_data.missing_lines.len() as u32;
        let covered_lines = file_data.executed_lines.len() as u32;

        let line_coverage = if let Some(summary) = &file_data.summary {
            summary.percent_covered
        } else if total_lines > 0 {
            (covered_lines as f64 / total_lines as f64) * 100.0
        } else {
            0.0
        };

        files.push(FileCoverage {
            path,
            line_coverage,
            branch_coverage: None, // coverage.py JSON doesn't include branch by default
            total_lines,
            covered_lines,
            total_branches: None,
            covered_branches: None,
            uncovered_lines: file_data.missing_lines,
            functions: Vec::new(), // coverage.py JSON doesn't include function data by default
            file_exists: None,
        });
    }

    let summary = CoverageSummary {
        line_coverage: parsed.totals.percent_covered,
        branch_coverage: None,
        function_coverage: None,
        total_lines: parsed.totals.num_statements,
        covered_lines: parsed.totals.covered_lines,
        total_branches: None,
        covered_branches: None,
        total_functions: None,
        covered_functions: None,
        threshold_met: false,
    };

    Ok(CoverageReport {
        format: CoverageFormat::CoveragePy,
        summary,
        files,
        uncovered: None,
        warnings,
    })
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Build summary of uncovered code
fn build_uncovered_summary(files: &[FileCoverage]) -> UncoveredSummary {
    let mut uncovered_functions: Vec<UncoveredFunction> = Vec::new();
    let mut line_ranges: Vec<UncoveredLineRange> = Vec::new();

    for file in files {
        // Collect uncovered functions
        for func in &file.functions {
            if func.hits == 0 {
                uncovered_functions.push(UncoveredFunction {
                    file: file.path.clone(),
                    name: func.name.clone(),
                    line: func.line,
                });
            }
        }

        // Consolidate uncovered lines into ranges
        if !file.uncovered_lines.is_empty() {
            let mut sorted_lines: Vec<u32> = file.uncovered_lines.clone();
            sorted_lines.sort();

            let mut start = sorted_lines[0];
            let mut end = start;

            for &line in &sorted_lines[1..] {
                if line == end + 1 {
                    end = line;
                } else {
                    line_ranges.push(UncoveredLineRange {
                        file: file.path.clone(),
                        start,
                        end,
                    });
                    start = line;
                    end = line;
                }
            }

            // Push the last range
            line_ranges.push(UncoveredLineRange {
                file: file.path.clone(),
                start,
                end,
            });
        }
    }

    UncoveredSummary {
        functions: uncovered_functions,
        line_ranges,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_format_cobertura() {
        let xml = r#"<?xml version="1.0" ?><coverage></coverage>"#;
        assert_eq!(detect_format(xml), CoverageFormat::Cobertura);
    }

    #[test]
    fn test_detect_format_lcov() {
        let lcov = "TN:test\nSF:/path/file.py\nDA:1,1\nend_of_record";
        assert_eq!(detect_format(lcov), CoverageFormat::Lcov);
    }

    #[test]
    fn test_detect_format_coveragepy() {
        let json = r#"{"meta": {}, "files": {}}"#;
        assert_eq!(detect_format(json), CoverageFormat::CoveragePy);
    }

    #[test]
    fn test_parse_cobertura_basic() {
        // Test without root line-rate to verify recalculation from actual line data
        let xml = r#"<?xml version="1.0" ?>
<coverage>
    <packages>
        <package name="pkg">
            <classes>
                <class filename="src/test.py">
                    <methods>
                        <method name="func1" line-rate="1.0" />
                    </methods>
                    <lines>
                        <line number="1" hits="5"/>
                        <line number="2" hits="0"/>
                    </lines>
                </class>
            </classes>
        </package>
    </packages>
</coverage>"#;

        let report = parse_cobertura(xml).expect("Should parse");
        // 1 of 2 lines covered = 50%
        assert!(
            (report.summary.line_coverage - 50.0).abs() < 1.0,
            "Expected ~50%, got {}",
            report.summary.line_coverage
        );
        assert_eq!(report.files.len(), 1);
        assert_eq!(report.files[0].path, "src/test.py");
        // Verify per-file coverage also recalculated
        assert!(
            (report.files[0].line_coverage - 50.0).abs() < 1.0,
            "File coverage should be ~50%, got {}",
            report.files[0].line_coverage
        );
    }

    #[test]
    fn test_parse_lcov_basic() {
        let lcov = r#"TN:test
SF:/path/test.py
FN:10,func1
FNDA:5,func1
DA:1,5
DA:2,0
DA:3,3
LF:3
LH:2
end_of_record"#;

        let report = parse_lcov(lcov).expect("Should parse");
        assert!((report.summary.line_coverage - 66.67).abs() < 1.0); // 2 of 3 lines
        assert_eq!(report.files.len(), 1);
        assert_eq!(report.files[0].functions.len(), 1);
        assert_eq!(report.files[0].functions[0].hits, 5);
    }

    #[test]
    fn test_parse_coveragepy_basic() {
        let json = r#"{
            "meta": {"version": "7.0"},
            "files": {
                "src/test.py": {
                    "executed_lines": [1, 2, 3],
                    "missing_lines": [4, 5]
                }
            },
            "totals": {
                "covered_lines": 3,
                "num_statements": 5,
                "percent_covered": 60.0
            }
        }"#;

        let report = parse_coverage_py_json(json).expect("Should parse");
        assert!((report.summary.line_coverage - 60.0).abs() < 0.1);
        assert_eq!(report.files.len(), 1);
    }

    #[test]
    fn test_coverage_range_consolidation() {
        let files = vec![FileCoverage {
            path: "test.py".to_string(),
            line_coverage: 50.0,
            branch_coverage: None,
            total_lines: 10,
            covered_lines: 5,
            total_branches: None,
            covered_branches: None,
            uncovered_lines: vec![1, 2, 3, 7, 8, 10], // Should become [1-3], [7-8], [10-10]
            functions: Vec::new(),
            file_exists: None,
        }];

        let summary = build_uncovered_summary(&files);
        assert_eq!(summary.line_ranges.len(), 3);
        assert_eq!(summary.line_ranges[0].start, 1);
        assert_eq!(summary.line_ranges[0].end, 3);
        assert_eq!(summary.line_ranges[1].start, 7);
        assert_eq!(summary.line_ranges[1].end, 8);
        assert_eq!(summary.line_ranges[2].start, 10);
        assert_eq!(summary.line_ranges[2].end, 10);
    }

    #[test]
    fn test_empty_coverage_report() {
        let json = r#"{
            "meta": {"version": "7.0"},
            "files": {},
            "totals": {
                "covered_lines": 0,
                "num_statements": 0,
                "percent_covered": 0.0
            }
        }"#;

        let report = parse_coverage_py_json(json).expect("Should parse empty report");
        assert!((report.summary.line_coverage - 0.0).abs() < 0.1);
        assert_eq!(report.files.len(), 0);
    }

    #[test]
    fn test_malformed_xml_error() {
        let bad_xml = r#"<?xml version="1.0" ?>
<coverage>
    <packages>
        <package>
            <!-- Missing closing tag"#;

        let result = parse_cobertura(bad_xml);
        assert!(result.is_err());
        if let Err(TldrError::CoverageParseError { format, detail }) = result {
            assert_eq!(format, "cobertura");
            assert!(detail.contains("XML parse error"));
        }
    }
}
