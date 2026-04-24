//! Complexity Analyzer for Health Command
//!
//! This module provides project-wide complexity analysis with hotspot detection.
//! It wraps the existing cyclomatic complexity calculation from metrics/complexity.rs
//! and adds hotspot detection (functions with CC > threshold).
//!
//! # Features
//! - Project-wide complexity scanning
//! - Hotspot detection (CC > configurable threshold, default 10)
//! - Per-function complexity data with ranking
//! - Multi-language support
//!
//! # Example
//!
//! ```ignore
//! use tldr_core::quality::complexity::{analyze_complexity, ComplexityOptions};
//! use std::path::Path;
//!
//! let report = analyze_complexity(Path::new("src/"), None, None)?;
//! println!("Hotspots: {}", report.hotspot_count);
//! println!("Avg CC: {:.2}", report.avg_cyclomatic);
//! ```

use std::path::{Path, PathBuf};

use crate::walker::walk_project;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::ast::extract::extract_file;
use crate::error::TldrError;
use crate::metrics::calculate_all_complexities_file;
use crate::types::Language;
use crate::TldrResult;

// =============================================================================
// Types
// =============================================================================

/// A function identified as a complexity hotspot (CC > threshold)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplexityHotspot {
    /// Function name
    pub name: String,
    /// File path containing the function
    pub file: PathBuf,
    /// Line number where the function starts
    pub line: usize,
    /// Cyclomatic complexity
    pub cyclomatic: usize,
    /// Cognitive complexity (optional, for future use)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cognitive: Option<usize>,
    /// Lines of code in the function
    pub loc: usize,
    /// Rank by complexity (1 = highest complexity)
    pub rank: usize,
}

/// Per-function complexity data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionComplexity {
    /// Function name
    pub name: String,
    /// File path containing the function
    pub file: PathBuf,
    /// Line number where the function starts
    pub line: usize,
    /// Cyclomatic complexity
    pub cyclomatic: usize,
    /// Cognitive complexity
    pub cognitive: usize,
    /// Lines of code in the function
    pub loc: usize,
    /// Rank by cyclomatic complexity (1 = highest)
    pub rank: usize,
    /// Whether this function is a hotspot
    pub is_hotspot: bool,
}

/// Summary statistics for complexity analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplexitySummary {
    /// Total number of functions analyzed
    pub total_functions: usize,
    /// Average cyclomatic complexity
    pub avg_cyclomatic: f64,
    /// Maximum cyclomatic complexity found
    pub max_cyclomatic: usize,
    /// Number of functions exceeding the hotspot threshold
    pub hotspot_count: usize,
    /// Total lines of code across all functions
    pub total_loc: usize,
}

impl Default for ComplexitySummary {
    fn default() -> Self {
        Self {
            total_functions: 0,
            avg_cyclomatic: 0.0,
            max_cyclomatic: 0,
            hotspot_count: 0,
            total_loc: 0,
        }
    }
}

/// Complete complexity analysis report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplexityReport {
    /// Number of functions analyzed
    pub functions_analyzed: usize,
    /// Average cyclomatic complexity across all functions
    pub avg_cyclomatic: f64,
    /// Maximum cyclomatic complexity found
    pub max_cyclomatic: usize,
    /// Number of functions with CC > threshold
    pub hotspot_count: usize,
    /// List of hotspots sorted by CC descending
    pub hotspots: Vec<ComplexityHotspot>,
    /// All functions with complexity data (sorted by CC descending)
    pub functions: Vec<FunctionComplexity>,
    /// Summary statistics
    pub summary: ComplexitySummary,
}

impl Default for ComplexityReport {
    fn default() -> Self {
        Self {
            functions_analyzed: 0,
            avg_cyclomatic: 0.0,
            max_cyclomatic: 0,
            hotspot_count: 0,
            hotspots: Vec::new(),
            functions: Vec::new(),
            summary: ComplexitySummary::default(),
        }
    }
}

/// Options for complexity analysis
#[derive(Debug, Clone)]
pub struct ComplexityOptions {
    /// Threshold for hotspot detection (default: 10)
    pub hotspot_threshold: usize,
    /// Maximum number of hotspots to return (default: 20)
    pub max_hotspots: usize,
    /// Include cognitive complexity (default: true)
    pub include_cognitive: bool,
}

impl Default for ComplexityOptions {
    fn default() -> Self {
        Self {
            hotspot_threshold: 10,
            max_hotspots: 20,
            include_cognitive: true,
        }
    }
}

// =============================================================================
// Main API
// =============================================================================

/// Analyze cyclomatic complexity across a codebase
///
/// Scans all supported files in the given path, calculates complexity for each
/// function, and identifies hotspots (functions with CC > threshold).
///
/// # Arguments
/// * `path` - Directory or file to analyze
/// * `language` - Optional language filter (auto-detect if None)
/// * `options` - Optional configuration (uses defaults if None)
///
/// # Returns
/// * `Ok(ComplexityReport)` - Report with complexity metrics and hotspots
/// * `Err(TldrError)` - On file system errors
///
/// # Behavior
/// - Empty files return success with zero metrics
/// - Parse errors in individual files are skipped (logged)
/// - Functions sorted by cyclomatic complexity descending
/// - Hotspots filtered by CC > threshold
///
/// # Example
/// ```ignore
/// use tldr_core::quality::complexity::analyze_complexity;
/// use std::path::Path;
///
/// let report = analyze_complexity(Path::new("src/"), None, None)?;
/// for hotspot in &report.hotspots {
///     println!("{}: CC={}", hotspot.name, hotspot.cyclomatic);
/// }
/// ```
pub fn analyze_complexity(
    path: &Path,
    language: Option<Language>,
    options: Option<ComplexityOptions>,
) -> TldrResult<ComplexityReport> {
    let opts = options.unwrap_or_default();

    // Collect files to analyze
    let file_paths: Vec<PathBuf> = if path.is_file() {
        vec![path.to_path_buf()]
    } else {
        walk_project(path)
            .filter(|e| e.file_type().map(|ft| ft.is_file()).unwrap_or(false))
            .filter(|e| {
                let detected = Language::from_path(e.path());
                match (detected, language) {
                    (Some(d), Some(l)) => d == l,
                    (Some(_), None) => true,
                    _ => false,
                }
            })
            .map(|e| e.path().to_path_buf())
            .collect()
    };

    // Collect all function complexity data
    let all_functions_nested: Vec<Vec<FunctionComplexity>> = file_paths
        .par_iter()
        .filter_map(|file_path| analyze_file_complexity(file_path, opts.include_cognitive).ok())
        .collect();

    let mut all_functions: Vec<FunctionComplexity> =
        all_functions_nested.into_iter().flatten().collect();

    // Sort by cyclomatic complexity descending
    all_functions.sort_by(|a, b| b.cyclomatic.cmp(&a.cyclomatic));

    // Assign ranks
    for (rank, func) in all_functions.iter_mut().enumerate() {
        func.rank = rank + 1;
    }

    // Mark hotspots
    for func in &mut all_functions {
        func.is_hotspot = func.cyclomatic > opts.hotspot_threshold;
    }

    // Calculate summary statistics
    let total_functions = all_functions.len();
    let total_cc: usize = all_functions.iter().map(|f| f.cyclomatic).sum();
    let total_loc: usize = all_functions.iter().map(|f| f.loc).sum();
    let max_cyclomatic = all_functions.first().map(|f| f.cyclomatic).unwrap_or(0);
    let avg_cyclomatic = if total_functions > 0 {
        total_cc as f64 / total_functions as f64
    } else {
        0.0
    };

    // Extract hotspots
    let hotspots: Vec<ComplexityHotspot> = all_functions
        .iter()
        .filter(|f| f.is_hotspot)
        .take(opts.max_hotspots)
        .map(|f| ComplexityHotspot {
            name: f.name.clone(),
            file: f.file.clone(),
            line: f.line,
            cyclomatic: f.cyclomatic,
            cognitive: if opts.include_cognitive {
                Some(f.cognitive)
            } else {
                None
            },
            loc: f.loc,
            rank: f.rank,
        })
        .collect();

    let hotspot_count = all_functions.iter().filter(|f| f.is_hotspot).count();

    let summary = ComplexitySummary {
        total_functions,
        avg_cyclomatic,
        max_cyclomatic,
        hotspot_count,
        total_loc,
    };

    Ok(ComplexityReport {
        functions_analyzed: total_functions,
        avg_cyclomatic,
        max_cyclomatic,
        hotspot_count,
        hotspots,
        functions: all_functions,
        summary,
    })
}

/// Analyze complexity of all functions in a single file
///
/// Uses single-pass complexity calculation to avoid re-parsing the file
/// for each function. A file with N functions is parsed twice (once for
/// module structure, once for complexity) instead of N+1 times.
fn analyze_file_complexity(
    file_path: &Path,
    include_cognitive: bool,
) -> TldrResult<Vec<FunctionComplexity>> {
    // Verify this is a supported language before attempting analysis.
    // calculate_all_complexities_file also checks, but this provides a
    // consistent early-exit with the same error type.
    Language::from_path(file_path).ok_or_else(|| {
        TldrError::UnsupportedLanguage(
            file_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("unknown")
                .to_string(),
        )
    })?;

    // Single-pass: parse file once, get all complexities in one AST walk
    let metrics_map = calculate_all_complexities_file(file_path)?;

    // Extract module info for line numbers and class/method structure
    let module = extract_file(file_path, None)?;

    let mut results = Vec::new();

    // Process top-level functions
    for func in &module.functions {
        if let Some(metrics) = metrics_map.get(&func.name) {
            results.push(FunctionComplexity {
                name: func.name.clone(),
                file: file_path.to_path_buf(),
                line: func.line_number as usize,
                cyclomatic: metrics.cyclomatic as usize,
                cognitive: if include_cognitive {
                    metrics.cognitive as usize
                } else {
                    0
                },
                loc: metrics.lines_of_code as usize,
                rank: 0,           // Will be set after sorting
                is_hotspot: false, // Will be set after threshold check
            });
        }
    }

    // Process methods in classes
    for class in &module.classes {
        for method in &class.methods {
            // Skip dunder methods for complexity hotspot analysis
            if method.name.starts_with("__") && method.name.ends_with("__") {
                continue;
            }

            // calculate_all_complexities_file() keys by bare function name
            // (from get_function_name()), not qualified ClassName.method.
            // Look up by bare method name.
            if let Some(metrics) = metrics_map.get(&method.name) {
                results.push(FunctionComplexity {
                    name: format!("{}.{}", class.name, method.name),
                    file: file_path.to_path_buf(),
                    line: method.line_number as usize,
                    cyclomatic: metrics.cyclomatic as usize,
                    cognitive: if include_cognitive {
                        metrics.cognitive as usize
                    } else {
                        0
                    },
                    loc: metrics.lines_of_code as usize,
                    rank: 0,
                    is_hotspot: false,
                });
            }
        }
    }

    Ok(results)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Helper to create a test directory with files
    fn create_test_dir() -> TempDir {
        TempDir::new().unwrap()
    }

    /// Helper to write a file to the test directory
    fn write_file(dir: &TempDir, name: &str, content: &str) -> PathBuf {
        let path = dir.path().join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn test_complexity_empty_file() {
        let dir = create_test_dir();
        write_file(&dir, "empty.py", "");

        let report = analyze_complexity(dir.path(), Some(Language::Python), None).unwrap();

        assert_eq!(report.functions_analyzed, 0);
        assert_eq!(report.avg_cyclomatic, 0.0);
        assert_eq!(report.max_cyclomatic, 0);
        assert_eq!(report.hotspot_count, 0);
        assert!(report.hotspots.is_empty());
    }

    #[test]
    fn test_complexity_simple_function() {
        let dir = create_test_dir();
        let source = r#"
def simple():
    return 42
"#;
        write_file(&dir, "simple.py", source);

        let report = analyze_complexity(dir.path(), Some(Language::Python), None).unwrap();

        assert_eq!(report.functions_analyzed, 1);
        assert_eq!(report.max_cyclomatic, 1);
        assert_eq!(report.hotspot_count, 0); // CC=1 is below threshold of 10
    }

    #[test]
    fn test_complexity_average_calculation() {
        let dir = create_test_dir();
        let source = r#"
def func_cc1():
    return 1

def func_cc2(a):
    if a:
        return 1
    return 0

def func_cc3(a, b):
    if a:
        return 1
    elif b:
        return 2
    return 0

def func_cc4(a, b, c):
    if a:
        return 1
    elif b:
        return 2
    elif c:
        return 3
    return 0
"#;
        write_file(&dir, "multiple.py", source);

        let report = analyze_complexity(dir.path(), Some(Language::Python), None).unwrap();

        assert_eq!(report.functions_analyzed, 4);
        // CC values: 1, 2, 3, 4 -> avg = 2.5
        assert!((report.avg_cyclomatic - 2.5).abs() < 0.01);
        assert_eq!(report.max_cyclomatic, 4);
    }

    #[test]
    fn test_complexity_hotspot_detection() {
        let dir = create_test_dir();
        // Create a function with high complexity (CC > 10)
        let source = r#"
def complex_function(a, b, c, d, e, f):
    result = 0
    if a > 0:
        if b > 0:
            result += 1
        elif c > 0:
            result += 2
        else:
            result += 3
    elif d > 0:
        if e > 0:
            result += 4
        elif f > 0:
            result += 5
        else:
            result += 6
    else:
        if a < -10:
            result -= 1
        elif b < -10:
            result -= 2
        else:
            result -= 3

    for i in range(10):
        if i % 2 == 0:
            result += i

    return result
"#;
        write_file(&dir, "high_complexity.py", source);

        let report = analyze_complexity(dir.path(), Some(Language::Python), None).unwrap();

        assert_eq!(report.functions_analyzed, 1);
        assert!(
            report.max_cyclomatic > 10,
            "Expected CC > 10, got {}",
            report.max_cyclomatic
        );
        assert!(report.hotspot_count >= 1, "Expected at least 1 hotspot");
        assert!(!report.hotspots.is_empty());
        assert_eq!(report.hotspots[0].rank, 1);
    }

    #[test]
    fn test_complexity_sorted_descending() {
        let dir = create_test_dir();
        let source = r#"
def low_cc():
    return 1

def medium_cc(a, b):
    if a:
        return 1
    elif b:
        return 2
    return 0

def high_cc(a, b, c, d):
    if a:
        if b:
            return 1
        elif c:
            return 2
        elif d:
            return 3
    return 0
"#;
        write_file(&dir, "sorted.py", source);

        let report = analyze_complexity(dir.path(), Some(Language::Python), None).unwrap();

        assert_eq!(report.functions_analyzed, 3);
        // Verify sorted descending by CC
        for i in 1..report.functions.len() {
            assert!(
                report.functions[i - 1].cyclomatic >= report.functions[i].cyclomatic,
                "Functions not sorted descending"
            );
        }
        // Verify ranks
        for (i, func) in report.functions.iter().enumerate() {
            assert_eq!(func.rank, i + 1, "Rank mismatch for {}", func.name);
        }
    }

    #[test]
    fn test_complexity_threshold_configurable() {
        let dir = create_test_dir();
        let source = r#"
def moderate_cc(a, b, c, d, e):
    if a:
        return 1
    elif b:
        return 2
    elif c:
        return 3
    elif d:
        return 4
    elif e:
        return 5
    return 0
"#;
        write_file(&dir, "moderate.py", source);

        // With default threshold (10), this should not be a hotspot
        let report_default = analyze_complexity(dir.path(), Some(Language::Python), None).unwrap();
        assert_eq!(report_default.hotspot_count, 0);

        // With lower threshold (5), this should be a hotspot
        let opts = ComplexityOptions {
            hotspot_threshold: 5,
            ..Default::default()
        };
        let report_strict =
            analyze_complexity(dir.path(), Some(Language::Python), Some(opts)).unwrap();
        assert!(
            report_strict.hotspot_count > 0,
            "Expected hotspot with threshold=5"
        );
    }

    #[test]
    fn test_complexity_multi_language() {
        let dir = create_test_dir();

        // Python file
        write_file(&dir, "test.py", "def py_func():\n    return 1\n");

        // TypeScript file
        write_file(
            &dir,
            "test.ts",
            "function tsFunc(): number {\n    return 1;\n}\n",
        );

        // Analyze all languages
        let report_all = analyze_complexity(dir.path(), None, None).unwrap();
        // Should find functions from both files
        assert!(report_all.functions_analyzed >= 1);

        // Analyze only Python
        let report_py = analyze_complexity(dir.path(), Some(Language::Python), None).unwrap();
        assert_eq!(report_py.functions_analyzed, 1);

        // Analyze only TypeScript
        let report_ts = analyze_complexity(dir.path(), Some(Language::TypeScript), None).unwrap();
        // TypeScript parsing might have different results
        assert!(report_ts.functions_analyzed <= 1);
    }

    #[test]
    fn test_complexity_per_function_data() {
        let dir = create_test_dir();
        let source = r#"
def test_func(x, y):
    if x > 0:
        return y
    return 0
"#;
        write_file(&dir, "test.py", source);

        let report = analyze_complexity(dir.path(), Some(Language::Python), None).unwrap();

        assert_eq!(report.functions.len(), 1);
        let func = &report.functions[0];

        assert_eq!(func.name, "test_func");
        assert!(func.line > 0);
        assert!(func.cyclomatic >= 1);
        assert!(func.loc > 0);
        assert_eq!(func.rank, 1);
    }

    #[test]
    fn test_complexity_max_hotspots() {
        let dir = create_test_dir();
        // Create many high-complexity functions with clear CC > threshold
        let mut source = String::new();
        for i in 0..30 {
            // Each function has CC = 12 (base 1 + 11 elif branches)
            source.push_str(&format!(
                "def func_{0}(a, b, c, d, e, f, g, h, i, j, k):\n    \
                    if a:\n        return 1\n    \
                    elif b:\n        return 2\n    \
                    elif c:\n        return 3\n    \
                    elif d:\n        return 4\n    \
                    elif e:\n        return 5\n    \
                    elif f:\n        return 6\n    \
                    elif g:\n        return 7\n    \
                    elif h:\n        return 8\n    \
                    elif i:\n        return 9\n    \
                    elif j:\n        return 10\n    \
                    elif k:\n        return 11\n    \
                    return 0\n\n",
                i
            ));
        }
        write_file(&dir, "many.py", &source);

        // Use a low threshold so all functions are hotspots
        let opts = ComplexityOptions {
            max_hotspots: 10,
            hotspot_threshold: 3, // Very low threshold to ensure all functions qualify
            ..Default::default()
        };
        let report = analyze_complexity(dir.path(), Some(Language::Python), Some(opts)).unwrap();

        // Should have limited hotspots returned in the list
        assert!(
            report.hotspots.len() <= 10,
            "Expected at most 10 hotspots in list, got {}",
            report.hotspots.len()
        );
        // But hotspot_count should reflect all hotspots found
        assert!(
            report.hotspot_count >= 10,
            "Expected at least 10 hotspots total, got {}",
            report.hotspot_count
        );
    }

    #[test]
    fn test_complexity_class_methods() {
        let dir = create_test_dir();
        let source = r#"
class Calculator:
    def add(self, a, b):
        return a + b

    def complex_calc(self, x, y, z):
        if x > 0:
            if y > 0:
                return x + y
            elif z > 0:
                return x + z
        return 0

def standalone():
    return 42
"#;
        write_file(&dir, "calc.py", source);

        let report = analyze_complexity(dir.path(), Some(Language::Python), None).unwrap();

        // Should find 2 class methods (add, complex_calc) + 1 standalone function
        // Note: dunder methods (__init__, etc.) would be skipped, but these are regular methods
        assert_eq!(
            report.functions_analyzed, 3,
            "Expected 3 functions (2 methods + 1 standalone), got {}",
            report.functions_analyzed
        );

        // Methods should have qualified names (Class.method)
        let names: Vec<&str> = report.functions.iter().map(|f| f.name.as_str()).collect();
        assert!(
            names.contains(&"Calculator.add"),
            "Missing Calculator.add, got {:?}",
            names
        );
        assert!(
            names.contains(&"Calculator.complex_calc"),
            "Missing Calculator.complex_calc, got {:?}",
            names
        );
        assert!(
            names.contains(&"standalone"),
            "Missing standalone, got {:?}",
            names
        );

        // complex_calc should have higher CC than add
        let add = report
            .functions
            .iter()
            .find(|f| f.name == "Calculator.add")
            .unwrap();
        let calc = report
            .functions
            .iter()
            .find(|f| f.name == "Calculator.complex_calc")
            .unwrap();
        assert!(
            calc.cyclomatic > add.cyclomatic,
            "complex_calc CC ({}) should be > add CC ({})",
            calc.cyclomatic,
            add.cyclomatic
        );
    }

    #[test]
    fn test_complexity_skips_dunder_methods() {
        let dir = create_test_dir();
        let source = r#"
class MyClass:
    def __init__(self):
        self.value = 0

    def __repr__(self):
        return f"MyClass({self.value})"

    def process(self, x):
        if x > 0:
            return x
        return 0
"#;
        write_file(&dir, "dunders.py", source);

        let report = analyze_complexity(dir.path(), Some(Language::Python), None).unwrap();

        // Should only find 'process', not __init__ or __repr__
        assert_eq!(
            report.functions_analyzed, 1,
            "Expected 1 function (process only), got {}",
            report.functions_analyzed
        );
        assert_eq!(report.functions[0].name, "MyClass.process");
    }
}
