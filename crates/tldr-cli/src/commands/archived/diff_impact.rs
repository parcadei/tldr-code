//! Diff Impact Command - Change Impact Analysis
//!
//! The diff-impact command analyzes the impact of code changes:
//! - Detects changed functions (from --files list or --git mode)
//! - Finds callers of changed functions (transitive up to --depth)
//! - Suggests affected tests
//! - Provides summary statistics
//!
//! # Example
//!
//! ```bash
//! # Explicit file list
//! tldr diff-impact --files src/utils.py src/core.py
//!
//! # Git mode (get changed files from git diff)
//! tldr diff-impact --git
//!
//! # Custom depth and base
//! tldr diff-impact --git --git-base main --depth 3
//! ```
//!
//! # TIGER Mitigations
//!
//! - TIGER-02: Cycle detection in call graph traversal using CycleDetector
//! - MAX_CALLERS limit prevents unbounded growth

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;
use clap::Args;
use tree_sitter::{Node, Parser};

use super::error::RemainingError;
use super::graph_utils::CycleDetector;
use super::types::{CallInfo, ChangedFunction, DiffImpactReport, DiffImpactSummary};

use crate::output::{OutputFormat, OutputWriter};

// =============================================================================
// CLI Arguments
// =============================================================================

/// Analyze impact of code changes - identify affected functions and suggest tests.
#[derive(Debug, Clone, Args)]
pub struct DiffImpactArgs {
    /// Project root path (required)
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Explicit list of changed files
    #[arg(long, num_args = 1..)]
    pub files: Option<Vec<PathBuf>>,

    /// Get changed files from git diff
    #[arg(long)]
    pub git: bool,

    /// Git ref for diff base
    #[arg(long, default_value = "HEAD~1")]
    pub git_base: String,

    /// Caller search depth
    #[arg(long, default_value = "2")]
    pub depth: u32,

    /// Output file (stdout if not specified)
    #[arg(long, short = 'o')]
    pub output: Option<PathBuf>,
}

// =============================================================================
// Constants
// =============================================================================

/// Maximum callers to collect (TIGER-02 mitigation)
const MAX_CALLERS: usize = 100;

/// Maximum depth for call graph traversal
const MAX_DEPTH: u32 = 10;

// =============================================================================
// Tree-sitter Python Parsing
// =============================================================================

/// Initialize tree-sitter parser for Python
fn get_python_parser() -> Result<Parser, RemainingError> {
    let mut parser = Parser::new();
    let language = tree_sitter_python::LANGUAGE;
    parser
        .set_language(&language.into())
        .map_err(|e| RemainingError::parse_error(PathBuf::new(), format!("Failed to set language: {}", e)))?;
    Ok(parser)
}

/// Get text for a node from source
fn node_text<'a>(node: Node, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
}

/// Get the line number (1-indexed) for a node
fn get_line_number(node: Node) -> u32 {
    node.start_position().row as u32 + 1
}

// =============================================================================
// Git Integration
// =============================================================================

/// Get changed files from git diff
fn get_changed_files_git(project_root: &PathBuf, base: &str) -> Result<Vec<PathBuf>, RemainingError> {
    let output = Command::new("git")
        .args(["diff", "--name-only", base])
        .current_dir(project_root)
        .output()
        .map_err(|e| RemainingError::AnalysisError {
            message: format!("Failed to run git: {}", e),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(RemainingError::AnalysisError {
            message: format!("Git diff failed: {}", stderr),
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let files: Vec<PathBuf> = stdout
        .lines()
        .filter(|line| !line.is_empty())
        .filter(|line| line.ends_with(".py")) // Filter to Python files
        .map(|line| project_root.join(line))
        .collect();

    Ok(files)
}

// =============================================================================
// Function Extraction
// =============================================================================

/// Extract all function definitions from a file
fn extract_functions_from_file(
    file_path: &PathBuf,
) -> Result<Vec<(String, u32)>, RemainingError> {
    if !file_path.exists() {
        return Err(RemainingError::file_not_found(file_path));
    }

    let source = std::fs::read_to_string(file_path)
        .map_err(|e| RemainingError::parse_error(file_path, e.to_string()))?;
    let source_bytes = source.as_bytes();

    let mut parser = get_python_parser()?;
    let tree = parser
        .parse(&source, None)
        .ok_or_else(|| RemainingError::parse_error(file_path, "Failed to parse file"))?;

    let root = tree.root_node();
    let mut functions = Vec::new();
    collect_functions_recursive(root, source_bytes, &mut functions);

    Ok(functions)
}

fn collect_functions_recursive(
    node: Node,
    source: &[u8],
    functions: &mut Vec<(String, u32)>,
) {
    match node.kind() {
        "function_definition" | "async_function_definition" => {
            for child in node.children(&mut node.walk()) {
                if child.kind() == "identifier" {
                    let name = node_text(child, source).to_string();
                    let line = get_line_number(node);
                    functions.push((name, line));
                    break;
                }
            }
        }
        _ => {}
    }

    for child in node.children(&mut node.walk()) {
        collect_functions_recursive(child, source, functions);
    }
}

// =============================================================================
// Caller Analysis
// =============================================================================

/// Find all callers of a function within a set of files
fn find_callers_in_project(
    target_function: &str,
    target_file: &Path,
    project_files: &[PathBuf],
    depth: u32,
    detector: &mut CycleDetector,
) -> Vec<CallInfo> {
    let mut callers = Vec::new();

    if depth == 0 || detector.visited_count() >= MAX_CALLERS {
        return callers;
    }

    // Mark this function as visited (using Path-based API)
    if detector.visit(target_file, target_function) {
        // Already visited - cycle detected
        return callers;
    }

    for file_path in project_files {
        if !file_path.exists() || !file_path.to_string_lossy().ends_with(".py") {
            continue;
        }

        if let Ok(file_callers) = find_callers_in_file(target_function, file_path) {
            for caller in file_callers {
                if callers.len() >= MAX_CALLERS {
                    break;
                }

                // Add the caller
                callers.push(caller.clone());

                // Recursively find callers of this caller (transitive)
                if depth > 1 {
                    let caller_path = PathBuf::from(&caller.file);
                    let transitive = find_callers_in_project(
                        &caller.name,
                        &caller_path,
                        project_files,
                        depth - 1,
                        detector,
                    );
                    for tc in transitive {
                        if callers.len() >= MAX_CALLERS {
                            break;
                        }
                        if !callers.iter().any(|c| c.name == tc.name && c.file == tc.file) {
                            callers.push(tc);
                        }
                    }
                }
            }
        }
    }

    callers
}

/// Find callers of a function within a single file
fn find_callers_in_file(
    target_function: &str,
    file_path: &PathBuf,
) -> Result<Vec<CallInfo>, RemainingError> {
    let source = std::fs::read_to_string(file_path)
        .map_err(|e| RemainingError::parse_error(file_path, e.to_string()))?;
    let source_bytes = source.as_bytes();

    let mut parser = get_python_parser()?;
    let tree = parser
        .parse(&source, None)
        .ok_or_else(|| RemainingError::parse_error(file_path, "Failed to parse file"))?;

    let root = tree.root_node();
    let file_str = file_path.to_string_lossy().to_string();

    let mut callers = Vec::new();
    find_callers_recursive(root, source_bytes, target_function, &file_str, &mut callers, None);

    Ok(callers)
}

fn find_callers_recursive(
    node: Node,
    source: &[u8],
    target_function: &str,
    file_path: &str,
    callers: &mut Vec<CallInfo>,
    current_function: Option<&str>,
) {
    match node.kind() {
        "function_definition" | "async_function_definition" => {
            // Get this function's name
            let mut func_name = None;
            for child in node.children(&mut node.walk()) {
                if child.kind() == "identifier" {
                    func_name = Some(node_text(child, source));
                    break;
                }
            }

            // Recurse with this function as current
            for child in node.children(&mut node.walk()) {
                find_callers_recursive(child, source, target_function, file_path, callers, func_name);
            }
            return;
        }
        "call" => {
            if let Some(name) = extract_call_name(node, source) {
                // Check if this call is to our target function
                let base = name.split('.').last().unwrap_or(&name);
                if base == target_function || name == target_function {
                    if let Some(caller_name) = current_function {
                        // Avoid duplicates and self-references
                        if caller_name != target_function && !callers.iter().any(|c| c.name == caller_name) {
                            callers.push(CallInfo {
                                name: caller_name.to_string(),
                                file: file_path.to_string(),
                                line: get_line_number(node),
                            });
                        }
                    }
                }
            }
        }
        _ => {}
    }

    for child in node.children(&mut node.walk()) {
        find_callers_recursive(child, source, target_function, file_path, callers, current_function);
    }
}

/// Extract call name from a call node
fn extract_call_name(node: Node, source: &[u8]) -> Option<String> {
    if let Some(func) = node.child_by_field_name("function") {
        return Some(extract_name_from_expr(func, source));
    }

    for child in node.children(&mut node.walk()) {
        match child.kind() {
            "identifier" => return Some(node_text(child, source).to_string()),
            "attribute" => return Some(extract_name_from_expr(child, source)),
            _ => continue,
        }
    }
    None
}

/// Extract a dotted name from an expression
fn extract_name_from_expr(node: Node, source: &[u8]) -> String {
    match node.kind() {
        "identifier" => node_text(node, source).to_string(),
        "attribute" => {
            let mut parts = Vec::new();
            let mut current = node;

            loop {
                if let Some(attr) = current.child_by_field_name("attribute") {
                    parts.push(node_text(attr, source).to_string());
                }

                if let Some(obj) = current.child_by_field_name("object") {
                    if obj.kind() == "attribute" {
                        current = obj;
                    } else if obj.kind() == "identifier" {
                        parts.push(node_text(obj, source).to_string());
                        break;
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }

            parts.reverse();
            parts.join(".")
        }
        _ => node_text(node, source).to_string(),
    }
}

// =============================================================================
// Test Suggestion
// =============================================================================

/// Check if a file is a test file
fn is_test_file(path: &PathBuf) -> bool {
    let name = path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    name.starts_with("test_") || name.ends_with("_test.py") || name == "conftest.py"
}

/// Suggest tests that might test the changed functions
fn suggest_tests(
    changed_functions: &[ChangedFunction],
    project_files: &[PathBuf],
) -> Vec<String> {
    let mut suggested = Vec::new();

    // Collect test files
    let test_files: Vec<&PathBuf> = project_files
        .iter()
        .filter(|f| is_test_file(f))
        .collect();

    // For each test file, check if it imports any changed module
    for test_file in test_files {
        if let Ok(source) = std::fs::read_to_string(test_file) {
            let test_path = test_file.to_string_lossy().to_string();

            // Check if any changed function is called in this test
            for changed in changed_functions {
                // Simple heuristic: check if the function name appears in the test file
                if source.contains(&changed.name) {
                    if !suggested.contains(&test_path) {
                        suggested.push(test_path.clone());
                    }
                    break;
                }

                // Also check if the file's module is imported
                if let Some(stem) = PathBuf::from(&changed.file).file_stem() {
                    let module_name = stem.to_string_lossy();
                    if source.contains(&format!("from {} import", module_name))
                        || source.contains(&format!("import {}", module_name))
                    {
                        if !suggested.contains(&test_path) {
                            suggested.push(test_path.clone());
                        }
                        break;
                    }
                }
            }
        }
    }

    suggested
}

/// Collect all Python files in a directory
fn collect_python_files(dir: &PathBuf) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_python_files_recursive(dir, &mut files);
    files
}

fn collect_python_files_recursive(dir: &PathBuf, files: &mut Vec<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // Skip common non-source directories
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if !name.starts_with('.') && name != "__pycache__" && name != "node_modules" && name != "venv" && name != ".venv" {
                    collect_python_files_recursive(&path, files);
                }
            } else if path.extension().and_then(|e| e.to_str()) == Some("py") {
                files.push(path);
            }
        }
    }
}

// =============================================================================
// Text Formatting
// =============================================================================

/// Format a DiffImpactReport as human-readable text
fn format_diff_impact_text(report: &DiffImpactReport) -> String {
    let mut lines = Vec::new();

    lines.push("Diff Impact Report".to_string());
    lines.push("==================".to_string());
    lines.push(String::new());

    // Summary
    lines.push("Summary:".to_string());
    lines.push(format!("  Files changed: {}", report.summary.files_changed));
    lines.push(format!("  Functions changed: {}", report.summary.functions_changed));
    lines.push(format!("  Tests to run: {}", report.summary.tests_to_run));
    lines.push(String::new());

    // Changed functions
    if !report.changed_functions.is_empty() {
        lines.push(format!("Changed Functions ({}):", report.changed_functions.len()));
        for func in &report.changed_functions {
            lines.push(format!("  - {} ({}:{})", func.name, func.file, func.line));
            if !func.callers.is_empty() {
                lines.push(format!("    Callers ({}):", func.callers.len()));
                for caller in &func.callers {
                    lines.push(format!("      - {} ({}:{})", caller.name, caller.file, caller.line));
                }
            }
        }
        lines.push(String::new());
    }

    // Suggested tests
    if !report.suggested_tests.is_empty() {
        lines.push(format!("Suggested Tests ({}):", report.suggested_tests.len()));
        for test in &report.suggested_tests {
            lines.push(format!("  - {}", test));
        }
    }

    lines.join("\n")
}

// =============================================================================
// Entry Point
// =============================================================================

impl DiffImpactArgs {
    /// Run the diff-impact command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        // Validate arguments
        if self.files.is_none() && !self.git {
            return Err(RemainingError::InvalidArgument {
                message: "Either --files or --git must be specified".to_string(),
            }.into());
        }

        let depth = self.depth.min(MAX_DEPTH);

        writer.progress("Analyzing change impact...");

        // Get changed files
        let changed_files = if let Some(ref files) = self.files {
            files.clone()
        } else {
            get_changed_files_git(&self.path, &self.git_base)?
        };

        // Check if any files were found
        if changed_files.is_empty() {
            let report = DiffImpactReport::new();
            if writer.is_text() {
                let text = format_diff_impact_text(&report);
                writer.write_text(&text)?;
            } else {
                writer.write(&report)?;
            }
            return Ok(());
        }

        // Validate all files exist
        for file in &changed_files {
            if !file.exists() {
                return Err(RemainingError::file_not_found(file).into());
            }
        }

        // Collect all project files for cross-file analysis
        // Also include directories containing the changed files
        let mut project_files = collect_python_files(&self.path);
        for file in &changed_files {
            if let Some(parent) = file.parent() {
                let parent_files = collect_python_files(&parent.to_path_buf());
                for pf in parent_files {
                    if !project_files.contains(&pf) {
                        project_files.push(pf);
                    }
                }
            }
        }

        // Build changed functions list
        let mut changed_functions = Vec::new();
        let mut unique_files: HashSet<String> = HashSet::new();

        for file in &changed_files {
            let file_str = file.to_string_lossy().to_string();
            unique_files.insert(file_str.clone());

            if let Ok(functions) = extract_functions_from_file(file) {
                for (name, line) in functions {
                    // Find callers using cycle detector
                    let mut detector = CycleDetector::new();
                    let callers = find_callers_in_project(
                        &name,
                        file.as_path(),
                        &project_files,
                        depth,
                        &mut detector,
                    );

                    changed_functions.push(ChangedFunction {
                        name,
                        file: file_str.clone(),
                        line,
                        callers,
                    });
                }
            }
        }

        // Suggest tests
        let suggested_tests = suggest_tests(&changed_functions, &project_files);

        // Build report
        let report = DiffImpactReport {
            changed_functions: changed_functions.clone(),
            suggested_tests: suggested_tests.clone(),
            summary: DiffImpactSummary {
                files_changed: unique_files.len() as u32,
                functions_changed: changed_functions.len() as u32,
                tests_to_run: suggested_tests.len() as u32,
            },
        };

        // Output based on format
        if writer.is_text() {
            let text = format_diff_impact_text(&report);
            writer.write_text(&text)?;
        } else {
            writer.write(&report)?;
        }

        // Write to output file if specified
        if let Some(ref output_path) = self.output {
            let output_str = if format == OutputFormat::Text {
                format_diff_impact_text(&report)
            } else {
                serde_json::to_string_pretty(&report)?
            };
            std::fs::write(output_path, &output_str)?;
        }

        Ok(())
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_cycle_detector_basic() {
        let mut detector = CycleDetector::new();
        let file = Path::new("file.py");

        // First visit should return false (not already visited)
        assert!(!detector.visit(file, "func_a"));

        // Second visit to same should return true (cycle)
        assert!(detector.visit(file, "func_a"));

        // Different function same file should return false
        assert!(!detector.visit(file, "func_b"));

        // Check visited count
        assert_eq!(detector.visited_count(), 2);
    }

    #[test]
    fn test_cycle_detector_was_visited() {
        let mut detector = CycleDetector::new();
        let file = Path::new("file.py");

        assert!(!detector.was_visited(file, "func_a"));
        detector.visit(file, "func_a");
        assert!(detector.was_visited(file, "func_a"));
        assert!(!detector.was_visited(file, "func_b"));
    }

    #[test]
    fn test_is_test_file() {
        assert!(is_test_file(&PathBuf::from("test_foo.py")));
        assert!(is_test_file(&PathBuf::from("foo_test.py")));
        assert!(is_test_file(&PathBuf::from("conftest.py")));
        assert!(!is_test_file(&PathBuf::from("foo.py")));
        assert!(!is_test_file(&PathBuf::from("tests.py")));
    }

    #[test]
    fn test_extract_functions_from_file() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("sample.py");
        std::fs::write(&file_path, r#"
def function_a():
    return 1

def function_b():
    return function_a() + 1

async def async_func():
    pass
"#).unwrap();

        let functions = extract_functions_from_file(&file_path).unwrap();
        assert_eq!(functions.len(), 3);

        let names: Vec<&str> = functions.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"function_a"));
        assert!(names.contains(&"function_b"));
        assert!(names.contains(&"async_func"));
    }

    #[test]
    fn test_find_callers_in_file() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("sample.py");
        std::fs::write(&file_path, r#"
def helper():
    return 42

def caller1():
    return helper() + 1

def caller2():
    return helper() * 2

def unrelated():
    return 0
"#).unwrap();

        let callers = find_callers_in_file("helper", &file_path).unwrap();
        assert_eq!(callers.len(), 2);

        let names: Vec<&str> = callers.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"caller1"));
        assert!(names.contains(&"caller2"));
        assert!(!names.contains(&"unrelated"));
    }

    #[test]
    fn test_diff_impact_report_json_schema() {
        let report = DiffImpactReport {
            changed_functions: vec![
                ChangedFunction {
                    name: "test_func".to_string(),
                    file: "test.py".to_string(),
                    line: 10,
                    callers: vec![
                        CallInfo {
                            name: "caller".to_string(),
                            file: "test.py".to_string(),
                            line: 20,
                        }
                    ],
                }
            ],
            suggested_tests: vec!["test_module.py".to_string()],
            summary: DiffImpactSummary {
                files_changed: 1,
                functions_changed: 1,
                tests_to_run: 1,
            },
        };

        let json = serde_json::to_value(&report).unwrap();

        assert!(json.get("changed_functions").is_some());
        assert!(json.get("suggested_tests").is_some());
        assert!(json.get("summary").is_some());

        let summary = json.get("summary").unwrap();
        assert!(summary.get("files_changed").is_some());
        assert!(summary.get("functions_changed").is_some());
        assert!(summary.get("tests_to_run").is_some());
    }

    #[test]
    fn test_format_diff_impact_text() {
        let report = DiffImpactReport {
            changed_functions: vec![
                ChangedFunction {
                    name: "my_func".to_string(),
                    file: "module.py".to_string(),
                    line: 5,
                    callers: vec![],
                }
            ],
            suggested_tests: vec!["test_module.py".to_string()],
            summary: DiffImpactSummary {
                files_changed: 1,
                functions_changed: 1,
                tests_to_run: 1,
            },
        };

        let text = format_diff_impact_text(&report);

        assert!(text.contains("Diff Impact Report"));
        assert!(text.contains("Files changed: 1"));
        assert!(text.contains("my_func"));
        assert!(text.contains("test_module.py"));
    }
}
