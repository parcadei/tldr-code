//! Similarity analysis module (Session 8)
//!
//! This module provides code similarity analysis capabilities:
//! - Dice coefficient (Sorensen-Dice)
//! - Jaccard coefficient
//! - Cosine similarity (TF-IDF)
//! - N-gram similarity
//!
//! # Example
//!
//! ```rust,ignore
//! use tldr_core::analysis::similarity::{compute_similarity, SimilarityOptions};
//!
//! let options = SimilarityOptions::default();
//! let report = compute_similarity(
//!     Path::new("src/a.py"),
//!     Path::new("src/b.py"),
//!     &options
//! )?;
//! println!("Dice: {:.2}, Jaccard: {:.2}", report.similarity.dice, report.similarity.jaccard);
//! ```
//!
//! Reference: session8-spec.md

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::analysis::clones::{normalize_tokens, NormalizationMode, NormalizedToken};

// =============================================================================
// Core Similarity Types
// =============================================================================

/// Complete similarity analysis report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimilarityReport {
    /// First fragment analyzed
    pub fragment1: SimilarityFragment,

    /// Second fragment analyzed
    pub fragment2: SimilarityFragment,

    /// Similarity scores
    pub similarity: SimilarityScores,

    /// Token breakdown
    pub token_breakdown: TokenBreakdown,

    /// Configuration used
    pub config: SimilarityConfig,
}

impl SimilarityReport {
    /// Create a new similarity report
    pub fn new(
        fragment1: SimilarityFragment,
        fragment2: SimilarityFragment,
        similarity: SimilarityScores,
        token_breakdown: TokenBreakdown,
        config: SimilarityConfig,
    ) -> Self {
        Self {
            fragment1,
            fragment2,
            similarity,
            token_breakdown,
            config,
        }
    }
}

/// A code fragment for similarity analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimilarityFragment {
    /// File path
    pub file: PathBuf,

    /// Function name (if function-level)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub function: Option<String>,

    /// Line range (if block-level)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_range: Option<(usize, usize)>,

    /// Token count
    pub tokens: usize,

    /// Line count
    pub lines: usize,
}

impl SimilarityFragment {
    /// Create a new fragment
    pub fn new(file: PathBuf, tokens: usize, lines: usize) -> Self {
        Self {
            file,
            function: None,
            line_range: None,
            tokens,
            lines,
        }
    }

    /// Add function context
    pub fn with_function(mut self, function: String) -> Self {
        self.function = Some(function);
        self
    }

    /// Add line range context
    pub fn with_line_range(mut self, start: usize, end: usize) -> Self {
        self.line_range = Some((start, end));
        self
    }
}

/// Similarity scores
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimilarityScores {
    /// Dice coefficient (0.0 - 1.0)
    pub dice: f64,

    /// Jaccard coefficient (0.0 - 1.0)
    pub jaccard: f64,

    /// Cosine similarity (0.0 - 1.0, optional)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cosine: Option<f64>,

    /// Human-readable interpretation
    pub interpretation: String,
}

impl SimilarityScores {
    /// Create new similarity scores with automatic interpretation
    pub fn new(dice: f64, jaccard: f64) -> Self {
        let interpretation = interpret_similarity_score(dice);
        Self {
            dice,
            jaccard,
            cosine: None,
            interpretation,
        }
    }

    /// Add cosine similarity
    pub fn with_cosine(mut self, cosine: f64) -> Self {
        self.cosine = Some(cosine);
        self
    }
}

impl Default for SimilarityScores {
    fn default() -> Self {
        Self::new(0.0, 0.0)
    }
}

/// Token breakdown for detailed analysis
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TokenBreakdown {
    /// Tokens shared between fragments
    pub shared_tokens: usize,

    /// Tokens unique to fragment 1
    pub unique_to_fragment1: usize,

    /// Tokens unique to fragment 2
    pub unique_to_fragment2: usize,

    /// Total unique tokens across both
    pub total_unique: usize,
}

impl TokenBreakdown {
    /// Create from token counts
    pub fn new(shared: usize, unique1: usize, unique2: usize) -> Self {
        Self {
            shared_tokens: shared,
            unique_to_fragment1: unique1,
            unique_to_fragment2: unique2,
            total_unique: shared + unique1 + unique2,
        }
    }
}

/// Similarity analysis configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimilarityConfig {
    /// Similarity metric used
    pub metric: SimilarityMetric,

    /// N-gram size (1 = unigrams, 2 = bigrams, etc.)
    pub ngram_size: usize,

    /// Language
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

impl Default for SimilarityConfig {
    fn default() -> Self {
        Self {
            metric: SimilarityMetric::All,
            ngram_size: 1,
            language: None,
        }
    }
}

/// Similarity metric to compute
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SimilarityMetric {
    /// Dice coefficient only
    Dice,

    /// Jaccard coefficient only
    Jaccard,

    /// Cosine similarity (TF-IDF)
    Cosine,

    /// All metrics (default)
    #[default]
    All,
}

impl SimilarityMetric {
    /// Returns the string representation of this similarity metric.
    pub fn as_str(&self) -> &'static str {
        match self {
            SimilarityMetric::Dice => "dice",
            SimilarityMetric::Jaccard => "jaccard",
            SimilarityMetric::Cosine => "cosine",
            SimilarityMetric::All => "all",
        }
    }
}

/// Options for similarity analysis
#[derive(Debug, Clone, Default)]
pub struct SimilarityOptions {
    /// Similarity metric to compute
    pub metric: SimilarityMetric,

    /// N-gram size (default: 1)
    pub ngram_size: usize,

    /// Language
    pub language: Option<String>,

    /// Comparison level
    pub level: Option<ComparisonLevel>,
}

impl SimilarityOptions {
    /// Create a new `SimilarityOptions` with default values.
    pub fn new() -> Self {
        Self {
            metric: SimilarityMetric::All,
            ngram_size: 1,
            language: None,
            level: None,
        }
    }
}

/// Comparison granularity level
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonLevel {
    /// File-level comparison (default)
    File,

    /// Function-level comparison
    Function,

    /// Block-level comparison (line range)
    Block,
}

// =============================================================================
// Pairwise Similarity Types
// =============================================================================

/// Pairwise similarity matrix report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairwiseSimilarityReport {
    /// All computed pairs
    pub pairs: Vec<PairwiseSimilarityEntry>,

    /// Files analyzed
    pub files_analyzed: usize,

    /// Configuration used
    pub config: SimilarityConfig,
}

/// Entry in pairwise similarity matrix
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairwiseSimilarityEntry {
    /// First file
    pub file1: PathBuf,

    /// Second file
    pub file2: PathBuf,

    /// Dice coefficient
    pub dice: f64,

    /// Jaccard coefficient
    pub jaccard: f64,
}

// =============================================================================
// Target Parsing Types
// =============================================================================

/// Parsed target specification
#[derive(Debug, Clone)]
pub struct ParsedTarget {
    /// File path
    pub file: PathBuf,

    /// Function name (if specified with ::)
    pub function: Option<String>,

    /// Line range (if specified with :start:end)
    pub line_range: Option<(usize, usize)>,
}

impl ParsedTarget {
    /// Create a target for an entire file.
    pub fn file_only(file: PathBuf) -> Self {
        Self {
            file,
            function: None,
            line_range: None,
        }
    }

    /// Create a target for a specific function within a file.
    pub fn with_function(file: PathBuf, function: String) -> Self {
        Self {
            file,
            function: Some(function),
            line_range: None,
        }
    }

    /// Create a target for a specific line range within a file.
    pub fn with_line_range(file: PathBuf, start: usize, end: usize) -> Self {
        Self {
            file,
            function: None,
            line_range: Some((start, end)),
        }
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Interpret a similarity score as human-readable description
///
/// # Score Interpretation
/// - >= 0.95: Near-identical
/// - 0.85-0.95: High similarity
/// - 0.70-0.85: Moderate similarity
/// - 0.50-0.70: Some similarity
/// - 0.30-0.50: Low similarity
/// - < 0.30: Very different
pub fn interpret_similarity_score(score: f64) -> String {
    match score {
        s if s >= 0.95 => "Near-identical (Type-1/2 clone)".to_string(),
        s if s >= 0.85 => "High similarity (likely derived from same source)".to_string(),
        s if s >= 0.70 => "Moderate similarity (possible Type-3 clone)".to_string(),
        s if s >= 0.50 => "Some similarity (shared patterns/idioms)".to_string(),
        s if s >= 0.30 => "Low similarity".to_string(),
        _ => "Very different (minimal shared code)".to_string(),
    }
}

/// Compute Dice coefficient from shared and total token counts
///
/// Formula: 2 * |A ∩ B| / (|A| + |B|)
///
/// # Edge Cases (S8-P1-T11)
/// - Both empty: returns 1.0 (both have nothing = identical)
/// - One empty, one not: returns 0.0
pub fn dice_coefficient(shared: usize, total_a: usize, total_b: usize) -> f64 {
    // S8-P1-T11: Handle empty input edge cases
    if total_a == 0 && total_b == 0 {
        return 1.0; // Both empty = identical
    }
    let total = total_a + total_b;
    if total == 0 {
        return 0.0;
    }
    (2.0 * shared as f64) / total as f64
}

/// Compute Jaccard coefficient from shared and union token counts
///
/// Formula: |A ∩ B| / |A ∪ B|
///
/// # Edge Cases (S8-P1-T11)
/// - Both empty: returns 1.0 (both have nothing = identical)
/// - One empty, one not: returns 0.0
pub fn jaccard_coefficient(shared: usize, total_a: usize, total_b: usize) -> f64 {
    // S8-P1-T11: Handle empty input edge cases
    if total_a == 0 && total_b == 0 {
        return 1.0; // Both empty = identical
    }
    // |A ∪ B| = |A| + |B| - |A ∩ B| = total_a + total_b - shared
    let union = total_a + total_b - shared;
    if union == 0 {
        return 0.0;
    }
    shared as f64 / union as f64
}

/// Convert Dice to Jaccard
///
/// Formula: jaccard = dice / (2 - dice)
pub fn dice_to_jaccard(dice: f64) -> f64 {
    if dice >= 2.0 {
        return 1.0;
    }
    dice / (2.0 - dice)
}

/// Convert Jaccard to Dice
///
/// Formula: dice = 2 * jaccard / (1 + jaccard)
pub fn jaccard_to_dice(jaccard: f64) -> f64 {
    (2.0 * jaccard) / (1.0 + jaccard)
}

// =============================================================================
// Main API Functions (Stubs)
// =============================================================================

/// Compute similarity between two code fragments
///
/// # Arguments
/// * `path1` - First fragment (file or file::function or file:start:end)
/// * `path2` - Second fragment
/// * `options` - Similarity analysis options
///
/// # Returns
/// * `SimilarityReport` containing similarity scores and breakdown
pub fn compute_similarity(
    path1: &Path,
    path2: &Path,
    options: &SimilarityOptions,
) -> anyhow::Result<SimilarityReport> {
    // Detect language from file extension
    let language = detect_language_from_path(path1)
        .or_else(|| detect_language_from_path(path2))
        .or_else(|| options.language.clone())
        .ok_or_else(|| anyhow::anyhow!("Could not detect language from file extension"))?;

    // Read and tokenize both files
    let source1 = std::fs::read_to_string(path1)
        .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", path1.display(), e))?;
    let source2 = std::fs::read_to_string(path2)
        .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", path2.display(), e))?;

    // Normalize tokens
    let normalization = NormalizationMode::All;
    let tokens1 = normalize_tokens(&source1, &language, normalization)?;
    let tokens2 = normalize_tokens(&source2, &language, normalization)?;

    // Compute line counts
    let lines1 = source1.lines().count();
    let lines2 = source2.lines().count();

    // Create fragments
    let fragment1 = SimilarityFragment::new(path1.to_path_buf(), tokens1.len(), lines1);
    let fragment2 = SimilarityFragment::new(path2.to_path_buf(), tokens2.len(), lines2);

    // Compute similarity metrics
    let results = compute_similarity_from_tokens(&tokens1, &tokens2, options);

    // Compute token breakdown
    let breakdown = compute_token_breakdown(&tokens1, &tokens2);

    // Build config
    let config = SimilarityConfig {
        metric: options.metric,
        ngram_size: if options.ngram_size == 0 {
            1
        } else {
            options.ngram_size
        },
        language: Some(language),
    };

    // Build scores from results
    let scores = SimilarityScores::from(results);

    Ok(SimilarityReport::new(
        fragment1, fragment2, scores, breakdown, config,
    ))
}

/// Detect language from file path extension
fn detect_language_from_path(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .and_then(|ext| match ext {
            "py" => Some("python"),
            "ts" | "tsx" => Some("typescript"),
            "js" | "jsx" => Some("javascript"),
            "go" => Some("go"),
            "rs" => Some("rust"),
            "java" => Some("java"),
            _ => None,
        })
        .map(String::from)
}

/// Compute pairwise similarity for all files in a directory
///
/// # Arguments
/// * `path` - Directory to analyze
/// * `options` - Similarity analysis options
///
/// # Returns
/// * `PairwiseSimilarityReport` containing all pair similarities
pub fn compute_pairwise_similarity(
    _path: &Path,
    _options: &SimilarityOptions,
) -> anyhow::Result<PairwiseSimilarityReport> {
    todo!("Pairwise similarity computation not yet implemented")
}

/// Parse a target string into components
///
/// Formats supported:
/// - `file.py` - entire file
/// - `file.py::function` - specific function
/// - `file.py:start:end` - line range
///
/// # Arguments
/// * `target` - Target string to parse
///
/// # Returns
/// * `ParsedTarget` with file, function, and/or line range
pub fn parse_target(target: &str) -> anyhow::Result<ParsedTarget> {
    // Check for function syntax (::)
    if let Some(pos) = target.find("::") {
        let file = PathBuf::from(&target[..pos]);
        let function = target[pos + 2..].to_string();
        return Ok(ParsedTarget::with_function(file, function));
    }

    // Check for line range syntax (:start:end)
    let parts: Vec<&str> = target.rsplitn(3, ':').collect();
    if parts.len() == 3 {
        if let (Ok(end), Ok(start)) = (parts[0].parse::<usize>(), parts[1].parse::<usize>()) {
            if start > end {
                anyhow::bail!("Invalid line range: start ({}) > end ({})", start, end);
            }
            let file = PathBuf::from(parts[2]);
            return Ok(ParsedTarget::with_line_range(file, start, end));
        }
    }

    // Just a file path
    Ok(ParsedTarget::file_only(PathBuf::from(target)))
}

// =============================================================================
// Similarity Computation Implementation (Phase 4)
// =============================================================================

/// Compute all similarity metrics for two token sequences
///
/// This is the core similarity computation function.
/// Uses bag-of-tokens (multiset) representation for accurate computation.
///
/// # Premortem Risks Addressed
/// - S8-P1-T3: Uses bag (HashMap) sizes for Dice denominator, not vec lengths
/// - S8-P1-T11: Handles empty inputs correctly
///
/// # Arguments
/// * `tokens1` - Normalized tokens from first fragment
/// * `tokens2` - Normalized tokens from second fragment
/// * `options` - Similarity analysis options
///
/// # Returns
/// * `SimilarityResults` containing dice, jaccard, and optionally cosine
pub fn compute_similarity_from_tokens(
    tokens1: &[NormalizedToken],
    tokens2: &[NormalizedToken],
    options: &SimilarityOptions,
) -> SimilarityResults {
    // S8-P1-T11: Handle empty inputs
    if tokens1.is_empty() && tokens2.is_empty() {
        // Both empty = identical
        return SimilarityResults {
            dice: 1.0,
            jaccard: 1.0,
            cosine: if matches!(
                options.metric,
                SimilarityMetric::Cosine | SimilarityMetric::All
            ) {
                Some(1.0)
            } else {
                None
            },
            interpretation: interpret_similarity_score(1.0),
        };
    }

    if tokens1.is_empty() || tokens2.is_empty() {
        // One empty, one not = no similarity
        return SimilarityResults {
            dice: 0.0,
            jaccard: 0.0,
            cosine: if matches!(
                options.metric,
                SimilarityMetric::Cosine | SimilarityMetric::All
            ) {
                Some(0.0)
            } else {
                None
            },
            interpretation: interpret_similarity_score(0.0),
        };
    }

    // Build token multisets (bags) for accurate computation
    // S8-P1-T3: Use HashMap<&str, usize> to preserve duplicate counts
    let bag1 = build_token_bag(tokens1);
    let bag2 = build_token_bag(tokens2);

    // Compute metrics
    let dice = compute_dice(&bag1, &bag2);
    let jaccard = compute_jaccard(&bag1, &bag2);
    let cosine = if matches!(
        options.metric,
        SimilarityMetric::Cosine | SimilarityMetric::All
    ) {
        Some(compute_cosine_tf(&bag1, &bag2))
    } else {
        None
    };

    let mut scores = SimilarityScores::new(dice, jaccard);
    if let Some(cos) = cosine {
        scores = scores.with_cosine(cos);
    }

    SimilarityResults {
        dice: scores.dice,
        jaccard: scores.jaccard,
        cosine: scores.cosine,
        interpretation: scores.interpretation,
    }
}

/// Build a bag (multiset) representation from tokens
///
/// Returns HashMap mapping token value -> count
fn build_token_bag(tokens: &[NormalizedToken]) -> HashMap<&str, usize> {
    let mut bag: HashMap<&str, usize> = HashMap::new();
    for token in tokens {
        *bag.entry(token.value.as_str()).or_insert(0) += 1;
    }
    bag
}

/// Compute Dice coefficient for two token bags (multisets)
///
/// Formula: 2 * |A ∩ B| / (|A| + |B|)
///
/// Where |A| and |B| are multiset sizes (sum of counts), not unique token counts.
/// |A ∩ B| = sum of min(count_A[t], count_B[t]) for each token t
///
/// # Premortem S8-P1-T3
/// CRITICAL: Uses sum of bag counts for denominator, not HashMap::len()
fn compute_dice(bag1: &HashMap<&str, usize>, bag2: &HashMap<&str, usize>) -> f64 {
    // S8-P1-T3: Compute multiset sizes (sum of counts)
    let size1: usize = bag1.values().sum();
    let size2: usize = bag2.values().sum();

    // S8-P1-T11: Handle empty bags
    if size1 == 0 && size2 == 0 {
        return 1.0; // Both empty = identical
    }
    if size1 == 0 || size2 == 0 {
        return 0.0; // One empty = no similarity
    }

    // Compute multiset intersection: sum of min(count1, count2) for each token
    let mut intersection = 0usize;
    for (token, &count1) in bag1 {
        if let Some(&count2) = bag2.get(token) {
            intersection += count1.min(count2);
        }
    }

    // Dice = 2 * intersection / (size1 + size2)
    (2.0 * intersection as f64) / (size1 + size2) as f64
}

/// Compute Jaccard coefficient for two token bags (multisets)
///
/// Formula: |A ∩ B| / |A ∪ B|
///
/// For multisets:
/// |A ∪ B| = |A| + |B| - |A ∩ B|
fn compute_jaccard(bag1: &HashMap<&str, usize>, bag2: &HashMap<&str, usize>) -> f64 {
    // Compute multiset sizes
    let size1: usize = bag1.values().sum();
    let size2: usize = bag2.values().sum();

    // S8-P1-T11: Handle empty bags
    if size1 == 0 && size2 == 0 {
        return 1.0; // Both empty = identical
    }
    if size1 == 0 || size2 == 0 {
        return 0.0; // One empty = no similarity
    }

    // Compute multiset intersection
    let mut intersection = 0usize;
    for (token, &count1) in bag1 {
        if let Some(&count2) = bag2.get(token) {
            intersection += count1.min(count2);
        }
    }

    // Union = size1 + size2 - intersection
    let union = size1 + size2 - intersection;

    if union == 0 {
        return 1.0; // Shouldn't happen if we get here, but handle defensively
    }

    intersection as f64 / union as f64
}

/// Compute cosine similarity using term frequency (TF)
///
/// Formula: (A . B) / (||A|| * ||B||)
///
/// Where A and B are term frequency vectors.
/// This is a simplified version without IDF weighting.
///
/// For full TF-IDF, a corpus would be needed for IDF computation.
fn compute_cosine_tf(bag1: &HashMap<&str, usize>, bag2: &HashMap<&str, usize>) -> f64 {
    // S8-P1-T11: Handle empty bags
    if bag1.is_empty() && bag2.is_empty() {
        return 1.0; // Both empty = identical
    }
    if bag1.is_empty() || bag2.is_empty() {
        return 0.0; // One empty = no similarity
    }

    // Compute dot product
    let mut dot_product = 0.0f64;
    for (token, &count1) in bag1 {
        if let Some(&count2) = bag2.get(token) {
            dot_product += (count1 * count2) as f64;
        }
    }

    // Compute norms
    let mut norm1 = 0.0f64;
    for &count in bag1.values() {
        norm1 += (count as f64).powi(2);
    }

    let mut norm2 = 0.0f64;
    for &count in bag2.values() {
        norm2 += (count as f64).powi(2);
    }

    // Handle zero norms (shouldn't happen with non-empty bags)
    if norm1 == 0.0 || norm2 == 0.0 {
        return 0.0;
    }

    dot_product / (norm1.sqrt() * norm2.sqrt())
}

/// Compute token breakdown (shared, unique1, unique2)
///
/// Returns counts for shared tokens and tokens unique to each fragment.
pub fn compute_token_breakdown(
    tokens1: &[NormalizedToken],
    tokens2: &[NormalizedToken],
) -> TokenBreakdown {
    let bag1 = build_token_bag(tokens1);
    let bag2 = build_token_bag(tokens2);

    let size1: usize = bag1.values().sum();
    let size2: usize = bag2.values().sum();

    // Compute intersection (shared tokens with multiplicity)
    let mut shared = 0usize;
    for (token, &count1) in &bag1 {
        if let Some(&count2) = bag2.get(token) {
            shared += count1.min(count2);
        }
    }

    // Unique to fragment 1 = size1 - shared
    let unique1 = size1.saturating_sub(shared);

    // Unique to fragment 2 = size2 - shared
    let unique2 = size2.saturating_sub(shared);

    TokenBreakdown::new(shared, unique1, unique2)
}

/// Result type for similarity computation (internal use)
#[derive(Debug, Clone)]
pub struct SimilarityResults {
    /// Dice (Sorensen-Dice) coefficient
    pub dice: f64,
    /// Jaccard similarity coefficient
    pub jaccard: f64,
    /// Cosine similarity score (computed only when requested)
    pub cosine: Option<f64>,
    /// Human-readable interpretation of the similarity score
    pub interpretation: String,
}

impl From<SimilarityResults> for SimilarityScores {
    fn from(results: SimilarityResults) -> Self {
        let mut scores = SimilarityScores::new(results.dice, results.jaccard);
        if let Some(cosine) = results.cosine {
            scores = scores.with_cosine(cosine);
        }
        scores
    }
}

// =============================================================================
// Unit Tests for Similarity Computation
// =============================================================================

#[cfg(test)]
mod similarity_unit_tests {
    use super::*;
    use crate::analysis::clones::{NormalizedToken, TokenCategory};

    /// Helper to create a token
    fn make_token(value: &str) -> NormalizedToken {
        NormalizedToken {
            value: value.to_string(),
            original: value.to_string(),
            category: TokenCategory::Other,
        }
    }

    /// Helper to create token vector
    fn make_tokens(values: &[&str]) -> Vec<NormalizedToken> {
        values.iter().map(|v| make_token(v)).collect()
    }

    // =========================================================================
    // Dice Coefficient Tests
    // =========================================================================

    #[test]
    fn test_dice_identical_tokens() {
        let tokens = make_tokens(&["a", "b", "c"]);
        let options = SimilarityOptions::default();
        let result = compute_similarity_from_tokens(&tokens, &tokens, &options);
        assert!(
            (result.dice - 1.0).abs() < 0.001,
            "Identical tokens should have dice=1.0, got {}",
            result.dice
        );
    }

    #[test]
    fn test_dice_completely_different() {
        let tokens1 = make_tokens(&["a", "b", "c"]);
        let tokens2 = make_tokens(&["x", "y", "z"]);
        let options = SimilarityOptions::default();
        let result = compute_similarity_from_tokens(&tokens1, &tokens2, &options);
        assert!(
            (result.dice - 0.0).abs() < 0.001,
            "Disjoint tokens should have dice=0.0, got {}",
            result.dice
        );
    }

    #[test]
    fn test_dice_partial_overlap() {
        // A = {a, b, c}, B = {b, c, d}
        // Intersection = {b, c} = 2
        // |A| = 3, |B| = 3
        // Dice = 2 * 2 / (3 + 3) = 4/6 = 0.667
        let tokens1 = make_tokens(&["a", "b", "c"]);
        let tokens2 = make_tokens(&["b", "c", "d"]);
        let options = SimilarityOptions::default();
        let result = compute_similarity_from_tokens(&tokens1, &tokens2, &options);
        let expected = 4.0 / 6.0;
        assert!(
            (result.dice - expected).abs() < 0.001,
            "Expected dice={:.3}, got {:.3}",
            expected,
            result.dice
        );
    }

    #[test]
    fn test_dice_with_duplicates() {
        // S8-P1-T3: Test multiset handling
        // A = {a, a, b} (multiset sizes: a=2, b=1, total=3)
        // B = {a, b, b} (multiset sizes: a=1, b=2, total=3)
        // Intersection = min(2,1) + min(1,2) = 1 + 1 = 2
        // Dice = 2 * 2 / (3 + 3) = 4/6 = 0.667
        let tokens1 = make_tokens(&["a", "a", "b"]);
        let tokens2 = make_tokens(&["a", "b", "b"]);
        let options = SimilarityOptions::default();
        let result = compute_similarity_from_tokens(&tokens1, &tokens2, &options);
        let expected = 4.0 / 6.0;
        assert!(
            (result.dice - expected).abs() < 0.001,
            "Expected dice={:.3}, got {:.3}",
            expected,
            result.dice
        );
    }

    #[test]
    fn test_dice_symmetry() {
        let tokens1 = make_tokens(&["a", "b", "c"]);
        let tokens2 = make_tokens(&["b", "c", "d"]);
        let options = SimilarityOptions::default();
        let result_ab = compute_similarity_from_tokens(&tokens1, &tokens2, &options);
        let result_ba = compute_similarity_from_tokens(&tokens2, &tokens1, &options);
        assert!(
            (result_ab.dice - result_ba.dice).abs() < 0.001,
            "Dice should be symmetric"
        );
    }

    // =========================================================================
    // Empty Input Edge Cases (S8-P1-T11)
    // =========================================================================

    #[test]
    fn test_dice_both_empty() {
        // S8-P1-T11: Two empty = identical
        let tokens1: Vec<NormalizedToken> = vec![];
        let tokens2: Vec<NormalizedToken> = vec![];
        let options = SimilarityOptions::default();
        let result = compute_similarity_from_tokens(&tokens1, &tokens2, &options);
        assert!(
            (result.dice - 1.0).abs() < 0.001,
            "Both empty should have dice=1.0, got {}",
            result.dice
        );
    }

    #[test]
    fn test_dice_one_empty() {
        // S8-P1-T11: One empty = no similarity
        let tokens1 = make_tokens(&["a", "b", "c"]);
        let tokens2: Vec<NormalizedToken> = vec![];
        let options = SimilarityOptions::default();
        let result = compute_similarity_from_tokens(&tokens1, &tokens2, &options);
        assert!(
            (result.dice - 0.0).abs() < 0.001,
            "One empty should have dice=0.0, got {}",
            result.dice
        );
    }

    // =========================================================================
    // Jaccard Coefficient Tests
    // =========================================================================

    #[test]
    fn test_jaccard_identical_tokens() {
        let tokens = make_tokens(&["a", "b", "c"]);
        let options = SimilarityOptions::default();
        let result = compute_similarity_from_tokens(&tokens, &tokens, &options);
        assert!(
            (result.jaccard - 1.0).abs() < 0.001,
            "Identical tokens should have jaccard=1.0, got {}",
            result.jaccard
        );
    }

    #[test]
    fn test_jaccard_completely_different() {
        let tokens1 = make_tokens(&["a", "b", "c"]);
        let tokens2 = make_tokens(&["x", "y", "z"]);
        let options = SimilarityOptions::default();
        let result = compute_similarity_from_tokens(&tokens1, &tokens2, &options);
        assert!(
            (result.jaccard - 0.0).abs() < 0.001,
            "Disjoint tokens should have jaccard=0.0, got {}",
            result.jaccard
        );
    }

    #[test]
    fn test_jaccard_partial_overlap() {
        // A = {a, b, c}, B = {b, c, d}
        // Intersection = 2, Union = 4
        // Jaccard = 2/4 = 0.5
        let tokens1 = make_tokens(&["a", "b", "c"]);
        let tokens2 = make_tokens(&["b", "c", "d"]);
        let options = SimilarityOptions::default();
        let result = compute_similarity_from_tokens(&tokens1, &tokens2, &options);
        let expected = 0.5;
        assert!(
            (result.jaccard - expected).abs() < 0.001,
            "Expected jaccard={:.3}, got {:.3}",
            expected,
            result.jaccard
        );
    }

    #[test]
    fn test_jaccard_less_than_or_equal_dice() {
        // Jaccard should always be <= Dice
        let tokens1 = make_tokens(&["a", "b", "c", "d"]);
        let tokens2 = make_tokens(&["b", "c", "d", "e"]);
        let options = SimilarityOptions::default();
        let result = compute_similarity_from_tokens(&tokens1, &tokens2, &options);
        assert!(
            result.jaccard <= result.dice + 0.001,
            "Jaccard ({}) should be <= Dice ({})",
            result.jaccard,
            result.dice
        );
    }

    // =========================================================================
    // Cosine Similarity Tests
    // =========================================================================

    #[test]
    fn test_cosine_identical_tokens() {
        let tokens = make_tokens(&["a", "b", "c"]);
        let options = SimilarityOptions {
            metric: SimilarityMetric::All,
            ..Default::default()
        };
        let result = compute_similarity_from_tokens(&tokens, &tokens, &options);
        assert!(result.cosine.is_some(), "Cosine should be computed");
        assert!(
            (result.cosine.unwrap() - 1.0).abs() < 0.001,
            "Identical tokens should have cosine=1.0"
        );
    }

    #[test]
    fn test_cosine_completely_different() {
        let tokens1 = make_tokens(&["a", "b", "c"]);
        let tokens2 = make_tokens(&["x", "y", "z"]);
        let options = SimilarityOptions {
            metric: SimilarityMetric::All,
            ..Default::default()
        };
        let result = compute_similarity_from_tokens(&tokens1, &tokens2, &options);
        assert!(result.cosine.is_some(), "Cosine should be computed");
        assert!(
            (result.cosine.unwrap() - 0.0).abs() < 0.001,
            "Disjoint tokens should have cosine=0.0"
        );
    }

    #[test]
    fn test_cosine_with_duplicates() {
        // A = {a, a, b} -> vector: a=2, b=1
        // B = {a, b, b} -> vector: a=1, b=2
        // dot product = 2*1 + 1*2 = 4
        // ||A|| = sqrt(4 + 1) = sqrt(5)
        // ||B|| = sqrt(1 + 4) = sqrt(5)
        // cosine = 4 / 5 = 0.8
        let tokens1 = make_tokens(&["a", "a", "b"]);
        let tokens2 = make_tokens(&["a", "b", "b"]);
        let options = SimilarityOptions {
            metric: SimilarityMetric::All,
            ..Default::default()
        };
        let result = compute_similarity_from_tokens(&tokens1, &tokens2, &options);
        let expected = 4.0 / 5.0;
        assert!(result.cosine.is_some(), "Cosine should be computed");
        assert!(
            (result.cosine.unwrap() - expected).abs() < 0.001,
            "Expected cosine={:.3}, got {:.3}",
            expected,
            result.cosine.unwrap()
        );
    }

    // =========================================================================
    // Token Breakdown Tests
    // =========================================================================

    #[test]
    fn test_token_breakdown_identical() {
        let tokens = make_tokens(&["a", "b", "c"]);
        let breakdown = compute_token_breakdown(&tokens, &tokens);
        assert_eq!(breakdown.shared_tokens, 3, "All tokens should be shared");
        assert_eq!(
            breakdown.unique_to_fragment1, 0,
            "No unique tokens in fragment1"
        );
        assert_eq!(
            breakdown.unique_to_fragment2, 0,
            "No unique tokens in fragment2"
        );
    }

    #[test]
    fn test_token_breakdown_disjoint() {
        let tokens1 = make_tokens(&["a", "b"]);
        let tokens2 = make_tokens(&["x", "y", "z"]);
        let breakdown = compute_token_breakdown(&tokens1, &tokens2);
        assert_eq!(breakdown.shared_tokens, 0, "No shared tokens");
        assert_eq!(
            breakdown.unique_to_fragment1, 2,
            "All tokens unique to fragment1"
        );
        assert_eq!(
            breakdown.unique_to_fragment2, 3,
            "All tokens unique to fragment2"
        );
    }

    #[test]
    fn test_token_breakdown_partial_overlap() {
        let tokens1 = make_tokens(&["a", "b", "c"]);
        let tokens2 = make_tokens(&["b", "c", "d"]);
        let breakdown = compute_token_breakdown(&tokens1, &tokens2);
        assert_eq!(breakdown.shared_tokens, 2, "Two shared tokens");
        assert_eq!(breakdown.unique_to_fragment1, 1, "One unique to fragment1");
        assert_eq!(breakdown.unique_to_fragment2, 1, "One unique to fragment2");
    }

    // =========================================================================
    // Helper Function Tests
    // =========================================================================

    #[test]
    fn test_dice_coefficient_helper() {
        // 2 * 2 / (3 + 3) = 4/6 = 0.667
        let dice = dice_coefficient(2, 3, 3);
        assert!((dice - 0.667).abs() < 0.001);
    }

    #[test]
    fn test_jaccard_coefficient_helper() {
        // 2 / (3 + 3 - 2) = 2/4 = 0.5
        let jaccard = jaccard_coefficient(2, 3, 3);
        assert!((jaccard - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_dice_jaccard_conversion() {
        // Test the relationship: jaccard = dice / (2 - dice)
        let dice = 0.8;
        let jaccard = dice_to_jaccard(dice);
        let expected = 0.8 / 1.2; // 0.667
        assert!((jaccard - expected).abs() < 0.001);

        // And back
        let dice_back = jaccard_to_dice(jaccard);
        assert!((dice_back - dice).abs() < 0.001);
    }

    #[test]
    fn test_dice_coefficient_both_empty() {
        // S8-P1-T11: Both empty should return 1.0
        let dice = dice_coefficient(0, 0, 0);
        assert!(
            (dice - 1.0).abs() < 0.001,
            "Both empty should give dice=1.0"
        );
    }

    #[test]
    fn test_jaccard_coefficient_both_empty() {
        // S8-P1-T11: Both empty should return 1.0
        let jaccard = jaccard_coefficient(0, 0, 0);
        assert!(
            (jaccard - 1.0).abs() < 0.001,
            "Both empty should give jaccard=1.0"
        );
    }
}
