//! Shared metric types for code analysis (Session 15)
//!
//! This module defines the core data structures used across all metrics commands:
//! - LOC (Lines of Code)
//! - Cognitive Complexity
//! - Halstead Metrics
//! - Hotspots
//! - Coverage
//!
//! All types implement Serialize/Deserialize for JSON output.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// =============================================================================
// Lines of Code (LOC) Types
// =============================================================================

/// Lines of code breakdown for a single file or aggregated result.
///
/// Invariant: `code_lines + comment_lines + blank_lines == total_lines`
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocInfo {
    /// Lines containing executable code (excluding pure comments/blanks)
    pub code_lines: usize,
    /// Lines containing only comments (including inline comments that are comment-only)
    pub comment_lines: usize,
    /// Empty lines or lines with only whitespace
    pub blank_lines: usize,
    /// Total lines in the file
    pub total_lines: usize,
}

impl LocInfo {
    /// Create a new LocInfo with the given counts.
    pub fn new(code_lines: usize, comment_lines: usize, blank_lines: usize) -> Self {
        Self {
            code_lines,
            comment_lines,
            blank_lines,
            total_lines: code_lines + comment_lines + blank_lines,
        }
    }

    /// Check the invariant: total == code + comment + blank
    pub fn is_valid(&self) -> bool {
        self.total_lines == self.code_lines + self.comment_lines + self.blank_lines
    }

    /// Calculate code percentage (0.0 - 100.0)
    pub fn code_percentage(&self) -> f64 {
        if self.total_lines == 0 {
            0.0
        } else {
            (self.code_lines as f64 / self.total_lines as f64) * 100.0
        }
    }

    /// Calculate comment percentage (0.0 - 100.0)
    pub fn comment_percentage(&self) -> f64 {
        if self.total_lines == 0 {
            0.0
        } else {
            (self.comment_lines as f64 / self.total_lines as f64) * 100.0
        }
    }

    /// Merge another LocInfo into this one (for aggregation).
    pub fn merge(&mut self, other: &LocInfo) {
        self.code_lines += other.code_lines;
        self.comment_lines += other.comment_lines;
        self.blank_lines += other.blank_lines;
        self.total_lines += other.total_lines;
    }
}

// =============================================================================
// Cognitive Complexity Types
// =============================================================================

/// Cognitive complexity information for a function.
///
/// Based on SonarSource's Cognitive Complexity whitepaper.
/// Invariants:
/// - `score >= 0`
/// - `score >= nesting_penalty` (base increments are non-negative)
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CognitiveInfo {
    /// Total cognitive complexity score
    pub score: u32,
    /// Cumulative nesting penalty (subset of total score)
    pub nesting_penalty: u32,
    /// List of threshold violations (if score exceeds thresholds)
    pub threshold_violations: Vec<ThresholdViolation>,
    /// Detailed contributors to the score (optional, for --show-contributors)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contributors: Option<Vec<CognitiveContributor>>,
}

impl CognitiveInfo {
    /// Create a new CognitiveInfo with score and nesting penalty.
    pub fn new(score: u32, nesting_penalty: u32) -> Self {
        Self {
            score,
            nesting_penalty,
            threshold_violations: Vec::new(),
            contributors: None,
        }
    }

    /// Check if the score exceeds a threshold.
    pub fn exceeds_threshold(&self, threshold: u32) -> bool {
        self.score > threshold
    }

    /// Get the base increment (score minus nesting penalty).
    pub fn base_increment(&self) -> u32 {
        self.score.saturating_sub(self.nesting_penalty)
    }

    /// Check invariants.
    pub fn is_valid(&self) -> bool {
        self.score >= self.nesting_penalty
    }
}

/// A threshold violation for cognitive complexity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThresholdViolation {
    /// Threshold level name (e.g., "warning", "high", "severe")
    pub level: String,
    /// Threshold value that was exceeded
    pub threshold: u32,
    /// Actual score
    pub actual: u32,
}

/// A single contributor to cognitive complexity score.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CognitiveContributor {
    /// Line number where the construct appears
    pub line: u32,
    /// Type of construct (e.g., "if", "for", "&&", "else if")
    pub construct: String,
    /// Base increment for this construct
    pub base_increment: u32,
    /// Nesting increment for this construct
    pub nesting_increment: u32,
    /// Current nesting level when this construct was encountered
    pub nesting_level: u32,
}

// =============================================================================
// Halstead Metrics Types
// =============================================================================

/// Halstead software science metrics for a function or file.
///
/// Based on Maurice Halstead's software metrics.
/// Invariants:
/// - `vocabulary == n1 + n2`
/// - `length == N1 + N2`
/// - `effort == difficulty * volume` (approximately, due to floating point)
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HalsteadInfo {
    /// Number of distinct operators (n1)
    pub n1: usize,
    /// Number of distinct operands (n2)
    pub n2: usize,
    /// Total number of operators (N1)
    #[serde(rename = "N1")]
    pub big_n1: usize,
    /// Total number of operands (N2)
    #[serde(rename = "N2")]
    pub big_n2: usize,
    /// Program vocabulary: n1 + n2
    pub vocabulary: usize,
    /// Program length: N1 + N2
    pub length: usize,
    /// Program volume: length * log2(vocabulary)
    pub volume: f64,
    /// Program difficulty: (n1/2) * (N2/n2)
    pub difficulty: f64,
    /// Program effort: difficulty * volume
    pub effort: f64,
    /// Estimated time to program (seconds): effort / 18
    pub time: f64,
    /// Estimated number of delivered bugs: volume / 3000
    pub bugs: f64,
}

impl HalsteadInfo {
    /// Create HalsteadInfo from raw counts.
    ///
    /// Automatically calculates derived metrics.
    /// Handles edge case where n2=0 by capping difficulty at 1000.
    pub fn from_counts(n1: usize, n2: usize, big_n1: usize, big_n2: usize) -> Self {
        let vocabulary = n1 + n2;
        let length = big_n1 + big_n2;

        // Volume = length * log2(vocabulary)
        // For empty functions (vocabulary=0), use volume=1 to avoid log(0)
        let volume = if vocabulary == 0 {
            1.0
        } else {
            length as f64 * (vocabulary as f64).log2()
        };

        // Difficulty = (n1/2) * (N2/n2)
        // Cap at 1000 when n2=0 (per spec)
        let difficulty = if n2 == 0 {
            1000.0
        } else {
            (n1 as f64 / 2.0) * (big_n2 as f64 / n2 as f64)
        };

        // Derived metrics
        let effort = difficulty * volume;
        let time = effort / 18.0;
        let bugs = volume / 3000.0;

        Self {
            n1,
            n2,
            big_n1,
            big_n2,
            vocabulary,
            length,
            volume,
            difficulty,
            effort,
            time,
            bugs,
        }
    }

    /// Check invariants (allowing for floating point tolerance).
    pub fn is_valid(&self) -> bool {
        self.vocabulary == self.n1 + self.n2 && self.length == self.big_n1 + self.big_n2
    }
}

// =============================================================================
// Hotspots Types
// =============================================================================

/// Trend direction for hotspot analysis.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HotspotTrend {
    /// Complexity increasing over time
    Increasing,
    /// Complexity stable
    Stable,
    /// Complexity decreasing over time
    Decreasing,
    /// Not enough history to determine trend
    #[default]
    Unknown,
}

/// Hotspot information combining git churn with complexity.
///
/// Invariant: `0.0 <= hotspot_score <= 1.0`
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HotspotInfo {
    /// File path (relative to project root)
    pub file: PathBuf,
    /// Function name (if function-level granularity)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<String>,
    /// Normalized churn score (0.0 - 1.0)
    pub churn_score: f64,
    /// Normalized complexity score (0.0 - 1.0)
    pub complexity_score: f64,
    /// Combined hotspot score: churn_score * complexity_score (0.0 - 1.0)
    pub hotspot_score: f64,
    /// Trend direction (if history available)
    pub trend: HotspotTrend,
    /// Raw churn count (number of commits touching this file/function)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_churn: Option<u32>,
    /// Raw complexity score (cognitive or cyclomatic)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_complexity: Option<u32>,
}

impl HotspotInfo {
    /// Create a new HotspotInfo.
    pub fn new(
        file: PathBuf,
        function: Option<String>,
        churn_score: f64,
        complexity_score: f64,
    ) -> Self {
        let hotspot_score = churn_score * complexity_score;
        Self {
            file,
            function,
            churn_score,
            complexity_score,
            hotspot_score,
            trend: HotspotTrend::Unknown,
            raw_churn: None,
            raw_complexity: None,
        }
    }

    /// Check invariant: scores are in valid range.
    pub fn is_valid(&self) -> bool {
        (0.0..=1.0).contains(&self.churn_score)
            && (0.0..=1.0).contains(&self.complexity_score)
            && (0.0..=1.0).contains(&self.hotspot_score)
    }
}

// =============================================================================
// Coverage Types
// =============================================================================

/// Code coverage information parsed from coverage reports.
///
/// Invariants:
/// - `0.0 <= line_coverage <= 100.0`
/// - `0.0 <= branch_coverage <= 100.0` (if present)
/// - `0.0 <= function_coverage <= 100.0` (if present)
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CoverageInfo {
    /// Line coverage percentage (0.0 - 100.0)
    pub line_coverage: f64,
    /// Branch coverage percentage (None if not available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_coverage: Option<f64>,
    /// Function coverage percentage (None if not available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_coverage: Option<f64>,
    /// List of uncovered line numbers
    pub uncovered_lines: Vec<u32>,
    /// List of uncovered function names
    pub uncovered_functions: Vec<String>,
    /// Total lines instrumented
    pub total_lines: usize,
    /// Lines covered
    pub covered_lines: usize,
    /// Total branches (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_branches: Option<usize>,
    /// Branches covered (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub covered_branches: Option<usize>,
}

impl CoverageInfo {
    /// Create a new CoverageInfo from line counts.
    pub fn from_line_counts(covered: usize, total: usize, uncovered_lines: Vec<u32>) -> Self {
        let line_coverage = if total == 0 {
            100.0 // 0/0 is considered 100% coverage
        } else {
            (covered as f64 / total as f64) * 100.0
        };

        Self {
            line_coverage,
            branch_coverage: None,
            function_coverage: None,
            uncovered_lines,
            uncovered_functions: Vec::new(),
            total_lines: total,
            covered_lines: covered,
            total_branches: None,
            covered_branches: None,
        }
    }

    /// Check invariants.
    pub fn is_valid(&self) -> bool {
        let line_valid = (0.0..=100.0).contains(&self.line_coverage);
        let branch_valid = self
            .branch_coverage
            .map(|b| (0.0..=100.0).contains(&b))
            .unwrap_or(true);
        let func_valid = self
            .function_coverage
            .map(|f| (0.0..=100.0).contains(&f))
            .unwrap_or(true);

        line_valid && branch_valid && func_valid
    }

    /// Calculate line coverage from counts.
    pub fn calculate_line_coverage(&self) -> f64 {
        if self.total_lines == 0 {
            100.0
        } else {
            (self.covered_lines as f64 / self.total_lines as f64) * 100.0
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // LocInfo Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_loc_info_new() {
        let loc = LocInfo::new(100, 20, 10);
        assert_eq!(loc.code_lines, 100);
        assert_eq!(loc.comment_lines, 20);
        assert_eq!(loc.blank_lines, 10);
        assert_eq!(loc.total_lines, 130);
        assert!(loc.is_valid());
    }

    #[test]
    fn test_loc_info_percentages() {
        let loc = LocInfo::new(80, 10, 10);
        assert!((loc.code_percentage() - 80.0).abs() < 0.01);
        assert!((loc.comment_percentage() - 10.0).abs() < 0.01);
    }

    #[test]
    fn test_loc_info_empty() {
        let loc = LocInfo::default();
        assert_eq!(loc.code_percentage(), 0.0);
        assert!(loc.is_valid());
    }

    #[test]
    fn test_loc_info_merge() {
        let mut loc1 = LocInfo::new(100, 20, 10);
        let loc2 = LocInfo::new(50, 10, 5);
        loc1.merge(&loc2);
        assert_eq!(loc1.code_lines, 150);
        assert_eq!(loc1.comment_lines, 30);
        assert_eq!(loc1.blank_lines, 15);
        assert_eq!(loc1.total_lines, 195);
        assert!(loc1.is_valid());
    }

    #[test]
    fn test_loc_info_serialization() {
        let loc = LocInfo::new(100, 20, 10);
        let json = serde_json::to_string(&loc).unwrap();
        let parsed: LocInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(loc, parsed);
    }

    // -------------------------------------------------------------------------
    // CognitiveInfo Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_cognitive_info_new() {
        let cog = CognitiveInfo::new(15, 5);
        assert_eq!(cog.score, 15);
        assert_eq!(cog.nesting_penalty, 5);
        assert_eq!(cog.base_increment(), 10);
        assert!(cog.is_valid());
    }

    #[test]
    fn test_cognitive_info_threshold() {
        let cog = CognitiveInfo::new(20, 5);
        assert!(cog.exceeds_threshold(15));
        assert!(!cog.exceeds_threshold(25));
    }

    #[test]
    fn test_cognitive_info_serialization() {
        let cog = CognitiveInfo::new(10, 3);
        let json = serde_json::to_string(&cog).unwrap();
        let parsed: CognitiveInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(cog.score, parsed.score);
        assert_eq!(cog.nesting_penalty, parsed.nesting_penalty);
    }

    // -------------------------------------------------------------------------
    // HalsteadInfo Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_halstead_from_counts() {
        let hal = HalsteadInfo::from_counts(10, 20, 50, 100);
        assert_eq!(hal.vocabulary, 30); // n1 + n2
        assert_eq!(hal.length, 150); // N1 + N2
        assert!(hal.is_valid());
    }

    #[test]
    fn test_halstead_empty_function() {
        let hal = HalsteadInfo::from_counts(0, 0, 0, 0);
        assert_eq!(hal.volume, 1.0); // Avoid log(0)
        assert!(hal.is_valid());
    }

    #[test]
    fn test_halstead_n2_zero_caps_difficulty() {
        let hal = HalsteadInfo::from_counts(10, 0, 50, 100);
        assert_eq!(hal.difficulty, 1000.0); // Capped
    }

    #[test]
    fn test_halstead_derived_metrics() {
        let hal = HalsteadInfo::from_counts(10, 20, 50, 100);
        // effort = difficulty * volume
        let expected_effort = hal.difficulty * hal.volume;
        assert!((hal.effort - expected_effort).abs() < 0.001);
        // time = effort / 18
        assert!((hal.time - hal.effort / 18.0).abs() < 0.001);
        // bugs = volume / 3000
        assert!((hal.bugs - hal.volume / 3000.0).abs() < 0.001);
    }

    #[test]
    fn test_halstead_serialization() {
        let hal = HalsteadInfo::from_counts(5, 10, 25, 50);
        let json = serde_json::to_string(&hal).unwrap();
        assert!(json.contains("\"N1\""));
        assert!(json.contains("\"N2\""));
        let parsed: HalsteadInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(hal.vocabulary, parsed.vocabulary);
    }

    // -------------------------------------------------------------------------
    // HotspotInfo Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_hotspot_info_new() {
        let hot = HotspotInfo::new(PathBuf::from("src/main.rs"), None, 0.8, 0.6);
        assert!((hot.hotspot_score - 0.48).abs() < 0.001); // 0.8 * 0.6
        assert!(hot.is_valid());
    }

    #[test]
    fn test_hotspot_info_with_function() {
        let hot = HotspotInfo::new(
            PathBuf::from("src/lib.rs"),
            Some("process_data".to_string()),
            0.5,
            0.5,
        );
        assert_eq!(hot.function, Some("process_data".to_string()));
        assert!((hot.hotspot_score - 0.25).abs() < 0.001);
    }

    #[test]
    fn test_hotspot_trend_default() {
        let hot = HotspotInfo::default();
        assert_eq!(hot.trend, HotspotTrend::Unknown);
    }

    #[test]
    fn test_hotspot_serialization() {
        let hot = HotspotInfo::new(PathBuf::from("test.py"), None, 0.7, 0.3);
        let json = serde_json::to_string(&hot).unwrap();
        let parsed: HotspotInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(hot.file, parsed.file);
        assert!((hot.hotspot_score - parsed.hotspot_score).abs() < 0.001);
    }

    // -------------------------------------------------------------------------
    // CoverageInfo Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_coverage_from_line_counts() {
        let cov = CoverageInfo::from_line_counts(80, 100, vec![5, 10, 15]);
        assert!((cov.line_coverage - 80.0).abs() < 0.01);
        assert_eq!(cov.uncovered_lines, vec![5, 10, 15]);
        assert!(cov.is_valid());
    }

    #[test]
    fn test_coverage_empty_file() {
        let cov = CoverageInfo::from_line_counts(0, 0, vec![]);
        assert!((cov.line_coverage - 100.0).abs() < 0.01); // 0/0 = 100%
        assert!(cov.is_valid());
    }

    #[test]
    fn test_coverage_with_branches() {
        let mut cov = CoverageInfo::from_line_counts(90, 100, vec![]);
        cov.branch_coverage = Some(75.0);
        cov.function_coverage = Some(95.0);
        assert!(cov.is_valid());
    }

    #[test]
    fn test_coverage_invalid_percentage() {
        let cov = CoverageInfo {
            line_coverage: 150.0, // Invalid
            ..Default::default()
        };
        assert!(!cov.is_valid());
    }

    #[test]
    fn test_coverage_serialization() {
        let cov = CoverageInfo::from_line_counts(85, 100, vec![1, 2, 3]);
        let json = serde_json::to_string(&cov).unwrap();
        let parsed: CoverageInfo = serde_json::from_str(&json).unwrap();
        assert!((cov.line_coverage - parsed.line_coverage).abs() < 0.01);
        assert_eq!(cov.uncovered_lines, parsed.uncovered_lines);
    }

    #[test]
    fn test_coverage_optional_fields_not_serialized_when_none() {
        let cov = CoverageInfo::from_line_counts(50, 100, vec![]);
        let json = serde_json::to_string(&cov).unwrap();
        assert!(!json.contains("branch_coverage"));
        assert!(!json.contains("function_coverage"));
    }
}
