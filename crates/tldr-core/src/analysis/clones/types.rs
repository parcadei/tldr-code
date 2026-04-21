//! Clone detection types and utility functions.
//!
//! This module contains all the shared types (ClonesReport, ClonePair, CloneFragment,
//! CloneType, etc.) and utility functions (hash_token, normalize_tokens, categorize_token,
//! etc.) used by the clone detection pipeline.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tree_sitter::Tree;

use crate::ast::parser::parse;
use crate::types::Language;

// =============================================================================
// Core Clone Types
// =============================================================================

/// Complete clone detection report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClonesReport {
    /// Root path analyzed
    pub root: PathBuf,

    /// Language(s) detected/analyzed
    pub language: String,

    /// Detected clone pairs
    pub clone_pairs: Vec<ClonePair>,

    /// Clone classes (if --show-classes enabled)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clone_classes: Vec<CloneClass>,

    /// Analysis statistics
    pub stats: CloneStats,

    /// Configuration used
    pub config: CloneConfig,
}

impl Default for ClonesReport {
    fn default() -> Self {
        Self {
            root: PathBuf::new(),
            language: String::new(),
            clone_pairs: Vec::new(),
            clone_classes: Vec::new(),
            stats: CloneStats::default(),
            config: CloneConfig::default(),
        }
    }
}

/// A detected clone pair
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClonePair {
    /// Unique pair identifier (1-indexed)
    pub id: usize,

    /// Clone type classification
    pub clone_type: CloneType,

    /// Similarity score (0.0 - 1.0)
    pub similarity: f64,

    /// First fragment
    pub fragment1: CloneFragment,

    /// Second fragment
    pub fragment2: CloneFragment,

    /// Human-readable interpretation of similarity
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interpretation: Option<String>,
}

impl ClonePair {
    /// Create a new clone pair with automatic interpretation
    pub fn new(
        id: usize,
        clone_type: CloneType,
        similarity: f64,
        fragment1: CloneFragment,
        fragment2: CloneFragment,
    ) -> Self {
        let interpretation = Some(interpret_similarity(similarity));
        Self {
            id,
            clone_type,
            similarity,
            fragment1,
            fragment2,
            interpretation,
        }
    }

    /// Canonical ordering: fragment1.path < fragment2.path (or by line if same file)
    pub fn canonical(&self) -> Self {
        if self.fragment1.file > self.fragment2.file
            || (self.fragment1.file == self.fragment2.file
                && self.fragment1.start_line > self.fragment2.start_line)
        {
            Self {
                id: self.id,
                clone_type: self.clone_type,
                similarity: self.similarity,
                fragment1: self.fragment2.clone(),
                fragment2: self.fragment1.clone(),
                interpretation: self.interpretation.clone(),
            }
        } else {
            self.clone()
        }
    }
}

/// A code fragment that is part of a clone
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct CloneFragment {
    /// File containing the fragment
    pub file: PathBuf,

    /// Start line (1-indexed)
    pub start_line: usize,

    /// End line (1-indexed, inclusive)
    pub end_line: usize,

    /// Token count
    pub tokens: usize,

    /// Line count
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines: Option<usize>,

    /// Function name if fragment is within a function
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub function: Option<String>,

    /// Code preview (first few lines, truncated)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
}

impl CloneFragment {
    /// Create a new fragment with required fields
    pub fn new(file: PathBuf, start_line: usize, end_line: usize, tokens: usize) -> Self {
        let lines = Some(end_line.saturating_sub(start_line) + 1);
        Self {
            file,
            start_line,
            end_line,
            tokens,
            lines,
            function: None,
            preview: None,
        }
    }

    /// Add function context
    pub fn with_function(mut self, function: String) -> Self {
        self.function = Some(function);
        self
    }

    /// Add code preview (truncated to 100 chars)
    pub fn with_preview(mut self, preview: String) -> Self {
        let truncated = if preview.len() > 100 {
            // Find the nearest char boundary at or before byte offset 97
            let mut end = 97;
            while end > 0 && !preview.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}...", &preview[..end])
        } else {
            preview
        };
        self.preview = Some(truncated);
        self
    }
}

/// Clone type classification (Type-1, Type-2, Type-3)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum CloneType {
    /// Type-1: Exact clone (identical except whitespace/comments)
    #[serde(rename = "Type-1")]
    Type1,

    /// Type-2: Parameterized clone (renamed identifiers/literals)
    #[serde(rename = "Type-2")]
    Type2,

    /// Type-3: Gapped clone (statements added/removed/modified)
    #[serde(rename = "Type-3")]
    Type3,
}

impl CloneType {
    /// Returns the display string for this clone type (e.g., "Type-1").
    pub fn as_str(&self) -> &'static str {
        match self {
            CloneType::Type1 => "Type-1",
            CloneType::Type2 => "Type-2",
            CloneType::Type3 => "Type-3",
        }
    }

    /// Minimum similarity threshold for this clone type
    pub fn min_similarity(&self) -> f64 {
        match self {
            CloneType::Type1 => 1.0,
            CloneType::Type2 => 0.9,
            CloneType::Type3 => 0.7,
        }
    }
}

impl std::fmt::Display for CloneType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A clone class (group of related fragments)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloneClass {
    /// Class identifier (1-indexed)
    pub id: usize,

    /// All fragments in this class
    pub fragments: Vec<CloneFragment>,

    /// Number of fragments
    pub size: usize,

    /// Dominant clone type in class
    pub clone_type: CloneType,

    /// Average similarity within class
    pub avg_similarity: f64,
}

impl CloneClass {
    /// Create a new clone class
    pub fn new(
        id: usize,
        fragments: Vec<CloneFragment>,
        clone_type: CloneType,
        avg_similarity: f64,
    ) -> Self {
        let size = fragments.len();
        Self {
            id,
            fragments,
            size,
            clone_type,
            avg_similarity,
        }
    }
}

/// Clone detection statistics
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CloneStats {
    /// Files analyzed
    pub files_analyzed: usize,

    /// Total tokens processed
    pub total_tokens: usize,

    /// Total clone pairs found
    pub clones_found: usize,

    /// Number of Type-1 (exact) clones detected
    pub type1_count: usize,
    /// Number of Type-2 (parameterized) clones detected
    pub type2_count: usize,
    /// Number of Type-3 (gapped) clones detected
    pub type3_count: usize,

    /// Clone classes (if computed)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_count: Option<usize>,

    /// Detection time in milliseconds
    pub detection_time_ms: u64,
}

/// Clone detection configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloneConfig {
    /// Minimum tokens for clone detection
    pub min_tokens: usize,

    /// Minimum lines for clone detection
    pub min_lines: usize,

    /// Similarity threshold for Type-3 clones
    pub similarity_threshold: f64,

    /// Normalization mode
    pub normalization: NormalizationMode,

    /// Clone type filter (None = all types)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_filter: Option<CloneType>,
}

impl Default for CloneConfig {
    fn default() -> Self {
        Self {
            min_tokens: 25,
            min_lines: 5,
            similarity_threshold: 0.7,
            normalization: NormalizationMode::All,
            type_filter: None,
        }
    }
}

/// Normalization mode for clone detection
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum NormalizationMode {
    /// No normalization (Type-1 only)
    None,

    /// Normalize identifiers only
    Identifiers,

    /// Normalize literals only
    Literals,

    /// Normalize both identifiers and literals (default)
    #[default]
    All,
}

impl NormalizationMode {
    /// Returns the string representation of this normalization mode.
    pub fn as_str(&self) -> &'static str {
        match self {
            NormalizationMode::None => "none",
            NormalizationMode::Identifiers => "identifiers",
            NormalizationMode::Literals => "literals",
            NormalizationMode::All => "all",
        }
    }

    /// Parse a normalization mode from a string (case-insensitive).
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "none" => Some(NormalizationMode::None),
            "identifiers" => Some(NormalizationMode::Identifiers),
            "literals" => Some(NormalizationMode::Literals),
            "all" => Some(NormalizationMode::All),
            _ => None,
        }
    }

    /// Whether to normalize identifiers
    pub fn normalize_identifiers(&self) -> bool {
        matches!(
            self,
            NormalizationMode::Identifiers | NormalizationMode::All
        )
    }

    /// Whether to normalize literals
    pub fn normalize_literals(&self) -> bool {
        matches!(self, NormalizationMode::Literals | NormalizationMode::All)
    }
}

/// Options for clone detection
#[derive(Debug, Clone)]
pub struct ClonesOptions {
    /// Minimum tokens (default: 50)
    pub min_tokens: usize,

    /// Minimum lines (default: 6)
    pub min_lines: usize,

    /// Similarity threshold (default: 0.7)
    pub threshold: f64,

    /// Clone type filter
    pub type_filter: Option<CloneType>,

    /// Normalization mode
    pub normalization: NormalizationMode,

    /// Language filter
    pub language: Option<String>,

    /// Compute clone classes
    pub show_classes: bool,

    /// Include within-file clones
    pub include_within_file: bool,

    /// Maximum pairs to return
    pub max_clones: usize,

    /// Maximum files to analyze
    pub max_files: usize,

    /// Exclude generated files (e.g., .pb.go, _generated.ts, etc.)
    pub exclude_generated: bool,

    /// Exclude test files (e.g., test_*.py, *_test.go, *_spec.rb, tests/, __tests__/)
    pub exclude_tests: bool,
}

impl ClonesOptions {
    /// Create a new `ClonesOptions` with default values.
    pub fn new() -> Self {
        Self {
            min_tokens: 25,
            min_lines: 5,
            threshold: 0.7,
            type_filter: None,
            normalization: NormalizationMode::All,
            language: None,
            show_classes: false,
            include_within_file: false,
            max_clones: 100,
            max_files: 1000,
            exclude_generated: false,
            exclude_tests: false,
        }
    }
}

impl Default for ClonesOptions {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Tokenization Types
// =============================================================================

/// Normalized token for comparison
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NormalizedToken {
    /// The normalized token value
    pub value: String,

    /// Original token (for debugging/display)
    pub original: String,

    /// Token category
    pub category: TokenCategory,
}

/// Token category for normalization
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenCategory {
    /// Variable, function, or type name
    Identifier,
    /// String literal value
    StringLiteral,
    /// Numeric literal value (integer or float)
    NumericLiteral,
    /// Language keyword (if, for, class, etc.)
    Keyword,
    /// Operator symbol (+, -, ==, etc.)
    Operator,
    /// Punctuation (brackets, commas, semicolons, etc.)
    Punctuation,
    /// Other token types not fitting above categories
    Other,
}

// =============================================================================
// Token Sequence Types
// =============================================================================

/// Token sequence from a file region
///
/// Used internally to represent a tokenized code fragment for clone detection.
#[derive(Debug, Clone)]
pub struct TokenSequence {
    /// File path
    pub file: PathBuf,

    /// Start line (1-indexed)
    pub start_line: usize,

    /// End line (1-indexed)
    pub end_line: usize,

    /// Normalized tokens
    pub tokens: Vec<NormalizedToken>,

    /// Hash of the token sequence (for quick comparison)
    pub hash: u64,
}

impl TokenSequence {
    /// Create a new token sequence
    pub fn new(
        file: PathBuf,
        start_line: usize,
        end_line: usize,
        tokens: Vec<NormalizedToken>,
        hash: u64,
    ) -> Self {
        Self {
            file,
            start_line,
            end_line,
            tokens,
            hash,
        }
    }

    /// Get the number of tokens in the sequence
    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    /// Check if the sequence is empty
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    /// Get the line count
    pub fn line_count(&self) -> usize {
        self.end_line.saturating_sub(self.start_line) + 1
    }
}

// =============================================================================
// Rolling Hash Types
// =============================================================================

/// Rolling hash state for Rabin-Karp algorithm
pub struct RollingHash {
    /// Current hash value
    pub value: u64,

    /// Window size
    pub window_size: usize,

    /// Hash base (prime)
    pub base: u64,

    /// Hash modulus (prime)
    pub modulus: u64,

    /// Precomputed base^(window_size-1) mod modulus
    pub base_power: u64,
}

impl RollingHash {
    /// Create new rolling hash with window size
    ///
    /// # Overflow Safety (S8-P1-T1)
    ///
    /// Uses wrapping multiplication to prevent overflow when computing base^(n-1).
    /// Even though the current implementation applies modulus at each step, we use
    /// wrapping_mul for defense-in-depth. The intermediate product is at most
    /// (MODULUS-1) * BASE which fits in u64, but wrapping_mul ensures safety even
    /// if parameters change in the future.
    pub fn new(window_size: usize) -> Self {
        const BASE: u64 = 31;
        const MODULUS: u64 = 1_000_000_007;

        // Precompute base^(window_size-1) mod modulus
        // Using wrapping_mul for overflow safety (S8-P1-T1)
        let mut base_power = 1u64;
        for _ in 0..window_size.saturating_sub(1) {
            base_power = base_power.wrapping_mul(BASE) % MODULUS;
        }

        Self {
            value: 0,
            window_size,
            base: BASE,
            modulus: MODULUS,
            base_power,
        }
    }

    /// Add a token to the hash
    ///
    /// # Overflow Safety
    ///
    /// Token hashes can be arbitrarily large (from hash_token using wrapping arithmetic).
    /// We reduce token_hash modulo MODULUS first to ensure the addition doesn't overflow.
    pub fn push(&mut self, token_hash: u64) {
        // Reduce token_hash first to prevent overflow when adding
        let reduced_token = token_hash % self.modulus;
        self.value = (self.value.wrapping_mul(self.base) % self.modulus)
            .wrapping_add(reduced_token)
            % self.modulus;
    }

    /// Remove oldest token and add new token (rolling)
    ///
    /// # Overflow Safety
    ///
    /// Token hashes are reduced modulo MODULUS before use to prevent overflow.
    /// The subtraction uses (value + MODULUS - x) pattern to handle underflow.
    pub fn roll(&mut self, old_token_hash: u64, new_token_hash: u64) {
        // Reduce token hashes to prevent overflow
        let old_reduced = old_token_hash % self.modulus;
        let new_reduced = new_token_hash % self.modulus;

        // Remove contribution of oldest token: (value - old * base_power) mod M
        // Using (value + M - (old * base_power % M)) % M to handle underflow
        let old_contribution = old_reduced.wrapping_mul(self.base_power) % self.modulus;
        self.value = (self.value + self.modulus - old_contribution) % self.modulus;

        // Shift and add new token: (value * BASE + new) mod M
        self.value = (self.value.wrapping_mul(self.base) % self.modulus).wrapping_add(new_reduced)
            % self.modulus;
    }

    /// Reset the hash to initial state
    pub fn reset(&mut self) {
        self.value = 0;
    }

    /// Get the current hash value
    pub fn current(&self) -> u64 {
        self.value
    }
}

// =============================================================================
// Hash Index Types
// =============================================================================

/// Hash entry storing location information
#[derive(Debug, Clone)]
pub struct HashEntry {
    /// Rolling hash value
    pub hash: u64,

    /// File index in the file list
    pub file_idx: usize,

    /// Start position (token index)
    pub start_pos: usize,

    /// End position (token index)
    pub end_pos: usize,
}

impl HashEntry {
    /// Create a new hash entry
    pub fn new(hash: u64, file_idx: usize, start_pos: usize, end_pos: usize) -> Self {
        Self {
            hash,
            file_idx,
            start_pos,
            end_pos,
        }
    }
}

/// Hash index for clone detection
///
/// Maps hash values to (file_index, position) pairs for O(1) lookup.
/// Used in Rabin-Karp algorithm to find clone candidates.
pub struct HashIndex {
    /// Hash -> list of entries with that hash
    index: HashMap<u64, Vec<HashEntry>>,
}

impl HashIndex {
    /// Create a new empty hash index
    pub fn new() -> Self {
        Self {
            index: HashMap::new(),
        }
    }

    /// Insert a hash entry
    pub fn insert(&mut self, entry: HashEntry) {
        self.index.entry(entry.hash).or_default().push(entry);
    }

    /// Insert a hash with its location (convenience method)
    pub fn insert_location(&mut self, hash: u64, file_idx: usize, start: usize, end: usize) {
        let entry = HashEntry::new(hash, file_idx, start, end);
        self.insert(entry);
    }

    /// Find all entries with a given hash
    pub fn find(&self, hash: u64) -> Option<&Vec<HashEntry>> {
        self.index.get(&hash)
    }

    /// Find all candidate pairs (hash collisions)
    /// Returns pairs of entries that share the same hash
    pub fn find_candidates(&self) -> Vec<(&HashEntry, &HashEntry)> {
        let mut candidates = Vec::new();
        for entries in self.index.values() {
            if entries.len() >= 2 {
                // Generate all pairs within this bucket
                for i in 0..entries.len() {
                    for j in (i + 1)..entries.len() {
                        candidates.push((&entries[i], &entries[j]));
                    }
                }
            }
        }
        candidates
    }

    /// Get the number of unique hashes in the index
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// Check if the index is empty
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Get the total number of entries
    pub fn total_entries(&self) -> usize {
        self.index.values().map(|v| v.len()).sum()
    }
}

impl Default for HashIndex {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Union-Find Types
// =============================================================================

/// Union-Find data structure for clone class merging
///
/// Used to efficiently group transitive clone relationships.
/// If A ~ B and B ~ C, then {A, B, C} form a clone class.
pub struct UnionFind {
    parent: Vec<usize>,
    rank: Vec<usize>,
}

impl UnionFind {
    /// Create a new Union-Find structure with `size` elements
    pub fn new(size: usize) -> Self {
        Self {
            parent: (0..size).collect(),
            rank: vec![0; size],
        }
    }

    /// Find the root of element x with path compression
    pub fn find(&mut self, x: usize) -> usize {
        if self.parent[x] != x {
            self.parent[x] = self.find(self.parent[x]);
        }
        self.parent[x]
    }

    /// Union two sets by rank
    pub fn union(&mut self, x: usize, y: usize) {
        let px = self.find(x);
        let py = self.find(y);

        if px == py {
            return;
        }

        if self.rank[px] < self.rank[py] {
            self.parent[px] = py;
        } else if self.rank[px] > self.rank[py] {
            self.parent[py] = px;
        } else {
            self.parent[py] = px;
            self.rank[px] += 1;
        }
    }

    /// Check if two elements are in the same set
    pub fn connected(&mut self, x: usize, y: usize) -> bool {
        self.find(x) == self.find(y)
    }

    /// Get all connected components
    ///
    /// Returns a map from root -> list of elements in that component
    pub fn components(&mut self) -> HashMap<usize, Vec<usize>> {
        // Pre-compute all roots to avoid borrow issues
        let roots: Vec<usize> = (0..self.parent.len()).map(|i| self.find(i)).collect();

        let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
        for (i, &root) in roots.iter().enumerate() {
            groups.entry(root).or_default().push(i);
        }
        groups
    }

    /// Get the number of elements
    pub fn len(&self) -> usize {
        self.parent.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.parent.is_empty()
    }

    /// Get the number of distinct components
    pub fn component_count(&mut self) -> usize {
        let roots: std::collections::HashSet<usize> =
            (0..self.parent.len()).map(|i| self.find(i)).collect();
        roots.len()
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Interpret similarity score as human-readable description
pub fn interpret_similarity(score: f64) -> String {
    match score {
        s if s >= 0.95 => "Near-identical (exact or trivial differences)".to_string(),
        s if s >= 0.90 => "Very high similarity (Type-1/2 clone)".to_string(),
        s if s >= 0.80 => "High similarity (likely Type-2 clone)".to_string(),
        s if s >= 0.70 => "Moderate similarity (Type-3 clone candidate)".to_string(),
        s if s >= 0.50 => "Some similarity (possible shared ancestry)".to_string(),
        _ => "Low similarity (different code)".to_string(),
    }
}

/// Classify clone type based on similarity score
///
/// Uses epsilon tolerance for floating point comparison (S8-P1-T5).
/// This prevents misclassification at boundary values due to precision issues.
///
/// # Classification
/// - Type-1: similarity == 1.0 (within epsilon)
/// - Type-2: 0.9 <= similarity < 1.0
/// - Type-3: similarity < 0.9
pub fn classify_clone_type(similarity: f64) -> CloneType {
    const EPSILON: f64 = 1e-9;

    if (similarity - 1.0).abs() < EPSILON {
        CloneType::Type1
    } else if similarity >= 0.9 - EPSILON {
        CloneType::Type2
    } else {
        CloneType::Type3
    }
}

// =============================================================================
// Similarity Functions (Phase 6 - S8-P1-T2, S8-P1-T6)
// =============================================================================

/// Compute Dice coefficient similarity between two token sequences
///
/// Dice = 2 * |A intersect B| / (|A| + |B|)
///
/// For multisets (bags of tokens), intersection is sum of min(count_A[t], count_B[t])
/// for each unique token t.
///
/// # Arguments
/// * `tokens1` - First token sequence
/// * `tokens2` - Second token sequence
///
/// # Returns
/// * Dice coefficient in range [0.0, 1.0]
pub fn compute_dice_similarity(tokens1: &[NormalizedToken], tokens2: &[NormalizedToken]) -> f64 {
    // Handle empty cases
    if tokens1.is_empty() && tokens2.is_empty() {
        return 1.0; // Both empty = identical
    }
    if tokens1.is_empty() || tokens2.is_empty() {
        return 0.0; // One empty = no similarity
    }

    // Build token multisets (bags)
    let mut bag1: HashMap<&str, usize> = HashMap::new();
    let mut bag2: HashMap<&str, usize> = HashMap::new();

    for t in tokens1 {
        *bag1.entry(&t.value).or_insert(0) += 1;
    }
    for t in tokens2 {
        *bag2.entry(&t.value).or_insert(0) += 1;
    }

    // Compute intersection size (sum of min counts for each token)
    let mut intersection = 0usize;
    for (token, &count1) in &bag1 {
        if let Some(&count2) = bag2.get(token) {
            intersection += count1.min(count2);
        }
    }

    // Dice coefficient: 2 * intersection / (size1 + size2)
    // Using tokens.len() is correct because Vec length counts duplicates
    let size1 = tokens1.len();
    let size2 = tokens2.len();

    (2.0 * intersection as f64) / (size1 + size2) as f64
}

/// Verify that a hash match is a real clone (not just a hash collision)
///
/// This is CRITICAL for S8-P1-T2: Hash collisions will produce false positives
/// if not verified by comparing actual token sequences.
///
/// # Arguments
/// * `tokens1` - First token sequence
/// * `tokens2` - Second token sequence
/// * `threshold` - Minimum similarity threshold (e.g., 0.7 for Type-3)
///
/// # Returns
/// * `Some(similarity)` if similarity >= threshold (real clone)
/// * `None` if similarity < threshold (hash collision, not a clone)
pub fn verify_clone_match(
    tokens1: &[NormalizedToken],
    tokens2: &[NormalizedToken],
    threshold: f64,
) -> Option<f64> {
    let similarity = compute_dice_similarity(tokens1, tokens2);
    if similarity >= threshold {
        Some(similarity)
    } else {
        None // Hash collision, not a real clone
    }
}

/// Find verified clones from a hash index
///
/// After finding hash matches in the index, this function verifies each candidate
/// pair by comparing actual token sequences to filter out hash collisions.
///
/// # Arguments
/// * `index` - The hash index with candidate clone locations
/// * `file_sequences` - Token sequences for each file (file_idx -> sequences)
/// * `threshold` - Minimum similarity threshold
///
/// # Returns
/// * Vector of verified clone pairs: (file1_idx, seq1_idx, file2_idx, seq2_idx, similarity)
pub fn find_verified_clones(
    index: &HashIndex,
    file_sequences: &[Vec<TokenSequence>],
    threshold: f64,
) -> Vec<(usize, usize, usize, usize, f64)> {
    let mut verified = Vec::new();

    // Get all candidate pairs from hash collisions
    let candidates = index.find_candidates();

    for (entry1, entry2) in candidates {
        // Get the actual token sequences
        let seq1 = file_sequences
            .get(entry1.file_idx)
            .and_then(|seqs| seqs.get(entry1.start_pos));
        let seq2 = file_sequences
            .get(entry2.file_idx)
            .and_then(|seqs| seqs.get(entry2.start_pos));

        if let (Some(seq1), Some(seq2)) = (seq1, seq2) {
            // Verify by comparing actual tokens
            if let Some(similarity) = verify_clone_match(&seq1.tokens, &seq2.tokens, threshold) {
                verified.push((
                    entry1.file_idx,
                    entry1.start_pos,
                    entry2.file_idx,
                    entry2.start_pos,
                    similarity,
                ));
            }
        }
    }

    verified
}

// =============================================================================
// Normalization Functions
// =============================================================================

/// Normalize tokens from source code
///
/// # Arguments
/// * `source` - Raw source code
/// * `language` - Language name (python, typescript, go, rust)
/// * `mode` - Normalization mode
///
/// # Returns
/// * Vector of normalized tokens
pub fn normalize_tokens(
    source: &str,
    language: &str,
    mode: NormalizationMode,
) -> anyhow::Result<Vec<NormalizedToken>> {
    let lang: Language = language
        .parse()
        .map_err(|_| anyhow::anyhow!("Unsupported language: {}", language))?;

    let tree = parse(source, lang)?;

    // Check for parse errors before tokenizing (S8-P2-T11)
    if has_parse_errors(&tree) {
        return Err(anyhow::anyhow!(
            "File has parse errors, skipping tokenization"
        ));
    }

    let tokens = extract_tokens_from_ast(&tree, source.as_bytes(), language);
    Ok(apply_normalization(tokens, mode))
}

/// Compute rolling hashes for a token sequence
///
/// # Arguments
/// * `tokens` - Normalized tokens
/// * `window_size` - Hash window size (minimum clone size in tokens)
///
/// # Returns
/// * Vector of (hash, position) pairs where position is the start index of the window
///
/// # Algorithm
///
/// Uses Rabin-Karp rolling hash:
/// 1. If tokens.len() < window_size, return empty vec
/// 2. Initialize hash for first window [0..window_size)
/// 3. Roll through remaining positions, updating hash in O(1) per position
///
/// # Example
///
/// ```ignore
/// let tokens = vec![make_token("a"), make_token("b"), make_token("c"), make_token("d")];
/// let hashes = compute_rolling_hashes(&tokens, 2);
/// // Returns: [(hash_ab, 0), (hash_bc, 1), (hash_cd, 2)]
/// ```
pub fn compute_rolling_hashes(tokens: &[NormalizedToken], window_size: usize) -> Vec<(u64, usize)> {
    // Edge case: not enough tokens for even one window
    if tokens.len() < window_size || window_size == 0 {
        return vec![];
    }

    // Pre-allocate result vector
    let num_windows = tokens.len() - window_size + 1;
    let mut result = Vec::with_capacity(num_windows);

    // Create rolling hash
    let mut hasher = RollingHash::new(window_size);

    // Compute hash for first window [0..window_size)
    for token in tokens.iter().take(window_size) {
        hasher.push(hash_token(token));
    }
    result.push((hasher.current(), 0));

    // Roll through remaining positions
    for i in 1..num_windows {
        // Remove token at (i-1), add token at (i + window_size - 1)
        let old_token_hash = hash_token(&tokens[i - 1]);
        let new_token_hash = hash_token(&tokens[i + window_size - 1]);
        hasher.roll(old_token_hash, new_token_hash);
        result.push((hasher.current(), i));
    }

    result
}

/// Hash a single token
pub fn hash_token(token: &NormalizedToken) -> u64 {
    // Simple hash: sum of char values with position weighting
    let mut h = 0u64;
    for (i, c) in token.value.chars().enumerate() {
        h = h.wrapping_add((c as u64).wrapping_mul(31u64.wrapping_pow(i as u32)));
    }
    h
}

// =============================================================================
// Tokenization Pipeline (Phase 2)
// =============================================================================

/// Check if a tree has parse errors (S8-P2-T11)
///
/// Files with parse errors should be skipped to avoid incorrect clone detection.
pub fn has_parse_errors(tree: &Tree) -> bool {
    // Count ERROR nodes vs total named nodes. C/C++ files with preprocessor
    // macros commonly have partial parse errors -- only reject if >50% errors.
    let root = tree.root_node();
    if !root.has_error() {
        return false;
    }
    let total = root.named_child_count().max(1);
    let errors = (0..root.named_child_count())
        .filter(|&i| {
            root.named_child(i)
                .is_some_and(|c| c.kind() == "ERROR" || c.is_error())
        })
        .count();
    errors * 2 > total // >50% error nodes = reject
}

/// Extract tokens from a tree-sitter AST
///
/// Walks the AST recursively, extracting leaf nodes as tokens.
/// Handles language-specific cases including:
/// - Python f-strings (S8-P2-T1)
/// - TypeScript template literals (S8-P2-T2)
/// - Rust macros (S8-P2-T3)
/// - Go identifier variants (S8-P2-T4)
fn extract_tokens_from_ast(tree: &Tree, source: &[u8], language: &str) -> Vec<NormalizedToken> {
    let mut tokens = Vec::new();
    let root = tree.root_node();
    extract_tokens_recursive(&root, source, language, &mut tokens);
    tokens
}

/// Recursively extract tokens from a node and its children
fn extract_tokens_recursive(
    node: &tree_sitter::Node,
    source: &[u8],
    language: &str,
    tokens: &mut Vec<NormalizedToken>,
) {
    let kind = node.kind();

    // Skip comment nodes
    if is_comment_node(kind, language) {
        return;
    }

    // Handle special cases that need recursion into children
    match language {
        "python" => {
            // S8-P2-T1: Python f-strings - recurse into interpolation nodes
            if kind == "interpolation" {
                // Recurse into children to extract embedded identifiers
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    extract_tokens_recursive(&child, source, language, tokens);
                }
                return;
            }
        }
        "typescript" | "javascript" => {
            // S8-P2-T2: TypeScript template literals - recurse into template_substitution
            if kind == "template_substitution" {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    extract_tokens_recursive(&child, source, language, tokens);
                }
                return;
            }
        }
        _ => {}
    }

    // If this is a leaf node (no children) or a node type we want to capture
    if node.child_count() == 0 || should_capture_as_token(kind, language) {
        if let Ok(text) = node.utf8_text(source) {
            let text = text.trim();
            if !text.is_empty() && !is_whitespace_only(text) {
                let category = categorize_token(kind, language);
                tokens.push(NormalizedToken {
                    value: text.to_string(),
                    original: text.to_string(),
                    category,
                });
            }
        }
    }

    // Recurse into children for non-leaf nodes
    if node.child_count() > 0 && !should_capture_as_token(kind, language) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            extract_tokens_recursive(&child, source, language, tokens);
        }
    }
}

/// Check if a string is only whitespace
fn is_whitespace_only(s: &str) -> bool {
    s.chars().all(|c| c.is_whitespace())
}

/// Check if a node kind should be captured as a single token
/// (vs recursing into its children)
fn should_capture_as_token(kind: &str, language: &str) -> bool {
    match language {
        "rust" => {
            // S8-P2-T3: Rust macros - capture macro_invocation as single token
            // to preserve macro name (don't normalize)
            kind == "macro_invocation"
        }
        _ => false,
    }
}

/// Check if a node kind is a comment
pub fn is_comment_node(kind: &str, language: &str) -> bool {
    match language {
        "python" => matches!(kind, "comment"),
        "typescript" | "javascript" => matches!(kind, "comment" | "jsx_comment"),
        "go" => matches!(kind, "comment"),
        "rust" => matches!(kind, "line_comment" | "block_comment"),
        "java" => matches!(kind, "comment" | "line_comment" | "block_comment"),
        _ => kind.contains("comment"),
    }
}

/// Categorize a token based on its AST node kind
///
/// Maps tree-sitter node kinds to TokenCategory for normalization.
/// Language-specific handling for:
/// - Python: identifier, string, integer, float
/// - TypeScript: identifier, property_identifier, string, number
/// - Go: identifier, type_identifier, package_identifier, field_identifier (S8-P2-T4)
/// - Rust: identifier, macro_invocation (S8-P2-T3)
pub fn categorize_token(kind: &str, language: &str) -> TokenCategory {
    match language {
        "python" => categorize_python_token(kind),
        "typescript" | "javascript" => categorize_typescript_token(kind),
        "go" => categorize_go_token(kind),
        "rust" => categorize_rust_token(kind),
        "java" => categorize_java_token(kind),
        _ => TokenCategory::Other,
    }
}

/// Categorize Python tokens
fn categorize_python_token(kind: &str) -> TokenCategory {
    match kind {
        "identifier" => TokenCategory::Identifier,
        "string" | "string_content" | "string_start" | "string_end" => TokenCategory::StringLiteral,
        "integer" | "float" => TokenCategory::NumericLiteral,
        // Python keywords
        "def" | "class" | "if" | "elif" | "else" | "for" | "while" | "try" | "except"
        | "finally" | "with" | "as" | "import" | "from" | "return" | "yield" | "raise" | "pass"
        | "break" | "continue" | "and" | "or" | "not" | "in" | "is" | "lambda" | "global"
        | "nonlocal" | "assert" | "del" | "True" | "False" | "None" | "async" | "await" => {
            TokenCategory::Keyword
        }
        // Operators
        "+" | "-" | "*" | "/" | "//" | "%" | "**" | "@" | "&" | "|" | "^" | "~" | "<<" | ">>"
        | "<" | ">" | "<=" | ">=" | "==" | "!=" | "=" | "+=" | "-=" | "*=" | "/=" | "//="
        | "%=" | "**=" | "&=" | "|=" | "^=" | "<<=" | ">>=" | "@=" => TokenCategory::Operator,
        // Punctuation
        "(" | ")" | "[" | "]" | "{" | "}" | "," | ":" | "." | ";" | "->" => {
            TokenCategory::Punctuation
        }
        _ => TokenCategory::Other,
    }
}

/// Categorize TypeScript/JavaScript tokens
fn categorize_typescript_token(kind: &str) -> TokenCategory {
    match kind {
        "identifier"
        | "property_identifier"
        | "shorthand_property_identifier"
        | "shorthand_property_identifier_pattern" => TokenCategory::Identifier,
        "string" | "template_string" | "string_fragment" | "template_fragment" => {
            TokenCategory::StringLiteral
        }
        "number" => TokenCategory::NumericLiteral,
        // TypeScript/JavaScript keywords
        "function" | "class" | "if" | "else" | "for" | "while" | "do" | "switch" | "case"
        | "default" | "try" | "catch" | "finally" | "throw" | "return" | "break" | "continue"
        | "const" | "let" | "var" | "new" | "delete" | "typeof" | "instanceof" | "in" | "of"
        | "void" | "this" | "super" | "import" | "export" | "from" | "as" | "async" | "await"
        | "yield" | "true" | "false" | "null" | "undefined" | "interface" | "type" | "enum"
        | "implements" | "extends" | "public" | "private" | "protected" | "static" | "readonly"
        | "abstract" => TokenCategory::Keyword,
        // Operators
        "+" | "-" | "*" | "/" | "%" | "**" | "&" | "|" | "^" | "~" | "<<" | ">>" | ">>>" | "<"
        | ">" | "<=" | ">=" | "==" | "===" | "!=" | "!==" | "=" | "+=" | "-=" | "*=" | "/="
        | "%=" | "**=" | "&=" | "|=" | "^=" | "<<=" | ">>=" | ">>>=" | "&&" | "||" | "!" | "??"
        | "?." | "?" | "=>" => TokenCategory::Operator,
        // Punctuation
        "(" | ")" | "[" | "]" | "{" | "}" | "," | ":" | "." | ";" | "`" | "${" => {
            TokenCategory::Punctuation
        }
        _ => TokenCategory::Other,
    }
}

/// Categorize Go tokens (S8-P2-T4: handle all identifier types)
fn categorize_go_token(kind: &str) -> TokenCategory {
    match kind {
        // S8-P2-T4: Go has multiple identifier types - all are identifiers
        "identifier" | "type_identifier" | "package_identifier" | "field_identifier" => {
            TokenCategory::Identifier
        }
        "raw_string_literal" | "interpreted_string_literal" | "rune_literal" => {
            TokenCategory::StringLiteral
        }
        "int_literal" | "float_literal" | "imaginary_literal" => TokenCategory::NumericLiteral,
        // Go keywords
        "break" | "case" | "chan" | "const" | "continue" | "default" | "defer" | "else"
        | "fallthrough" | "for" | "func" | "go" | "goto" | "if" | "import" | "interface"
        | "map" | "package" | "range" | "return" | "select" | "struct" | "switch" | "type"
        | "var" | "true" | "false" | "nil" | "iota" => TokenCategory::Keyword,
        // Operators
        "+" | "-" | "*" | "/" | "%" | "&" | "|" | "^" | "<<" | ">>" | "&^" | "+=" | "-=" | "*="
        | "/=" | "%=" | "&=" | "|=" | "^=" | "<<=" | ">>=" | "&^=" | "&&" | "||" | "<-" | "++"
        | "--" | "==" | "<" | ">" | "=" | "!" | "!=" | "<=" | ">=" | ":=" | "..." => {
            TokenCategory::Operator
        }
        // Punctuation
        "(" | ")" | "[" | "]" | "{" | "}" | "," | ":" | "." | ";" => TokenCategory::Punctuation,
        _ => TokenCategory::Other,
    }
}

/// Categorize Rust tokens (S8-P2-T3: preserve macro names)
fn categorize_rust_token(kind: &str) -> TokenCategory {
    match kind {
        "identifier" | "type_identifier" | "field_identifier" | "scoped_identifier" => {
            TokenCategory::Identifier
        }
        // S8-P2-T3: Rust macros - keep as Other (not normalized)
        // This ensures macro names like println!, vec! are preserved
        "macro_invocation" => TokenCategory::Other,
        "string_literal" | "raw_string_literal" | "char_literal" => TokenCategory::StringLiteral,
        "integer_literal" | "float_literal" => TokenCategory::NumericLiteral,
        // Rust keywords
        "as" | "async" | "await" | "break" | "const" | "continue" | "crate" | "dyn" | "else"
        | "enum" | "extern" | "false" | "fn" | "for" | "if" | "impl" | "in" | "let" | "loop"
        | "match" | "mod" | "move" | "mut" | "pub" | "ref" | "return" | "self" | "Self"
        | "static" | "struct" | "super" | "trait" | "true" | "type" | "unsafe" | "use"
        | "where" | "while" => TokenCategory::Keyword,
        // Operators
        "+" | "-" | "*" | "/" | "%" | "&" | "|" | "^" | "!" | "<<" | ">>" | "+=" | "-=" | "*="
        | "/=" | "%=" | "&=" | "|=" | "^=" | "<<=" | ">>=" | "==" | "!=" | "<" | ">" | "<="
        | ">=" | "=" | "&&" | "||" | ".." | "..=" | "->" | "=>" | "::" | "?" => {
            TokenCategory::Operator
        }
        // Punctuation
        "(" | ")" | "[" | "]" | "{" | "}" | "," | ":" | "." | ";" | "#" | "'" => {
            TokenCategory::Punctuation
        }
        _ => TokenCategory::Other,
    }
}

/// Categorize Java tokens
fn categorize_java_token(kind: &str) -> TokenCategory {
    match kind {
        "identifier" | "type_identifier" => TokenCategory::Identifier,
        "string_literal" | "character_literal" => TokenCategory::StringLiteral,
        "decimal_integer_literal"
        | "hex_integer_literal"
        | "octal_integer_literal"
        | "binary_integer_literal"
        | "decimal_floating_point_literal"
        | "hex_floating_point_literal" => TokenCategory::NumericLiteral,
        // Java keywords
        "abstract" | "assert" | "boolean" | "break" | "byte" | "case" | "catch" | "char"
        | "class" | "const" | "continue" | "default" | "do" | "double" | "else" | "enum"
        | "extends" | "final" | "finally" | "float" | "for" | "goto" | "if" | "implements"
        | "import" | "instanceof" | "int" | "interface" | "long" | "native" | "new" | "package"
        | "private" | "protected" | "public" | "return" | "short" | "static" | "strictfp"
        | "super" | "switch" | "synchronized" | "this" | "throw" | "throws" | "transient"
        | "try" | "void" | "volatile" | "while" | "true" | "false" | "null" => {
            TokenCategory::Keyword
        }
        // Operators
        "+" | "-" | "*" | "/" | "%" | "&" | "|" | "^" | "~" | "<<" | ">>" | ">>>" | "<" | ">"
        | "<=" | ">=" | "==" | "!=" | "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|="
        | "^=" | "<<=" | ">>=" | ">>>=" | "&&" | "||" | "!" | "++" | "--" | "?" | ":" | "->"
        | "::" => TokenCategory::Operator,
        // Punctuation
        "(" | ")" | "[" | "]" | "{" | "}" | "," | "." | ";" | "@" => TokenCategory::Punctuation,
        _ => TokenCategory::Other,
    }
}

/// Apply normalization to tokens based on NormalizationMode
pub fn apply_normalization(
    tokens: Vec<NormalizedToken>,
    mode: NormalizationMode,
) -> Vec<NormalizedToken> {
    if mode == NormalizationMode::None {
        return tokens;
    }

    tokens
        .into_iter()
        .map(|token| normalize_single_token(token, mode))
        .collect()
}

/// Normalize a single token based on its category and the normalization mode
fn normalize_single_token(token: NormalizedToken, mode: NormalizationMode) -> NormalizedToken {
    let normalized_value = match token.category {
        TokenCategory::Identifier if mode.normalize_identifiers() => "$ID".to_string(),
        TokenCategory::StringLiteral if mode.normalize_literals() => "$STR".to_string(),
        TokenCategory::NumericLiteral if mode.normalize_literals() => "$NUM".to_string(),
        // Keywords, Operators, Punctuation, Other are NEVER normalized
        _ => token.value.clone(),
    };

    NormalizedToken {
        value: normalized_value,
        original: token.original,
        category: token.category,
    }
}

/// Check if a file appears to be auto-generated
///
/// This function is used by `--exclude-generated` to filter out generated files.
///
/// Common generated file patterns:
/// - Protobuf: *.pb.go, *_pb2.py, *.pb.ts
/// - GraphQL: *.generated.ts, *.graphql.ts
/// - OpenAPI/Swagger: *_client.go, *_server.go (when in /gen/ or /generated/)
/// - General: *_generated.*, *.gen.*, *Generated.*
/// - Vendor: files in vendor/, node_modules/, __pycache__/
/// - Build: files in dist/, build/, target/
pub fn is_generated_file(path: &Path) -> bool {
    let path_str = path.to_string_lossy();
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

    // Check directory patterns (vendor, build artifacts, etc.)
    // Match both /dir/ in middle and dir/ at start
    let skip_dirs = [
        "vendor/",
        "node_modules/",
        "__pycache__/",
        "dist/",
        "build/",
        "target/",
        "gen/",
        "generated/",
        ".gen/",
        "third_party/",
        "external/",
    ];
    for dir in skip_dirs {
        // Match: starts with dir/, or contains /dir/
        if path_str.starts_with(dir) || path_str.contains(&format!("/{}", dir)) {
            return true;
        }
    }

    // Check file name patterns
    let generated_patterns = [
        // Protobuf
        ".pb.go",
        "_pb2.py",
        ".pb.ts",
        ".pb.js",
        ".pb.rs",
        "_grpc.pb.go",
        "_pb2_grpc.py",
        // GraphQL / Codegen
        ".generated.ts",
        ".generated.tsx",
        ".generated.js",
        ".graphql.ts",
        ".graphql.tsx",
        // General generated
        "_generated.go",
        "_generated.ts",
        "_generated.rs",
        "_generated.py",
        ".gen.go",
        ".gen.ts",
        ".gen.rs",
        // Mock/Test generated
        "_mock.go",
        "_mocks.go",
        // Thrift
        ".thrift.go",
        // FlatBuffers
        "_generated.rs",
        "_generated.go",
    ];
    for pattern in generated_patterns {
        if file_name.ends_with(pattern) {
            return true;
        }
    }

    // Check file name prefixes/patterns
    let generated_names = [
        "generated_",
        "auto_generated",
        "autogenerated",
        "mock_",
        "mocks_",
    ];
    let file_lower = file_name.to_lowercase();
    for name in generated_names {
        if file_lower.starts_with(name) {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_with_preview_ascii_short() {
        // Short ASCII string should be kept as-is
        let frag = CloneFragment::new(PathBuf::from("test.rs"), 1, 10, 50)
            .with_preview("hello world".to_string());
        assert_eq!(frag.preview.as_deref(), Some("hello world"));
    }

    #[test]
    fn test_with_preview_ascii_exactly_100() {
        // Exactly 100 ASCII chars should not be truncated
        let s = "a".repeat(100);
        let frag = CloneFragment::new(PathBuf::from("test.rs"), 1, 10, 50).with_preview(s.clone());
        assert_eq!(frag.preview.as_deref(), Some(s.as_str()));
    }

    #[test]
    fn test_with_preview_ascii_over_100() {
        // Over 100 ASCII chars should be truncated to 97 + "..."
        let s = "a".repeat(120);
        let frag = CloneFragment::new(PathBuf::from("test.rs"), 1, 10, 50).with_preview(s.clone());
        let expected = format!("{}...", "a".repeat(97));
        assert_eq!(frag.preview.as_deref(), Some(expected.as_str()));
        assert!(frag.preview.as_ref().unwrap().len() <= 100);
    }

    #[test]
    fn test_with_preview_multibyte_utf8_no_panic() {
        // Multi-byte UTF-8 chars where byte offset 97 falls mid-character.
        // Each CJK character is 3 bytes, so 40 chars = 120 bytes.
        // Byte 97 falls in the middle of char 33 (bytes 96..99), so naive
        // &preview[..97] would panic.
        let s: String = std::iter::repeat_n('\u{4e16}', 40).collect(); // CJK "world" char
        assert_eq!(s.len(), 120); // 40 * 3 = 120 bytes
        assert!(!s.is_char_boundary(97)); // Byte 97 is NOT a char boundary

        let frag = CloneFragment::new(PathBuf::from("test.rs"), 1, 10, 50).with_preview(s);
        // Should not panic and should end with "..."
        let preview = frag.preview.unwrap();
        assert!(preview.ends_with("..."));
        assert!(preview.len() <= 100);
    }

    #[test]
    fn test_with_preview_2byte_utf8_no_panic() {
        // 2-byte UTF-8 chars (e.g., accented Latin chars)
        // Each char is 2 bytes, so 60 chars = 120 bytes.
        // Byte 97 is odd, falls in the middle of a 2-byte char.
        let s: String = std::iter::repeat_n('\u{00e9}', 60).collect(); // e-acute
        assert_eq!(s.len(), 120);
        assert!(!s.is_char_boundary(97)); // Byte 97 is NOT a char boundary

        let frag = CloneFragment::new(PathBuf::from("test.rs"), 1, 10, 50).with_preview(s);
        let preview = frag.preview.unwrap();
        assert!(preview.ends_with("..."));
        assert!(preview.len() <= 100);
    }

    #[test]
    fn test_with_preview_4byte_utf8_no_panic() {
        // 4-byte UTF-8 chars (emoji)
        // Each char is 4 bytes, so 30 chars = 120 bytes.
        // Byte 97 = 4*24 + 1 = mid-char
        let s: String = std::iter::repeat_n('\u{1F600}', 30).collect(); // grinning face
        assert_eq!(s.len(), 120);
        assert!(!s.is_char_boundary(97)); // Byte 97 is NOT a char boundary

        let frag = CloneFragment::new(PathBuf::from("test.rs"), 1, 10, 50).with_preview(s);
        let preview = frag.preview.unwrap();
        assert!(preview.ends_with("..."));
        // Total length should be reasonable (may exceed 100 bytes slightly
        // due to char boundary rounding, but should be close)
    }

    #[test]
    fn test_with_preview_empty_string() {
        let frag =
            CloneFragment::new(PathBuf::from("test.rs"), 1, 10, 50).with_preview(String::new());
        assert_eq!(frag.preview.as_deref(), Some(""));
    }

    #[test]
    fn test_with_preview_mixed_ascii_and_multibyte() {
        // Mix of ASCII and multi-byte to create a boundary edge case
        let mut s = "a".repeat(95);
        s.push('\u{4e16}'); // 3-byte char at byte offset 95..98
        s.push_str("more text after");
        // Total > 100 bytes, and byte 97 is mid-character
        assert!(s.len() > 100);
        assert!(!s.is_char_boundary(97));

        let frag = CloneFragment::new(PathBuf::from("test.rs"), 1, 10, 50).with_preview(s);
        let preview = frag.preview.unwrap();
        assert!(preview.ends_with("..."));
    }
}
