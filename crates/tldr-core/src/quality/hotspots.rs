//! Hotspots analysis combining git churn with cognitive complexity
//!
//! This module identifies high-risk code regions by combining:
//! - Recency-weighted relative churn (per-commit, size-normalized)
//! - Cognitive complexity metrics (hard to understand code)
//! - Knowledge fragmentation (ownership spread across authors)
//!
//! # Algorithm v2 (additive percentile)
//! ```text
//! hotspot_score = w_churn * percentile(relative_churn)
//!              + w_complexity * percentile(cognitive_complexity)
//!              + w_fragmentation * percentile(knowledge_fragmentation)
//!              + w_temporal * percentile(temporal_coupling)  // Phase 2
//! ```
//!
//! Default weights (Phase 1, temporal=0): churn=0.4118, complexity=0.4118, fragmentation=0.1765
//!
//! # Key features
//! - Bot commit filtering (dependabot, renovate, etc.)
//! - Recency weighting via exponential decay (configurable half-life)
//! - Relative churn (normalized by file size) prevents large-file bias
//! - Percentile ranking prevents outlier compression (vs. min-max)
//! - Zero-variance dimensions auto-excluded with weight renormalization
//!
//! # Edge Cases
//! - Single file project: all percentiles = 1.0
//! - All files same churn: dimension excluded, weights renormalized
//! - No git history: empty report with warning
//!
//! # References
//! - [code-maat hotspots](https://github.com/adamtornhill/code-maat)
//! - Session 15 Phase 6 specification
//! - Hotspot upgrade spec: thoughts/hotspot-upgrade/spec.md

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use std::process::Command;

use crate::metrics::cognitive::{analyze_cognitive, CognitiveOptions, FunctionCognitive};
use crate::quality::churn::{
    check_shallow_clone, get_file_churn_detailed, is_git_repository, ChurnError, FileChurn,
    FileChurnDetailed,
};
use crate::types::Language;

#[allow(unused_imports)]
use crate::metrics::cognitive::CognitiveReport;

/// Get the git repository root for a path
fn get_git_root(path: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(path)
        .output()
        .ok()?;

    if output.status.success() {
        let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Some(PathBuf::from(root))
    } else {
        None
    }
}

/// Compute the path prefix for the analysis directory relative to git root
fn get_analysis_prefix(analysis_path: &Path, git_root: &Path) -> Option<String> {
    let canonical_analysis = analysis_path.canonicalize().ok()?;
    let canonical_root = git_root.canonicalize().ok()?;

    canonical_analysis
        .strip_prefix(&canonical_root)
        .ok()
        .map(|p| {
            let s = p.to_string_lossy().to_string();
            // Normalize path separators and ensure no leading/trailing slashes
            s.trim_start_matches('/').trim_end_matches('/').to_string()
        })
}

// =============================================================================
// Data Types
// =============================================================================

/// A single hotspot entry representing a high-risk code region.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HotspotEntry {
    /// File path relative to analysis root
    pub file: String,

    /// Function name (only present when by_function=true)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<String>,

    /// Line number of the function (only when by_function=true)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,

    /// Normalized churn score (0.0 to 1.0)
    pub churn_score: f64,

    /// Normalized complexity score (0.0 to 1.0)
    pub complexity_score: f64,

    /// Combined hotspot score: churn_score * complexity_score
    pub hotspot_score: f64,

    /// Number of commits in the time window
    pub commit_count: u32,

    /// Lines changed (added + deleted)
    pub lines_changed: u32,

    /// Cognitive complexity value
    pub complexity: u32,

    /// Trend direction: "improving", "stable", or "degrading"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trend: Option<TrendInfo>,

    /// Recommendation based on score thresholds
    pub recommendation: String,

    /// Relative churn: lines_changed / max(current_loc, 10). Recency-weighted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relative_churn: Option<f64>,

    /// Knowledge fragmentation score (0.0 = single owner, 1.0 = fully fragmented).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub knowledge_fragmentation: Option<f64>,

    /// Current line count of file on disk.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_loc: Option<u32>,

    /// Number of unique authors.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_count: Option<u32>,

    /// Algorithm version: 1 = multiplicative min-max (legacy), 2 = additive percentile.
    #[serde(default = "default_algorithm_version")]
    pub algorithm_version: u32,
}

/// Default algorithm version for deserialization backwards compatibility.
fn default_algorithm_version() -> u32 {
    1
}

/// Trend information for a hotspot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrendInfo {
    /// Trend direction
    pub direction: TrendDirection,

    /// Complexity delta (positive = getting worse)
    pub complexity_delta: i32,

    /// Period in months for the comparison
    pub period_months: u32,
}

/// Trend direction enum
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrendDirection {
    /// Complexity decreased
    Improving,
    /// Complexity stayed the same (+/- 2)
    Stable,
    /// Complexity increased
    Degrading,
}

/// Documents the weights used in the composite score for transparency.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoringWeights {
    /// Weight for the churn dimension.
    pub churn: f64,
    /// Weight for the complexity dimension.
    pub complexity: f64,
    /// Weight for the knowledge fragmentation dimension.
    pub knowledge_fragmentation: f64,
    /// Weight for the temporal coupling dimension.
    pub temporal_coupling: f64,
}

impl Default for ScoringWeights {
    fn default() -> Self {
        Self {
            churn: 0.35,
            complexity: 0.35,
            knowledge_fragmentation: 0.15,
            temporal_coupling: 0.15,
        }
    }
}

impl ScoringWeights {
    /// Return Phase 1 default weights (temporal_coupling=0, renormalized).
    ///
    /// Phase 1 does not compute temporal coupling, so the weight is 0.0
    /// and the remaining weights are renormalized to sum to 1.0.
    /// Result: churn=0.4118, complexity=0.4118, fragmentation=0.1765.
    pub fn default_phase1() -> Self {
        let base = Self::default();
        let phase1 = Self {
            churn: base.churn,
            complexity: base.complexity,
            knowledge_fragmentation: base.knowledge_fragmentation,
            temporal_coupling: 0.0,
        };
        phase1.renormalize()
    }

    /// Renormalize active weights to sum to 1.0.
    /// Dimensions with weight=0.0 are considered inactive.
    pub fn renormalize(&self) -> Self {
        let sum =
            self.churn + self.complexity + self.knowledge_fragmentation + self.temporal_coupling;
        if sum <= 0.0 {
            return self.clone();
        }
        Self {
            churn: self.churn / sum,
            complexity: self.complexity / sum,
            knowledge_fragmentation: self.knowledge_fragmentation / sum,
            temporal_coupling: self.temporal_coupling / sum,
        }
    }

    /// Return weights with zero-variance dimensions zeroed out,
    /// then renormalized.
    ///
    /// `active_dimensions` is a 4-element array:
    /// `[churn_has_variance, complexity_has_variance, frag_has_variance, temporal_has_variance]`
    pub fn for_active_dimensions(&self, active: [bool; 4]) -> Self {
        let w = Self {
            churn: if active[0] { self.churn } else { 0.0 },
            complexity: if active[1] { self.complexity } else { 0.0 },
            knowledge_fragmentation: if active[2] {
                self.knowledge_fragmentation
            } else {
                0.0
            },
            temporal_coupling: if active[3] {
                self.temporal_coupling
            } else {
                0.0
            },
        };
        w.renormalize()
    }
}

/// Summary statistics for the hotspots analysis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HotspotsSummary {
    /// Total files analyzed
    pub total_files_analyzed: usize,

    /// Total commits in the time window
    pub total_commits: u32,

    /// Time window in days
    pub time_window_days: u32,

    /// Percentage of changes concentrated in top 10% of files
    pub hotspot_concentration: f64,

    /// Overall recommendation
    pub recommendation: String,

    /// Total bot commits filtered across all files.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_bot_commits_filtered: Option<u32>,

    /// Average knowledge fragmentation across hotspots.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_knowledge_fragmentation: Option<f64>,
}

/// Metadata about the analysis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HotspotsMetadata {
    /// Path analyzed
    pub path: String,

    /// Days of history analyzed
    pub days: u32,

    /// Whether function-level analysis was used
    pub by_function: bool,

    /// Minimum commits threshold
    pub min_commits: u32,

    /// Whether repository is a shallow clone
    pub is_shallow: bool,

    /// Shallow clone depth if applicable
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shallow_depth: Option<u32>,

    /// Number of bot commits filtered out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bot_commits_filtered: Option<u32>,

    /// Recency half-life used (None if 0 / no decay).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recency_halflife: Option<u32>,

    /// Scoring weights used in composite formula.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scoring_weights: Option<ScoringWeights>,

    /// Algorithm version: 1 = multiplicative min-max (legacy), 2 = additive percentile.
    #[serde(default)]
    pub algorithm_version: u32,
}

/// Complete hotspots analysis report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HotspotsReport {
    /// Ranked list of hotspots (highest score first)
    pub hotspots: Vec<HotspotEntry>,

    /// Summary statistics
    pub summary: HotspotsSummary,

    /// Analysis metadata
    pub metadata: HotspotsMetadata,

    /// Warning messages
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub warnings: Vec<String>,
}

// =============================================================================
// Options
// =============================================================================

/// Options for hotspots analysis.
#[derive(Debug, Clone)]
pub struct HotspotsOptions {
    /// Days of git history to analyze
    pub days: u32,

    /// Maximum hotspots to return
    pub top: usize,

    /// Minimum commits for a file to be considered
    pub min_commits: u32,

    /// Analyze at function level (default: file level)
    pub by_function: bool,

    /// Include trend analysis
    pub show_trend: bool,

    /// Exclude patterns (glob syntax)
    pub exclude: Vec<String>,

    /// Filter by minimum hotspot score threshold
    pub threshold: Option<f64>,

    /// Since date (ISO format) - alternative to days
    pub since: Option<String>,

    /// Half-life in days for exponential decay weighting.
    /// 0 = no decay (all commits weighted equally, legacy behavior).
    pub recency_halflife: f64,

    /// Whether to include commits from known bot authors.
    /// Default: false (bots filtered out).
    pub include_bots: bool,
}

impl Default for HotspotsOptions {
    fn default() -> Self {
        Self {
            days: 365,
            top: 20,
            min_commits: 3,
            by_function: false,
            show_trend: false,
            exclude: Vec::new(),
            threshold: None,
            since: None,
            recency_halflife: 90.0,
            include_bots: false,
        }
    }
}

impl HotspotsOptions {
    /// Create new options with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set days of history.
    pub fn with_days(mut self, days: u32) -> Self {
        self.days = days;
        self
    }

    /// Set top limit.
    pub fn with_top(mut self, top: usize) -> Self {
        self.top = top;
        self
    }

    /// Set minimum commits threshold.
    pub fn with_min_commits(mut self, min_commits: u32) -> Self {
        self.min_commits = min_commits;
        self
    }

    /// Enable function-level analysis.
    pub fn with_by_function(mut self, by_function: bool) -> Self {
        self.by_function = by_function;
        self
    }

    /// Enable trend analysis.
    pub fn with_show_trend(mut self, show_trend: bool) -> Self {
        self.show_trend = show_trend;
        self
    }

    /// Set exclude patterns.
    pub fn with_exclude(mut self, exclude: Vec<String>) -> Self {
        self.exclude = exclude;
        self
    }

    /// Set minimum score threshold.
    pub fn with_threshold(mut self, threshold: f64) -> Self {
        self.threshold = Some(threshold);
        self
    }

    /// Set since date.
    pub fn with_since(mut self, since: String) -> Self {
        self.since = Some(since);
        self
    }

    /// Set recency half-life in days.
    pub fn with_recency_halflife(mut self, days: f64) -> Self {
        self.recency_halflife = days;
        self
    }

    /// Set whether to include bot commits.
    pub fn with_include_bots(mut self, include_bots: bool) -> Self {
        self.include_bots = include_bots;
        self
    }
}

// =============================================================================
// Error Types
// =============================================================================

/// Errors specific to hotspots analysis.
#[derive(Debug, Error)]
pub enum HotspotsError {
    /// Path does not exist
    #[error("Path not found: {0}")]
    PathNotFound(PathBuf),

    /// Not a git repository
    #[error("Not a git repository: {0}")]
    NotGitRepository(PathBuf),

    /// Churn analysis failed
    #[error("Churn analysis failed: {0}")]
    ChurnError(#[from] ChurnError),

    /// Complexity analysis failed
    #[error("Complexity analysis failed for {file}: {reason}")]
    ComplexityError {
        /// File whose complexity analysis failed.
        file: PathBuf,
        /// Human-readable reason for the analysis failure.
        reason: String,
    },

    /// I/O error
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// No files to analyze
    #[error("No files found to analyze in {0}")]
    NoFilesFound(PathBuf),
}

// =============================================================================
// Core Analysis Function
// =============================================================================

/// Analyze hotspots by combining churn and complexity data.
///
/// # Arguments
/// * `path` - Directory to analyze (must be in a git repository)
/// * `options` - Analysis options
///
/// # Returns
/// * `Ok(HotspotsReport)` - Analysis results
/// * `Err(HotspotsError)` - If analysis fails
///
/// # Algorithm (v2: additive percentile)
/// 1. Get detailed churn data with per-commit breakdowns (bot filtering, recency weighting)
/// 2. Calculate cognitive complexity for each file/function
/// 3. Compute recency-weighted relative churn and knowledge fragmentation per file
/// 4. Percentile-rank each dimension; exclude zero-variance dimensions
/// 5. Compute composite score = weighted sum of percentile ranks
/// 6. Sort by hotspot_score descending, apply threshold, truncate to top N
pub fn analyze_hotspots(
    path: &Path,
    options: &HotspotsOptions,
) -> Result<HotspotsReport, HotspotsError> {
    // Validate path exists
    if !path.exists() {
        return Err(HotspotsError::PathNotFound(path.to_path_buf()));
    }

    // Check if it's a git repository
    if !is_git_repository(path)? {
        return Err(HotspotsError::NotGitRepository(path.to_path_buf()));
    }

    let mut warnings = Vec::new();

    // Check for shallow clone
    let (is_shallow, shallow_depth) = check_shallow_clone(path)?;
    if is_shallow {
        let depth_info = shallow_depth
            .map(|d| format!(" (~{} commits)", d))
            .unwrap_or_default();
        warnings.push(format!(
            "Repository is a shallow clone{}. Churn analysis may be incomplete.",
            depth_info
        ));
    }

    // --- V2: Use get_file_churn_detailed for per-commit data ---
    let (detailed_churn_raw, total_bot_filtered) =
        get_file_churn_detailed(path, options.days, &options.exclude, options.include_bots)?;

    // Get git root and analysis prefix to filter files correctly
    let git_root = get_git_root(path);
    let analysis_prefix = git_root
        .as_ref()
        .and_then(|root| get_analysis_prefix(path, root));

    let detailed_churn =
        remap_detailed_churn_for_analysis(detailed_churn_raw, analysis_prefix.as_deref());

    // Convert to FileChurn map for backward-compatible filtering and complexity analysis
    let file_churn: HashMap<String, FileChurn> = detailed_churn
        .iter()
        .map(|(k, v)| (k.clone(), v.base.clone()))
        .collect();

    // Handle empty churn data
    if file_churn.is_empty() {
        warnings.push("No commits found in the specified time window.".to_string());
        return Ok(build_empty_hotspots_report(
            path,
            options,
            is_shallow,
            shallow_depth,
            total_bot_filtered,
            warnings,
            "No data available for analysis.".to_string(),
        ));
    }

    // Filter files by min_commits
    let filtered_churn: HashMap<String, FileChurn> = file_churn
        .into_iter()
        .filter(|(_, fc)| fc.commit_count >= options.min_commits)
        .collect();

    if filtered_churn.is_empty() {
        warnings.push(format!(
            "No files found with {} or more commits.",
            options.min_commits
        ));
        return Ok(build_empty_hotspots_report(
            path,
            options,
            is_shallow,
            shallow_depth,
            total_bot_filtered,
            warnings,
            "No files meet the minimum commit threshold.".to_string(),
        ));
    }

    let hotspots = if options.by_function {
        analyze_function_level(path, &filtered_churn, &mut warnings)?
    } else {
        analyze_file_level(path, &filtered_churn, &mut warnings)?
    };

    let total_commits: u32 = filtered_churn.values().map(|f| f.commit_count).sum();
    let total_files = filtered_churn.len();
    let scored = score_hotspots_v2(
        path,
        options,
        detailed_churn,
        hotspots,
        &mut warnings,
        total_commits,
        total_files,
    );

    Ok(HotspotsReport {
        hotspots: scored.hotspots,
        summary: HotspotsSummary {
            total_files_analyzed: total_files,
            total_commits,
            time_window_days: options.days,
            hotspot_concentration: scored.hotspot_concentration,
            recommendation: scored.summary_recommendation,
            total_bot_commits_filtered: Some(total_bot_filtered),
            avg_knowledge_fragmentation: Some(scored.avg_frag),
        },
        metadata: HotspotsMetadata {
            path: path.to_string_lossy().to_string(),
            days: options.days,
            by_function: options.by_function,
            min_commits: options.min_commits,
            is_shallow,
            shallow_depth,
            bot_commits_filtered: Some(total_bot_filtered),
            recency_halflife: if options.recency_halflife > 0.0 {
                Some(options.recency_halflife as u32)
            } else {
                None
            },
            scoring_weights: Some(scored.effective_weights.clone()),
            algorithm_version: 2,
        },
        warnings,
    })
}

struct ScoredHotspots {
    hotspots: Vec<HotspotEntry>,
    effective_weights: ScoringWeights,
    hotspot_concentration: f64,
    avg_frag: f64,
    summary_recommendation: String,
}

fn remap_detailed_churn_for_analysis(
    detailed_churn_raw: HashMap<String, FileChurnDetailed>,
    analysis_prefix: Option<&str>,
) -> HashMap<String, FileChurnDetailed> {
    let Some(prefix) = analysis_prefix else {
        return detailed_churn_raw;
    };
    if prefix.is_empty() {
        return detailed_churn_raw;
    }
    detailed_churn_raw
        .into_iter()
        .filter(|(file_path, _)| {
            file_path.starts_with(prefix) || file_path.starts_with(&format!("{}/", prefix))
        })
        .map(|(file_path, fcd)| {
            let relative_path = file_path
                .strip_prefix(prefix)
                .unwrap_or(&file_path)
                .trim_start_matches('/')
                .to_string();
            (relative_path, fcd)
        })
        .collect()
}

fn build_empty_hotspots_report(
    path: &Path,
    options: &HotspotsOptions,
    is_shallow: bool,
    shallow_depth: Option<u32>,
    total_bot_filtered: u32,
    warnings: Vec<String>,
    recommendation: String,
) -> HotspotsReport {
    HotspotsReport {
        hotspots: Vec::new(),
        summary: HotspotsSummary {
            total_files_analyzed: 0,
            total_commits: 0,
            time_window_days: options.days,
            hotspot_concentration: 0.0,
            recommendation,
            total_bot_commits_filtered: Some(total_bot_filtered),
            avg_knowledge_fragmentation: None,
        },
        metadata: HotspotsMetadata {
            path: path.to_string_lossy().to_string(),
            days: options.days,
            by_function: options.by_function,
            min_commits: options.min_commits,
            is_shallow,
            shallow_depth,
            bot_commits_filtered: Some(total_bot_filtered),
            recency_halflife: if options.recency_halflife > 0.0 {
                Some(options.recency_halflife as u32)
            } else {
                None
            },
            scoring_weights: None,
            algorithm_version: 2,
        },
        warnings,
    }
}

fn score_hotspots_v2(
    path: &Path,
    options: &HotspotsOptions,
    detailed_churn: HashMap<String, FileChurnDetailed>,
    mut hotspots: Vec<HotspotEntry>,
    warnings: &mut Vec<String>,
    total_commits: u32,
    total_files: usize,
) -> ScoredHotspots {
    let today = chrono::Utc::now().date_naive();
    let halflife = options.recency_halflife;

    for hotspot in &mut hotspots {
        let full_path = path.join(&hotspot.file);
        let loc = if full_path.exists() {
            std::fs::read_to_string(&full_path)
                .map(|s| s.lines().count() as u32)
                .unwrap_or(0)
        } else {
            0
        };
        hotspot.current_loc = Some(loc);

        if let Some(detail) = detailed_churn.get(&hotspot.file) {
            let weighted_lines: f64 = detail
                .commits
                .iter()
                .map(|c| {
                    let commit_date = chrono::NaiveDate::parse_from_str(
                        &c.date[..10.min(c.date.len())],
                        "%Y-%m-%d",
                    )
                    .unwrap_or(today);
                    let age = (today - commit_date).num_days().max(0) as f64;
                    let weight = recency_weight(age, halflife);
                    weight * (c.lines_added + c.lines_deleted) as f64
                })
                .sum();
            hotspot.relative_churn = Some(weighted_lines / (loc.max(MIN_LOC_FLOOR)) as f64);

            let mut author_counts: HashMap<String, u32> = HashMap::new();
            for c in &detail.commits {
                *author_counts.entry(c.author_email.clone()).or_insert(0) += 1;
            }
            let author_vec: Vec<(String, u32)> = author_counts.into_iter().collect();
            hotspot.knowledge_fragmentation = Some(knowledge_fragmentation(&author_vec));
            hotspot.author_count = Some(detail.base.author_count);
        }
    }

    let churn_values: Vec<f64> = hotspots
        .iter()
        .map(|h| h.relative_churn.unwrap_or(0.0))
        .collect();
    let complexity_values: Vec<f64> = hotspots.iter().map(|h| h.complexity as f64).collect();
    let frag_values: Vec<f64> = hotspots
        .iter()
        .map(|h| h.knowledge_fragmentation.unwrap_or(0.0))
        .collect();

    let churn_has_variance = has_variance(&churn_values);
    let complexity_has_variance = has_variance(&complexity_values);
    let frag_has_variance = has_variance(&frag_values);
    let active = [
        churn_has_variance,
        complexity_has_variance,
        frag_has_variance,
        false,
    ];

    if !churn_has_variance && hotspots.len() > 1 {
        warnings.push(
            "All files have similar churn. This dimension excluded from scoring.".to_string(),
        );
    }
    if !complexity_has_variance && hotspots.len() > 1 {
        warnings.push(
            "All files have similar complexity. This dimension excluded from scoring.".to_string(),
        );
    }
    if !frag_has_variance && hotspots.len() > 1 {
        warnings.push(
            "All files have similar knowledge fragmentation. This dimension excluded from scoring."
                .to_string(),
        );
    }
    if hotspots.len() == 1 {
        warnings.push("Only one file analyzed. Consider expanding the search scope.".to_string());
    }

    let pct_churn = if churn_has_variance {
        percentile_ranks(&churn_values)
    } else {
        vec![0.0; hotspots.len()]
    };
    let pct_complexity = if complexity_has_variance {
        percentile_ranks(&complexity_values)
    } else {
        vec![0.0; hotspots.len()]
    };
    let pct_frag = if frag_has_variance {
        percentile_ranks(&frag_values)
    } else {
        vec![0.0; hotspots.len()]
    };

    let base_weights = ScoringWeights::default_phase1();
    let effective_weights = base_weights.for_active_dimensions(active);

    for (i, hotspot) in hotspots.iter_mut().enumerate() {
        hotspot.churn_score = pct_churn[i];
        hotspot.complexity_score = pct_complexity[i];
        hotspot.hotspot_score = composite_score_weighted(
            pct_churn[i],
            pct_complexity[i],
            pct_frag[i],
            0.0,
            &effective_weights,
        );
        hotspot.recommendation = get_recommendation(hotspot.hotspot_score);
    }

    hotspots.sort_by(|a, b| {
        b.hotspot_score
            .partial_cmp(&a.hotspot_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let top_10_percent = (total_files / 10).max(1);
    let top_commits: u32 = hotspots
        .iter()
        .take(top_10_percent)
        .map(|h| h.commit_count)
        .sum();
    let hotspot_concentration = if total_commits > 0 {
        (top_commits as f64 / total_commits as f64) * 100.0
    } else {
        0.0
    };

    let avg_frag = {
        let frags: Vec<f64> = hotspots
            .iter()
            .filter_map(|h| h.knowledge_fragmentation)
            .collect();
        if frags.is_empty() {
            0.0
        } else {
            frags.iter().sum::<f64>() / frags.len() as f64
        }
    };

    if let Some(threshold) = options.threshold {
        hotspots.retain(|h| h.hotspot_score >= threshold);
    }
    hotspots.truncate(options.top);

    let summary_recommendation = if hotspot_concentration > 70.0 {
        "High concentration of changes in few files. Consider breaking up large modules."
            .to_string()
    } else if hotspot_concentration > 40.0 {
        "Moderate change concentration. Monitor hotspots for potential refactoring.".to_string()
    } else {
        "Changes are well distributed across the codebase.".to_string()
    };

    ScoredHotspots {
        hotspots,
        effective_weights,
        hotspot_concentration,
        avg_frag,
        summary_recommendation,
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Analyze at file level (aggregate complexity per file).
fn analyze_file_level(
    path: &Path,
    churn_data: &HashMap<String, FileChurn>,
    warnings: &mut Vec<String>,
) -> Result<Vec<HotspotEntry>, HotspotsError> {
    let mut hotspots = Vec::new();

    for (file_path, file_churn) in churn_data {
        let full_path = path.join(file_path);

        // Skip if file doesn't exist (deleted in history)
        if !full_path.exists() {
            continue;
        }

        // Skip if language not supported
        if Language::from_path(&full_path).is_none() {
            continue;
        }

        // Calculate max cognitive complexity across all functions
        let cognitive_options = CognitiveOptions::new()
            .with_threshold(1000) // High threshold to get all functions
            .with_high_threshold(10000);

        let max_complexity = match analyze_cognitive(&full_path, &cognitive_options) {
            Ok(report) => report
                .functions
                .iter()
                .map(|f| f.cognitive)
                .max()
                .unwrap_or(0),
            Err(e) => {
                warnings.push(format!(
                    "Complexity analysis failed for {}: {}",
                    file_path, e
                ));
                0
            }
        };

        hotspots.push(HotspotEntry {
            file: file_path.clone(),
            function: None,
            line: None,
            churn_score: 0.0, // Will be normalized later
            complexity_score: 0.0,
            hotspot_score: 0.0,
            commit_count: file_churn.commit_count,
            lines_changed: file_churn.lines_changed,
            complexity: max_complexity,
            trend: None,
            recommendation: String::new(),
            relative_churn: None,
            knowledge_fragmentation: None,
            current_loc: None,
            author_count: None,
            algorithm_version: 2,
        });
    }

    Ok(hotspots)
}

/// Analyze at function level (individual function hotspots).
fn analyze_function_level(
    path: &Path,
    churn_data: &HashMap<String, FileChurn>,
    warnings: &mut Vec<String>,
) -> Result<Vec<HotspotEntry>, HotspotsError> {
    let mut hotspots = Vec::new();

    for (file_path, file_churn) in churn_data {
        let full_path = path.join(file_path);

        // Skip if file doesn't exist (deleted in history)
        if !full_path.exists() {
            continue;
        }

        // Skip if language not supported
        if Language::from_path(&full_path).is_none() {
            continue;
        }

        // Get cognitive complexity for all functions
        let cognitive_options = CognitiveOptions::new()
            .with_threshold(1000)
            .with_high_threshold(10000);

        let functions: Vec<FunctionCognitive> =
            match analyze_cognitive(&full_path, &cognitive_options) {
                Ok(report) => report.functions,
                Err(e) => {
                    warnings.push(format!(
                        "Complexity analysis failed for {}: {}",
                        file_path, e
                    ));
                    continue;
                }
            };

        // Create hotspot entry for each function
        for func in functions {
            hotspots.push(HotspotEntry {
                file: file_path.clone(),
                function: Some(func.name),
                line: Some(func.line),
                churn_score: 0.0,
                complexity_score: 0.0,
                hotspot_score: 0.0,
                commit_count: file_churn.commit_count, // File-level churn (limitation)
                lines_changed: file_churn.lines_changed,
                complexity: func.cognitive,
                trend: None,
                recommendation: String::new(),
                relative_churn: None,
                knowledge_fragmentation: None,
                current_loc: None,
                author_count: None,
                algorithm_version: 2,
            });
        }
    }

    Ok(hotspots)
}

/// Normalize a value to 0.0-1.0 range using min-max scaling.
pub fn normalize_value(value: f64, min: f64, max: f64) -> f64 {
    if (max - min).abs() < f64::EPSILON {
        // Uniform distribution - use 0.5 as fallback
        return 0.5;
    }
    ((value - min) / (max - min)).clamp(0.0, 1.0)
}

/// Get recommendation text based on hotspot score.
///
/// Thresholds recalibrated for v2 additive-percentile algorithm (RISK-A8):
/// - Critical > 0.74
/// - High > 0.63
/// - Medium > 0.50
/// - Monitor otherwise
fn get_recommendation(score: f64) -> String {
    if score > 0.74 {
        "Critical: High churn + high complexity + fragmented knowledge. Prioritize refactoring."
            .to_string()
    } else if score > 0.63 {
        "High priority: Frequent changes to complex code.".to_string()
    } else if score > 0.50 {
        "Medium priority: Consider simplification.".to_string()
    } else {
        "Monitor for changes.".to_string()
    }
}

/// Calculate trend direction based on complexity delta.
pub fn calculate_trend(complexity_delta: i32) -> TrendDirection {
    if complexity_delta < -2 {
        TrendDirection::Improving
    } else if complexity_delta > 2 {
        TrendDirection::Degrading
    } else {
        TrendDirection::Stable
    }
}

// =============================================================================
// New Algorithm Functions (v2: additive percentile)
// =============================================================================

// Bot patterns are defined in churn.rs; is_bot_author is imported via the use statement above.

/// Minimum LOC floor for relative churn denominator.
/// Files smaller than this are treated as this size to prevent
/// tiny files from dominating hotspot rankings.
const MIN_LOC_FLOOR: u32 = 10;

/// Compute percentile ranks for a slice of values.
///
/// Returns a `Vec<f64>` of percentile ranks in `[0.0, 1.0]`, same order as input.
/// Uses the average-rank method for ties: tied values get the mean of their
/// positional ranks. Formula: `(average_rank - 1) / (N - 1)` for N >= 2.
/// For N = 1, returns `[1.0]`. For N = 0, returns `[]`.
///
/// This produces the range `[0.0, 1.0]` where the lowest value maps to 0.0
/// and the highest maps to 1.0, which is essential for calibrated thresholds.
pub fn percentile_ranks(values: &[f64]) -> Vec<f64> {
    let n = values.len();
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![1.0];
    }

    // Build (original_index, value) pairs, sort by value
    let mut indexed: Vec<(usize, f64)> = values.iter().copied().enumerate().collect();
    indexed.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    // Assign average ranks for tied groups
    let mut ranks = vec![0.0_f64; n];
    let mut i = 0;
    while i < n {
        let mut j = i;
        while j < n && (indexed[j].1 - indexed[i].1).abs() < f64::EPSILON {
            j += 1;
        }
        // Positions i..j share the same value
        // Positional ranks are (i+1) through j (1-based)
        let avg_rank = (i + 1 + j) as f64 / 2.0;
        for k in i..j {
            ranks[indexed[k].0] = avg_rank;
        }
        i = j;
    }

    // Convert to percentile: (rank - 1) / (N - 1)
    let denom = (n - 1) as f64;
    ranks.iter().map(|&r| (r - 1.0) / denom).collect()
}

/// Compute recency weight for a commit given its age.
///
/// `weight = e^(-lambda * age_in_days)` where `lambda = ln(2) / halflife_days`.
/// When `halflife_days <= 0`, returns 1.0 (no decay, legacy behavior).
/// Clamps age to >= 0 to handle future-dated commits.
pub fn recency_weight(age_days: f64, halflife_days: f64) -> f64 {
    if halflife_days <= 0.0 {
        return 1.0; // RISK-A13: guard against division by zero
    }
    let clamped_age = age_days.max(0.0);
    let lambda = (2.0_f64).ln() / halflife_days;
    (-lambda * clamped_age).exp()
}

/// Compute relative churn for a file.
///
/// `relative_churn = lines_changed / max(current_loc, MIN_LOC_FLOOR)`.
/// Uses `MIN_LOC_FLOOR=10` to prevent tiny/empty files from dominating rankings
/// (RISK-C6 mitigation).
pub fn relative_churn(lines_changed: u32, current_loc: u32) -> f64 {
    lines_changed as f64 / (current_loc.max(MIN_LOC_FLOOR)) as f64
}

// is_bot_author is defined in churn.rs and re-exported below.
pub use crate::quality::churn::is_bot_author;

/// Compute knowledge fragmentation for a file.
///
/// Arguments:
///   `author_commits`: slice of (author_email, commit_count) pairs
///
/// Returns fragmentation score in `[0.0, 1.0]`.
/// `0.0` = single owner, `1.0` = fully fragmented.
///
/// Formula:
///   `top_fraction = max_commits / total_commits`
///   `fragmentation = 1.0 - top_fraction`
///   if minor_contributors (< 5% of total) > 3:
///     `fragmentation = min(1.0, fragmentation * 1.2)`
pub fn knowledge_fragmentation(author_commits: &[(String, u32)]) -> f64 {
    if author_commits.is_empty() {
        return 0.0;
    }
    let total: u32 = author_commits.iter().map(|(_, c)| c).sum();
    if total == 0 {
        return 0.0;
    }
    let max_commits = author_commits.iter().map(|(_, c)| *c).max().unwrap_or(0);
    let top_fraction = max_commits as f64 / total as f64;
    let mut frag = 1.0 - top_fraction;

    // Penalty for many minor contributors (< 5% of total commits)
    let threshold = ((total as f64 * 0.05) as u32).max(1);
    let minor_count = author_commits
        .iter()
        .filter(|(_, c)| *c < threshold)
        .count();
    if minor_count > 3 {
        frag = (frag * 1.2).min(1.0);
    }
    frag
}

/// Compute the composite hotspot score from percentile-ranked dimensions.
///
/// Uses the provided weights (should be renormalized to sum to 1.0).
pub fn composite_score_weighted(
    pct_churn: f64,
    pct_complexity: f64,
    pct_fragmentation: f64,
    pct_temporal_coupling: f64,
    weights: &ScoringWeights,
) -> f64 {
    weights.churn * pct_churn
        + weights.complexity * pct_complexity
        + weights.knowledge_fragmentation * pct_fragmentation
        + weights.temporal_coupling * pct_temporal_coupling
}

/// Check if a dimension has variance (not all values identical).
///
/// Returns false if the slice has fewer than 2 elements or all values
/// are equal within `f64::EPSILON`.
pub fn has_variance(values: &[f64]) -> bool {
    if values.len() < 2 {
        return false;
    }
    let first = values[0];
    values.iter().any(|&v| (v - first).abs() > f64::EPSILON)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_value() {
        // Normal case
        assert!((normalize_value(50.0, 0.0, 100.0) - 0.5).abs() < 0.001);
        assert!((normalize_value(0.0, 0.0, 100.0) - 0.0).abs() < 0.001);
        assert!((normalize_value(100.0, 0.0, 100.0) - 1.0).abs() < 0.001);

        // Edge case: min == max (uniform distribution)
        assert!((normalize_value(50.0, 50.0, 50.0) - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_calculate_trend() {
        assert_eq!(calculate_trend(-5), TrendDirection::Improving);
        assert_eq!(calculate_trend(0), TrendDirection::Stable);
        assert_eq!(calculate_trend(2), TrendDirection::Stable);
        assert_eq!(calculate_trend(5), TrendDirection::Degrading);
    }

    #[test]
    fn test_get_recommendation() {
        // v2 thresholds: Critical > 0.74, High > 0.63, Medium > 0.50
        assert!(get_recommendation(0.8).contains("Critical"));
        assert!(get_recommendation(0.75).contains("Critical"));
        assert!(get_recommendation(0.7).contains("High priority"));
        assert!(get_recommendation(0.64).contains("High priority"));
        assert!(get_recommendation(0.55).contains("Medium priority"));
        assert!(get_recommendation(0.51).contains("Medium priority"));
        assert!(get_recommendation(0.4).contains("Monitor"));
        assert!(get_recommendation(0.2).contains("Monitor"));
    }

    #[test]
    fn test_options_builder() {
        let opts = HotspotsOptions::new()
            .with_days(30)
            .with_top(10)
            .with_min_commits(5)
            .with_by_function(true);

        assert_eq!(opts.days, 30);
        assert_eq!(opts.top, 10);
        assert_eq!(opts.min_commits, 5);
        assert!(opts.by_function);
    }
}
