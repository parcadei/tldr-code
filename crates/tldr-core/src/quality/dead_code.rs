//! Dead Code Analyzer for Health Command
//!
//! This module provides unreachable function detection for the health analysis.
//! It wraps the existing analysis/dead.rs functionality with health-specific
//! types and reporting.
//!
//! # Algorithm
//!
//! 1. Build project call graph (cross-file calls)
//! 2. Identify all called functions
//! 3. Mark entry points as reachable (main, test_, setup, etc.)
//! 4. Mark dunder methods as reachable (__init__, __str__, etc.)
//! 5. Functions never reached = dead code
//!
//! # Entry Point Exclusions
//!
//! - main, __main__, cli, app, run, start
//! - test_*, pytest_*
//! - setup, teardown
//! - Custom patterns from entry_points parameter
//!
//! # Dunder Exclusions
//!
//! All Python dunder methods (__init__, __str__, __repr__, etc.) are excluded
//! as they may be called implicitly by the Python runtime.
//!
//! # References
//!
//! - Health spec section 4.3
//! - Premortem T11: Remove/use unused callers variable

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::walker::walk_project;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::analysis::dead::{
    collect_all_functions, dead_code_analysis, dead_code_analysis_refcount,
};
use crate::analysis::refcount::count_identifiers_in_tree;
use crate::ast::extract::extract_file;
use crate::ast::parser::parse_file;
use crate::callgraph::build_project_call_graph;
use crate::error::TldrError;
use crate::types::{Language, ModuleInfo, ProjectCallGraph};
use crate::TldrResult;

// =============================================================================
// Types
// =============================================================================

/// Visibility of a function
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    /// Public function (no leading underscore)
    Public,
    /// Private function (single leading underscore)
    Private,
    /// Internal function (double leading underscore)
    Internal,
}

impl Visibility {
    /// Determine visibility from function name
    pub fn from_name(name: &str) -> Self {
        // Strip class prefix if present (e.g., "MyClass.method" -> "method")
        let base_name = name.rsplit('.').next().unwrap_or(name);

        // Dunder methods (__init__, __str__, etc.) are public/special
        if base_name.starts_with("__") && base_name.ends_with("__") && base_name.len() > 4 {
            // Dunder method (e.g., __init__, __str__)
            Visibility::Public
        } else if base_name.starts_with("__") {
            // Name mangled (e.g., __private - starts with __ but doesn't end with __)
            Visibility::Internal
        } else if base_name.starts_with('_') {
            // Private (single underscore prefix)
            Visibility::Private
        } else {
            // Public (no underscore prefix)
            Visibility::Public
        }
    }
}

/// Reason why a function is considered dead
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeadReason {
    /// Function is never called from any reachable code
    NeverCalled,
    /// Function is only called by other dead code (transitively dead)
    OnlyCalledByDead,
}

/// A function identified as dead code
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadFunction {
    /// Function name
    pub name: String,
    /// File path containing the function
    pub file: PathBuf,
    /// Line number where the function starts
    pub line: usize,
    /// Visibility of the function
    pub visibility: Visibility,
    /// Reason for being flagged as dead
    pub reason: DeadReason,
}

/// Summary statistics for dead code analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadCodeSummary {
    /// Total dead functions found
    pub total_dead: usize,
    /// Total functions analyzed
    pub total_functions: usize,
    /// Percentage of dead functions
    pub dead_percentage: f64,
    /// Dead public functions (most concerning)
    pub dead_public: usize,
    /// Dead private functions
    pub dead_private: usize,
}

impl Default for DeadCodeSummary {
    fn default() -> Self {
        Self {
            total_dead: 0,
            total_functions: 0,
            dead_percentage: 0.0,
            dead_public: 0,
            dead_private: 0,
        }
    }
}

/// Complete dead code analysis report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadCodeReport {
    /// Number of functions analyzed
    pub functions_analyzed: usize,
    /// Number of dead functions found
    pub dead_count: usize,
    /// Percentage of dead code
    pub dead_percentage: f64,
    /// List of dead functions (sorted by file, then line)
    pub dead_functions: Vec<DeadFunction>,
    /// Dead functions grouped by file
    pub by_file: IndexMap<PathBuf, Vec<DeadFunction>>,
    /// Summary statistics
    pub summary: DeadCodeSummary,
}

impl Default for DeadCodeReport {
    fn default() -> Self {
        Self {
            functions_analyzed: 0,
            dead_count: 0,
            dead_percentage: 0.0,
            dead_functions: Vec::new(),
            by_file: IndexMap::new(),
            summary: DeadCodeSummary::default(),
        }
    }
}

// =============================================================================
// Main API
// =============================================================================

/// Analyze dead code in a codebase
///
/// Detects unreachable functions using call graph analysis.
///
/// # Arguments
/// * `path` - Directory or file to analyze
/// * `language` - Optional language filter (auto-detect if None)
/// * `entry_points` - Additional entry point patterns to exclude
///
/// # Returns
/// * `Ok(DeadCodeReport)` - Report with dead code findings
/// * `Err(TldrError)` - On file system errors
///
/// # Exclusions
///
/// The following are NOT flagged as dead code:
/// - Entry points: main, test_, setup, teardown, etc.
/// - Dunder methods: __init__, __str__, __repr__, etc.
/// - Functions that are called by any reachable code
///
/// # Example
/// ```ignore
/// use tldr_core::quality::dead_code::analyze_dead_code;
/// use std::path::Path;
///
/// let report = analyze_dead_code(Path::new("src/"), None, &[])?;
/// for func in &report.dead_functions {
///     println!("{}: {} (line {})", func.file.display(), func.name, func.line);
/// }
/// ```
pub fn analyze_dead_code(
    path: &Path,
    language: Option<Language>,
    entry_points: &[&str],
) -> TldrResult<DeadCodeReport> {
    // Detect language if not specified (delegates to the canonical
    // `Language::from_path` / `Language::from_directory` detectors — VAL-002).
    let lang = match language {
        Some(l) => l,
        None => {
            if path.is_file() {
                Language::from_path(path).ok_or_else(|| {
                    TldrError::UnsupportedLanguage(
                        path.extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or("unknown")
                            .to_string(),
                    )
                })?
            } else {
                Language::from_directory(path)
                    .ok_or_else(|| TldrError::NoSupportedFiles(path.to_path_buf()))?
            }
        }
    };

    // Collect module infos for function enumeration
    let module_infos = collect_module_infos_for_dead(path, lang);

    // Build refcounts by walking and parsing all files
    let ref_counts = build_refcounts(path, lang);

    // Use the underlying refcount-based dead code analysis
    let entry_patterns: Vec<String> = entry_points.iter().map(|s| s.to_string()).collect();
    let entry_ref = if entry_patterns.is_empty() {
        None
    } else {
        Some(entry_patterns.as_slice())
    };

    // Collect all functions with line info
    let all_functions = collect_all_functions(&module_infos);
    let function_lines = collect_function_lines(&module_infos);

    // Run refcount-based dead code analysis
    let core_report = dead_code_analysis_refcount(&all_functions, &ref_counts, entry_ref)?;

    // Transform to quality report format
    transform_core_report(&core_report, &function_lines)
}

/// Analyze dead code using a pre-built call graph (backward-compatible)
///
/// **Deprecated**: Prefer `analyze_dead_code_with_refcount()` which uses
/// reference counting instead of the call graph for more accurate results.
///
/// This variant is kept for callers that already have a call graph and
/// want the CG-based dead code analysis.
///
/// # Arguments
/// * `call_graph` - Pre-built project call graph
/// * `module_infos` - Module information from AST extraction
/// * `entry_points` - Additional entry point patterns to exclude
///
/// # Returns
/// * `DeadCodeReport` - Report with dead code findings
pub fn analyze_dead_code_with_graph(
    call_graph: &ProjectCallGraph,
    module_infos: &[(PathBuf, ModuleInfo)],
    entry_points: &[&str],
) -> TldrResult<DeadCodeReport> {
    let entry_patterns: Vec<String> = entry_points.iter().map(|s| s.to_string()).collect();
    let entry_ref = if entry_patterns.is_empty() {
        None
    } else {
        Some(entry_patterns.as_slice())
    };

    // Collect all functions with line info
    let all_functions = collect_all_functions(module_infos);
    let function_lines = collect_function_lines(module_infos);

    // Run CG-based dead code analysis
    let core_report = dead_code_analysis(call_graph, &all_functions, entry_ref)?;

    // Transform to quality report format
    transform_core_report(&core_report, &function_lines)
}

/// Analyze dead code using reference counting (refcount-based)
///
/// This variant uses identifier reference counting across the codebase
/// instead of a call graph. It is used by the health dashboard where
/// `path` and `language` are already known.
///
/// # Arguments
/// * `path` - Directory or file to analyze (used to walk and parse files for refcounts)
/// * `language` - Programming language
/// * `module_infos` - Module information from AST extraction (for function enumeration)
/// * `entry_points` - Additional entry point patterns to exclude
///
/// # Returns
/// * `DeadCodeReport` - Report with dead code findings
pub fn analyze_dead_code_with_refcount(
    path: &Path,
    language: Language,
    module_infos: &[(PathBuf, ModuleInfo)],
    entry_points: &[&str],
) -> TldrResult<DeadCodeReport> {
    let entry_patterns: Vec<String> = entry_points.iter().map(|s| s.to_string()).collect();
    let entry_ref = if entry_patterns.is_empty() {
        None
    } else {
        Some(entry_patterns.as_slice())
    };

    // Collect all functions with line info
    let all_functions = collect_all_functions(module_infos);
    let function_lines = collect_function_lines(module_infos);

    // Build refcounts by walking and parsing all files
    let ref_counts = build_refcounts(path, language);

    // Run refcount-based dead code analysis
    let core_report = dead_code_analysis_refcount(&all_functions, &ref_counts, entry_ref)?;

    // Transform to quality report format
    transform_core_report(&core_report, &function_lines)
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Transform a core DeadCodeReport (from analysis::dead) into a quality DeadCodeReport.
///
/// This is shared by all wrapper functions (CG-based and refcount-based).
fn transform_core_report(
    core_report: &crate::types::DeadCodeReport,
    function_lines: &HashMap<(PathBuf, String), usize>,
) -> TldrResult<DeadCodeReport> {
    let mut dead_functions: Vec<DeadFunction> = Vec::new();
    let mut by_file: IndexMap<PathBuf, Vec<DeadFunction>> = IndexMap::new();
    let mut dead_public = 0;
    let mut dead_private = 0;

    for func_ref in &core_report.dead_functions {
        let line = function_lines
            .get(&(func_ref.file.clone(), func_ref.name.clone()))
            .copied()
            .unwrap_or(0);

        let visibility = Visibility::from_name(&func_ref.name);

        match visibility {
            Visibility::Public => dead_public += 1,
            Visibility::Private | Visibility::Internal => dead_private += 1,
        }

        let dead_func = DeadFunction {
            name: func_ref.name.clone(),
            file: func_ref.file.clone(),
            line,
            visibility,
            reason: DeadReason::NeverCalled,
        };

        dead_functions.push(dead_func.clone());
        by_file
            .entry(func_ref.file.clone())
            .or_default()
            .push(dead_func);
    }

    // Sort dead functions by file then line
    dead_functions.sort_by(|a, b| a.file.cmp(&b.file).then_with(|| a.line.cmp(&b.line)));

    // Sort by_file entries by line
    for funcs in by_file.values_mut() {
        funcs.sort_by_key(|f| f.line);
    }

    let total_functions = core_report.total_functions;
    let dead_count = dead_functions.len();
    let dead_percentage = if total_functions > 0 {
        (dead_count as f64 / total_functions as f64) * 100.0
    } else {
        0.0
    };

    Ok(DeadCodeReport {
        functions_analyzed: total_functions,
        dead_count,
        dead_percentage,
        dead_functions,
        by_file,
        summary: DeadCodeSummary {
            total_dead: dead_count,
            total_functions,
            dead_percentage,
            dead_public,
            dead_private,
        },
    })
}

/// Build identifier reference counts by walking and parsing all files in a path.
///
/// Walks the directory (or single file), parses each file with tree-sitter,
/// and aggregates identifier counts across all files.
fn build_refcounts(path: &Path, language: Language) -> HashMap<String, usize> {
    let mut ref_counts: HashMap<String, usize> = HashMap::new();

    if path.is_file() {
        if let Ok((tree, source, _lang)) = parse_file(path) {
            let counts = count_identifiers_in_tree(&tree, source.as_bytes(), language);
            for (name, count) in counts {
                *ref_counts.entry(name).or_insert(0) += count;
            }
        }
    } else {
        let extensions = language.extensions();
        for entry in walk_project(path) {
            let file_path = entry.path();
            if file_path.is_file() {
                if let Some(ext) = file_path.extension().and_then(|e| e.to_str()) {
                    let ext_with_dot = format!(".{}", ext);
                    if extensions.contains(&ext_with_dot.as_str()) {
                        if let Ok((tree, source, _lang)) = parse_file(file_path) {
                            let counts =
                                count_identifiers_in_tree(&tree, source.as_bytes(), language);
                            for (name, count) in counts {
                                *ref_counts.entry(name).or_insert(0) += count;
                            }
                        }
                    }
                }
            }
        }
    }

    ref_counts
}

/// Collect module infos for dead code analysis (without building a call graph).
///
/// This replaces the call-graph-based `build_call_graph_and_collect` for the
/// refcount path, collecting only the ModuleInfo data needed for function enumeration.
fn collect_module_infos_for_dead(path: &Path, language: Language) -> Vec<(PathBuf, ModuleInfo)> {
    let mut module_infos: Vec<(PathBuf, ModuleInfo)> = Vec::new();

    if path.is_file() {
        if let Ok(info) = extract_file(path, path.parent()) {
            module_infos.push((path.to_path_buf(), info));
        }
    } else {
        let extensions = language.extensions();
        for entry in walk_project(path) {
            let file_path = entry.path();
            if file_path.is_file() {
                if let Some(ext) = file_path.extension().and_then(|e| e.to_str()) {
                    let ext_with_dot = format!(".{}", ext);
                    if extensions.contains(&ext_with_dot.as_str()) {
                        if let Ok(info) = extract_file(file_path, Some(path)) {
                            module_infos.push((file_path.to_path_buf(), info));
                        }
                    }
                }
            }
        }
    }

    module_infos
}

/// Build call graph and collect module info in one pass (backward compat for CG-based path)
#[allow(dead_code)]
fn build_call_graph_and_collect(
    path: &Path,
    language: Language,
) -> TldrResult<(ProjectCallGraph, Vec<(PathBuf, ModuleInfo)>)> {
    let call_graph = build_project_call_graph(path, language, None, true)?;
    let module_infos = collect_module_infos_for_dead(path, language);
    Ok((call_graph, module_infos))
}

/// Collect function line numbers from module info
fn collect_function_lines(
    module_infos: &[(PathBuf, ModuleInfo)],
) -> HashMap<(PathBuf, String), usize> {
    let mut lines = HashMap::new();

    for (file_path, info) in module_infos {
        // Top-level functions
        for func in &info.functions {
            lines.insert(
                (file_path.clone(), func.name.clone()),
                func.line_number as usize,
            );
        }

        // Class methods
        for class in &info.classes {
            for method in &class.methods {
                let full_name = format!("{}.{}", class.name, method.name);
                lines.insert((file_path.clone(), full_name), method.line_number as usize);
            }
        }
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_visibility_from_name() {
        // Public functions (no leading underscore)
        assert_eq!(Visibility::from_name("public_func"), Visibility::Public);
        assert_eq!(Visibility::from_name("MyClass.method"), Visibility::Public);

        // Dunder methods are considered public (special methods)
        assert_eq!(Visibility::from_name("__dunder__"), Visibility::Public);
        assert_eq!(Visibility::from_name("__init__"), Visibility::Public);

        // Private functions (single leading underscore, but not dunder)
        assert_eq!(Visibility::from_name("_private_func"), Visibility::Private);
        assert_eq!(
            Visibility::from_name("MyClass._private"),
            Visibility::Private
        );

        // Internal/name-mangled functions (double underscore, not dunder)
        assert_eq!(
            Visibility::from_name("__internal_func"),
            Visibility::Internal
        );
        assert_eq!(Visibility::from_name("__mangled"), Visibility::Internal);
    }

    #[test]
    fn test_dead_code_report_default() {
        let report = DeadCodeReport::default();
        assert_eq!(report.functions_analyzed, 0);
        assert_eq!(report.dead_count, 0);
        assert_eq!(report.dead_percentage, 0.0);
        assert!(report.dead_functions.is_empty());
    }

    /// T-W1: analyze_dead_code with refcount rescues referenced functions
    ///
    /// A function that is referenced (called) elsewhere in the file should NOT
    /// appear as dead when using refcount-based analysis.
    #[test]
    fn test_analyze_dead_code_refcount_rescues_called() {
        use std::fs;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let content = r#"
def helper():
    return 42

def main_func():
    return helper()
"#;
        fs::write(dir.path().join("example.py"), content).unwrap();

        let result = analyze_dead_code(dir.path(), Some(Language::Python), &[]);
        assert!(result.is_ok(), "analyze_dead_code should succeed");
        let report = result.unwrap();

        // "helper" is referenced by main_func, so refcount > 1 => not dead
        let dead_names: Vec<&str> = report
            .dead_functions
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        assert!(
            !dead_names.contains(&"helper"),
            "helper should NOT be dead (refcount > 1), but got dead_names: {:?}",
            dead_names
        );
    }

    /// T-W2: analyze_dead_code with refcount flags unreferenced private functions as dead
    #[test]
    fn test_analyze_dead_code_refcount_flags_unreferenced() {
        use std::fs;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let content = r#"
def _unused_helper():
    return 42

def main_func():
    return 99
"#;
        fs::write(dir.path().join("example.py"), content).unwrap();

        let result = analyze_dead_code(dir.path(), Some(Language::Python), &[]);
        assert!(result.is_ok(), "analyze_dead_code should succeed");
        let report = result.unwrap();

        // _unused_helper has refcount == 1 (only its definition), private => dead
        let dead_names: Vec<&str> = report
            .dead_functions
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        assert!(
            dead_names.contains(&"_unused_helper"),
            "_unused_helper should be dead (refcount == 1, private), got dead_names: {:?}",
            dead_names
        );
    }

    /// T-W3: analyze_dead_code_with_refcount accepts path + language and returns correct report
    #[test]
    fn test_analyze_dead_code_with_refcount_api() {
        use std::fs;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let content = r#"
def _orphan():
    return 1

def used_func():
    return 2

def caller():
    return used_func()
"#;
        fs::write(dir.path().join("mod.py"), content).unwrap();

        // Collect module infos (simulating health dashboard context)
        let module_infos = {
            let mut infos = Vec::new();
            for entry in crate::walker::walk_project(dir.path()) {
                if entry.path().extension().map(|e| e == "py").unwrap_or(false) {
                    if let Ok(info) =
                        crate::ast::extract::extract_file(entry.path(), Some(dir.path()))
                    {
                        infos.push((entry.path().to_path_buf(), info));
                    }
                }
            }
            infos
        };

        let result = analyze_dead_code_with_refcount(
            dir.path(),
            Language::Python,
            &module_infos,
            &["main", "test_"],
        );
        assert!(
            result.is_ok(),
            "analyze_dead_code_with_refcount should succeed"
        );
        let report = result.unwrap();

        // _orphan is private + unreferenced => dead
        let dead_names: Vec<&str> = report
            .dead_functions
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        assert!(
            dead_names.contains(&"_orphan"),
            "_orphan should be dead, got: {:?}",
            dead_names
        );

        // used_func is referenced by caller => not dead
        assert!(
            !dead_names.contains(&"used_func"),
            "used_func should NOT be dead (referenced), got: {:?}",
            dead_names
        );
    }

    /// T-W4: health.rs dead code path no longer requires call graph
    ///
    /// analyze_dead_code_with_refcount should work without a call graph,
    /// using only path, language, and module_infos.
    #[test]
    fn test_dead_code_with_refcount_no_cg_required() {
        use std::fs;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let content = "def _lonely():\n    pass\n";
        fs::write(dir.path().join("solo.py"), content).unwrap();

        let module_infos = {
            let mut infos = Vec::new();
            if let Ok(info) =
                crate::ast::extract::extract_file(&dir.path().join("solo.py"), Some(dir.path()))
            {
                infos.push((dir.path().join("solo.py"), info));
            }
            infos
        };

        // This function should NOT require a ProjectCallGraph parameter
        let result =
            analyze_dead_code_with_refcount(dir.path(), Language::Python, &module_infos, &[]);
        assert!(result.is_ok());
        let report = result.unwrap();
        assert!(report.functions_analyzed > 0);
    }
}
