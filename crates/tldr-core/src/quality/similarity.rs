//! Similarity Analyzer for Health Command
//!
//! This module provides function similarity detection to identify potential code
//! clones. It uses structural comparison with weighted scoring to find functions
//! that may be candidates for refactoring into shared abstractions.
//!
//! # Algorithm
//!
//! 1. Extract all functions from the project (up to max_functions limit)
//! 2. For each function pair, compute similarity score (O(n^2) comparisons)
//! 3. Filter pairs with score >= threshold
//! 4. Return pairs sorted by score descending
//!
//! # Similarity Calculation (weighted)
//!
//! ```text
//! score = 0.3 * signature_similarity
//!       + 0.2 * complexity_similarity
//!       + 0.3 * call_pattern_similarity
//!       + 0.2 * loc_similarity
//! ```
//!
//! # Performance
//!
//! - Uses rayon for parallel O(n^2) comparisons (T12 mitigation)
//! - Limited to max_functions (default: 500) to prevent explosion
//! - 500 functions = 124,750 comparisons
//!
//! # References
//!
//! - Health spec section 4.6
//! - Premortem T12: Use rayon for parallelization
//! - Premortem T17: Validate weights sum to 1.0

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::walker::walk_project;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::ast::extract::extract_file;
use crate::types::{FunctionInfo, Language};
use crate::TldrResult;

// =============================================================================
// Constants - Similarity Weights (must sum to 1.0)
// =============================================================================

/// Weight for signature similarity (param count, return type)
const SIGNATURE_WEIGHT: f64 = 0.3;

/// Weight for complexity similarity (cyclomatic complexity)
const COMPLEXITY_WEIGHT: f64 = 0.2;

/// Weight for call pattern similarity (set of callees)
const CALL_PATTERN_WEIGHT: f64 = 0.3;

/// Weight for LOC similarity (lines of code)
const LOC_WEIGHT: f64 = 0.2;

// Compile-time assertion that weights sum to 1.0
const _: () = {
    let sum = SIGNATURE_WEIGHT + COMPLEXITY_WEIGHT + CALL_PATTERN_WEIGHT + LOC_WEIGHT;
    // Allow small floating point error
    assert!((sum - 1.0).abs() < 0.0001);
};

// =============================================================================
// Types
// =============================================================================

/// Reason why two functions are considered similar
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SimilarityReason {
    /// Same number of parameters
    SameSignature,
    /// Similar cyclomatic complexity
    SimilarComplexity,
    /// Similar call pattern (same callees)
    SimilarCallPattern,
    /// Similar lines of code
    SimilarLoc,
}

impl SimilarityReason {
    /// Get human-readable description
    pub fn description(&self) -> &'static str {
        match self {
            SimilarityReason::SameSignature => "same parameter count",
            SimilarityReason::SimilarComplexity => "similar complexity",
            SimilarityReason::SimilarCallPattern => "similar call pattern",
            SimilarityReason::SimilarLoc => "similar lines of code",
        }
    }
}

/// Reference to a function with its location
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionRef {
    /// Function name
    pub name: String,
    /// File containing the function
    pub file: PathBuf,
    /// Line number where function starts
    pub line: usize,
}

/// A pair of similar functions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimilarPair {
    /// First function
    pub func_a: FunctionRef,
    /// Second function
    pub func_b: FunctionRef,
    /// Similarity score (0.0 - 1.0)
    pub score: f64,
    /// Reasons why functions are similar
    pub reasons: Vec<SimilarityReason>,
}

/// Complete similarity analysis report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimilarityReport {
    /// Number of functions analyzed
    pub functions_analyzed: usize,
    /// Number of pairs compared (n*(n-1)/2)
    pub pairs_compared: usize,
    /// Number of similar pairs found (above threshold)
    pub similar_pairs_count: usize,
    /// Similarity threshold used
    pub threshold: f64,
    /// Similar function pairs (sorted by score descending)
    pub similar_pairs: Vec<SimilarPair>,
    /// Whether the results were truncated due to max_pairs limit
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    /// Total number of similar pairs before truncation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_pairs: Option<usize>,
    /// Number of pairs shown (after truncation)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shown_pairs: Option<usize>,
}

impl Default for SimilarityReport {
    fn default() -> Self {
        Self {
            functions_analyzed: 0,
            pairs_compared: 0,
            similar_pairs_count: 0,
            threshold: 0.7,
            similar_pairs: Vec::new(),
            truncated: None,
            total_pairs: None,
            shown_pairs: None,
        }
    }
}

/// Options for similarity analysis
#[derive(Debug, Clone)]
pub struct SimilarityOptions {
    /// Minimum similarity score to report (default: 0.7)
    pub threshold: f64,
    /// Maximum number of functions to compare (default: 500)
    pub max_functions: usize,
    /// Maximum number of similar pairs to return (default: 50)
    pub max_pairs: usize,
}

impl Default for SimilarityOptions {
    fn default() -> Self {
        Self {
            threshold: 0.7,
            max_functions: 500,
            max_pairs: 50,
        }
    }
}

// =============================================================================
// Internal Types for Analysis
// =============================================================================

/// Extracted function data for similarity comparison
#[derive(Debug, Clone)]
struct FunctionData {
    /// Function reference
    func_ref: FunctionRef,
    /// Number of parameters
    param_count: usize,
    /// Has return type annotation
    has_return_type: bool,
    /// Cyclomatic complexity
    complexity: usize,
    /// Lines of code (approximate)
    loc: usize,
    /// Set of callee names
    callees: HashSet<String>,
}

// =============================================================================
// Main API
// =============================================================================

/// Find structurally similar function pairs
///
/// Detects potential code clones using weighted structural comparison.
///
/// # Arguments
/// * `path` - Directory to analyze
/// * `language` - Optional language filter (auto-detect if None)
/// * `threshold` - Minimum similarity score (default: 0.7)
/// * `max_functions` - Maximum functions to compare (default: 500)
///
/// # Returns
/// * `Ok(SimilarityReport)` - Report with similar function pairs
/// * `Err(TldrError)` - On file system errors
///
/// # Performance
///
/// O(n^2) comparisons parallelized with rayon. Limited to max_functions
/// to prevent explosion for large codebases.
///
/// # Example
/// ```ignore
/// use tldr_core::quality::similarity::find_similar;
/// use std::path::Path;
///
/// let report = find_similar(Path::new("src/"), None, 0.7, Some(500))?;
/// for pair in &report.similar_pairs {
///     println!("{} <-> {}: {:.2}",
///         pair.func_a.name,
///         pair.func_b.name,
///         pair.score
///     );
/// }
/// ```
pub fn find_similar(
    path: &Path,
    language: Option<Language>,
    threshold: f64,
    max_functions: Option<usize>,
) -> TldrResult<SimilarityReport> {
    let options = SimilarityOptions {
        threshold,
        max_functions: max_functions.unwrap_or(500),
        ..Default::default()
    };

    find_similar_with_options(path, language, &options)
}

/// Find similar functions with full options
pub fn find_similar_with_options(
    path: &Path,
    language: Option<Language>,
    options: &SimilarityOptions,
) -> TldrResult<SimilarityReport> {
    // Detect language if not specified
    let lang = language.unwrap_or_else(|| detect_dominant_language(path));

    // Extract all functions
    let mut functions = extract_all_functions(path, lang)?;

    // Limit to max_functions
    if functions.len() > options.max_functions {
        // Sort by complexity descending to keep most interesting functions
        functions.sort_by(|a, b| b.complexity.cmp(&a.complexity));
        functions.truncate(options.max_functions);
    }

    let function_count = functions.len();
    if function_count < 2 {
        return Ok(SimilarityReport {
            functions_analyzed: function_count,
            pairs_compared: 0,
            similar_pairs_count: 0,
            threshold: options.threshold,
            similar_pairs: Vec::new(),
            truncated: None,
            total_pairs: None,
            shown_pairs: None,
        });
    }

    // Calculate number of pairs: n*(n-1)/2
    let pairs_count = function_count * (function_count - 1) / 2;

    // Generate all pair indices
    let pair_indices: Vec<(usize, usize)> = (0..function_count)
        .flat_map(|i| ((i + 1)..function_count).map(move |j| (i, j)))
        .collect();

    // Compute similarities in parallel using rayon (T12 mitigation)
    let threshold = options.threshold;
    let similar_pairs: Vec<SimilarPair> = pair_indices
        .par_iter()
        .filter_map(|(i, j)| {
            let (score, reasons) = calculate_similarity(&functions[*i], &functions[*j]);
            if score >= threshold {
                Some(SimilarPair {
                    func_a: functions[*i].func_ref.clone(),
                    func_b: functions[*j].func_ref.clone(),
                    score,
                    reasons,
                })
            } else {
                None
            }
        })
        .collect();

    // Sort by score descending
    let mut similar_pairs = similar_pairs;
    similar_pairs.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Limit results
    let similar_count = similar_pairs.len();
    let shown_pairs = similar_pairs.len().min(options.max_pairs);
    let was_truncated = similar_pairs.len() > options.max_pairs;
    similar_pairs.truncate(options.max_pairs);

    Ok(SimilarityReport {
        functions_analyzed: function_count,
        pairs_compared: pairs_count,
        similar_pairs_count: similar_count,
        threshold: options.threshold,
        similar_pairs,
        truncated: if was_truncated { Some(true) } else { None },
        total_pairs: if was_truncated {
            Some(similar_count)
        } else {
            None
        },
        shown_pairs: if was_truncated {
            Some(shown_pairs)
        } else {
            None
        },
    })
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Detect the dominant language in a directory
fn detect_dominant_language(path: &Path) -> Language {
    let mut counts: HashMap<Language, usize> = HashMap::new();

    for entry in walk_project(path) {
        if let Some(lang) = Language::from_path(entry.path()) {
            *counts.entry(lang).or_insert(0) += 1;
        }
    }

    counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(lang, _)| lang)
        .unwrap_or(Language::Python)
}

/// Extract all functions from source files
fn extract_all_functions(path: &Path, language: Language) -> TldrResult<Vec<FunctionData>> {
    let mut functions = Vec::new();

    let extensions: HashSet<String> = language
        .extensions()
        .iter()
        .map(|s| s.to_string())
        .collect();

    for entry in walk_project(path) {
        let entry_path = entry.path();
        if !entry_path.is_file() {
            continue;
        }

        if let Some(ext) = entry_path.extension().and_then(|e| e.to_str()) {
            let ext_with_dot = format!(".{}", ext);
            if !extensions.contains(&ext_with_dot) {
                continue;
            }
        } else {
            continue;
        }

        match extract_file(entry_path, Some(path)) {
            Ok(info) => {
                // Extract top-level functions
                for func in &info.functions {
                    if let Some(func_data) =
                        function_info_to_data(func, entry_path, &info.call_graph.calls)
                    {
                        functions.push(func_data);
                    }
                }

                // Extract methods from classes
                for class in &info.classes {
                    for method in &class.methods {
                        // Skip dunder methods
                        if method.name.starts_with("__") && method.name.ends_with("__") {
                            continue;
                        }

                        let qualified_name = format!("{}.{}", class.name, method.name);
                        if let Some(mut func_data) =
                            function_info_to_data(method, entry_path, &info.call_graph.calls)
                        {
                            func_data.func_ref.name = qualified_name;
                            functions.push(func_data);
                        }
                    }
                }
            }
            Err(_) => {
                // Skip files that fail to parse
                continue;
            }
        }
    }

    Ok(functions)
}

/// Convert FunctionInfo to FunctionData for comparison
fn function_info_to_data(
    func: &FunctionInfo,
    file: &Path,
    call_graph: &HashMap<String, Vec<String>>,
) -> Option<FunctionData> {
    // Get callees for this function
    let callees: HashSet<String> = call_graph
        .get(&func.name)
        .map(|v| v.iter().cloned().collect())
        .unwrap_or_default();

    // Estimate LOC (very rough approximation)
    // In a real implementation, we'd calculate this from the AST
    let loc = func.params.len() * 2 + 5; // Rough heuristic

    Some(FunctionData {
        func_ref: FunctionRef {
            name: func.name.clone(),
            file: file.to_path_buf(),
            line: func.line_number as usize,
        },
        param_count: func.params.len(),
        has_return_type: func.return_type.is_some(),
        complexity: 1, // Default complexity; would need CFG analysis for accurate value
        loc,
        callees,
    })
}

/// Calculate similarity between two functions
///
/// Returns (score, reasons) where score is in [0.0, 1.0] and reasons
/// explain which aspects contributed to the similarity.
fn calculate_similarity(a: &FunctionData, b: &FunctionData) -> (f64, Vec<SimilarityReason>) {
    let mut reasons = Vec::new();

    // Signature similarity (param count and return type)
    let signature_sim = calculate_signature_similarity(a, b);
    if signature_sim > 0.8 {
        reasons.push(SimilarityReason::SameSignature);
    }

    // Complexity similarity
    let complexity_sim = calculate_complexity_similarity(a, b);
    if complexity_sim > 0.8 {
        reasons.push(SimilarityReason::SimilarComplexity);
    }

    // Call pattern similarity
    let call_pattern_sim = calculate_call_pattern_similarity(a, b);
    if call_pattern_sim > 0.8 {
        reasons.push(SimilarityReason::SimilarCallPattern);
    }

    // LOC similarity
    let loc_sim = calculate_loc_similarity(a, b);
    if loc_sim > 0.8 {
        reasons.push(SimilarityReason::SimilarLoc);
    }

    // Weighted sum
    let score = SIGNATURE_WEIGHT * signature_sim
        + COMPLEXITY_WEIGHT * complexity_sim
        + CALL_PATTERN_WEIGHT * call_pattern_sim
        + LOC_WEIGHT * loc_sim;

    (score, reasons)
}

/// Calculate signature similarity based on parameter count and return type
fn calculate_signature_similarity(a: &FunctionData, b: &FunctionData) -> f64 {
    // Parameter count similarity
    let max_params = a.param_count.max(b.param_count);
    let param_sim = if max_params == 0 {
        1.0
    } else {
        let diff = (a.param_count as i32 - b.param_count as i32).unsigned_abs() as usize;
        1.0 - (diff as f64 / max_params as f64)
    };

    // Return type similarity (both have or both don't have)
    let return_sim = if a.has_return_type == b.has_return_type {
        1.0
    } else {
        0.5
    };

    // Average of both
    (param_sim + return_sim) / 2.0
}

/// Calculate complexity similarity
fn calculate_complexity_similarity(a: &FunctionData, b: &FunctionData) -> f64 {
    let max_complexity = a.complexity.max(b.complexity);
    if max_complexity == 0 {
        return 1.0;
    }

    let diff = (a.complexity as i32 - b.complexity as i32).unsigned_abs() as usize;
    1.0 - (diff as f64 / max_complexity as f64).min(1.0)
}

/// Calculate call pattern similarity using Jaccard index
fn calculate_call_pattern_similarity(a: &FunctionData, b: &FunctionData) -> f64 {
    if a.callees.is_empty() && b.callees.is_empty() {
        return 1.0; // Both call nothing -> similar
    }

    let intersection = a.callees.intersection(&b.callees).count();
    let union = a.callees.union(&b.callees).count();

    if union == 0 {
        1.0
    } else {
        intersection as f64 / union as f64
    }
}

/// Calculate LOC similarity
fn calculate_loc_similarity(a: &FunctionData, b: &FunctionData) -> f64 {
    let max_loc = a.loc.max(b.loc);
    if max_loc == 0 {
        return 1.0;
    }

    let diff = (a.loc as i32 - b.loc as i32).unsigned_abs() as usize;
    1.0 - (diff as f64 / max_loc as f64).min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_weights_sum_to_one() {
        let sum = SIGNATURE_WEIGHT + COMPLEXITY_WEIGHT + CALL_PATTERN_WEIGHT + LOC_WEIGHT;
        assert!((sum - 1.0).abs() < 0.0001);
    }

    #[test]
    fn test_signature_similarity_same_params() {
        let a = FunctionData {
            func_ref: FunctionRef {
                name: "a".to_string(),
                file: PathBuf::from("a.py"),
                line: 1,
            },
            param_count: 3,
            has_return_type: true,
            complexity: 1,
            loc: 10,
            callees: HashSet::new(),
        };
        let b = FunctionData {
            func_ref: FunctionRef {
                name: "b".to_string(),
                file: PathBuf::from("b.py"),
                line: 1,
            },
            param_count: 3,
            has_return_type: true,
            complexity: 1,
            loc: 10,
            callees: HashSet::new(),
        };

        let sim = calculate_signature_similarity(&a, &b);
        assert!((sim - 1.0).abs() < 0.0001);
    }

    #[test]
    fn test_call_pattern_jaccard() {
        let mut callees_a = HashSet::new();
        callees_a.insert("foo".to_string());
        callees_a.insert("bar".to_string());

        let mut callees_b = HashSet::new();
        callees_b.insert("foo".to_string());
        callees_b.insert("baz".to_string());

        let a = FunctionData {
            func_ref: FunctionRef {
                name: "a".to_string(),
                file: PathBuf::from("a.py"),
                line: 1,
            },
            param_count: 0,
            has_return_type: false,
            complexity: 1,
            loc: 10,
            callees: callees_a,
        };
        let b = FunctionData {
            func_ref: FunctionRef {
                name: "b".to_string(),
                file: PathBuf::from("b.py"),
                line: 1,
            },
            param_count: 0,
            has_return_type: false,
            complexity: 1,
            loc: 10,
            callees: callees_b,
        };

        // Jaccard: intersection=1 (foo), union=3 (foo, bar, baz)
        let sim = calculate_call_pattern_similarity(&a, &b);
        assert!((sim - 1.0 / 3.0).abs() < 0.0001);
    }

    #[test]
    fn test_similarity_report_default() {
        let report = SimilarityReport::default();
        assert_eq!(report.functions_analyzed, 0);
        assert_eq!(report.pairs_compared, 0);
        assert!(report.similar_pairs.is_empty());
    }

    #[test]
    fn test_similarity_reason_description() {
        assert_eq!(
            SimilarityReason::SameSignature.description(),
            "same parameter count"
        );
        assert_eq!(
            SimilarityReason::SimilarComplexity.description(),
            "similar complexity"
        );
    }
}
