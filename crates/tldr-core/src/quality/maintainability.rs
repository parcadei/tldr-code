//! Maintainability Index calculation
//!
//! Implements Maintainability Index as per spec Section 2.8.2:
//! MI = max(0, (171 - 5.2*ln(V) - 0.23*G - 16.2*ln(LOC)) * 100/171)
//!
//! Where:
//! - V = Halstead Volume
//! - G = Cyclomatic Complexity
//! - LOC = Lines of Code
//!
//! # Grading
//! - A: MI > 85 (highly maintainable)
//! - B: 65 < MI <= 85 (moderately maintainable)
//! - C: 45 < MI <= 65 (difficult to maintain)
//! - D: 25 < MI <= 45 (very difficult to maintain)
//! - F: MI <= 25 (unmaintainable)
//!
//! # References
//! - Oman, P. and Hagemeister, J. (1992). "Metrics for Assessing a Software System's Maintainability"
//! - Microsoft Visual Studio Maintainability Index documentation

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::walker::walk_project;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::ast::parser::parse;
use crate::error::TldrError;
use crate::metrics::calculate_all_complexities_file;
use crate::types::Language;
use crate::TldrResult;

// =============================================================================
// Types
// =============================================================================

/// Halstead software metrics
///
/// Based on distinct operators/operands and their total occurrences:
/// - n1 = number of distinct operators
/// - n2 = number of distinct operands
/// - N1 = total number of operators
/// - N2 = total number of operands
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HalsteadMetrics {
    /// Number of distinct operators (n1)
    pub distinct_operators: usize,
    /// Number of distinct operands (n2)
    pub distinct_operands: usize,
    /// Total number of operators (N1)
    pub total_operators: usize,
    /// Total number of operands (N2)
    pub total_operands: usize,
    /// Program vocabulary: n = n1 + n2
    pub vocabulary: usize,
    /// Program length: N = N1 + N2
    pub length: usize,
    /// Calculated program length: N^ = n1*log2(n1) + n2*log2(n2)
    pub calculated_length: f64,
    /// Volume: V = N * log2(n)
    pub volume: f64,
    /// Difficulty: D = (n1/2) * (N2/n2)
    pub difficulty: f64,
    /// Effort: E = D * V
    pub effort: f64,
    /// Time to program: T = E / 18 seconds
    pub time_to_program: f64,
    /// Number of delivered bugs: B = V / 3000
    pub bugs: f64,
}

impl Default for HalsteadMetrics {
    fn default() -> Self {
        Self {
            distinct_operators: 0,
            distinct_operands: 0,
            total_operators: 0,
            total_operands: 0,
            vocabulary: 0,
            length: 0,
            calculated_length: 0.0,
            volume: 1.0, // Avoid log(0) issues
            difficulty: 0.0,
            effort: 0.0,
            time_to_program: 0.0,
            bugs: 0.0,
        }
    }
}

impl HalsteadMetrics {
    /// Calculate all derived metrics from counts
    pub fn calculate(&mut self) {
        self.vocabulary = self.distinct_operators + self.distinct_operands;
        self.length = self.total_operators + self.total_operands;

        // Calculated length (Halstead's estimate)
        self.calculated_length = if self.distinct_operators > 0 && self.distinct_operands > 0 {
            self.distinct_operators as f64 * (self.distinct_operators as f64).log2()
                + self.distinct_operands as f64 * (self.distinct_operands as f64).log2()
        } else {
            0.0
        };

        // Volume
        self.volume = if self.vocabulary > 0 && self.length > 0 {
            self.length as f64 * (self.vocabulary as f64).log2()
        } else {
            1.0 // Minimum volume to avoid log(0)
        };

        // Difficulty
        self.difficulty = if self.distinct_operands > 0 {
            (self.distinct_operators as f64 / 2.0)
                * (self.total_operands as f64 / self.distinct_operands as f64)
        } else {
            0.0
        };

        // Effort
        self.effort = self.difficulty * self.volume;

        // Time to program (Halstead's estimate: E/18 seconds)
        self.time_to_program = self.effort / 18.0;

        // Number of delivered bugs (V/3000)
        self.bugs = self.volume / 3000.0;
    }
}

/// Maintainability Index for a single file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMI {
    /// File path (relative to project root)
    pub path: PathBuf,
    /// Maintainability Index (0-100)
    pub mi: f64,
    /// Letter grade (A-F)
    pub grade: char,
    /// Lines of code
    pub loc: usize,
    /// Average cyclomatic complexity
    pub avg_complexity: f64,
    /// Halstead metrics (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub halstead: Option<HalsteadMetrics>,
}

/// Summary of maintainability across files
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MISummary {
    /// Average MI across all files
    pub average_mi: f64,
    /// Minimum MI found
    pub min_mi: f64,
    /// Maximum MI found
    pub max_mi: f64,
    /// Number of files analyzed
    pub files_analyzed: usize,
    /// Distribution by grade
    pub by_grade: HashMap<char, usize>,
    /// Total lines of code
    pub total_loc: usize,
}

/// Report from maintainability analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaintainabilityReport {
    /// Per-file maintainability metrics
    pub files: Vec<FileMI>,
    /// Summary statistics
    pub summary: MISummary,
}

// =============================================================================
// Main API
// =============================================================================

/// Calculate Maintainability Index for a file or directory
///
/// Uses the Microsoft formula:
/// MI = max(0, (171 - 5.2*ln(V) - 0.23*G - 16.2*ln(LOC)) * 100/171)
///
/// # Arguments
/// * `path` - File or directory to analyze
/// * `include_halstead` - Whether to include detailed Halstead metrics
/// * `language` - Optional language filter
///
/// # Returns
/// * `Ok(MaintainabilityReport)` - Report with MI scores
/// * `Err(TldrError)` - On file system or parse errors
///
/// # Example
/// ```ignore
/// use tldr_core::quality::maintainability::maintainability_index;
///
/// let report = maintainability_index(Path::new("src/"), true, None)?;
/// println!("Average MI: {:.1}", report.summary.average_mi);
/// ```
pub fn maintainability_index(
    path: &Path,
    include_halstead: bool,
    language: Option<Language>,
) -> TldrResult<MaintainabilityReport> {
    // Max file size to analyze (500KB) - skip minified/generated files
    const MAX_FILE_SIZE: u64 = 500 * 1024;

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
            .filter(|e| {
                e.metadata()
                    .map(|m| m.len() <= MAX_FILE_SIZE)
                    .unwrap_or(true)
            })
            .map(|e| e.path().to_path_buf())
            .collect()
    };

    // Analyze files in parallel using rayon
    let files: Vec<FileMI> = file_paths
        .par_iter()
        .filter_map(|file_path| analyze_file_mi(file_path, include_halstead).ok())
        .collect();

    // Calculate summary
    let summary = calculate_summary(&files);

    Ok(MaintainabilityReport { files, summary })
}

// =============================================================================
// Internal Implementation
// =============================================================================

/// Analyze maintainability of a single file
fn analyze_file_mi(path: &Path, include_halstead: bool) -> TldrResult<FileMI> {
    // Read and parse file
    let source = std::fs::read_to_string(path)?;
    let language = Language::from_path(path).ok_or_else(|| {
        TldrError::UnsupportedLanguage(
            path.extension()
                .and_then(|e| e.to_str())
                .unwrap_or("unknown")
                .to_string(),
        )
    })?;

    // Count lines of code (non-blank, non-comment)
    let loc = count_loc(&source, language);

    // Calculate Halstead metrics
    let halstead = if include_halstead {
        Some(calculate_halstead(&source, language))
    } else {
        None
    };

    // Get average cyclomatic complexity
    let avg_complexity = calculate_avg_complexity(path, language);

    // Calculate MI using Halstead volume or estimate
    let volume = halstead
        .as_ref()
        .map(|h| h.volume)
        .unwrap_or_else(|| estimate_volume(loc));

    let mi = calculate_mi(volume, avg_complexity, loc);
    let grade = mi_to_grade(mi);

    Ok(FileMI {
        path: path.to_path_buf(),
        mi,
        grade,
        loc,
        avg_complexity,
        halstead,
    })
}

/// Calculate Maintainability Index using the Microsoft formula
/// MI = max(0, (171 - 5.2*ln(V) - 0.23*G - 16.2*ln(LOC)) * 100/171)
fn calculate_mi(volume: f64, complexity: f64, loc: usize) -> f64 {
    if loc == 0 {
        return 100.0; // Empty file is perfectly maintainable
    }

    let v_ln = if volume > 0.0 { volume.ln() } else { 0.0 };
    let loc_ln = (loc as f64).ln();

    let raw_mi = 171.0 - 5.2 * v_ln - 0.23 * complexity - 16.2 * loc_ln;
    let normalized = (raw_mi * 100.0) / 171.0;

    normalized.clamp(0.0, 100.0)
}

/// Convert MI to letter grade
fn mi_to_grade(mi: f64) -> char {
    if mi > 85.0 {
        'A'
    } else if mi > 65.0 {
        'B'
    } else if mi > 45.0 {
        'C'
    } else if mi > 25.0 {
        'D'
    } else {
        'F'
    }
}

/// Estimate volume when Halstead metrics not computed
fn estimate_volume(loc: usize) -> f64 {
    // Rough estimate: average of 5 tokens per line, vocabulary of sqrt(loc)
    let n = loc * 5;
    let vocab = (loc as f64).sqrt().max(1.0);
    n as f64 * vocab.log2()
}

/// Count non-blank, non-comment lines of code
fn count_loc(source: &str, language: Language) -> usize {
    let comment_prefixes = match language {
        Language::Python => vec!["#"],
        Language::TypeScript
        | Language::JavaScript
        | Language::Go
        | Language::Rust
        | Language::Java
        | Language::C
        | Language::Cpp
        | Language::Kotlin
        | Language::Swift
        | Language::CSharp
        | Language::Scala
        | Language::Php => vec!["//", "/*", "*"],
        Language::Ruby | Language::Elixir => vec!["#"],
        Language::Ocaml => vec!["(*", "*"],
        Language::Lua | Language::Luau => vec!["--"],
    };

    source
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !comment_prefixes.iter().any(|p| trimmed.starts_with(p))
        })
        .count()
}

/// Calculate average cyclomatic complexity for all functions in file.
///
/// Uses batch complexity calculation to parse the file only once,
/// instead of re-parsing per function (10-25x faster on large files).
fn calculate_avg_complexity(path: &Path, _language: Language) -> f64 {
    if let Ok(complexity_map) = calculate_all_complexities_file(path) {
        if complexity_map.is_empty() {
            return 1.0; // Default complexity for files with no functions
        }
        let total: u32 = complexity_map.values().map(|m| m.cyclomatic).sum();
        total as f64 / complexity_map.len() as f64
    } else {
        1.0
    }
}

/// Calculate Halstead metrics for a source file
fn calculate_halstead(source: &str, language: Language) -> HalsteadMetrics {
    let mut operators: HashSet<String> = HashSet::new();
    let mut operands: HashSet<String> = HashSet::new();
    let mut total_operators = 0usize;
    let mut total_operands = 0usize;

    // Parse with tree-sitter
    let Ok(tree) = parse(source, language) else {
        return HalsteadMetrics::default();
    };

    // Walk the tree and categorize tokens
    let mut cursor = tree.root_node().walk();
    let mut stack = vec![tree.root_node()];

    while let Some(node) = stack.pop() {
        let kind = node.kind();
        let text = node.utf8_text(source.as_bytes()).unwrap_or("");

        // Classify as operator or operand based on node kind
        if is_operator(kind, language) {
            operators.insert(text.to_string());
            total_operators += 1;
        } else if is_operand(kind, language) {
            operands.insert(text.to_string());
            total_operands += 1;
        }

        // Push children
        cursor.reset(node);
        if cursor.goto_first_child() {
            loop {
                stack.push(cursor.node());
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
    }

    let mut halstead = HalsteadMetrics {
        distinct_operators: operators.len(),
        distinct_operands: operands.len(),
        total_operators,
        total_operands,
        ..Default::default()
    };

    halstead.calculate();
    halstead
}

/// Check if a node kind represents an operator
fn is_operator(kind: &str, _language: Language) -> bool {
    matches!(
        kind,
        "+" | "-"
            | "*"
            | "/"
            | "%"
            | "**"
            | "//"
            | "=="
            | "!="
            | "<"
            | ">"
            | "<="
            | ">="
            | "="
            | "+="
            | "-="
            | "*="
            | "/="
            | "and"
            | "or"
            | "not"
            | "&&"
            | "||"
            | "!"
            | "if"
            | "else"
            | "elif"
            | "for"
            | "while"
            | "return"
            | "try"
            | "except"
            | "catch"
            | "finally"
            | "def"
            | "class"
            | "function"
            | "fn"
            | "func"
            | "import"
            | "from"
            | "use"
            | "require"
            | "("
            | ")"
            | "["
            | "]"
            | "{"
            | "}"
            | "."
            | ","
            | ":"
            | ";"
            | "->"
            | "binary_operator"
            | "unary_operator"
            | "comparison_operator"
            | "boolean_operator"
            | "assignment"
    )
}

/// Check if a node kind represents an operand
fn is_operand(kind: &str, _language: Language) -> bool {
    matches!(
        kind,
        "identifier"
            | "string"
            | "integer"
            | "float"
            | "number"
            | "true"
            | "false"
            | "none"
            | "null"
            | "nil"
            | "string_literal"
            | "integer_literal"
            | "float_literal"
            | "property_identifier"
            | "field_identifier"
    )
}

/// Calculate summary statistics
fn calculate_summary(files: &[FileMI]) -> MISummary {
    if files.is_empty() {
        return MISummary {
            average_mi: 0.0,
            min_mi: 0.0,
            max_mi: 0.0,
            files_analyzed: 0,
            by_grade: HashMap::new(),
            total_loc: 0,
        };
    }

    let total_mi: f64 = files.iter().map(|f| f.mi).sum();
    let min_mi = files.iter().map(|f| f.mi).fold(f64::INFINITY, f64::min);
    let max_mi = files.iter().map(|f| f.mi).fold(f64::NEG_INFINITY, f64::max);
    let total_loc: usize = files.iter().map(|f| f.loc).sum();

    let mut by_grade: HashMap<char, usize> = HashMap::new();
    for file in files {
        *by_grade.entry(file.grade).or_insert(0) += 1;
    }

    MISummary {
        average_mi: total_mi / files.len() as f64,
        min_mi,
        max_mi,
        files_analyzed: files.len(),
        by_grade,
        total_loc,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mi_calculation_simple() {
        // Simple file: low volume, low complexity, few lines
        let mi = calculate_mi(100.0, 2.0, 20);
        assert!(mi > 50.0, "Simple code should be maintainable");
    }

    #[test]
    fn test_mi_calculation_complex() {
        // Complex file: high volume, high complexity, many lines
        let mi = calculate_mi(10000.0, 50.0, 1000);
        assert!(mi < 50.0, "Complex code should be less maintainable");
    }

    #[test]
    fn test_mi_grades() {
        assert_eq!(mi_to_grade(90.0), 'A');
        assert_eq!(mi_to_grade(75.0), 'B');
        assert_eq!(mi_to_grade(55.0), 'C');
        assert_eq!(mi_to_grade(35.0), 'D');
        assert_eq!(mi_to_grade(15.0), 'F');
    }

    #[test]
    fn test_halstead_calculation() {
        let mut h = HalsteadMetrics {
            distinct_operators: 10,
            distinct_operands: 20,
            total_operators: 50,
            total_operands: 100,
            ..Default::default()
        };
        h.calculate();

        assert_eq!(h.vocabulary, 30);
        assert_eq!(h.length, 150);
        assert!(h.volume > 0.0);
        assert!(h.difficulty > 0.0);
        assert!(h.effort > 0.0);
    }

    #[test]
    fn test_count_loc_python() {
        let source = r#"
# Comment
def foo():
    pass

# Another comment
"#;
        let loc = count_loc(source, Language::Python);
        assert_eq!(loc, 2); // def foo(): and pass
    }

    #[test]
    fn test_empty_file_mi() {
        let mi = calculate_mi(1.0, 1.0, 0);
        assert_eq!(mi, 100.0);
    }
}
